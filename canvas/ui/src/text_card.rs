//! A draggable, resizable sticky note with an always-editable text input.
use auro::gpui::{
    App, Bounds, Context, CursorStyle, DragMoveEvent, Entity, EntityId, MouseButton, MouseDownEvent,
    Pixels, Point, Render, ScrollWheelEvent, Size, WeakEntity, Window, div, px, rgb,
};
use auro::prelude::*;
use canvas_types::WorkspaceId;
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

/// A canvas card that owns one multi-line text input. No PTY, no persistence.
pub struct TextCard {
    pub id: u64,
    pub workspace_id: WorkspaceId,
    pub bounds: Bounds<Pixels>,
    input: Entity<InputState>,
    canvas: WeakEntity<Canvas>,
    drag: Option<DragStart>,
    view_zoom: f32,
    view_offset_x: f32,
    view_offset_y: f32,
}

impl TextCard {
    pub fn new(
        id: u64,
        workspace_id: WorkspaceId,
        bounds: Bounds<Pixels>,
        canvas: WeakEntity<Canvas>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("Escreva aqui...")
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
        }
    }

    /// Apply the canvas zoom/pan to this card's position and size.
    pub fn set_view(&mut self, zoom: f32, offset_x: f32, offset_y: f32, cx: &mut Context<Self>) {
        self.view_zoom = zoom;
        self.view_offset_x = offset_x;
        self.view_offset_y = offset_y;
        cx.notify();
    }

    /// Focus the note's input so placement drops the caret straight into it.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus_input(window, cx);
    }

    fn focus_input(&self, window: &mut Window, cx: &mut App) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
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
            DragMode::Move => Bounds::new(
                Point::new(
                    px((f32::from(origin.x) + dx).max(SPACE_NORMAL)),
                    px((f32::from(origin.y) + dy).max(SPACE_NORMAL)),
                ),
                size,
            ),
            DragMode::ResizeBottomRight => Bounds::new(
                origin,
                Size::new(
                    px((f32::from(size.width) + dx).max(TEXT_CARD_WIDTH / 2.0)),
                    px((f32::from(size.height) + dy).max(TEXT_CARD_HEIGHT / 2.0)),
                ),
            ),
        };
        self.bounds = next;
        cx.notify(entity_id);
    }

}

impl Render for TextCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let id = self.id;
        let workspace_id = self.workspace_id;
        let canvas = self.canvas.clone();
        let scroll_canvas = self.canvas.clone();
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
                cx.listener(move |this, _, window, cx| this.focus_input(window, cx)),
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
            .border_1()
            .border_color(rgb(COLOR_BORDER))
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
                            .child(format!("Bloco de texto {id}")),
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
                            this.focus_input(window, cx);
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
                    .cursor(CursorStyle::ResizeUpRightDownLeft)
                    .id(("text-card-resize", id))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.focus_input(window, cx);
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
