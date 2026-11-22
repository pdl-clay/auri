//! Canvas view: drawer, workspace groups and the lifecycle of PTY-backed cards.

use std::collections::HashMap;
use std::path::PathBuf;

use auro::gpui::{
    Bounds, Context, CursorStyle, Entity, MouseButton, MouseDownEvent, Pixels, Point, Render,
    ScrollWheelEvent, Size, Window, canvas, fill, px, rgb, svg,
};
use auro::prelude::*;
use gpui_component::{
    Disableable,
    button::{Button as ComponentButton, ButtonVariant, ButtonVariants as _},
};
use canvas_types::{CanvasState, TerminalId, WorkspaceIcon, WorkspaceId, WorkspaceState, WorkspaceStore};

use crate::card::{TerminalCard, scroll_delta, terminal_grid};
use crate::workspaces::{DRAWER_WIDTH, DrawerAction, WorkspaceDrawer};
use crate::config::{self, LoadOutcome};
use crate::pty::PtySession;

const GRID_STEP: f32 = 24.0;
const CARD_WIDTH: f32 = 640.0;
const CARD_HEIGHT: f32 = 400.0;

type CardGroups = HashMap<WorkspaceId, Vec<(TerminalId, Entity<TerminalCard>)>>;

/// Terminals are never restored from disk; their runtime state must not survive a save.
fn clear_terminals(store: &mut WorkspaceStore) {
    let ids: Vec<WorkspaceId> = store.workspaces().iter().map(|workspace| workspace.id).collect();
    for id in ids {
        if let Some(workspace) = store.workspace_mut(id) {
            workspace.canvas = CanvasState::new();
        }
    }
}

fn persisted_snapshot(store: &WorkspaceStore) -> WorkspaceStore {
    let mut snapshot = store.clone();
    clear_terminals(&mut snapshot);
    snapshot
}
/// Why no terminal can be spawned in the active workspace.
#[derive(Debug, PartialEq, Eq)]
enum SpawnTargetError {
    NoWorkspace,
    InvalidRoot(PathBuf),
}

impl SpawnTargetError {
    fn message(&self) -> String {
        match self {
            Self::NoWorkspace => "Crie um espaço de trabalho antes de adicionar terminais.".into(),
            Self::InvalidRoot(root) => {
                format!("A raiz {} não está disponível.", root.display())
            }
        }
    }
}

/// Create a workspace, refusing a blank or unusable root.
fn create_workspace(
    store: &mut WorkspaceStore,
    name: String,
    icon: WorkspaceIcon,
    root: PathBuf,
) -> Result<WorkspaceId, String> {
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err("Informe um nome para o espaço.".into());
    }
    if root.as_os_str().is_empty() {
        return Err("Escolha uma raiz para o espaço.".into());
    }
    if !root.is_dir() {
        return Err(format!("A raiz {} não é um diretório.", root.display()));
    }
    Ok(store.add(name, icon, root).id)
}

/// Remove one workspace's state and card group, leaving every other group alone.
fn remove_workspace(
    store: &mut WorkspaceStore,
    groups: &mut CardGroups,
    id: WorkspaceId,
) -> Option<(WorkspaceState, Vec<(TerminalId, Entity<TerminalCard>)>)> {
    let removed = store.remove(id)?;
    let cards = groups.remove(&id).unwrap_or_default();
    Some((removed, cards))
}

/// Allocate a terminal in the active workspace and return its spawn target.
fn active_spawn_target(
    store: &mut WorkspaceStore,
) -> Result<(WorkspaceId, TerminalId, PathBuf), SpawnTargetError> {
    let workspace = store
        .active_mut()
        .ok_or(SpawnTargetError::NoWorkspace)?;
    if workspace.root.as_os_str().is_empty()
        || workspace.invalid_root
        || !workspace.root.is_dir()
    {
        return Err(SpawnTargetError::InvalidRoot(workspace.root.clone()));
    }
    let id = workspace.canvas.add().id;
    Ok((workspace.id, id, workspace.root.clone()))
}

/// Remove a terminal from exactly one workspace; ids are only unique within a workspace.
fn remove_terminal_state(
    store: &mut WorkspaceStore,
    groups: &mut CardGroups,
    workspace_id: WorkspaceId,
    id: TerminalId,
) -> bool {
    let Some(workspace) = store.workspace_mut(workspace_id) else {
        return false;
    };
    if workspace.canvas.remove(id).is_none() {
        return false;
    }
    if let Some(cards) = groups.get_mut(&workspace_id) {
        cards.retain(|(card_id, _)| *card_id != id);
    }
    true
}

/// Owns the workspace store, its drawer and every workspace' terminal cards.
pub struct Canvas {
    store: WorkspaceStore,
    drawer: Entity<WorkspaceDrawer>,
    cards: CardGroups,
    error: Option<String>,
    placing: bool,
    zoom: f32,
    offset_x: f32,
    offset_y: f32,
}
impl Canvas {
    /// `load` is authoritative for the drawer banner; the store was already
    /// loaded by the caller and is pushed back into the drawer explicitly.
    pub fn new(mut store: WorkspaceStore, load: LoadOutcome, cx: &mut Context<Self>) -> Self {
        clear_terminals(&mut store);
        let drawer = cx.new(|_| WorkspaceDrawer::new(load));
        drawer.update(cx, |drawer, _| drawer.set_store(store.clone()));
        Self {
            store,
            drawer,
            cards: HashMap::new(),
            error: None,
            placing: false,
            zoom: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }

    /// Wire drawer actions back to this canvas without creating a strong cycle.
    pub fn bind_actions(&mut self, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let restart_weak = weak.clone();
        self.drawer.update(cx, |drawer, _| {
            drawer.on_action(move |action, cx| {
                if let Some(canvas) = weak.upgrade() {
                    canvas
                        .update(cx, |canvas, cx| canvas.handle_action(action, cx))

                }
            });
            drawer.on_restart(move |store, cx| {
                if let Some(canvas) = restart_weak.upgrade() {
                    canvas
                        .update(cx, |canvas, cx| {
                            canvas.adopt_restarted_store(store.clone(), cx)
                        })

                }
            });
        });
    }

    fn handle_action(&mut self, action: DrawerAction, cx: &mut Context<Self>) {
        match action {
            DrawerAction::Create { name, icon, root } => {
                match create_workspace(&mut self.store, name, icon, root) {
                    Ok(id) => {
                        self.cards.entry(id).or_default();
                        self.finish_store_change(cx);
                    }
                    Err(error) => {
                        self.error = Some(error);
                        cx.notify();
                    }
                }
            }
            DrawerAction::Delete(id) => {
                if let Some((_, dropped)) = remove_workspace(&mut self.store, &mut self.cards, id) {
                    self.finish_store_change(cx);
                    // Dropping the card handles terminates that workspace's shells.
                    drop(dropped);
                }
            }
            DrawerAction::Select(id) => {
                if self.store.active() != Some(id) {
                    self.store.select(id);
                    self.finish_store_change(cx);
                }
            }
        }
    }

    fn finish_store_change(&mut self, cx: &mut Context<Self>) {
        self.error = match config::save(&persisted_snapshot(&self.store)) {
            Ok(()) => None,
            Err(error) => Some(format!("Não foi possível salvar os espaços: {error}")),
        };
        let store = self.store.clone();
        let drawer = self.drawer.clone();
        // The action is emitted from inside a drawer update; defer the resync.
        cx.defer(move |cx| {
            drawer
                .update(cx, |drawer, cx| {
                    drawer.set_store(store);
                    cx.notify();
                })

        });
        cx.notify();
    }

    /// `config::restart` already replaced the file and the drawer's store.
    fn adopt_restarted_store(&mut self, store: WorkspaceStore, cx: &mut Context<Self>) {
        self.store = store;
        self.cards.clear();
        self.error = None;
        cx.notify();
    }

    pub fn apply_scroll(
        &mut self,
        position: Point<Pixels>,
        dx: f32,
        dy: f32,
        control: bool,
        shift: bool,
        cx: &mut Context<Self>,
    ) {
        if control {
            let local_x = f32::from(position.x) - DRAWER_WIDTH;
            let local_y = f32::from(position.y);
            let (zoom, offset_x, offset_y) = zoom_at(
                self.zoom,
                self.offset_x,
                self.offset_y,
                local_x,
                local_y,
                zoom_factor(dy),
            );
            self.zoom = zoom;
            self.offset_x = offset_x;
            self.offset_y = offset_y;
        } else if shift {
            self.offset_x -= dx + dy;
        } else {
            self.offset_y -= dy;
        }
        let (zoom, offset_x, offset_y) = (self.zoom, self.offset_x, self.offset_y);
        let cards: Vec<Entity<TerminalCard>> = self
            .cards
            .values()
            .flat_map(|group| group.iter().map(|(_, card)| card.clone()))
            .collect();
        cx.defer(move |cx| {
            for card in cards {
                card.update(cx, |card, cx| card.set_view(zoom, offset_x, offset_y, cx));
            }
        });
        cx.notify();
    }

    /// Enter placement mode: the next canvas click decides where the terminal opens.
    pub fn begin_placement(&mut self, cx: &mut Context<Self>) {
        self.placing = true;
        self.error = None;
        cx.notify();
    }

    pub fn cancel_placement(&mut self, cx: &mut Context<Self>) {
        self.placing = false;
        cx.notify();
    }

    fn place_terminal(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let local_x = (f32::from(position.x) - DRAWER_WIDTH - self.offset_x) / self.zoom;
        let local_y = (f32::from(position.y) - self.offset_y) / self.zoom;
        let origin = Point::new(
            px((local_x - CARD_WIDTH / 2.0).max(8.0)),
            px((local_y - CARD_HEIGHT / 2.0).max(8.0)),
        );
        self.placing = false;
        self.spawn_terminal(Some(origin), window, cx);
    }

    pub fn add_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.spawn_terminal(None, window, cx);
    }

    fn spawn_terminal(
        &mut self,
        origin: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (workspace_id, id, root) = match active_spawn_target(&mut self.store) {
            Ok(target) => target,
            Err(error) => {
                self.error = Some(error.message());
                cx.notify();
                return;
            }
        };
        let origin = origin.unwrap_or_else(|| {
            let offset = GRID_STEP * (self.cards.get(&workspace_id).map_or(0, Vec::len) % 8) as f32;
            Point::new(px(16.0 + offset), px(16.0 + offset))
        });
        let bounds = Bounds::new(origin, Size::new(px(CARD_WIDTH), px(CARD_HEIGHT)));
        let (cols, rows) = terminal_grid(&bounds);
        let pty = match PtySession::spawn_shell_in(cols, rows, Some(&root)) {
            Ok(pty) => pty,
            Err(error) => {
                // A failed fork must not leave a phantom entry in the state.
                if let Some(workspace) = self.store.workspace_mut(workspace_id) {
                    workspace.canvas.remove(id);
                }
                self.error = Some(format!("Terminal {id} não iniciou: {error}"));
                cx.notify();
                return;
            }
        };
        self.error = None;
        let canvas = cx.entity().downgrade();
        let card = cx.new(|cx| {
            TerminalCard::new(id, workspace_id, bounds, canvas.clone(), root.clone(), pty, (cols, rows), cx)
        });
        card.read(cx).terminal.read(cx).focus_handle().focus(window);
        let (zoom, offset_x, offset_y) = (self.zoom, self.offset_x, self.offset_y);
        card.update(cx, |card, cx| card.set_view(zoom, offset_x, offset_y, cx));
        self.cards.entry(workspace_id).or_default().push((id, card));
        self.sync_card_selection(workspace_id, cx);
        self.sync_drawer(cx);
        cx.notify();
    }

    /// Terminal mutations only change the in-memory snapshot shown by the drawer:
    /// they are deliberately not persisted because shells are not restored.
    fn sync_drawer(&mut self, cx: &mut Context<Self>) {
        let store = self.store.clone();
        let drawer = self.drawer.clone();
        cx.defer(move |cx| {
            drawer
                .update(cx, |drawer, cx| {
                    drawer.set_store(store);
                    cx.notify();
                })

        });
    }

    pub fn remove(&mut self, workspace_id: WorkspaceId, id: TerminalId, cx: &mut Context<Self>) {
        remove_terminal_state(&mut self.store, &mut self.cards, workspace_id, id);
        self.sync_drawer(cx);
        cx.notify();
    }

    pub fn remove_selected(&mut self, cx: &mut Context<Self>) {
        let Some(workspace_id) = self.store.active() else {
            return;
        };
        let Some(id) = self
            .store
            .workspace_mut(workspace_id)
            .and_then(|workspace| workspace.canvas.selected())
        else {
            return;
        };
        self.remove(workspace_id, id, cx);
        cx.notify();
    }

    /// Keep each card's painted selection flag in step with the logical state.
    fn sync_card_selection(&self, workspace_id: WorkspaceId, cx: &mut Context<Self>) {
        let selected = self
            .store
            .workspaces()
            .iter()
            .find(|workspace| workspace.id == workspace_id)
            .and_then(|workspace| workspace.canvas.selected());
        if let Some(cards) = self.cards.get(&workspace_id) {
            for (id, card) in cards {
                card.update(cx, |card, cx| card.set_selected(Some(*id) == selected, cx));
            }
        }
    }


    pub fn select(&mut self, workspace_id: WorkspaceId, id: TerminalId, cx: &mut Context<Self>) {
        let Some(workspace) = self.store.workspace_mut(workspace_id) else {
            return;
        };
        if workspace.canvas.selected() != Some(id) {
            workspace.canvas.select(id);
            cx.notify();
        }
        let _ = workspace;
        // The click handler is invoked while the clicked TerminalCard is already
        // being updated by GPUI; updating it again here would panic. Apply the
        // painted flags once the current dispatch has finished.
        let store = self.store.clone();
        let cards = self.cards.get(&workspace_id).cloned().unwrap_or_default();
        cx.defer(move |cx| {
            let selected = store
                .workspaces()
                .iter()
                .find(|workspace| workspace.id == workspace_id)
                .and_then(|workspace| workspace.canvas.selected());
            for (card_id, card) in cards {
                card.update(cx, |card, cx| card.set_selected(Some(card_id) == selected, cx));
            }
        });
    }

    pub fn mark_exited(&mut self, workspace_id: WorkspaceId, id: TerminalId, cx: &mut Context<Self>) {
        if let Some(workspace) = self.store.workspace_mut(workspace_id) {
            workspace.canvas.mark_exited(id);
        }
        if let Some(card) = self
            .cards
            .get(&workspace_id)
            .and_then(|cards| cards.iter().find(|(card_id, _)| *card_id == id))
        {
            card.1.update(cx, |card, cx| card.set_exited(cx));
        }
        cx.notify();
    }
}

fn canvas_icon(path: &'static str) -> impl IntoElement {
    svg()
        .path(path)
        .w(px(16.0))
        .h(px(16.0))
        .text_color(rgb(0x93a0b5))
}

/// Scroll up shrinks the canvas; scroll down enlarges it.
fn zoom_factor(dy: f32) -> f32 {
    (1.0 + dy * 0.0015).clamp(0.8, 1.25)
}

/// Zoom around a canvas-local point so the world position under the cursor stays put.
fn zoom_at(
    zoom: f32,
    offset_x: f32,
    offset_y: f32,
    local_x: f32,
    local_y: f32,
    factor: f32,
) -> (f32, f32, f32) {
    let next = (zoom * factor).clamp(0.25, 3.0);
    let world_x = (local_x - offset_x) / zoom;
    let world_y = (local_y - offset_y) / zoom;
    (next, local_x - world_x * next, local_y - world_y * next)
}

impl Render for Canvas {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.store.active();
        let selected = active
            .and_then(|id| self.store.workspaces().iter().find(|item| item.id == id))
            .and_then(|workspace| workspace.canvas.selected());
        let zoom = self.zoom;
        let offset_x = self.offset_x;
        let offset_y = self.offset_y;
        let add = ComponentButton::new("add-terminal")
            .tooltip("Adicionar terminal")
            .child(canvas_icon("terminal.svg"))
            .w(px(28.0))
            .h(px(28.0))
            .p_0()
            .rounded(px(8.0))
            .with_variant(ButtonVariant::Ghost);
        let add = if active.is_some() {
            add.on_click(cx.listener(|this, _, _, cx| this.begin_placement(cx)))
        } else {
            add.disabled(true)
        };
        let remove = ComponentButton::new("remove-selected")
            .tooltip("Apagar selecionado")
            .child(canvas_icon("trash.svg"))
            .w(px(28.0))
            .h(px(28.0))
            .p_0()
            .rounded(px(8.0))
            .with_variant(ButtonVariant::Ghost)
            .disabled(selected.is_none())
            .on_click(cx.listener(|this, _, _, cx| this.remove_selected(cx)));
        let cards: Vec<_> = active
            .and_then(|id| self.cards.get(&id))
            .map(|group| {
                group
                    .iter()
                    .map(|(_, card)| div().child(card.clone()))
                    .collect()
            })
            .unwrap_or_default();

        div()
            .size_full()
            .flex()
            .flex_row()
            .bg(rgb(0x0f1117))
            .child(self.drawer.clone())
            .child(
                div()
                    .relative()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .min_h_0()
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .overflow_hidden()
                            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                                let (dx, dy) = scroll_delta(event);
                                this.apply_scroll(event.position, dx, dy, event.modifiers.control, event.modifiers.shift, cx);
                                cx.stop_propagation();
                            }))
                            .child(
                                canvas(
                                    |_, _, _| {},
                                    move |bounds, _, window, _| {
                                        let origin = bounds.origin;
                                        let width = f32::from(bounds.size.width);
                                        let height = f32::from(bounds.size.height);
                                        let step = (GRID_STEP * zoom).max(4.0);
                                        let mut x = offset_x.rem_euclid(step);
                                        while x < width {
                                            window.paint_quad(fill(
                                                Bounds::new(
                                                    Point::new(
                                                        px(f32::from(origin.x) + x),
                                                        origin.y,
                                                    ),
                                                    Size::new(px(1.0), bounds.size.height),
                                                ),
                                                rgb(0x242936),
                                            ));
                                            x += step;
                                        }
                                        let mut y = offset_y.rem_euclid(step);
                                        while y < height {
                                            window.paint_quad(fill(
                                                Bounds::new(
                                                    Point::new(
                                                        origin.x,
                                                        px(f32::from(origin.y) + y),
                                                    ),
                                                    Size::new(bounds.size.width, px(1.0)),
                                                ),
                                                rgb(0x242936),
                                            ));
                                            y += step;
                                        }
                                    },
                                )
                                .absolute()
                                .left(px(0.0))
                                .top(px(0.0))
                                .size_full(),
                            )
                            .children(cards)
                            .when(self.placing, |area| {
                                area.child(
                                    div()
                                        .absolute()
                                        .left(px(0.0))
                                        .top(px(0.0))
                                        .size_full()
                                        .cursor(CursorStyle::Crosshair)
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(|this, event: &MouseDownEvent, window, cx| {
                                                this.place_terminal(event.position, window, cx);
                                            }),
                                        )
                                        .child(
                                            div()
                                                .absolute()
                                                .top(px(16.0))
                                                .left(px(16.0))
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .rounded(px(20.0))
                                                .bg(rgb(0x1b2130))
                                                .border_1()
                                                .border_color(rgb(0x5b9dff))
                                                .px_3()
                                                .py_1()
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(rgb(0xc9d1d9))
                                                        .child("Clique no canvas para posicionar o terminal"),
                                                )
                                                .child(
                                                    div()
                                                        .id("cancel-placement")
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            cx.listener(|_, _, _, cx| cx.stop_propagation()),
                                                        )
                                                        .child(
                                                            ComponentButton::new("cancel-placement-btn")
                                                                .label("Cancelar")
                                                                .with_variant(ButtonVariant::Ghost)
                                                                .on_click(cx.listener(|this, _, _, cx| {
                                                                    this.cancel_placement(cx);
                                                                })),
                                                        ),
                                                ),
                                        ),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .top(px(0.0))
                    .bottom(px(0.0))
                    .right(px(12.0))
                    .flex()
                    .flex_col()
                    .items_end()
                    .justify_center()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_1()
                            .p_1()
                            .rounded(px(20.0))
                            .bg(rgb(0x1b2130))
                            .border_1()
                            .border_color(rgb(0x2b3547))
                            .child(add)
                            .child(remove),
                    )
                    .when(active.is_none(), |column| {
                        column.child(
                            div()
                                .text_xs()
                                .text_color(rgb(0x93a0b5))
                                .child("Crie um espaço de trabalho para adicionar terminais."),
                        )
                    })
                    .children(self.error.iter().map(|message| {
                        div()
                            .rounded(px(8.0))
                            .bg(rgb(0x1b2130))
                            .border_1()
                            .border_color(rgb(0x2b3547))
                            .p_2()
                            .text_xs()
                            .text_color(rgb(0xf08a8a))
                            .child(message.clone())
                    })),
            )
            .child(
                div()
                    .absolute()
                    .bottom(px(12.0))
                    .right(px(12.0))
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(px(20.0))
                    .bg(rgb(0x1b2130))
                    .border_1()
                    .border_color(rgb(0x2b3547))
                    .px_3()
                    .py_1()
                    .child(div().text_xs().text_color(rgb(0x93a0b5)).child("Zoom"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xc9d1d9))
                            .child(format!("{}%", (zoom * 100.0).round() as i32)),
                    ),
            )
            .children(gpui_component::Root::render_dialog_layer(window, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_scroll_direction_is_inverted() {
        assert!(zoom_factor(-110.0) < 1.0);
        assert!(zoom_factor(110.0) > 1.0);
    }

    #[test]
    fn zoom_keeps_world_point_under_cursor() {
        let (zoom, offset_x, offset_y) = zoom_at(1.0, 0.0, 0.0, 400.0, 300.0, 1.25);
        assert_eq!(zoom, 1.25);
        assert_eq!((400.0 - offset_x) / zoom, 400.0);
        assert_eq!((300.0 - offset_y) / zoom, 300.0);
    }

    fn groups_for(store: &WorkspaceStore) -> CardGroups {
        store
            .workspaces()
            .iter()
            .map(|workspace| (workspace.id, Vec::new()))
            .collect()
    }

    #[test]
    fn remove_workspace_drops_only_its_group_and_clears_active() {
        let mut store = WorkspaceStore::new();
        let first = store.add("A".into(), WorkspaceIcon::Star, PathBuf::from("/a"));
        let second = store.add("B".into(), WorkspaceIcon::Folder, PathBuf::from("/b"));
        let mut groups = groups_for(&store);
        store.active_mut().unwrap().canvas.add();

        let (removed, dropped) = remove_workspace(&mut store, &mut groups, second.id).unwrap();

        assert_eq!(removed.id, second.id);
        assert!(dropped.is_empty());
        assert!(!groups.contains_key(&second.id));
        assert!(groups.contains_key(&first.id));
        assert_eq!(store.active(), Some(first.id));
        assert!(store.active_mut().unwrap().canvas.terminals().is_empty());
    }

    #[test]
    fn remove_workspace_that_is_not_active_keeps_selection() {
        let mut store = WorkspaceStore::new();
        let first = store.add("A".into(), WorkspaceIcon::Star, PathBuf::from("/a"));
        let second = store.add("B".into(), WorkspaceIcon::Bolt, PathBuf::from("/b"));
        store.select(first.id);
        let mut groups = groups_for(&store);

        remove_workspace(&mut store, &mut groups, second.id).unwrap();

        assert_eq!(store.active(), Some(first.id));
        assert!(groups.contains_key(&first.id));
    }

    #[test]
    fn create_workspace_with_empty_root_is_refused() {
        let mut store = WorkspaceStore::new();

        let result = create_workspace(
            &mut store,
            "Vazia".into(),
            WorkspaceIcon::Star,
            PathBuf::new(),
        );

        assert!(result.is_err());
        assert!(store.workspaces().is_empty());
        assert_eq!(store.active(), None);
    }

    #[test]
    fn add_terminal_without_active_workspace_creates_nothing() {
        let mut store = WorkspaceStore::new();

        let result = active_spawn_target(&mut store);

        assert_eq!(result, Err(SpawnTargetError::NoWorkspace));
        assert!(store.workspaces().is_empty());
    }

    #[test]
    fn add_terminal_uses_active_workspace_root_and_state() {
        let mut store = WorkspaceStore::new();
        let workspace = store.add(
            "A".into(),
            WorkspaceIcon::Star,
            std::env::current_dir().unwrap(),
        );

        let (id, terminal, root) = active_spawn_target(&mut store).unwrap();

        assert_eq!(id, workspace.id);
        assert_eq!(root, workspace.root);
        assert_eq!(
            store
                .workspaces()
                .iter()
                .find(|item| item.id == id)
                .unwrap()
                .canvas
                .terminals()
                .len(),
            1
        );
        assert_eq!(terminal, 1);
    }

    #[test]
    fn persisted_snapshot_drops_runtime_terminals() {
        let mut store = WorkspaceStore::new();
        let workspace = store.add(
            "A".into(),
            WorkspaceIcon::Star,
            std::env::current_dir().unwrap(),
        );
        active_spawn_target(&mut store).unwrap();

        let snapshot = persisted_snapshot(&store);

        assert!(snapshot.workspaces()[0].canvas.terminals().is_empty());
        assert_eq!(snapshot.active(), Some(workspace.id));
        assert_eq!(store.workspaces()[0].canvas.terminals().len(), 1);
    }

    #[test]
    fn removing_colliding_terminal_id_affects_only_its_workspace() {
        let mut store = WorkspaceStore::new();
        let first = store.add("A".into(), WorkspaceIcon::Star, PathBuf::from("/a"));
        let second = store.add("B".into(), WorkspaceIcon::Folder, PathBuf::from("/b"));
        let first_id = store.workspace_mut(first.id).unwrap().canvas.add().id;
        let second_id = store.workspace_mut(second.id).unwrap().canvas.add().id;
        assert_eq!((first_id, second_id), (1, 1));
        let mut groups = groups_for(&store);

        assert!(remove_terminal_state(&mut store, &mut groups, second.id, second_id));

        assert_eq!(store.workspace_mut(first.id).unwrap().canvas.terminals().len(), 1);
        assert!(store.workspace_mut(second.id).unwrap().canvas.terminals().is_empty());
    }
}
