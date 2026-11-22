//! A draggable, resizable frame around one PTY-backed terminal.

use auro::gpui::{
    App, Bounds, Context, Div, DragMoveEvent, Edges, Entity, EntityId, MouseButton, MouseDownEvent,
    Pixels, Point, Render, Stateful, WeakEntity, Window, div, px, rgb,
};
use auro::prelude::*;
use canvas_types::{TerminalId, WorkspaceId};
use gpui_terminal::{TerminalConfig, TerminalView};
use portable_pty::PtySize;

use crate::canvas::Canvas;
use crate::pty::PtySession;

const FONT_PX: f32 = 13.0;
const LINE_HEIGHT: f32 = 1.2;
const TITLE_HEIGHT: f32 = 32.0;
const HANDLE: f32 = 12.0;
const MIN_WIDTH: f32 = 320.0;
const MIN_HEIGHT: f32 = 200.0;

/// Columns and rows that fit a monospace grid in the given pixel rectangle.
pub fn grid_size(width_px: f32, height_px: f32, font_px: f32, line_height: f32) -> (u16, u16) {
    let cell_width = 0.6 * font_px;
    let cell_height = font_px * line_height;
    let cols = (width_px / cell_width).floor().max(1.0) as u16;
    let rows = (height_px / cell_height).floor().max(1.0) as u16;
    (cols, rows)
}

pub(crate) fn terminal_grid(bounds: &Bounds<Pixels>) -> (u16, u16) {
    let content_height = (f32::from(bounds.size.height) - TITLE_HEIGHT).max(1.0);
    grid_size(f32::from(bounds.size.width), content_height, FONT_PX, LINE_HEIGHT)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DragMode {
    Move,
    ResizeWidth,
    ResizeHeight,
    ResizeBoth,
}

#[derive(Clone, Copy)]
struct DragToken {
    mode: DragMode,
    card_id: EntityId,
}

impl DragToken {
    fn new(mode: DragMode, card_id: EntityId) -> Self {
        Self { mode, card_id }
    }

    /// GPUI broadcasts drag events to every listener of the same type; only the
    /// card that started the drag may apply it.
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

/// Visual card for a single terminal, owning the PTY and its emulator view.
pub struct TerminalCard {
    pub id: TerminalId,
    pub workspace_id: WorkspaceId,
    pub bounds: Bounds<Pixels>,
    pub exited: bool,
    pub selected: bool,
    pub terminal: Entity<TerminalView>,
    canvas: WeakEntity<Canvas>,
    pty: PtySession,
    drag: Option<DragStart>,
}

impl TerminalCard {
    pub fn new(
        id: TerminalId,
        workspace_id: WorkspaceId,
        bounds: Bounds<Pixels>,
        canvas: WeakEntity<Canvas>,
        mut pty: PtySession,
        grid: (u16, u16),
        cx: &mut Context<Self>,
    ) -> Self {
        let (cols, rows) = grid;
        let resize_master = pty.master.clone();
        // PtySession implements Drop, so its streams cannot be moved out directly.
        let writer = std::mem::replace(&mut pty.writer, Box::new(std::io::sink()));
        let reader = std::mem::replace(&mut pty.reader, Box::new(std::io::empty()));
        let this = cx.weak_entity();
        let exit_canvas = canvas.clone();
        let exit_workspace = workspace_id;

        let terminal = cx.new(move |cx| {
            let config = TerminalConfig {
                cols: cols as usize,
                rows: rows as usize,
                font_size: px(FONT_PX),
                line_height_multiplier: LINE_HEIGHT,
                padding: Edges::all(px(10.0)),
                ..Default::default()
            };
            TerminalView::new(writer, reader, config, cx)
                .with_resize_callback(move |cols, rows| {
                    let size = PtySize {
                        rows: rows as u16,
                        cols: cols as u16,
                        pixel_width: 0,
                        pixel_height: 0,
                    };
                    if let Err(error) = resize_master.lock().resize(size) {
                        eprintln!("terminal {id}: PTY resize failed: {error}");
                    }
                })
                .with_exit_callback(move |_window, cx| {
                    let _ = exit_canvas.update(cx, |canvas, cx| {
                        canvas.mark_exited(exit_workspace, id, cx)
                    });
                    let _ = this.update(cx, |card, cx| card.set_exited(cx));
                })
        });

        Self {
            id,
            workspace_id,
            bounds,
            exited: false,
            selected: false,
            terminal,
            canvas,
            pty,
            drag: None,
        }
    }

    pub fn set_exited(&mut self, cx: &mut Context<Self>) {
        if !self.exited {
            self.exited = true;
            cx.notify();
        }
    }

    pub fn set_selected(&mut self, selected: bool, cx: &mut Context<Self>) {
        if self.selected != selected {
            self.selected = selected;
            cx.notify();
        }
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
        let dx = f32::from(event.event.position.x) - f32::from(start.position.x);
        let dy = f32::from(event.event.position.y) - f32::from(start.position.y);
        match start.mode {
            DragMode::Move => {
                self.bounds.origin = Point::new(
                    px(f32::from(start.bounds.origin.x) + dx),
                    px(f32::from(start.bounds.origin.y) + dy),
                );
            }
            DragMode::ResizeWidth => {
                self.bounds.size.width =
                    px((f32::from(start.bounds.size.width) + dx).max(MIN_WIDTH));
            }
            DragMode::ResizeHeight => {
                self.bounds.size.height =
                    px((f32::from(start.bounds.size.height) + dy).max(MIN_HEIGHT));
            }
            DragMode::ResizeBoth => {
                self.bounds.size.width =
                    px((f32::from(start.bounds.size.width) + dx).max(MIN_WIDTH));
                self.bounds.size.height =
                    px((f32::from(start.bounds.size.height) + dy).max(MIN_HEIGHT));
            }
        }
        if start.mode != DragMode::Move {
            let (cols, rows) = terminal_grid(&self.bounds);
            if let Err(error) = self.pty.resize(cols, rows) {
                eprintln!("terminal {}: PTY resize failed: {error}", self.id);
            }
            self.terminal
                .update(cx, |terminal, _| terminal.resize(cols as usize, rows as usize));
        }
        cx.notify(entity_id);
    }

    fn resize_handle(
        id: TerminalId,
        mode: DragMode,
        entity_id: EntityId,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let element_id: (&str, u64) = match mode {
            DragMode::Move => unreachable!("only resize modes have handles"),
            DragMode::ResizeWidth => ("resize-width", id),
            DragMode::ResizeHeight => ("resize-height", id),
            DragMode::ResizeBoth => ("resize-both", id),
        };
        let mut handle = div().absolute().id(element_id);
        handle = match mode {
            DragMode::Move => unreachable!("only resize modes have handles"),
            DragMode::ResizeWidth => {
                handle.right_0().top(px(TITLE_HEIGHT)).bottom_0().w(px(HANDLE))
            }
            DragMode::ResizeHeight => handle.bottom_0().left_0().right_0().h(px(HANDLE)),
            DragMode::ResizeBoth => handle.bottom_0().right_0().w(px(18.0)).h(px(18.0)),
        };
        if mode == DragMode::ResizeBoth {
            handle = handle.child(
                div()
                    .absolute()
                    .bottom(px(4.0))
                    .right(px(4.0))
                    .w(px(7.0))
                    .h(px(7.0))
                    .rounded_full()
                    .bg(rgb(0x7d90b0)),
            );
        }
        handle
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, _| {
                    this.begin_drag(mode, event.position);
                }),
            )
            .on_drag(DragToken::new(mode, entity_id), |token, _, _, cx| cx.new(|_| *token))
            .on_drag_move(
                cx.listener(move |this, event: &DragMoveEvent<DragToken>, _, cx| {
                    this.apply_drag(event, entity_id, cx);
                }),
            )
    }
}

impl Render for TerminalCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let id = self.id;
        let workspace_id = self.workspace_id;
        let exited = self.exited;
        let selected = self.selected;
        let canvas = self.canvas.clone();
        let entity_id = cx.entity_id();
        let select_canvas = self.canvas.clone();
        div()
            .id(("card-select", id))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, _, cx| {
                    let _ =
                        select_canvas.update(cx, |canvas, cx| canvas.select(workspace_id, id, cx));
                }),
            )
            .absolute()
            .left(self.bounds.origin.x)
            .top(self.bounds.origin.y)
            .w(self.bounds.size.width)
            .h(self.bounds.size.height)
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(10.0))
            .bg(rgb(0x14161b))
            .when(selected, |card| {
                card.border_2().border_dashed().border_color(rgb(0x5b9dff))
            })
            .when(!selected, |card| {
                card.border_1().border_color(rgb(0x2b3547))
            })
            .child(
                div()
                    .id(("card-title", id))
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(TITLE_HEIGHT))
                    .px_2()
                    .bg(rgb(0x1b2130))
                    .text_color(rgb(0xc9d1d9))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(8.0))
                                    .h(px(8.0))
                                    .rounded_full()
                                    .bg(if selected {
                                        rgb(0x5b9dff)
                                    } else {
                                        rgb(0x3a4354)
                                    }),
                            )
                            .child(div().text_sm().child(format!("Terminal {id}")))
                            .when(exited, |title| {
                                title
                                    .child("encerrado")
                                    .text_xs()
                                    .text_color(rgb(0xff8888))
                            }),
                    )
                    .child(
                        div()
                            .id(("close-terminal", id))
                            .px_1()
                            .on_click(cx.listener(move |_, _, _, cx| {
                                let _ = canvas.update(cx, |canvas, cx| {
                                    canvas.remove(workspace_id, id, cx);
                                    cx.notify();
                                });
                            }))
                            .child("×"),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            this.terminal.read(cx).focus_handle().focus(window);
                            this.begin_drag(DragMode::Move, event.position);
                        }),
                    )
                    .on_drag(DragToken::new(DragMode::Move, entity_id), |token, _, _, cx| {
                        cx.new(|_| *token)
                    })
                    .on_drag_move(
                        cx.listener(move |this, event: &DragMoveEvent<DragToken>, _, cx| {
                            this.apply_drag(event, entity_id, cx);
                        }),
                    ),
            )
            .child(div().flex_1().min_h_0().child(self.terminal.clone()))
            .child(Self::resize_handle(id, DragMode::ResizeWidth, entity_id, cx))
            .child(Self::resize_handle(id, DragMode::ResizeHeight, entity_id, cx))
            .child(Self::resize_handle(id, DragMode::ResizeBoth, entity_id, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::{DragMode, DragToken, grid_size};
    use auro::gpui::EntityId;

    #[test]
    fn grid_size_uses_monospace_metrics() {
        assert_eq!(grid_size(480.0, 300.0, 12.0, 1.2), (66, 20));
    }
    #[test]
    fn drag_token_targets_only_its_own_card() {
        let card_a = EntityId::from(1);
        let card_b = EntityId::from(2);
        let token = DragToken::new(DragMode::Move, card_b);
        assert!(token.targets(DragMode::Move, card_b));
        assert!(!token.targets(DragMode::Move, card_a));
        assert!(!token.targets(DragMode::ResizeWidth, card_b));
    }
}
