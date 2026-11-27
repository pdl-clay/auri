//! A draggable, resizable sticky note with an always-editable text input.
use auro::gpui::{
    App, Bounds, Context, CursorStyle, Div, DragMoveEvent, Entity, EntityId, MouseButton,
    MouseDownEvent, Pixels, Point, Render, ScrollWheelEvent, Size, Stateful, Subscription, WeakEntity,
    Window,
    div, px, rgb,
};
use auro::prelude::*;
use canvas_types::{ConnectionTarget, WorkspaceId};
use gpui_component::input::{Input, InputState};

use crate::card::{resize_axis, scroll_delta};
use crate::canvas::Canvas;
use crate::theme::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragMode {
    Move,
    ResizeLeft,
    ResizeRight,
    ResizeTop,
    ResizeBottom,
    ResizeTopLeft,
    ResizeTopRight,
    ResizeBottomLeft,
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
    connect_source: bool,
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

/// Pure geometry for every note handle, shared with the drag tests.
fn resize_note_bounds(start: Bounds<Pixels>, mode: DragMode, dx: f32, dy: f32) -> Bounds<Pixels> {
    let origin = start.origin;
    let size = start.size;
    let x = f32::from(origin.x);
    let y = f32::from(origin.y);
    let w = f32::from(size.width);
    let h = f32::from(size.height);
    let (nx, ny, nw, nh) = match mode {
        DragMode::Move => {
            let moved = move_origin(origin, dx, dy);
            (f32::from(moved.x), f32::from(moved.y), w, h)
        }
        DragMode::ResizeLeft => {
            let (nx, nw) = resize_axis(x, w, dx, MIN_NOTE_WIDTH, true);
            (nx, y, nw, h)
        }
        DragMode::ResizeRight => {
            let (_, nw) = resize_axis(x, w, dx, MIN_NOTE_WIDTH, false);
            (x, y, nw, h)
        }
        DragMode::ResizeTop => {
            let (ny, nh) = resize_axis(y, h, dy, MIN_NOTE_HEIGHT, true);
            (x, ny, w, nh)
        }
        DragMode::ResizeBottom => {
            let (_, nh) = resize_axis(y, h, dy, MIN_NOTE_HEIGHT, false);
            (x, y, w, nh)
        }
        DragMode::ResizeTopLeft => {
            let (nx, nw) = resize_axis(x, w, dx, MIN_NOTE_WIDTH, true);
            let (ny, nh) = resize_axis(y, h, dy, MIN_NOTE_HEIGHT, true);
            (nx, ny, nw, nh)
        }
        DragMode::ResizeTopRight => {
            let (_, nw) = resize_axis(x, w, dx, MIN_NOTE_WIDTH, false);
            let (ny, nh) = resize_axis(y, h, dy, MIN_NOTE_HEIGHT, true);
            (x, ny, nw, nh)
        }
        DragMode::ResizeBottomLeft => {
            let (nx, nw) = resize_axis(x, w, dx, MIN_NOTE_WIDTH, true);
            let (_, nh) = resize_axis(y, h, dy, MIN_NOTE_HEIGHT, false);
            (nx, y, nw, nh)
        }
        DragMode::ResizeBottomRight => {
            let (_, nw) = resize_axis(x, w, dx, MIN_NOTE_WIDTH, false);
            let (_, nh) = resize_axis(y, h, dy, MIN_NOTE_HEIGHT, false);
            (x, y, nw, nh)
        }
    };
    Bounds::new(Point::new(px(nx), px(ny)), Size::new(px(nw), px(nh)))
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
            connect_source: false,
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
    pub fn set_connect_mode(&mut self, connecting: bool, source: bool, cx: &mut Context<Self>) {
        if self.connect_mode != connecting || self.connect_source != source {
            self.connect_mode = connecting;
            self.connect_source = source;
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
        self.bounds = resize_note_bounds(start.bounds, start.mode, dx, dy);
        let canvas = self.canvas.clone();
        cx.defer(move |cx| {
            let _ = canvas.update(cx, |_, cx| cx.notify());
        });
        cx.notify(entity_id);
    }

    fn resize_handle(
        id: u64,
        mode: DragMode,
        entity_id: EntityId,
        cx: &mut Context<Self>,
        connect_mode: bool,
    ) -> Stateful<Div> {
        let (element_id, cursor): ((&str, u64), CursorStyle) = match mode {
            DragMode::Move => unreachable!("only resize modes have handles"),
            DragMode::ResizeLeft => (("note-resize-left", id), CursorStyle::ResizeLeftRight),
            DragMode::ResizeRight => (("note-resize-right", id), CursorStyle::ResizeLeftRight),
            DragMode::ResizeTop => (("note-resize-top", id), CursorStyle::ResizeUpDown),
            DragMode::ResizeBottom => (("note-resize-bottom", id), CursorStyle::ResizeUpDown),
            DragMode::ResizeTopLeft => (("note-resize-top-left", id), CursorStyle::ResizeUpRightDownLeft),
            DragMode::ResizeTopRight => (("note-resize-top-right", id), CursorStyle::ResizeUpLeftDownRight),
            DragMode::ResizeBottomLeft => (("note-resize-bottom-left", id), CursorStyle::ResizeUpLeftDownRight),
            DragMode::ResizeBottomRight => (("note-resize-bottom-right", id), CursorStyle::ResizeUpRightDownLeft),
        };
        let mut handle = div().absolute().id(element_id).cursor(if connect_mode {
            CursorStyle::Crosshair
        } else {
            cursor
        });
        handle = match mode {
            DragMode::Move => unreachable!("only resize modes have handles"),
            DragMode::ResizeLeft => handle.left_0().top(px(TITLE_HEIGHT)).bottom_0().w(px(HANDLE)),
            DragMode::ResizeRight => handle.right_0().top(px(TITLE_HEIGHT)).bottom_0().w(px(HANDLE)),
            DragMode::ResizeTop => handle.top_0().left(px(HANDLE)).right(px(HANDLE)).h(px(HANDLE)),
            DragMode::ResizeBottom => handle.bottom_0().left(px(HANDLE)).right(px(HANDLE)).h(px(HANDLE)),
            DragMode::ResizeTopLeft => handle.left_0().top_0().w(px(18.0)).h(px(18.0)),
            DragMode::ResizeTopRight => handle.right_0().top_0().w(px(18.0)).h(px(18.0)),
            DragMode::ResizeBottomLeft => handle.left_0().bottom_0().w(px(18.0)).h(px(18.0)),
            DragMode::ResizeBottomRight => handle.right_0().bottom_0().w(px(18.0)).h(px(18.0)),
        };
        handle
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    if this.connect_mode {
                        this.complete_connect(cx);
                        return;
                    }
                    this.select_and_focus(window, cx);
                    this.begin_drag(mode, event.position);
                }),
            )
            .on_drag(DragToken::new(mode, entity_id), |token, _, _, cx| cx.new(|_| *token))
            .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<DragToken>, _, cx| {
                this.apply_drag(event, entity_id, cx);
            }))
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
            .debug_selector(|| "note-card".to_string())
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
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    if let Some(canvas) = this.canvas.upgrade() {
                        let _ = canvas.update(cx, |canvas, cx| {
                            canvas.open_connect_menu(
                                this.workspace_id,
                                ConnectionTarget::Note(this.id),
                                event.position,
                                cx,
                            );
                        });
                    }
                }),
            )
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
            .rounded(px(RADIUS_CONTROL * zoom))
            .bg(rgb(COLOR_NOTE_BG))
            .text_size(px(FONT_NOTE_BODY * zoom))
            .when(selected, |card| {
                card.border_2().border_dashed().border_color(rgb(COLOR_SELECTION))
            })
            .when(!selected, |card| {
                card.border_1().border_color(rgb(COLOR_BORDER))
            })
            .when(self.connect_source, |card| card.border_2().border_color(rgb(CONNECTION_SOURCE_COLOR)))
            .when(connect_mode, |card| card.cursor(CursorStyle::Crosshair))
            .text_color(rgb(COLOR_NOTE_FG))
            .child(
                div()
                    .id(("text-card-title", id))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .h(px(TITLE_HEIGHT * zoom))
                    .px(px(SPACE_NORMAL * zoom))
                    .child(
                        div()
                            .text_size(px(FONT_NOTE_TITLE * zoom))
                            .text_color(rgb(COLOR_NOTE_PLACEHOLDER))
                            .child(format!("Bloco de texto {}", self.number)),
                    )
                    .child(
                        div()
                            .id(("close-text-card", id))
                            .w(px(CLOSE_BUTTON_SIZE * zoom))
                            .h(px(CLOSE_BUTTON_SIZE * zoom))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(RADIUS_CHIP * zoom))
                            .text_size(px(CLOSE_ICON_SIZE * zoom))
                            .text_color(rgb(COLOR_NOTE_PLACEHOLDER))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    if this.connect_mode {
                                        this.complete_connect(cx);
                                    }
                                }),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if this.connect_mode {
                                    return;
                                }
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
                    .debug_selector(|| "note-body".to_string())
                    .p(px(TEXT_CARD_PADDING * zoom))
                    .text_size(px(FONT_NOTE_BODY * zoom))
                    .child(
                        Input::new(&self.input)
                            .appearance(false)
                            .bordered(false)
                            .focus_bordered(false)
                            .text_color(rgb(COLOR_NOTE_FG))
                            .text_size(px(FONT_NOTE_BODY * zoom))
                            .when(connect_mode, |input| input.cursor(CursorStyle::Crosshair))
                            .h_full(),
                    ),
            )
            .child(Self::resize_handle(id, DragMode::ResizeLeft, entity_id, cx, connect_mode))
            .child(Self::resize_handle(id, DragMode::ResizeRight, entity_id, cx, connect_mode))
            .child(Self::resize_handle(id, DragMode::ResizeTop, entity_id, cx, connect_mode))
            .child(Self::resize_handle(id, DragMode::ResizeBottom, entity_id, cx, connect_mode))
            .child(Self::resize_handle(id, DragMode::ResizeTopLeft, entity_id, cx, connect_mode))
            .child(Self::resize_handle(id, DragMode::ResizeTopRight, entity_id, cx, connect_mode))
            .child(Self::resize_handle(id, DragMode::ResizeBottomLeft, entity_id, cx, connect_mode))
            .child(Self::resize_handle(id, DragMode::ResizeBottomRight, entity_id, cx, connect_mode))
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

    #[test]
    fn every_note_edge_and_corner_resizes() {
        let start = Bounds::new(Point::new(px(100.0), px(100.0)), Size::new(px(320.0), px(220.0)));
        let check = |mode, dx, dy, expected| {
            let next = resize_note_bounds(start, mode, dx, dy);
            assert_eq!(
                (
                    f32::from(next.origin.x),
                    f32::from(next.origin.y),
                    f32::from(next.size.width),
                    f32::from(next.size.height),
                ),
                expected,
                "{mode:?}"
            );
        };
        check(DragMode::ResizeLeft, -40.0, 0.0, (60.0, 100.0, 360.0, 220.0));
        check(DragMode::ResizeRight, 40.0, 0.0, (100.0, 100.0, 360.0, 220.0));
        check(DragMode::ResizeTop, 0.0, -40.0, (100.0, 60.0, 320.0, 260.0));
        check(DragMode::ResizeBottom, 0.0, 40.0, (100.0, 100.0, 320.0, 260.0));
        check(DragMode::ResizeTopLeft, -40.0, -40.0, (60.0, 60.0, 360.0, 260.0));
        check(DragMode::ResizeTopRight, 40.0, -40.0, (100.0, 60.0, 360.0, 260.0));
        check(DragMode::ResizeBottomLeft, -40.0, 40.0, (60.0, 100.0, 360.0, 260.0));
        check(DragMode::ResizeBottomRight, 40.0, 40.0, (100.0, 100.0, 360.0, 260.0));
    }

    #[test]
    fn note_resize_keeps_minimum_size_without_an_origin_barrier() {
        let start = Bounds::new(Point::new(px(100.0), px(100.0)), Size::new(px(320.0), px(220.0)));
        let next = resize_note_bounds(start, DragMode::ResizeTopLeft, 400.0, 400.0);
        assert_eq!(f32::from(next.size.width), MIN_NOTE_WIDTH);
        assert_eq!(f32::from(next.size.height), MIN_NOTE_HEIGHT);
        assert_eq!(f32::from(next.origin.x), 300.0);
        assert_eq!(f32::from(next.origin.y), 230.0);

        let next = resize_note_bounds(start, DragMode::ResizeTopLeft, -500.0, -500.0);
        assert_eq!(f32::from(next.origin.x), -400.0);
        assert_eq!(f32::from(next.origin.y), -400.0);
    }
}
