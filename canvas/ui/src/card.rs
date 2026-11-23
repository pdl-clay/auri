//! A draggable, resizable frame around one PTY-backed terminal.
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use auro::gpui::{
    App, Bounds, Context, CursorStyle, Div, DragMoveEvent, Edges, Entity, EntityId, MouseButton,
    MouseDownEvent, Pixels, Point, Render, ScrollDelta, ScrollWheelEvent, Size, Stateful,
    WeakEntity, Window, div, px, rgb, svg,
};
use auro::prelude::*;
use portable_pty::PtySize;
use gpui_terminal::{TerminalConfig, TerminalView};
use canvas_types::{AgentIcon, AgentKind, ConnectionTarget, TerminalId, WorkspaceId};

use crate::canvas::{Canvas, card_bounds};
use crate::pty::{BatchedReader, PtySession};
use crate::theme::*;

/// One PTY input stream shared by the terminal view and the host's `ask`
/// dispatch. The id is workspace-local, so callers key it with both ids.
pub type SharedPtyWriter = Arc<Mutex<Box<dyn Write + Send>>>;

fn lock_writer(writer: &SharedPtyWriter) -> MutexGuard<'_, Box<dyn Write + Send>> {
    writer.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Queue exact bytes for a PTY through a host-held handle.
pub(crate) fn write_shared(writer: &SharedPtyWriter, bytes: &[u8]) -> io::Result<()> {
    let mut writer = lock_writer(writer);
    writer.write_all(bytes)?;
    writer.flush()
}

/// Adapt the shared stream into another `Write` handle for the terminal view.
struct SharedWriter(SharedPtyWriter);

impl Write for SharedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        lock_writer(&self.0).write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        lock_writer(&self.0).flush()
    }
}

/// At most one terminal repaint per display frame while dragging.
const DRAG_NOTIFY_INTERVAL: Duration = Duration::from_millis(16);
/// Columns and rows that fit a monospace grid in the given pixel rectangle.
pub fn grid_size(width_px: f32, height_px: f32, font_px: f32, line_height: f32) -> (u16, u16) {
    let cell_width = 0.6 * font_px;
    let cell_height = font_px * line_height;
    let cols = (width_px / cell_width).floor().max(1.0) as u16;
    let rows = (height_px / cell_height).floor().max(1.0) as u16;
    (cols, rows)
}

pub(crate) fn terminal_grid(bounds: &Bounds<Pixels>) -> (u16, u16) {
    let content_height = (f32::from(bounds.size.height) - TITLE_HEIGHT - PATH_HEIGHT).max(1.0);
    grid_size(f32::from(bounds.size.width), content_height, FONT_TERMINAL, LINE_HEIGHT_TERMINAL)
}

/// Keep a dragged edge inside its minimum size and the canvas origin.
fn resize_axis(
    origin: f32,
    size: f32,
    delta: f32,
    min_size: f32,
    min_origin: f32,
    from_start: bool,
) -> (f32, f32) {
    if from_start {
        let max_step = size - min_size;
        let min_step = min_origin - origin;
        // When the origin is already left of `min_origin`, the two limits cannot
        // both hold: preserve the minimum size and let pan handle the origin.
        let step = if min_step > max_step {
            delta.min(max_step)
        } else {
            delta.clamp(min_step, max_step)
        };
        (origin + step, size - step)
    } else {
        (origin, (size + delta).max(min_size))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
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
    agent: Option<(AgentKind, String, AgentIcon)>,
    number: usize,
    pub selected: bool,
    /// Connection gesture: crosshair targeting and source highlight.
    connect_mode: bool,
    connect_source: bool,
    pub terminal: Entity<TerminalView>,
    writer: SharedPtyWriter,
    canvas: WeakEntity<Canvas>,
    root: PathBuf,
    /// Owns the child process group; dropping it terminates the session.
    _pty: PtySession,
    drag: Option<DragStart>,
    last_drag_notify: Option<Instant>,
    view_zoom: f32,
    view_offset_x: f32,
    view_offset_y: f32,
}

impl TerminalCard {
    pub fn new(
        id: TerminalId,
        workspace_id: WorkspaceId,
        bounds: Bounds<Pixels>,
        canvas: WeakEntity<Canvas>,
        root: PathBuf,
        mut pty: PtySession,
        grid: (u16, u16),
        cx: &mut Context<Self>,
    ) -> Self {
        let (cols, rows) = grid;
        let resize_master = pty.master.clone();
        // PtySession implements Drop, so its streams cannot be moved out directly.
        let writer: SharedPtyWriter = Arc::new(Mutex::new(std::mem::replace(
            &mut pty.writer,
            Box::new(std::io::sink()),
        )));
        let reader = BatchedReader::new(std::mem::replace(
            &mut pty.reader,
            Box::new(std::io::empty()),
        ));
        let this = cx.weak_entity();
        let exit_canvas = canvas.clone();
        let exit_workspace = workspace_id;
        let view_writer = writer.clone();

        let terminal = cx.new(move |cx| {
            let config = TerminalConfig {
                cols: cols as usize,
                rows: rows as usize,
                font_size: px(FONT_TERMINAL),
                font_family: "DejaVu Sans Mono".into(),
                line_height_multiplier: LINE_HEIGHT_TERMINAL,
                padding: Edges::all(px(10.0)),
                ..Default::default()
            };
            TerminalView::new(SharedWriter(view_writer), reader, config, cx)
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
            agent: None,
            number: 1,
            selected: false,
            terminal,
            connect_mode: false,
            connect_source: false,
            writer,
            canvas,
            _pty: pty,
            last_drag_notify: None,
            drag: None,
            root,
            view_zoom: 1.0,
            view_offset_x: 0.0,
            view_offset_y: 0.0,
        }
    }

    /// Shared handle to this card's PTY input, keyed by workspace and id.
    pub fn writer(&self) -> SharedPtyWriter {
        self.writer.clone()
    }

    pub fn write_input(&self, bytes: &[u8]) -> io::Result<()> {
        write_shared(&self.writer, bytes)
    }

    /// Visible screen rows, top to bottom, for `auri check`.
    pub fn visible_lines(&self, cx: &App) -> Vec<String> {
        self.terminal.read(cx).visible_lines()
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
    /// Connection mode turns a click into edge completion and marks the source.
    pub fn set_connect_mode(&mut self, connecting: bool, source: bool, cx: &mut Context<Self>) {
        if self.connect_mode != connecting || self.connect_source != source {
            self.connect_mode = connecting;
            self.connect_source = source;
            cx.notify();
        }
    }

    /// Attach or replace the agent shown in the card header.
    pub fn set_agent(
        &mut self,
        kind: AgentKind,
        name: String,
        icon: AgentIcon,
        cx: &mut Context<Self>,
    ) {
        self.agent = Some((kind, name, icon));
        cx.notify();
    }

    /// Display number is the card's current position among its type in the workspace.
    pub fn set_number(&mut self, number: usize, cx: &mut Context<Self>) {
        if self.number != number {
            self.number = number;
            cx.notify();
        }
    }

    /// Refresh the header after the workspace renumbers its agent cards.
    pub fn set_agent_name(&mut self, name: String, cx: &mut Context<Self>) {
        if let Some((_, current, _)) = self.agent.as_mut() {
            if *current != name {
                *current = name;
                cx.notify();
            }
        }
    }

    pub fn agent(&self) -> Option<(AgentKind, String, AgentIcon)> {
        self.agent.clone()
    }

    /// Open the agent edit menu at the event's own position, never a synthetic one.
    fn open_agent_menu_at(&self, position: Point<Pixels>, cx: &mut App) {
        if self.agent.is_none() {
            return;
        }
        let Some(canvas) = self.canvas.upgrade() else {
            return;
        };
        let workspace_id = self.workspace_id;
        let id = self.id;
        let _ = canvas.update(cx, |canvas, cx| {
            canvas.open_agent_menu(workspace_id, id, position, cx)
        });
    }

    /// Apply the canvas zoom/pan to this card and its terminal font.
    pub fn set_view(&mut self, zoom: f32, offset_x: f32, offset_y: f32, cx: &mut Context<Self>) {
        let zoom_changed = (self.view_zoom - zoom).abs() > 0.005;
        self.view_zoom = zoom;
        self.view_offset_x = offset_x;
        self.view_offset_y = offset_y;
        if zoom_changed {
            self.terminal.update(cx, |terminal, cx| {
                let mut config = terminal.config().clone();
                config.font_size = px(FONT_TERMINAL * zoom);
                terminal.update_config(config, cx);
            });
        }
        cx.notify();
    }


    fn begin_drag(&mut self, mode: DragMode, position: Point<Pixels>) {
        self.drag = Some(DragStart {
            mode,
            position,
            bounds: self.bounds,
        });
        self.last_drag_notify = None;
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
        let x = f32::from(bounds.origin.x);
        let y = f32::from(bounds.origin.y);
        let w = f32::from(bounds.size.width);
        let h = f32::from(bounds.size.height);
        let (nx, ny, nw, nh) = match start.mode {
            DragMode::Move => (x + dx, y + dy, w, h),
            DragMode::ResizeLeft => {
                let (nx, nw) = resize_axis(x, w, dx, MIN_TERMINAL_WIDTH, 8.0, true);
                (nx, y, nw, h)
            }
            DragMode::ResizeRight => {
                let (_, nw) = resize_axis(x, w, dx, MIN_TERMINAL_WIDTH, 8.0, false);
                (x, y, nw, h)
            }
            DragMode::ResizeTop => {
                let (ny, nh) = resize_axis(y, h, dy, MIN_TERMINAL_HEIGHT, 8.0, true);
                (x, ny, w, nh)
            }
            DragMode::ResizeBottom => {
                let (_, nh) = resize_axis(y, h, dy, MIN_TERMINAL_HEIGHT, 8.0, false);
                (x, y, w, nh)
            }
            DragMode::ResizeTopLeft => {
                let (nx, nw) = resize_axis(x, w, dx, MIN_TERMINAL_WIDTH, 8.0, true);
                let (ny, nh) = resize_axis(y, h, dy, MIN_TERMINAL_HEIGHT, 8.0, true);
                (nx, ny, nw, nh)
            }
            DragMode::ResizeTopRight => {
                let (_, nw) = resize_axis(x, w, dx, MIN_TERMINAL_WIDTH, 8.0, false);
                let (ny, nh) = resize_axis(y, h, dy, MIN_TERMINAL_HEIGHT, 8.0, true);
                (x, ny, nw, nh)
            }
            DragMode::ResizeBottomLeft => {
                let (nx, nw) = resize_axis(x, w, dx, MIN_TERMINAL_WIDTH, 8.0, true);
                let (_, nh) = resize_axis(y, h, dy, MIN_TERMINAL_HEIGHT, 8.0, false);
                (nx, y, nw, nh)
            }
            DragMode::ResizeBottomRight => {
                let (_, nw) = resize_axis(x, w, dx, MIN_TERMINAL_WIDTH, 8.0, false);
                let (_, nh) = resize_axis(y, h, dy, MIN_TERMINAL_HEIGHT, 8.0, false);
                (x, y, nw, nh)
            }
        };
        self.bounds = Bounds::new(Point::new(px(nx), px(ny)), Size::new(px(nw), px(nh)));
        // The terminal view resizes its PTY from its own measured metrics on
        // the next paint; doing it here as well would resize twice per event.
        let now = Instant::now();
        let due = self
            .last_drag_notify
            .map_or(true, |last| now.duration_since(last) >= DRAG_NOTIFY_INTERVAL);
        if due {
            self.last_drag_notify = Some(now);
            cx.notify(entity_id);
        }
    }

    fn resize_handle(
        id: TerminalId,
        mode: DragMode,
        entity_id: EntityId,
        cx: &mut Context<Self>,
        connect_mode: bool,
    ) -> Stateful<Div> {
        let (element_id, cursor): ((&str, u64), CursorStyle) = match mode {
            DragMode::Move => unreachable!("only resize modes have handles"),
            DragMode::ResizeLeft => (("resize-left", id), CursorStyle::ResizeLeftRight),
            DragMode::ResizeRight => (("resize-right", id), CursorStyle::ResizeLeftRight),
            DragMode::ResizeTop => (("resize-top", id), CursorStyle::ResizeUpDown),
            DragMode::ResizeBottom => (("resize-bottom", id), CursorStyle::ResizeUpDown),
            DragMode::ResizeTopLeft => (("resize-top-left", id), CursorStyle::ResizeUpRightDownLeft),
            DragMode::ResizeTopRight => (("resize-top-right", id), CursorStyle::ResizeUpLeftDownRight),
            DragMode::ResizeBottomLeft => (("resize-bottom-left", id), CursorStyle::ResizeUpLeftDownRight),
            DragMode::ResizeBottomRight => (("resize-bottom-right", id), CursorStyle::ResizeUpRightDownLeft),
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
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    if this.connect_mode {
                        let canvas = this.canvas.clone();
                        let _ = canvas.update(cx, |canvas, cx| {
                            canvas.complete_connect(ConnectionTarget::Card(id), cx)
                        });
                        cx.stop_propagation();
                        return;
                    }
                    cx.stop_propagation();
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
    /// End any active move/resize gesture and restore the terminal body.
    pub fn end_drag(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            self.last_drag_notify = None;
            cx.notify();
        }
    }

}

pub(crate) fn scroll_delta(event: &ScrollWheelEvent) -> (f32, f32) {
    match event.delta {
        ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),
        ScrollDelta::Lines(p) => (p.x * 16.0, p.y * 16.0),
    }
}

impl Render for TerminalCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let id = self.id;
        let workspace_id = self.workspace_id;
        let exited = self.exited;
        let selected = self.selected;
        let connect_mode = self.connect_mode;
        let connect_source = self.connect_source;
        let select_canvas = self.canvas.clone();
        let canvas = self.canvas.clone();
        let bounds_canvas = self.canvas.clone();
        let saved_bounds = card_bounds(&self.bounds);
        let entity_id = cx.entity_id();
        let scroll_canvas = self.canvas.clone();
        let zoom = self.view_zoom;
        let agent = self.agent();
        let left = f32::from(self.bounds.origin.x) * zoom + self.view_offset_x;
        let top = f32::from(self.bounds.origin.y) * zoom + self.view_offset_y;
        let width = f32::from(self.bounds.size.width) * zoom;
        let height = f32::from(self.bounds.size.height) * zoom;
        div()
            .id(("card-select", id))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if this.connect_mode {
                        let canvas = this.canvas.clone();
                        let _ = canvas.update(cx, |canvas, cx| {
                            canvas.complete_connect(ConnectionTarget::Card(id), cx)
                        });
                        cx.stop_propagation();
                        return;
                    }
                    let _ =
                        select_canvas.update(cx, |canvas, cx| canvas.select(workspace_id, id, cx));
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_agent_menu_at(event.position, cx);
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.end_drag(cx);
                    let _ = bounds_canvas.update(cx, |canvas, cx| {
                        canvas.commit_card_bounds(workspace_id, id, saved_bounds, cx)
                    });
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| this.end_drag(cx)),
            )
            .on_scroll_wheel(cx.listener(move |_, event: &ScrollWheelEvent, _, cx| {
                let (dx, dy) = scroll_delta(event);
                let _ = scroll_canvas.update(cx, |canvas, cx| {
                    canvas.apply_scroll(event.position, dx, dy, event.modifiers.control, event.modifiers.shift, cx)
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
            .rounded(px(10.0))
            .bg(rgb(COLOR_BASE))
            .when(selected, |card| {
                card.border_2().border_dashed().border_color(rgb(COLOR_SELECTION))
            })
            .when(!selected, |card| {
                card.border_1().border_color(rgb(COLOR_BORDER))
            })
            .when(connect_source, |card| {
                card.border_2().border_color(rgb(CONNECTION_SOURCE_COLOR))
            })
            .when(connect_mode, |card| card.cursor(CursorStyle::Crosshair))
            .child(
                div()
                    .id(("card-title", id))
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(TITLE_HEIGHT))
                    .px_2()
                    .bg(rgb(COLOR_ELEVATED))
                    .text_color(rgb(COLOR_FOREGROUND))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when_some(agent.clone(), |title, (_kind, name, icon)| {
                                title
                                    .child(
                                        svg()
                                            .path(agent_icon_path(icon))
                                            .w(px(ICON_SIZE))
                                            .h(px(ICON_SIZE))
                                            .text_color(rgb(agent_color(icon))),
                                    )
                                    .child(div().text_sm().child(name))
                            })
                            .when(agent.is_none(), |title| {
                                title
                                    .child(
                                        div()
                                            .w(px(8.0))
                                            .h(px(8.0))
                                            .rounded_full()
                                            .bg(if selected {
                                                rgb(COLOR_SELECTION)
                                            } else {
                                                rgb(COLOR_SUBTLE)
                                            }),
                                    )
                                    .child(div().text_sm().child(format!("Terminal {}", self.number)))
                            })
                            .when(exited, |title| {
                                title
                                    .child("encerrado")
                                    .text_xs()
                                    .text_color(rgb(COLOR_DANGER))
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
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            if this.connect_mode {
                                let canvas = this.canvas.clone();
                                let _ = canvas.update(cx, |canvas, cx| {
                                    canvas.complete_connect(ConnectionTarget::Card(id), cx)
                                });
                                cx.stop_propagation();
                                return;
                            }
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
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.open_agent_menu_at(event.position, cx);
                        }),
                    ),
            )
            .child(div().flex_1().min_h_0().child(self.terminal.clone()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(PATH_HEIGHT))
                    .px_2()
                    .overflow_hidden()
                    .bg(rgb(COLOR_PANEL))
                    .text_xs()
                    .text_color(rgb(COLOR_MUTED))
                    .child(self.root.to_string_lossy().into_owned()),
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
    use super::{DragMode, DragToken, grid_size, resize_axis};
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
        assert!(!token.targets(DragMode::ResizeLeft, card_b));
    }

    #[test]
    fn resizing_left_edge_moves_origin_and_keeps_minimum_width() {
        let (origin, size) = resize_axis(100.0, 640.0, -40.0, 320.0, 8.0, true);
        assert_eq!((origin, size), (60.0, 680.0));
        let (origin, size) = resize_axis(100.0, 400.0, -500.0, 320.0, 8.0, true);
        assert_eq!((origin, size), (8.0, 492.0));
        let (_, size) = resize_axis(100.0, 640.0, -40.0, 320.0, 8.0, false);
        assert_eq!(size, 600.0);
    }
    #[test]
    fn resize_axis_tolerates_inverted_constraint_bounds() {
        let (_, size) = resize_axis(-672.0, 935.0, -50.0, 320.0, 8.0, true);
        assert!(size >= 320.0);
    }
}
