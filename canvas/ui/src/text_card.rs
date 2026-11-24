//! A draggable, resizable sticky note with an always-editable text input.
use auro::gpui::{
    App, Bounds, Context, CursorStyle, DragMoveEvent, Entity, EntityId, MouseButton, MouseDownEvent,
    Pixels, Point, Render, ScrollWheelEvent, Size, Subscription, WeakEntity, Window, div, px, rgb,
};
use auro::prelude::*;
use canvas_types::{ConnectionTarget, WorkspaceId};
use gpui_component::input::{Input, InputState};

use crate::card::scroll_delta;
use crate::canvas::Canvas;
use crate::theme::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DragMode {
    Move,
    ResizeBottomRight,
}

/// GPUI broadcasts drag events to every listener of the same type, so the token
/// carries both the mode and the entity that started the drag.
#[derive(Clone, Copy)]
struct DragToken {
    mode: DragMode,
    card_id: EntityId,
}

impl DragToken {
    fn new(mode: DragMode, card_id: EntityId) -> Self {
        Self { mode, card_id }
    }

    fn targets(&self, mode: DragMode, card_id: EntityId) -> bool {
        self.mode == mode && self.card_id == card_id
    }
}

impl Render for DragToken {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[derive(Clone, Copy)]
struct DragStart {
    mode: DragMode,
    position: Point<Pixels>,
    bounds: Bounds<Pixels>,
}

/// A canvas card that owns one multi-line text input. Its text and geometry are
/// mirrored into the persisted store by the canvas.
pub struct TextCard {
    pub id: u64,
    pub workspace_id: WorkspaceId,
    pub bounds: Bounds<Pixels>,
    pub selected: bool,
    connect_mode: bool,
    number: usize,
    input: Entity<InputState>,
    canvas: WeakEntity<Canvas>,
    drag: Option<DragStart>,
    view_zoom: f32,
    view_offset_x: f32,
    view_offset_y: f32,
    _text_subscription: Subscription,
}

/// Notes move like terminal cards: the canvas does not clamp their origin.
fn move_origin(origin: Point<Pixels>, dx: f32, dy: f32) -> Point<Pixels> {
    Point::new(px(f32::from(origin.x) + dx), px(f32::from(origin.y) + dy))
}

impl TextCard {
    pub fn new(
        id: u64,
        workspace_id: WorkspaceId,
        bounds: Bounds<Pixels>,
        text: String,
        canvas: WeakEntity<Canvas>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            let mut input = InputState::new(window, cx)
                .multi_line(true)
                .placeholder("Escreva aqui...");
            input.text_color = Some(rgb(COLOR_NOTE_FG).into());
            if !text.is_empty() {
                input.set_value(text, window, cx);
            }
            input
        });
        let commit_canvas = canvas.clone();
        let text_subscription = cx.observe(&input, move |this: &mut Self, input, cx| {
            let text = input.read(cx).value().to_string();
            let _ = commit_canvas.update(cx, |canvas, cx| {
                canvas.commit_note_text(this.workspace_id, this.id, text, cx)
            });
        });
        Self {
            id,
            workspace_id,
            bounds,
            input,
            canvas,
            drag: None,
            view_zoom: 1.0,
            view_offset_x: 0.0,
            view_offset_y: 0.0,
            _text_subscription: text_subscription,
            selected: false,
            number: 1,
            connect_mode: false,
        }
    }

    /// Apply the canvas zoom/pan to this card's position and size.
    pub fn set_view(&mut self, zoom: f32, offset_x: f32, offset_y: f32, cx: &mut Context<Self>) {
        self.view_zoom = zoom;
        self.view_offset_x = offset_x;
        self.view_offset_y = offset_y;
        cx.notify();
    }

    pub fn set_selected(&mut self, selected: bool, cx: &mut Context<Self>) {
        if self.selected != selected {
            self.selected = selected;
            cx.notify();
        }
    }
    /// In connection mode a note click completes an edge instead of focusing.
    pub fn set_connect_mode(&mut self, connecting: bool, cx: &mut Context<Self>) {
        if self.connect_mode != connecting {
            self.connect_mode = connecting;
            cx.notify();
        }
    }

    /// Display number is the note's current position among workspace notes.
    pub fn set_number(&mut self, number: usize, cx: &mut Context<Self>) {
        if self.number != number {
            self.number = number;
            cx.notify();
        }
    }

    /// Current canvas geometry, used by the canvas-wide commit listener.
    pub fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }

    /// Current text, used when no input notification has fired yet.
    pub fn text(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    /// Replace the note's visible text after a host-side `auri note write`.
    /// The input's observer mirrors the value back into the store unchanged.
    pub fn set_text(&mut self, text: &str, window: &mut Window, cx: &mut App) {
        if self.text(cx) != text {
            self.input.update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        }
    }

    /// Focus the note's input so placement drops the caret straight into it.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus_input(window, cx);
    }

    fn focus_input(&self, window: &mut Window, cx: &mut App) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    /// Select this note on the canvas and focus its input on mouse-down.
    fn select_and_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(canvas) = self.canvas.upgrade() {
            let _ = canvas.update(cx, |canvas, cx| canvas.select_note(self.workspace_id, self.id, cx));
        }
        self.focus_input(window, cx);
    }
    /// Route a target click to the canvas before any note selection or drag.
    fn complete_connect(&self, cx: &mut App) {
        let canvas = self.canvas.clone();
        let id = self.id;
        let _ = canvas.update(cx, |canvas, cx| canvas.complete_connect(ConnectionTarget::Note(id), cx));
    }

    fn begin_drag(&mut self, mode: DragMode, position: Point<Pixels>) {
        self.drag = Some(DragStart {
            mode,
            position,
            bounds: self.bounds,
        });
    }

    fn apply_drag(
        &mut self,
        event: &DragMoveEvent<DragToken>,
        entity_id: EntityId,
        cx: &mut App,
    ) {
        let Some(start) = self.drag else {
            return;
        };
        if !event.drag(cx).targets(start.mode, entity_id) {
            return;
        }
        let zoom = self.view_zoom.max(0.01);
        let dx = (f32::from(event.event.position.x) - f32::from(start.position.x)) / zoom;
        let dy = (f32::from(event.event.position.y) - f32::from(start.position.y)) / zoom;
        let bounds = start.bounds;
        let origin = bounds.origin;
        let size = bounds.size;
        let next = match start.mode {
            DragMode::Move => Bounds::new(move_origin(origin, dx, dy), size),
            DragMode::ResizeBottomRight => Bounds::new(
                origin,
                Size::new(
                    px((f32::from(size.width) + dx).max(TEXT_CARD_WIDTH / 2.0)),
                    px((f32::from(size.height) + dy).max(TEXT_CARD_HEIGHT / 2.0)),
                ),
            ),
        };
        self.bounds = next;
        let canvas = self.canvas.clone();
        cx.defer(move |cx| {
            let _ = canvas.update(cx, |_, cx| cx.notify());
        });
        cx.notify(entity_id);
    }

}

impl Render for TextCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let id = self.id;
        let workspace_id = self.workspace_id;
        let selected = self.selected;
        let connect_mode = self.connect_mode;
        let canvas = self.canvas.clone();
        let scroll_canvas = self.canvas.clone();
        let bounds_canvas = self.canvas.clone();
        let entity_id = cx.entity_id();
        let zoom = self.view_zoom;
        let left = f32::from(self.bounds.origin.x) * zoom + self.view_offset_x;
        let top = f32::from(self.bounds.origin.y) * zoom + self.view_offset_y;
        let width = f32::from(self.bounds.size.width) * zoom;
        let height = f32::from(self.bounds.size.height) * zoom;
        div()
            .id(("text-card", id))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    if this.connect_mode {
                        this.complete_connect(cx);
                        cx.stop_propagation();
                        return;
                    }
                    this.select_and_focus(window, cx);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    let bounds = crate::canvas::card_bounds(&this.bounds);
                    let _ = bounds_canvas.update(cx, |canvas, cx| {
                        canvas.commit_note_bounds(workspace_id, id, bounds, cx)
                    });
                }),
            )
            .on_scroll_wheel(cx.listener(move |_, event: &ScrollWheelEvent, _, cx| {
                let (dx, dy) = scroll_delta(event);
                let _ = scroll_canvas.update(cx, |canvas, cx| {
                    canvas.apply_scroll(
                        event.position,
                        dx,
                        dy,
                        event.modifiers.control,
                        event.modifiers.shift,
                        cx,
                    )
                });
                cx.stop_propagation();
            }))
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(width))
            .h(px(height))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(RADIUS_CONTROL))
            .bg(rgb(COLOR_NOTE_BG))
            .when(selected, |card| {
                card.border_2().border_dashed().border_color(rgb(COLOR_SELECTION))
            })
            .when(!selected, |card| {
                card.border_1().border_color(rgb(COLOR_BORDER))
            })
            .when(connect_mode, |card| card.cursor(CursorStyle::Crosshair))
            .text_color(rgb(COLOR_NOTE_FG))
            .child(
                div()
                    .id(("text-card-title", id))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .h(px(TITLE_HEIGHT))
                    .px_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(COLOR_NOTE_PLACEHOLDER))
                            .child(format!("Bloco de texto {}", self.number)),
                    )
                    .child(
                        div()
                            .id(("close-text-card", id))
                            .px_1()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |_, _, _, cx| {
                                let _ = canvas.update(cx, |canvas, cx| {
                                    canvas.remove_text(workspace_id, id, cx)
                                });
                            }))
                            .child("×"),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            if this.connect_mode {
                                this.complete_connect(cx);
                                return;
                            }
                            this.select_and_focus(window, cx);
                            this.begin_drag(DragMode::Move, event.position);
                        }),
                    )
                    .on_drag(DragToken::new(DragMode::Move, entity_id), |token, _, _, cx| {
                        cx.new(|_| *token)
                    })
                    .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<DragToken>, _, cx| {
                        this.apply_drag(event, entity_id, cx);
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .p(px(TEXT_CARD_PADDING))
                    .child(
                        Input::new(&self.input)
                            .appearance(false)
                            .bordered(false)
                            .focus_bordered(false)
                            .text_color(rgb(COLOR_NOTE_FG))
                            .when(connect_mode, |input| input.cursor(CursorStyle::Crosshair))
                            .h_full(),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .right_0()
                    .bottom_0()
                    .w(px(HANDLE))
                    .h(px(HANDLE))
                    .cursor(if connect_mode { CursorStyle::Crosshair } else { CursorStyle::ResizeUpRightDownLeft })
                    .id(("text-card-resize", id))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            if this.connect_mode {
                                this.complete_connect(cx);
                                return;
                            }
                            this.select_and_focus(window, cx);
                            this.begin_drag(DragMode::ResizeBottomRight, event.position);
                        }),
                    )
                    .on_drag(
                        DragToken::new(DragMode::ResizeBottomRight, entity_id),
                        |token, _, _, cx| cx.new(|_| *token),
                    )
                    .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<DragToken>, _, cx| {
                        this.apply_drag(event, entity_id, cx);
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_origin_allows_the_top_left_corner() {
        let moved = move_origin(Point::new(px(100.0), px(100.0)), -240.0, -180.0);
        assert_eq!((f32::from(moved.x), f32::from(moved.y)), (-140.0, -80.0));
    }
}
