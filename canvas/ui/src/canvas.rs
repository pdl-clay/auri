//! Canvas view: drawer, workspace groups and the lifecycle of PTY-backed cards.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use auro::gpui::{
    AnyWindowHandle, Bounds, Context, CursorStyle, Entity, MouseButton, MouseDownEvent, Pixels, Point, Render,
    ScrollWheelEvent, Size, Subscription, Window, canvas, fill, px, rgb, svg,
};
use auro::prelude::*;
use gpui_component::{
    Disableable,
    button::{Button as ComponentButton, ButtonVariant, ButtonVariants as _},
};
use gpui_component::input::{Input, InputEvent, InputState};
use canvas_types::{AgentKind, AgentState, CardBounds, NoteState, TerminalId, TerminalState, WorkspaceColor, WorkspaceIcon, WorkspaceId, WorkspaceState, WorkspaceStore};

use crate::card::{TerminalCard, scroll_delta, terminal_grid};
use crate::text_card::TextCard;
use crate::workspaces::{DrawerAction, WorkspaceDrawer};
use crate::theme::*;
use crate::config::{self, LoadOutcome};
use crate::pty::PtySession;

type CardGroups = HashMap<WorkspaceId, Vec<(TerminalId, Entity<TerminalCard>)>>;

type NoteGroups = HashMap<WorkspaceId, Vec<(u64, Entity<TextCard>)>>;


/// Transient menu state. Grouping these flags makes every close path clear the
/// nested panels together, including the agent list.
#[derive(Clone, Debug, Default, PartialEq)]
struct MenuState {
    context_menu: Option<Point<Pixels>>,
    submenu: bool,
    agent_list: bool,
    placing_agent: Option<AgentKind>,
}

impl MenuState {
    fn close(&mut self) {
        *self = Self::default();
    }

    fn open_context_menu(&mut self, position: Point<Pixels>) {
        self.close();
        self.context_menu = Some(position);
    }

    fn begin_placement(&mut self, kind: Option<AgentKind>) {
        self.close();
        self.placing_agent = kind;
    }
}

/// UI-only sticky-note selection; never written to the workspace store.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct NoteSelection(Option<(WorkspaceId, u64)>);

impl NoteSelection {
    fn get(&self) -> Option<(WorkspaceId, u64)> {
        self.0
    }

    fn select(&mut self, workspace_id: WorkspaceId, id: u64) {
        self.0 = Some((workspace_id, id));
    }

    fn clear(&mut self) {
        self.0 = None;
    }

    fn clear_workspace(&mut self, workspace_id: WorkspaceId) {
        if self.0.is_some_and(|(selected_workspace, _)| selected_workspace == workspace_id) {
            self.0 = None;
        }
    }

    fn clear_note(&mut self, workspace_id: WorkspaceId, id: u64) {
        if self.0 == Some((workspace_id, id)) {
            self.0 = None;
        }
    }
}

/// Transient UI state for the header menu and its edit dialog.
#[derive(Clone, Debug)]
struct AgentMenu {
    workspace_id: WorkspaceId,
    card_id: TerminalId,
    position: Point<Pixels>,
    editing: bool,
    name: String,
    kind: AgentKind,
    original_name: String,
    original_kind: AgentKind,
}

/// The store is the persisted state: cards and notes stay in it, and the
/// framework-specific entities are rebuilt from it on startup.
fn persisted_snapshot(store: &WorkspaceStore) -> WorkspaceStore {
    store.clone()
}

/// Convert GPUI geometry into the framework-free persisted form.
pub(crate) fn card_bounds(bounds: &Bounds<Pixels>) -> CardBounds {
    CardBounds {
        x: f32::from(bounds.origin.x),
        y: f32::from(bounds.origin.y),
        width: f32::from(bounds.size.width),
        height: f32::from(bounds.size.height),
    }
}

fn pixel_bounds(bounds: CardBounds) -> Bounds<Pixels> {
    Bounds::new(
        Point::new(px(bounds.x), px(bounds.y)),
        Size::new(px(bounds.width), px(bounds.height)),
    )
}
/// Place a new card immediately to the right of an existing one.
fn adjacent_origin(last: Bounds<Pixels>) -> Point<Pixels> {
    Point::new(
        px(f32::from(last.origin.x) + f32::from(last.size.width) + CARD_GAP),
        last.origin.y,
    )
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

/// Validate the values submitted by both workspace form modes.
fn validate_workspace_details(name: &str, root: &Path) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Informe um nome para o espaço.".into());
    }
    if root.as_os_str().is_empty() {
        return Err("Escolha uma raiz para o espaço.".into());
    }
    if !root.is_dir() {
        return Err(format!("A raiz {} não é um diretório.", root.display()));
    }
    Ok(())
}

/// Create a workspace, refusing a blank or unusable root.
fn create_workspace(
    store: &mut WorkspaceStore,
    name: String,
    icon: WorkspaceIcon,
    color: WorkspaceColor,
    root: PathBuf,
) -> Result<WorkspaceId, String> {
    let name = name.trim().to_owned();
    validate_workspace_details(&name, &root)?;
    Ok(store.add_with_color(name, icon, color, root).id)
}

/// Apply edits to an existing workspace without touching its cards.
fn update_workspace(
    store: &mut WorkspaceStore,
    id: WorkspaceId,
    name: String,
    icon: WorkspaceIcon,
    color: WorkspaceColor,
    root: PathBuf,
) -> Result<(), String> {
    let name = name.trim().to_owned();
    validate_workspace_details(&name, &root)?;
    let workspace = store
        .workspace_mut(id)
        .ok_or_else(|| "Espaço de trabalho não encontrado.".to_owned())?;
    workspace.update_details(name, icon, color, root);
    Ok(())
}

/// End a workspace's cards, keeping the row and its identifier.
fn unload_workspace(store: &mut WorkspaceStore, id: WorkspaceId) -> bool {
    let Some(workspace) = store.workspace_mut(id) else {
        return false;
    };
    workspace.unload();
    true
}

/// Saved data needed to rebuild a workspace's live cards after a click.
struct WorkspaceLayout {
    root: PathBuf,
    root_ok: bool,
    terminals: Vec<TerminalState>,
    notes: Vec<NoteState>,
    agents: Vec<AgentState>,
}

/// Select a workspace and clear its unloaded flag, returning the saved layout
/// the GPUI side must rebuild. Already-loaded workspaces need no rebuild.
fn begin_workspace_load(store: &mut WorkspaceStore, id: WorkspaceId) -> Option<WorkspaceLayout> {
    let workspace = store.workspace_mut(id)?;
    if !workspace.unloaded {
        return None;
    }
    workspace.mark_loaded();
    store.select(id);
    let workspace = store.workspace_mut(id)?;
    Some(WorkspaceLayout {
        root: workspace.root.clone(),
        root_ok: !workspace.invalid_root
            && !workspace.root.as_os_str().is_empty()
            && workspace.root.is_dir(),
        terminals: workspace.canvas.terminals().to_vec(),
        notes: workspace.canvas.notes().to_vec(),
        agents: workspace.canvas.agents().to_vec(),
    })
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

/// Validate the active workspace as a shell spawn target without allocating an id.
fn active_workspace_root(store: &WorkspaceStore) -> Result<(WorkspaceId, PathBuf), SpawnTargetError> {
    let workspace = store
        .active()
        .and_then(|id| store.workspaces().iter().find(|item| item.id == id))
        .ok_or(SpawnTargetError::NoWorkspace)?;
    if workspace.root.as_os_str().is_empty()
        || workspace.invalid_root
        || !workspace.root.is_dir()
    {
        return Err(SpawnTargetError::InvalidRoot(workspace.root.clone()));
    }
    Ok((workspace.id, workspace.root.clone()))
}

/// Allocate a terminal in the active workspace and return its spawn target.
fn active_spawn_target(
    store: &mut WorkspaceStore,
) -> Result<(WorkspaceId, TerminalId, PathBuf), SpawnTargetError> {
    let (workspace_id, root) = active_workspace_root(store)?;
    let id = store
        .workspace_mut(workspace_id)
        .expect("active workspace")
        .canvas
        .add()
        .id;
    Ok((workspace_id, id, root))
}

/// Allocate an agent card from the same id space as terminals and notes.
fn active_agent_target(
    store: &mut WorkspaceStore,
    kind: AgentKind,
    bounds: CardBounds,
) -> Result<(WorkspaceId, TerminalId, PathBuf), SpawnTargetError> {
    let (workspace_id, root) = active_workspace_root(store)?;
    let id = store
        .workspace_mut(workspace_id)
        .expect("active workspace")
        .canvas
        .add_agent(kind, bounds)
        .id;
    Ok((workspace_id, id, root))
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
    if workspace.canvas.agent(id).is_some() {
        if workspace.canvas.remove_agent(id).is_none() {
            return false;
        }
    } else if workspace.canvas.remove(id).is_none() {
        return false;
    }
    if let Some(cards) = groups.get_mut(&workspace_id) {
        cards.retain(|(card_id, _)| *card_id != id);
    }
    true
}

/// Owns the workspace store, its drawer and every workspace' terminal cards.
pub struct Canvas {
    window: AnyWindowHandle,
    store: WorkspaceStore,
    drawer: Entity<WorkspaceDrawer>,
    cards: CardGroups,
    notes: NoteGroups,
    error: Option<String>,
    placing: bool,
    placing_text: bool,
    zoom: f32,
    offset_x: f32,
    offset_y: f32,
    menu: MenuState,
    selected_note: NoteSelection,
    agent_menu: Option<AgentMenu>,
    agent_name_input: Option<Entity<InputState>>,
    agent_name_subscription: Option<Subscription>,
}
impl Canvas {
    /// `load` is authoritative for the drawer banner; the store was already
    /// loaded by the caller and is pushed back into the drawer explicitly.
    pub fn new(store: WorkspaceStore, load: LoadOutcome, window: AnyWindowHandle, cx: &mut Context<Self>) -> Self {
        let drawer = cx.new(|_| WorkspaceDrawer::new(load));
        drawer.update(cx, |drawer, _| drawer.set_store(store.clone()));
        Self {
            window,
            store,
            drawer,
            cards: HashMap::new(),
            notes: HashMap::new(),
            error: None,
            placing: false,
            placing_text: false,
            zoom: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
            menu: MenuState::default(),
            selected_note: NoteSelection::default(),
            agent_menu: None,
            agent_name_input: None,
            agent_name_subscription: None,
        }
    }

    /// Wire drawer actions back to this canvas without creating a strong cycle.
    pub fn bind_actions(&mut self, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let restart_weak = weak.clone();
        let window = self.window;
        self.drawer.update(cx, |drawer, _| {
            drawer.on_action(move |action, app| {
                let Some(canvas) = weak.upgrade() else {
                    return;
                };
                match action {
                    DrawerAction::Select(id) => {
                        // The row click is dispatched while its window is borrowed;
                        // rebuild the cards once dispatch has finished.
                        app.defer(move |app| {
                            let _ = app.update_window(window, |_, window, app| {
                                canvas.update(app, |canvas, cx| canvas.handle_select(id, window, cx));
                            });
                        });
                    }
                    other => {
                        canvas.update(app, |canvas, cx| canvas.handle_action(other, cx));
                    }
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

    /// A row click always activates its workspace and loads its saved cards.
    fn handle_select(&mut self, id: WorkspaceId, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_note_selection(cx);
        if self.store.workspace_mut(id).is_some_and(|workspace| !workspace.unloaded) {
            self.store.select(id);
            self.finish_store_change(cx);
            return;
        }
        self.load_workspace(id, window, cx);
    }

    fn handle_action(&mut self, action: DrawerAction, cx: &mut Context<Self>) {
        match action {
            DrawerAction::Create { name, icon, color, root } => {
                self.clear_note_selection(cx);
                match create_workspace(&mut self.store, name, icon, color, root) {
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
            DrawerAction::Update { id, name, icon, color, root } => {
                match update_workspace(&mut self.store, id, name, icon, color, root) {
                    Ok(()) => {
                        self.error = None;
                        self.finish_store_change(cx);
                    }
                    Err(error) => {
                        self.error = Some(error);
                        cx.notify();
                    }
                }
            }
            DrawerAction::Unload(id) => {
                self.selected_note.clear_workspace(id);
                self.repaint_note_selection(cx);
                if unload_workspace(&mut self.store, id) {
                    let dropped = self.cards.remove(&id).unwrap_or_default();
                    let dropped_notes = self.notes.remove(&id).unwrap_or_default();
                    self.finish_store_change(cx);
                    // Dropping these entities ends every PTY in the workspace.
                    drop(dropped);
                    drop(dropped_notes);
                }
            }
            DrawerAction::Delete(id) => {
                self.selected_note.clear_workspace(id);
                self.repaint_note_selection(cx);
                if let Some((_, dropped)) = remove_workspace(&mut self.store, &mut self.cards, id) {
                    let dropped_notes = self.notes.remove(&id).unwrap_or_default();
                    self.finish_store_change(cx);
                    // Dropping the card handles terminates that workspace's shells.
                    drop(dropped);
                    drop(dropped_notes);
                }
            }
            DrawerAction::Select(id) => {
                self.clear_note_selection(cx);
                self.store.select(id);
                self.finish_store_change(cx);
            }
        }
    }

    fn persist_store(&mut self, cx: &mut Context<Self>) {
        self.error = match config::save(&persisted_snapshot(&self.store)) {
            Ok(()) => None,
            Err(error) => Some(format!("Não foi possível salvar os espaços: {error}")),
        };
        cx.notify();
    }

    fn finish_store_change(&mut self, cx: &mut Context<Self>) {
        self.persist_store(cx);
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
    }

    /// `config::restart` already replaced the file and the drawer's store.
    fn adopt_restarted_store(&mut self, mut store: WorkspaceStore, cx: &mut Context<Self>) {
        self.clear_note_selection(cx);
        store.mark_all_unloaded();
        self.store = store;
        self.cards.clear();
        self.notes.clear();
        self.error = None;
        self.persist_store(cx);
        self.sync_drawer(cx);
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
            let drawer_width = self.drawer.read(cx).drawer_width();
            let local_x = f32::from(position.x) - drawer_width;
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
            self.offset_y = pan_vertical(self.offset_y, dy);
        }
        let (zoom, offset_x, offset_y) = (self.zoom, self.offset_x, self.offset_y);
        let cards: Vec<Entity<TerminalCard>> = self
            .cards
            .values()
            .flat_map(|group| group.iter().map(|(_, card)| card.clone()))
            .collect();
        let notes: Vec<Entity<TextCard>> = self
            .notes
            .values()
            .flat_map(|group| group.iter().map(|(_, note)| note.clone()))
            .collect();
        cx.defer(move |cx| {
            for card in cards {
                card.update(cx, |card, cx| card.set_view(zoom, offset_x, offset_y, cx));
            }
            for note in notes {
                note.update(cx, |note, cx| note.set_view(zoom, offset_x, offset_y, cx));
            }
        });
        cx.notify();
    }

    /// Enter placement mode: the next canvas click decides where the terminal opens.
    pub fn begin_placement(&mut self, cx: &mut Context<Self>) {
        self.placing = true;
        self.placing_text = false;
        self.menu.begin_placement(None);
        self.error = None;
        self.agent_menu = None;
        cx.notify();
    }

    /// Enter placement mode for a new sticky note.
    pub fn begin_text_placement(&mut self, cx: &mut Context<Self>) {
        self.placing_text = true;
        self.placing = false;
        self.menu.begin_placement(None);
        self.error = None;
        self.agent_menu = None;
        cx.notify();
    }

    pub fn cancel_placement(&mut self, cx: &mut Context<Self>) {
        self.placing = false;
        self.placing_text = false;
        self.menu.close();
        cx.notify();
    }
    fn place_terminal(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let drawer_width = self.drawer.read(cx).drawer_width();
        let local_x = (f32::from(position.x) - drawer_width - self.offset_x) / self.zoom;
        let local_y = (f32::from(position.y) - self.offset_y) / self.zoom;
        let origin = Point::new(
            px((local_x - CARD_WIDTH / 2.0).max(8.0)),
            px((local_y - CARD_HEIGHT / 2.0).max(8.0)),
        );
        self.placing = false;
        if let Some(kind) = self.menu.placing_agent.take() {
            self.spawn_agent(kind, Some(origin), window, cx);
        } else {
            self.spawn_terminal(Some(origin), window, cx);
        }
    }

    fn place_text_at(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let drawer_width = self.drawer.read(cx).drawer_width();
        let local_x = (f32::from(position.x) - drawer_width - self.offset_x) / self.zoom;
        let local_y = (f32::from(position.y) - self.offset_y) / self.zoom;
        let origin = Point::new(
            px((local_x - TEXT_CARD_WIDTH / 2.0).max(SPACE_NORMAL)),
            px((local_y - TEXT_CARD_HEIGHT / 2.0).max(SPACE_NORMAL)),
        );
        let Some(workspace_id) = self.store.active() else {
            self.placing_text = false;
            self.error = Some(SpawnTargetError::NoWorkspace.message());
            cx.notify();
            return;
        };
        let bounds = Bounds::new(
            origin,
            Size::new(px(TEXT_CARD_WIDTH), px(TEXT_CARD_HEIGHT)),
        );
        let saved = card_bounds(&bounds);
        let id = {
            let Some(workspace) = self.store.workspace_mut(workspace_id) else {
                self.placing_text = false;
                self.error = Some(SpawnTargetError::NoWorkspace.message());
                cx.notify();
                return;
            };
            let note_id = workspace.canvas.add_note(String::new(), saved).id;
            workspace.mark_loaded();
            note_id
        };
        let canvas = cx.entity().downgrade();
        let note = cx.new(|cx| {
            TextCard::new(id, workspace_id, bounds, String::new(), canvas.clone(), window, cx)
        });
        let (zoom, offset_x, offset_y) = (self.zoom, self.offset_x, self.offset_y);
        note.update(cx, |note, cx| {
            note.set_view(zoom, offset_x, offset_y, cx);
            note.focus(window, cx);
        });
        self.notes.entry(workspace_id).or_default().push((id, note));
        self.placing_text = false;
        self.finish_store_change(cx);
    }
    /// Remove one note and its persisted state.
    pub fn remove_text(&mut self, workspace_id: WorkspaceId, id: u64, cx: &mut Context<Self>) {
        if let Some(workspace) = self.store.workspace_mut(workspace_id) {
            workspace.canvas.remove_note(id);
        }
        if let Some(notes) = self.notes.get_mut(&workspace_id) {
            notes.retain(|(note_id, _)| *note_id != id);
        }
        self.selected_note.clear_note(workspace_id, id);
        self.repaint_note_selection(cx);
        self.finish_store_change(cx);
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
        self.clear_note_selection(cx);
        let (workspace_id, id, root) = match active_spawn_target(&mut self.store) {
            Ok(target) => target,
            Err(error) => {
                self.error = Some(error.message());
                cx.notify();
                return;
            }
        };
        let origin = origin.unwrap_or_else(|| {
            self.cards
                .get(&workspace_id)
                .and_then(|cards| cards.last())
                .map(|(_, card)| adjacent_origin(card.read(cx).bounds))
                .unwrap_or_else(|| Point::new(px(16.0), px(16.0)))
        });
        let bounds = Bounds::new(origin, Size::new(px(CARD_WIDTH), px(CARD_HEIGHT)));
        let saved = card_bounds(&bounds);
        if let Some(workspace) = self.store.workspace_mut(workspace_id) {
            workspace.canvas.set_terminal_bounds(id, saved);
        }
        let (cols, rows) = terminal_grid(&bounds);
        let pty = match PtySession::spawn_shell_in(cols, rows, Some(&root)) {
            Ok(pty) => pty,
            Err(error) => {
                // A failed fork must not leave a phantom entry in the state.
                if let Some(workspace) = self.store.workspace_mut(workspace_id) {
                    workspace.canvas.remove(id);
                }
                self.error = Some(format!("Terminal {id} não iniciou: {error}"));
                self.finish_store_change(cx);
                return;
            }
        };
        if let Some(workspace) = self.store.workspace_mut(workspace_id) {
            workspace.mark_loaded();
        }
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
        self.finish_store_change(cx);
    }

    /// Enter placement mode for one of the five CLI agents.
    pub fn begin_agent_placement(&mut self, kind: AgentKind, cx: &mut Context<Self>) {
        self.placing = true;
        self.placing_text = false;
        self.menu.begin_placement(Some(kind));
        self.error = None;
        self.agent_menu = None;
        cx.notify();
    }

    /// Spawn a shell, type the agent's CLI into it and keep the card alive.
    fn spawn_agent(
        &mut self,
        kind: AgentKind,
        origin: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_note_selection(cx);
        let origin = origin.unwrap_or_else(|| {
            self.cards
                .get(&self.store.active().unwrap_or_default())
                .and_then(|cards| cards.last())
                .map(|(_, card)| adjacent_origin(card.read(cx).bounds))
                .unwrap_or_else(|| Point::new(px(SPACE_PANEL), px(SPACE_PANEL)))
        });
        let bounds = Bounds::new(origin, Size::new(px(CARD_WIDTH), px(CARD_HEIGHT)));
        let saved = card_bounds(&bounds);
        let (workspace_id, id, root) = match active_agent_target(&mut self.store, kind, saved) {
            Ok(target) => target,
            Err(error) => {
                self.error = Some(error.message());
                cx.notify();
                return;
            }
        };
        let (cols, rows) = terminal_grid(&bounds);
        let mut pty = match PtySession::spawn_shell_in(cols, rows, Some(&root)) {
            Ok(pty) => pty,
            Err(error) => {
                if let Some(workspace) = self.store.workspace_mut(workspace_id) {
                    workspace.canvas.remove_agent(id);
                }
                self.error = Some(format!("Agente {} não iniciou: {error}", kind.label()));
                self.finish_store_change(cx);
                return;
            }
        };
        if let Err(error) = pty.send_line(kind.command()) {
            if let Some(workspace) = self.store.workspace_mut(workspace_id) {
                workspace.canvas.remove_agent(id);
            }
            self.error = Some(format!("Agente {} não iniciou: {error}", kind.label()));
            self.finish_store_change(cx);
            return;
        }
        if let Some(workspace) = self.store.workspace_mut(workspace_id) {
            workspace.mark_loaded();
        }
        self.error = None;
        let name = self
            .store
            .workspace_mut(workspace_id)
            .and_then(|workspace| workspace.canvas.agent(id))
            .map(|agent| agent.name.clone())
            .unwrap_or_else(|| kind.label().to_owned());
        let canvas = cx.entity().downgrade();
        let card = cx.new(|cx| {
            TerminalCard::new(id, workspace_id, bounds, canvas.clone(), root.clone(), pty, (cols, rows), cx)
        });
        card.update(cx, |card, cx| card.set_agent(kind, name, cx));
        card.read(cx).terminal.read(cx).focus_handle().focus(window);
        let (zoom, offset_x, offset_y) = (self.zoom, self.offset_x, self.offset_y);
        card.update(cx, |card, cx| card.set_view(zoom, offset_x, offset_y, cx));
        self.cards.entry(workspace_id).or_default().push((id, card));
        self.sync_card_selection(workspace_id, cx);
        self.finish_store_change(cx);
    }

    /// Open the small header menu at the point where the strip was clicked.
    pub fn open_agent_menu(
        &mut self,
        workspace_id: WorkspaceId,
        card_id: TerminalId,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(agent) = self
            .store
            .workspace_mut(workspace_id)
            .and_then(|workspace| workspace.canvas.agent(card_id))
            .cloned()
        else {
            return;
        };
        self.agent_menu = Some(AgentMenu {
            workspace_id,
            card_id,
            position,
            editing: false,
            name: agent.name.clone(),
            kind: agent.kind,
            original_name: agent.name,
            original_kind: agent.kind,
        });
        self.menu.close();
        cx.notify();
    }

    fn begin_agent_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.agent_menu.as_mut() {
            menu.editing = true;
        }
        self.ensure_agent_name_input(window, cx);
        self.sync_agent_name_input(window, cx);
        cx.notify();
    }

    fn ensure_agent_name_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.agent_name_input.is_some() {
            return;
        }
        let value = self
            .agent_menu
            .as_ref()
            .map(|menu| menu.name.clone())
            .unwrap_or_default();
        let state = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(value)
                .placeholder("Nome do terminal")
        });
        self.agent_name_subscription = Some(cx.subscribe_in(
            &state,
            window,
            |this, state, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change)
                    && let Some(menu) = this.agent_menu.as_mut()
                {
                    menu.name = state.read(cx).value().to_string();
                    cx.notify();
                }
            },
        ));
        self.agent_name_input = Some(state);
    }

    fn sync_agent_name_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.agent_name_input.clone() else {
            return;
        };
        let value = self
            .agent_menu
            .as_ref()
            .map(|menu| menu.name.clone())
            .unwrap_or_default();
        if state.read(cx).value().to_string() != value {
            state.update(cx, |state, cx| state.set_value(value, window, cx));
        }
    }

    fn apply_agent_to_card(
        &self,
        workspace_id: WorkspaceId,
        card_id: TerminalId,
        kind: AgentKind,
        name: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(card) = self
            .cards
            .get(&workspace_id)
            .and_then(|group| group.iter().find(|(id, _)| *id == card_id))
            .map(|(_, card)| card)
        {
            card.update(cx, |card, cx| card.set_agent(kind, name, cx));
        }
    }

    fn close_agent_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(menu) = self.agent_menu.clone() {
            self.apply_agent_to_card(
                menu.workspace_id,
                menu.card_id,
                menu.original_kind,
                menu.original_name,
                cx,
            );
        }
        self.agent_menu = None;
        self.error = None;
        cx.notify();
    }

    fn save_agent_edits(&mut self, cx: &mut Context<Self>) {
        let Some(menu) = self.agent_menu.clone() else {
            return;
        };
        let name = menu.name.trim().to_owned();
        if name.is_empty() {
            self.error = Some("Informe um nome para o agente.".into());
            cx.notify();
            return;
        }
        if let Some(workspace) = self.store.workspace_mut(menu.workspace_id) {
            workspace.canvas.rename_agent(menu.card_id, name.clone());
            workspace.canvas.set_agent_kind(menu.card_id, menu.kind);
        }
        self.apply_agent_to_card(menu.workspace_id, menu.card_id, menu.kind, name, cx);
        self.agent_menu = None;
        self.error = None;
        self.finish_store_change(cx);
    }

    /// Commit geometry to the right persisted card type after a drag.
    pub fn commit_card_bounds(
        &mut self,
        workspace_id: WorkspaceId,
        id: TerminalId,
        bounds: CardBounds,
        cx: &mut Context<Self>,
    ) {
        let changed = self
            .store
            .workspace_mut(workspace_id)
            .is_some_and(|workspace| {
                if workspace.canvas.agent(id).is_some() {
                    workspace.canvas.set_agent_bounds(id, bounds)
                } else {
                    workspace.canvas.set_terminal_bounds(id, bounds)
                }
            });
        if changed {
            self.persist_store(cx);
        }
    }

    /// Push the current store to the drawer without touching the config file.
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
        if self
            .agent_menu
            .as_ref()
            .is_some_and(|menu| menu.workspace_id == workspace_id && menu.card_id == id)
        {
            self.agent_menu = None;
        }
        remove_terminal_state(&mut self.store, &mut self.cards, workspace_id, id);
        self.finish_store_change(cx);
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

    /// Repaint every note after the UI-only selection changes.
    fn repaint_note_selection(&self, cx: &mut Context<Self>) {
        let selected = self.selected_note.get();
        let notes: Vec<_> = self
            .notes
            .iter()
            .flat_map(|(workspace_id, group)| {
                group
                    .iter()
                    .map(move |(id, note)| (*workspace_id, *id, note.clone()))
            })
            .collect();
        cx.defer(move |cx| {
            for (workspace_id, id, note) in notes {
                note.update(cx, |note, cx| {
                    note.set_selected(selected == Some((workspace_id, id)), cx)
                });
            }
        });
    }

    /// Drop the note selection without persisting anything.
    fn clear_note_selection(&mut self, cx: &mut Context<Self>) {
        if self.selected_note.get().is_none() {
            return;
        }
        self.selected_note.clear();
        self.repaint_note_selection(cx);
        cx.notify();
    }

    /// Select a sticky note; terminal selection lives in the store, so clear it there.
    pub fn select_note(&mut self, workspace_id: WorkspaceId, id: u64, cx: &mut Context<Self>) {
        let had_terminal_selection = self
            .store
            .workspace_mut(workspace_id)
            .is_some_and(|workspace| workspace.canvas.selected().is_some());
        if let Some(workspace) = self.store.workspace_mut(workspace_id) {
            workspace.canvas.clear_selection();
        }
        if had_terminal_selection {
            self.persist_store(cx);
        }
        self.selected_note.select(workspace_id, id);
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
        self.repaint_note_selection(cx);
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
        self.clear_note_selection(cx);
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

    /// Persist a terminal's geometry after a drag or resize.
    pub fn commit_terminal_bounds(
        &mut self,
        workspace_id: WorkspaceId,
        id: TerminalId,
        bounds: CardBounds,
        cx: &mut Context<Self>,
    ) {
        let changed = self
            .store
            .workspace_mut(workspace_id)
            .is_some_and(|workspace| workspace.canvas.set_terminal_bounds(id, bounds));
        if changed {
            self.persist_store(cx);
        }
    }

    /// Persist a note's geometry after a drag or resize.
    pub fn commit_note_bounds(
        &mut self,
        workspace_id: WorkspaceId,
        id: u64,
        bounds: CardBounds,
        cx: &mut Context<Self>,
    ) {
        let changed = self
            .store
            .workspace_mut(workspace_id)
            .is_some_and(|workspace| workspace.canvas.set_note_bounds(id, bounds));
        if changed {
            self.persist_store(cx);
        }
    }

    /// Persist note text whenever its input notifies the canvas.
    pub fn commit_note_text(
        &mut self,
        workspace_id: WorkspaceId,
        id: u64,
        text: String,
        cx: &mut Context<Self>,
    ) {
        let changed = self
            .store
            .workspace_mut(workspace_id)
            .is_some_and(|workspace| workspace.canvas.set_note_text(id, text));
        if changed {
            self.persist_store(cx);
        }
    }

    /// Capture every live card, including drags released outside its own bounds.
    fn commit_all_bounds(&mut self, cx: &mut Context<Self>) {
        let mut terminals = Vec::new();
        for (workspace_id, group) in &self.cards {
            for (id, card) in group {
                terminals.push((*workspace_id, *id, card_bounds(&card.read(cx).bounds)));
            }
        }
        let mut notes = Vec::new();
        for (workspace_id, group) in &self.notes {
            for (id, note) in group {
                let note = note.read(cx);
                notes.push((*workspace_id, *id, card_bounds(&note.bounds()), note.text(cx)));
            }
        }

        let mut changed = false;
        for (workspace_id, id, bounds) in terminals {
            if let Some(workspace) = self.store.workspace_mut(workspace_id) {
                changed |= if workspace.canvas.agent(id).is_some() {
                    workspace.canvas.set_agent_bounds(id, bounds)
                } else {
                    workspace.canvas.set_terminal_bounds(id, bounds)
                };
            }
        }
        for (workspace_id, id, bounds, text) in notes {
            if let Some(workspace) = self.store.workspace_mut(workspace_id) {
                changed |= workspace.canvas.set_note_bounds(id, bounds);
                changed |= workspace.canvas.set_note_text(id, text);
            }
        }
        if changed {
            self.persist_store(cx);
        }
    }

    /// Startup keeps the saved layouts but creates no live cards: each workspace
    /// is loaded when its row is clicked.
    pub fn prepare_workspaces(&mut self, cx: &mut Context<Self>) {
        self.store.mark_all_unloaded();
        self.persist_store(cx);
        self.sync_drawer(cx);
        cx.notify();
    }

    /// Recreate a workspace's saved cards with fresh shells after a row click.
    /// Shell processes cannot survive a reload, so each terminal gets a new
    /// shell in the saved root and keeps its saved bounds; notes keep their text.
    pub fn load_workspace(
        &mut self,
        id: WorkspaceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(WorkspaceLayout { root, root_ok, terminals, notes, agents }) =
            begin_workspace_load(&mut self.store, id)
        else {
            return;
        };
        self.clear_note_selection(cx);
        // An unloaded workspace has no live cards; drop any stale entities.
        let dropped = self.cards.remove(&id).unwrap_or_default();
        let dropped_notes = self.notes.remove(&id).unwrap_or_default();
        drop(dropped);
        drop(dropped_notes);

        let canvas = cx.entity().downgrade();
        let (zoom, offset_x, offset_y) = (self.zoom, self.offset_x, self.offset_y);
        let ids = terminals
            .iter()
            .map(|terminal| terminal.id)
            .chain(notes.iter().map(|note| note.id))
            .chain(agents.iter().map(|agent| agent.id));
        if let Some(workspace) = self.store.workspace_mut(id) {
            for saved_id in ids {
                workspace.canvas.reserve_id_above(saved_id);
            }
        }

        for (index, terminal) in terminals.into_iter().enumerate() {
            let saved = terminal.bounds.unwrap_or_else(|| CardBounds {
                x: SPACE_PANEL + index as f32 * (CARD_WIDTH + CARD_GAP),
                y: SPACE_PANEL,
                width: CARD_WIDTH,
                height: CARD_HEIGHT,
            });
            let bounds = pixel_bounds(saved);
            let (cols, rows) = terminal_grid(&bounds);
            let pty = if root_ok {
                match PtySession::spawn_shell_in(cols, rows, Some(&root)) {
                    Ok(pty) => Some(pty),
                    Err(error) => {
                        self.error = Some(format!("Terminal {} não iniciou: {error}", terminal.id));
                        None
                    }
                }
            } else {
                None
            };
            let Some(pty) = pty else {
                if let Some(workspace) = self.store.workspace_mut(id) {
                    workspace.canvas.mark_exited(terminal.id);
                }
                continue;
            };
            // The new shell is alive: a stale `exited` flag must not hide it.
            if let Some(workspace) = self.store.workspace_mut(id) {
                workspace.canvas.set_terminal_bounds(terminal.id, saved);
                workspace.canvas.set_terminal_exited(terminal.id, false);
            }
            let card = cx.new(|cx| {
                TerminalCard::new(
                    terminal.id,
                    id,
                    bounds,
                    canvas.clone(),
                    root.clone(),
                    pty,
                    (cols, rows),
                    cx,
                )
            });
            card.update(cx, |card, cx| card.set_view(zoom, offset_x, offset_y, cx));
            self.cards.entry(id).or_default().push((terminal.id, card));
        }

        for agent in agents {
            let bounds = pixel_bounds(agent.bounds);
            let (cols, rows) = terminal_grid(&bounds);
            let mut pty = if root_ok {
                match PtySession::spawn_shell_in(cols, rows, Some(&root)) {
                    Ok(pty) => pty,
                    Err(error) => {
                        self.error = Some(format!("Agente {} não iniciou: {error}", agent.kind.label()));
                        continue;
                    }
                }
            } else {
                continue;
            };
            if let Err(error) = pty.send_line(agent.kind.command()) {
                self.error = Some(format!("Agente {} não iniciou: {error}", agent.kind.label()));
                continue;
            }
            let card = cx.new(|cx| {
                TerminalCard::new(
                    agent.id,
                    id,
                    bounds,
                    canvas.clone(),
                    root.clone(),
                    pty,
                    (cols, rows),
                    cx,
                )
            });
            card.update(cx, |card, cx| {
                card.set_agent(agent.kind, agent.name.clone(), cx);
                card.set_view(zoom, offset_x, offset_y, cx);
            });
            self.cards.entry(id).or_default().push((agent.id, card));
        }

        for note in notes {
            let bounds = pixel_bounds(note.bounds);
            let note_entity = cx.new(|cx| {
                TextCard::new(
                    note.id,
                    id,
                    bounds,
                    note.text.clone(),
                    canvas.clone(),
                    window,
                    cx,
                )
            });
            note_entity.update(cx, |note, cx| note.set_view(zoom, offset_x, offset_y, cx));
            self.notes.entry(id).or_default().push((note.id, note_entity));
        }

        self.finish_store_change(cx);
        cx.notify();
    }
}

fn canvas_icon(path: &'static str) -> impl IntoElement {
    svg()
        .path(path)
        .w(px(16.0))
        .h(px(16.0))
        .text_color(rgb(COLOR_MUTED))
}

/// Scroll down moves the canvas content up, like a normal document.
fn pan_vertical(offset_y: f32, dy: f32) -> f32 {
    offset_y + dy
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
        let placing_text = self.placing_text;
        let submenu = self.menu.submenu;
        let agent_list = self.menu.agent_list;
        let agent_menu = self.agent_menu.clone();
        let agent_name_input = self.agent_name_input.clone();
        let dialog_error = self.error.clone();
        let agent_root = self
            .agent_menu
            .as_ref()
            .and_then(|menu| {
                self.store
                    .workspaces()
                    .iter()
                    .find(|workspace| workspace.id == menu.workspace_id)
            })
            .map(|workspace| workspace.root.to_string_lossy().into_owned())
            .unwrap_or_default();
        let drawer_width = self.drawer.read(cx).drawer_width();
        let context_menu = self.menu.context_menu.map(|position| {
            (
                (f32::from(position.x) - drawer_width).max(SPACE_NORMAL),
                f32::from(position.y).max(SPACE_NORMAL),
            )
        });
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
        let add_note = ComponentButton::new("add-text-card")
            .tooltip("Adicionar bloco de texto")
            .child(canvas_icon("note.svg"))
            .w(px(28.0))
            .h(px(28.0))
            .p_0()
            .rounded(px(8.0))
            .with_variant(ButtonVariant::Ghost);
        let add_note = if active.is_some() {
            add_note.on_click(cx.listener(|this, _, _, cx| this.begin_text_placement(cx)))
        } else {
            add_note.disabled(true)
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
        let notes: Vec<_> = active
            .and_then(|id| self.notes.get(&id))
            .map(|group| {
                group
                    .iter()
                    .map(|(_, note)| div().child(note.clone()))
                    .collect()
            })
            .unwrap_or_default();

        div()
            .size_full()
            .flex()
            .flex_row()
            .bg(rgb(COLOR_BASE))
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
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                                    window.prevent_default();
                                    this.placing = false;
                                    this.placing_text = false;
                                    this.close_agent_menu(cx);
                                    this.menu.open_context_menu(event.position);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            )
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| this.commit_all_bounds(cx)),
                            )
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
                                                rgb(COLOR_GRID),
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
                                                rgb(COLOR_GRID),
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
                            .children(notes)
                            .when(self.placing || self.placing_text, |area| {
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
                                                if this.placing_text {
                                                    this.place_text_at(event.position, window, cx);
                                                } else {
                                                    this.place_terminal(event.position, window, cx);
                                                }
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
                                                .bg(rgb(COLOR_ELEVATED))
                                                .border_1()
                                                .border_color(rgb(COLOR_SELECTION))
                                                .px_3()
                                                .py_1()
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(rgb(COLOR_FOREGROUND))
                                                        .child(if placing_text {
                                                            "Clique no canvas para posicionar o bloco de texto"
                                                        } else {
                                                            "Clique no canvas para posicionar o terminal"
                                                        }),
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
                            })
                            .when_some(context_menu, |area, (x, y)| {
                                area.child(
                                    div()
                                        .absolute()
                                        .left(px(0.0))
                                        .top(px(0.0))
                                        .size_full()
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(|this, _, _, cx| {
                                                this.menu.close();
                                                cx.notify();
                                            }),
                                        ),
                                )
                                .child(
                                    div()
                                        .absolute()
                                        .left(px(x))
                                        .top(px(y))
                                        .w(px(CONTEXT_MENU_WIDTH))
                                        .flex()
                                        .flex_col()
                                        .p(px(SPACE_TIGHT))
                                        .rounded(px(RADIUS_CONTROL))
                                        .bg(rgb(COLOR_ELEVATED))
                                        .border_1()
                                        .border_color(rgb(COLOR_BORDER))
                                        .child(
                                            div()
                                                .id("context-add")
                                                .h(px(CONTEXT_MENU_ITEM_HEIGHT))
                                                .flex()
                                                .items_center()
                                                .gap(px(SPACE_NORMAL))
                                                .px(px(SPACE_NORMAL))
                                                .rounded(px(RADIUS_CHIP))
                                                .hover(|style| style.bg(rgb(COLOR_BORDER)))
                                                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                                    if *hovered {
                                                        this.menu.submenu = true;
                                                        cx.notify();
                                                    }
                                                }))
                                                .on_mouse_down(
                                                    MouseButton::Left,
                                                    cx.listener(|this, _, _, cx| {
                                                        this.menu.submenu = true;
                                                        cx.notify();
                                                    }),
                                                )
                                                .child(
                                                    svg()
                                                        .path("plus.svg")
                                                        .w(px(ICON_SIZE))
                                                        .h(px(ICON_SIZE))
                                                        .text_color(rgb(COLOR_FOREGROUND)),
                                                )
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .text_color(rgb(COLOR_FOREGROUND))
                                                        .child("Adicionar"),
                                                ),
                                        ),
                                )
                                .when(submenu, |area| {
                                    area.child(
                                        div()
                                            .absolute()
                                            .left(px(x + CONTEXT_MENU_WIDTH + SUBMENU_GAP))
                                            .top(px(y))
                                            .w(px(CONTEXT_MENU_WIDTH))
                                            .flex()
                                            .flex_col()
                                            .p(px(SPACE_TIGHT))
                                            .rounded(px(RADIUS_CONTROL))
                                            .bg(rgb(COLOR_ELEVATED))
                                            .border_1()
                                            .border_color(rgb(COLOR_BORDER))
                                            .child(
                                                div()
                                                    .id("context-add-terminal-option")
                                                    .h(px(CONTEXT_MENU_ITEM_HEIGHT))
                                                    .flex()
                                                    .items_center()
                                                    .gap(px(SPACE_NORMAL))
                                                    .px(px(SPACE_NORMAL))
                                                    .rounded(px(RADIUS_CHIP))
                                                    .hover(|style| style.bg(rgb(COLOR_BORDER)))
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(|this, _, _, cx| {
                                                            this.begin_placement(cx);
                                                        }),
                                                    )
                                                    .child(
                                                        svg()
                                                            .path("terminal.svg")
                                                            .w(px(ICON_SIZE))
                                                            .h(px(ICON_SIZE))
                                                            .text_color(rgb(COLOR_FOREGROUND)),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .text_color(rgb(COLOR_FOREGROUND))
                                                            .child("Terminal"),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .id("context-add-text-option")
                                                    .h(px(CONTEXT_MENU_ITEM_HEIGHT))
                                                    .flex()
                                                    .items_center()
                                                    .gap(px(SPACE_NORMAL))
                                                    .px(px(SPACE_NORMAL))
                                                    .rounded(px(RADIUS_CHIP))
                                                    .hover(|style| style.bg(rgb(COLOR_BORDER)))
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(|this, _, _, cx| {
                                                            this.begin_text_placement(cx);
                                                        }),
                                                    )
                                                    .child(
                                                        svg()
                                                            .path("note.svg")
                                                            .w(px(ICON_SIZE))
                                                            .h(px(ICON_SIZE))
                                                            .text_color(rgb(COLOR_FOREGROUND)),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .text_color(rgb(COLOR_FOREGROUND))
                                                            .child("Bloco de texto"),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .id("context-add-agent-option")
                                                    .h(px(CONTEXT_MENU_ITEM_HEIGHT))
                                                    .flex()
                                                    .items_center()
                                                    .gap(px(SPACE_NORMAL))
                                                    .px(px(SPACE_NORMAL))
                                                    .rounded(px(RADIUS_CHIP))
                                                    .hover(|style| style.bg(rgb(COLOR_BORDER)))
                                                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                                        if *hovered {
                                                            this.menu.agent_list = true;
                                                            cx.notify();
                                                        }
                                                    }))
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(|this, _, _, cx| {
                                                            this.menu.agent_list = true;
                                                            cx.notify();
                                                        }),
                                                    )
                                                    .child(
                                                        svg()
                                                            .path(agent_icon_path(AgentKind::Claude))
                                                            .w(px(ICON_SIZE))
                                                            .h(px(ICON_SIZE))
                                                            .text_color(rgb(COLOR_FOREGROUND)),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .text_color(rgb(COLOR_FOREGROUND))
                                                            .child("Agente"),
                                                    ),
                                            ),
                                    )
                                })
                            })
                            .when(agent_list, |area| {
                                area.when_some(context_menu, |area, (x, y)| {
                                    area.child(
                                        div()
                                            .absolute()
                                            .left(px(x + 2.0 * (CONTEXT_MENU_WIDTH + SUBMENU_GAP)))
                                            .top(px(y))
                                            .w(px(CONTEXT_MENU_WIDTH))
                                            .flex()
                                            .flex_col()
                                            .p(px(SPACE_TIGHT))
                                            .rounded(px(RADIUS_CONTROL))
                                            .bg(rgb(COLOR_ELEVATED))
                                            .border_1()
                                            .border_color(rgb(COLOR_BORDER))
                                            .children(AgentKind::ALL.into_iter().map(|kind| {
                                                div()
                                                    .id(("context-agent-option", kind as u64))
                                                    .h(px(CONTEXT_MENU_ITEM_HEIGHT))
                                                    .flex()
                                                    .items_center()
                                                    .gap(px(SPACE_NORMAL))
                                                    .px(px(SPACE_NORMAL))
                                                    .rounded(px(RADIUS_CHIP))
                                                    .hover(|style| style.bg(rgb(COLOR_BORDER)))
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(move |this, _, _, cx| {
                                                            this.begin_agent_placement(kind, cx);
                                                        }),
                                                    )
                                                    .child(
                                                        svg()
                                                            .path(agent_icon_path(kind))
                                                            .w(px(ICON_SIZE))
                                                            .h(px(ICON_SIZE))
                                                            .text_color(rgb(agent_color(kind))),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .text_color(rgb(COLOR_FOREGROUND))
                                                            .child(kind.label()),
                                                    )
                                            })),
                                    )
                                })
                            })
                            .when_some(agent_menu.clone(), |area, menu| {
                                let x = (f32::from(menu.position.x) - drawer_width).max(SPACE_NORMAL);
                                let y = f32::from(menu.position.y).max(SPACE_NORMAL);
                                let name_input = agent_name_input.clone();
                                let root_text = agent_root.clone();
                                let error = dialog_error.clone();
                                area.child(
                                    div()
                                        .absolute()
                                        .left(px(0.0))
                                        .top(px(0.0))
                                        .size_full()
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(|this, _, _, cx| this.close_agent_menu(cx)),
                                        ),
                                )
                                .child(if menu.editing {
                                    let cancel = ComponentButton::new("agent-edit-cancel")
                                        .label("Cancelar")
                                        .with_variant(ButtonVariant::Ghost)
                                        .on_click(cx.listener(|this, _, _, cx| this.close_agent_menu(cx)));
                                    let save = ComponentButton::new("agent-edit-save")
                                        .label("Salvar")
                                        .with_variant(ButtonVariant::Primary)
                                        .on_click(cx.listener(|this, _, _, cx| this.save_agent_edits(cx)));
                                    div()
                                        .absolute()
                                        .left(px(x))
                                        .top(px(y))
                                        .w(px(AGENT_DIALOG_WIDTH))
                                        .flex()
                                        .flex_col()
                                        .gap_3()
                                        .p(px(SPACE_PANEL))
                                        .rounded(px(RADIUS_DIALOG))
                                        .border_1()
                                        .border_color(rgb(COLOR_BORDER))
                                        .bg(rgb(COLOR_PANEL))
                                        .text_color(rgb(COLOR_FOREGROUND))
                                        .child(
                                            div()
                                                .text_lg()
                                                .text_color(rgb(COLOR_FOREGROUND))
                                                .child("Editar agente"),
                                        )
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(rgb(COLOR_MUTED))
                                                .child("Nome do terminal"),
                                        )
                                        .children(name_input.iter().map(|state| {
                                            Input::new(state)
                                                .w_full()
                                                .rounded(px(RADIUS_CONTROL))
                                                .bg(rgb(COLOR_ELEVATED))
                                                .border_color(rgb(COLOR_BORDER))
                                        }))
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(rgb(COLOR_MUTED))
                                                .child("Agente"),
                                        )
                                        .child(
                                            div().flex().gap_2().children(
                                                AgentKind::ALL.into_iter().map(|kind| {
                                                    let selected = kind == menu.kind;
                                                    ComponentButton::new(("agent-kind", kind as u64))
                                                        .child(
                                                            svg()
                                                                .path(agent_icon_path(kind))
                                                                .w(px(ICON_SIZE))
                                                                .h(px(ICON_SIZE))
                                                                .text_color(rgb(agent_color(kind))),
                                                        )
                                                        .w(px(AGENT_TILE_SIZE))
                                                        .h(px(AGENT_TILE_SIZE))
                                                        .p_0()
                                                        .rounded(px(RADIUS_CONTROL))
                                                        .border_1()
                                                        .border_color(rgb(if selected {
                                                            agent_color(kind)
                                                        } else {
                                                            COLOR_BORDER
                                                        }))
                                                        .bg(rgb(if selected {
                                                            COLOR_ELEVATED
                                                        } else {
                                                            COLOR_PANEL
                                                        }))
                                                        .with_variant(ButtonVariant::Ghost)
                                                        .on_click(cx.listener(move |this, _, _, cx| {
                                                            let (ws, id, name) = {
                                                                let Some(menu) = this.agent_menu.as_mut() else {
                                                                    return;
                                                                };
                                                                menu.kind = kind;
                                                                (menu.workspace_id, menu.card_id, menu.name.clone())
                                                            };
                                                            this.apply_agent_to_card(ws, id, kind, name, cx);
                                                            cx.notify();
                                                        }))
                                                }),
                                            ),
                                        )
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(rgb(COLOR_MUTED))
                                                .child("Comando"),
                                        )
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(rgb(COLOR_FOREGROUND))
                                                .child(menu.kind.command()),
                                        )
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(rgb(COLOR_MUTED))
                                                .child("Raiz"),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(COLOR_MUTED))
                                                .child(root_text),
                                        )
                                        .when_some(error, |dialog, error| {
                                            dialog
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(rgb(COLOR_DANGER))
                                                        .child(error),
                                                )
                                        })
                                        .child(
                                            div()
                                                .flex()
                                                .justify_end()
                                                .gap_2()
                                                .child(cancel)
                                                .child(save),
                                        )
                                } else {
                                    div()
                                        .absolute()
                                        .left(px(x))
                                        .top(px(y))
                                        .w(px(CONTEXT_MENU_WIDTH))
                                        .flex()
                                        .flex_col()
                                        .p(px(SPACE_TIGHT))
                                        .rounded(px(RADIUS_CONTROL))
                                        .bg(rgb(COLOR_ELEVATED))
                                        .border_1()
                                        .border_color(rgb(COLOR_BORDER))
                                        .child(
                                            div()
                                                .id("agent-menu-edit")
                                                .h(px(CONTEXT_MENU_ITEM_HEIGHT))
                                                .flex()
                                                .items_center()
                                                .gap(px(SPACE_NORMAL))
                                                .px(px(SPACE_NORMAL))
                                                .rounded(px(RADIUS_CHIP))
                                                .hover(|style| style.bg(rgb(COLOR_BORDER)))
                                                .on_mouse_down(
                                                    MouseButton::Left,
                                                    cx.listener(|this, _, window, cx| {
                                                        this.begin_agent_edit(window, cx);
                                                    }),
                                                )
                                                .child(
                                                    svg()
                                                        .path("edit.svg")
                                                        .w(px(ICON_SIZE))
                                                        .h(px(ICON_SIZE))
                                                        .text_color(rgb(COLOR_FOREGROUND)),
                                                )
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .text_color(rgb(COLOR_FOREGROUND))
                                                        .child("Editar"),
                                                ),
                                        )
                                })
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
                            .bg(rgb(COLOR_ELEVATED))
                            .border_1()
                            .border_color(rgb(COLOR_BORDER))
                            .child(add)
                            .child(add_note)
                            .child(remove),
                    )
                    .when(active.is_none(), |column| {
                        column.child(
                            div()
                                .text_xs()
                                .text_color(rgb(COLOR_MUTED))
                                .child("Crie um espaço de trabalho para adicionar terminais."),
                        )
                    })
                    .children(self.error.iter().map(|message| {
                        div()
                            .rounded(px(8.0))
                            .bg(rgb(COLOR_ELEVATED))
                            .border_1()
                            .border_color(rgb(COLOR_BORDER))
                            .p_2()
                            .text_xs()
                            .text_color(rgb(COLOR_DANGER))
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
                    .bg(rgb(COLOR_ELEVATED))
                    .border_1()
                    .border_color(rgb(COLOR_BORDER))
                    .px_3()
                    .py_1()
                    .child(div().text_xs().text_color(rgb(COLOR_MUTED)).child("Zoom"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(COLOR_FOREGROUND))
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
    fn closing_context_menu_clears_nested_panels() {
        let mut menu = MenuState::default();
        menu.open_context_menu(Point::new(px(40.0), px(60.0)));
        menu.submenu = true;
        menu.agent_list = true;

        menu.close();

        assert_eq!(menu, MenuState::default());
    }

    #[test]
    fn reopening_context_menu_does_not_restore_agent_list() {
        let mut menu = MenuState::default();
        menu.open_context_menu(Point::new(px(10.0), px(20.0)));
        menu.submenu = true;
        menu.agent_list = true;

        menu.open_context_menu(Point::new(px(30.0), px(40.0)));

        assert_eq!(menu.context_menu, Some(Point::new(px(30.0), px(40.0))));
        assert!(!menu.submenu);
        assert!(!menu.agent_list);
        assert_eq!(menu.placing_agent, None);
    }

    #[test]
    fn starting_placement_clears_menu_and_remembers_the_agent() {
        let mut menu = MenuState::default();
        menu.open_context_menu(Point::new(px(1.0), px(2.0)));
        menu.agent_list = true;

        menu.begin_placement(Some(AgentKind::Codex));

        assert_eq!(menu.context_menu, None);
        assert!(!menu.submenu);
        assert!(!menu.agent_list);
        assert_eq!(menu.placing_agent, Some(AgentKind::Codex));
    }

    #[test]
    fn note_selection_tracks_only_the_current_workspace_and_note() {
        let mut selected = NoteSelection::default();
        assert_eq!(selected.get(), None);

        selected.select(1, 7);
        assert_eq!(selected.get(), Some((1, 7)));
        selected.clear_note(2, 7);
        selected.clear_note(1, 8);
        assert_eq!(selected.get(), Some((1, 7)));

        selected.select(2, 9);
        selected.clear_workspace(1);
        assert_eq!(selected.get(), Some((2, 9)));
        selected.clear_workspace(2);
        assert_eq!(selected.get(), None);

        selected.select(3, 4);
        selected.clear();
        assert_eq!(selected.get(), None);
    }

    #[test]
    fn zoom_scroll_direction_is_inverted() {
        assert!(zoom_factor(-110.0) < 1.0);
        assert!(zoom_factor(110.0) > 1.0);
    }

    #[test]
    fn vertical_scroll_moves_canvas_down() {
        assert_eq!(pan_vertical(100.0, 20.0), 120.0);
        assert_eq!(pan_vertical(100.0, -20.0), 80.0);
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
            WorkspaceColor::Brass,
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
    fn persisted_snapshot_keeps_cards_and_notes() {
        let mut store = WorkspaceStore::new();
        let workspace = store.add(
            "A".into(),
            WorkspaceIcon::Star,
            std::env::current_dir().unwrap(),
        );
        let terminal = active_spawn_target(&mut store).unwrap().1;
        let bounds = CardBounds {
            x: 10.0,
            y: 20.0,
            width: 640.0,
            height: 400.0,
        };
        let canvas = &mut store.active_mut().unwrap().canvas;
        canvas.set_terminal_bounds(terminal, bounds);
        let note = canvas.add_note("guardado".into(), bounds);

        let snapshot = persisted_snapshot(&store);

        assert_eq!(snapshot.active(), Some(workspace.id));
        assert_eq!(snapshot.workspaces()[0].canvas.terminals()[0].bounds, Some(bounds));
        assert_eq!(snapshot.workspaces()[0].canvas.notes()[0].text, note.text);
        assert_eq!(snapshot, store);
    }

    #[test]
    fn persisted_snapshot_keeps_agents() {
        let mut store = WorkspaceStore::new();
        let workspace = store.add("A".into(), WorkspaceIcon::Star, PathBuf::from("/a"));
        let bounds = CardBounds { x: 8.0, y: 12.0, width: 640.0, height: 400.0 };
        let agent = store
            .workspace_mut(workspace.id)
            .unwrap()
            .canvas
            .add_agent(AgentKind::Pi, bounds);

        let snapshot = persisted_snapshot(&store);

        assert_eq!(snapshot.workspaces()[0].canvas.agents(), &[agent]);
        assert_eq!(snapshot, store);
    }

    #[test]
    fn card_bounds_survive_pixel_conversion() {
        let original = Bounds::new(Point::new(px(-12.5), px(30.0)), Size::new(px(320.0), px(220.0)));
        let restored = pixel_bounds(card_bounds(&original));
        assert_eq!(
            (f32::from(restored.origin.x), f32::from(restored.origin.y)),
            (-12.5, 30.0)
        );
        assert_eq!(
            (f32::from(restored.size.width), f32::from(restored.size.height)),
            (320.0, 220.0)
        );
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

    #[test]
    fn active_agent_target_allocates_a_removable_card() {
        let mut store = WorkspaceStore::new();
        let workspace = store.add("A".into(), WorkspaceIcon::Star, std::env::current_dir().unwrap());
        let bounds = CardBounds { x: 0.0, y: 0.0, width: 640.0, height: 400.0 };
        let mut groups = groups_for(&store);

        let (workspace_id, id, root) =
            active_agent_target(&mut store, AgentKind::Pi, bounds).unwrap();
        assert_eq!(workspace_id, workspace.id);
        assert_eq!(root, workspace.root);
        assert_eq!(id, 1);
        assert_eq!(
            store.workspace_mut(workspace_id).unwrap().canvas.agents()[0].kind,
            AgentKind::Pi
        );

        assert!(remove_terminal_state(&mut store, &mut groups, workspace_id, id));
        assert!(store.workspace_mut(workspace_id).unwrap().canvas.agents().is_empty());
        assert_eq!(
            active_agent_target(&mut store, AgentKind::Omp, bounds).unwrap().1,
            2
        );
    }

    #[test]
    fn unload_workspace_keeps_row_and_saved_cards() {
        let mut store = WorkspaceStore::new();
        let first = store.add("A".into(), WorkspaceIcon::Star, PathBuf::from("/a"));
        let second = store.add("B".into(), WorkspaceIcon::Bolt, PathBuf::from("/b"));
        let canvas = &mut store.workspace_mut(first.id).unwrap().canvas;
        canvas.add();
        canvas.add_note("nota".into(), CardBounds {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        });
        store.workspace_mut(second.id).unwrap().canvas.add();

        assert!(unload_workspace(&mut store, first.id));

        assert_eq!(store.workspaces().len(), 2);
        let unloaded = store.workspace_mut(first.id).unwrap();
        assert!(unloaded.unloaded);
        assert_eq!(unloaded.canvas.selected(), None);
        assert_eq!(unloaded.canvas.terminals().len(), 1);
        assert_eq!(unloaded.canvas.notes()[0].text, "nota");
        let other = store.workspace_mut(second.id).unwrap();
        assert!(other.unloaded);
        assert_eq!(other.canvas.terminals().len(), 1);
        assert!(!unload_workspace(&mut store, 999));
    }

    #[test]
    fn clicking_unloaded_workspace_loads_saved_layout_and_reselects_it() {
        let mut store = WorkspaceStore::new();
        let first = store.add("A".into(), WorkspaceIcon::Star, std::env::current_dir().unwrap());
        let second = store.add("B".into(), WorkspaceIcon::Bolt, PathBuf::from("/b"));
        let bounds = CardBounds {
            x: 12.0,
            y: 34.0,
            width: 640.0,
            height: 400.0,
        };
        {
            let canvas = &mut store.workspace_mut(first.id).unwrap().canvas;
            let terminal = canvas.add();
            canvas.set_terminal_bounds(terminal.id, bounds);
            canvas.add_note("nota".into(), bounds);
            canvas.add_agent(AgentKind::Claude, bounds);
        }
        store.workspace_mut(first.id).unwrap().unload();
        store.select(second.id);

        let layout = begin_workspace_load(&mut store, first.id).expect("unloaded workspace");

        assert!(layout.root_ok);
        assert_eq!(layout.terminals.len(), 1);
        assert_eq!(layout.terminals[0].bounds, Some(bounds));
        assert_eq!(layout.notes[0].text, "nota");
        assert_eq!(layout.agents.len(), 1);
        assert_eq!(layout.agents[0].kind, AgentKind::Claude);
        assert_eq!(store.active(), Some(first.id));
        assert!(!store.workspaces()[0].unloaded);
        assert!(store.workspaces()[1].unloaded);

        // Clicking an already-loaded workspace must not rebuild or duplicate cards.
        assert!(begin_workspace_load(&mut store, first.id).is_none());
        store.select(first.id);
        assert!(!store.workspaces()[0].unloaded);
        assert_eq!(store.workspaces()[0].canvas.terminals().len(), 1);
        assert_eq!(store.workspaces()[0].canvas.agents().len(), 1);
    }

    #[test]
    fn startup_marks_all_workspaces_unloaded_without_dropping_saved_cards() {
        let mut store = WorkspaceStore::new();
        let workspace = store.add("A".into(), WorkspaceIcon::Star, PathBuf::from("/a"));
        store.workspace_mut(workspace.id).unwrap().mark_loaded();
        store.workspace_mut(workspace.id).unwrap().canvas.add();

        store.mark_all_unloaded();

        assert!(store.workspaces()[0].unloaded);
        assert_eq!(store.workspaces()[0].canvas.terminals().len(), 1);
    }

    #[test]
    fn serde_accepts_saved_configs_without_unloaded_field() {
        let json = r#"{"id":1,"name":"Antigo","icon":"Folder","root":"/a","color":"Brass","invalid_root":false,"canvas":{"terminals":[{"id":3,"title":"Terminal 3","exited":false}],"notes":[{"id":4,"text":"nota","bounds":{"x":1.0,"y":2.0,"width":3.0,"height":4.0}}],"selected":3,"next_id":5}}"#;
        let workspace: WorkspaceState = serde_json::from_str(json).unwrap();

        assert!(!workspace.unloaded);
        assert_eq!(workspace.canvas.terminals().len(), 1);
        assert_eq!(workspace.canvas.notes()[0].text, "nota");
        assert!(serde_json::to_string(&workspace).unwrap().contains("\"unloaded\":false"));
    }

    #[test]
    fn update_workspace_changes_details_without_touching_cards() {
        let mut store = WorkspaceStore::new();
        let workspace = store.add("Antigo".into(), WorkspaceIcon::Star, PathBuf::from("/a"));
        store.workspace_mut(workspace.id).unwrap().canvas.add();
        let root = std::env::current_dir().unwrap();

        update_workspace(
            &mut store,
            workspace.id,
            "  Novo  ".into(),
            WorkspaceIcon::Globe,
            WorkspaceColor::Green,
            root.clone(),
        )
        .unwrap();

        let updated = store.workspace_mut(workspace.id).unwrap();
        assert_eq!(updated.name, "Novo");
        assert_eq!(updated.icon, WorkspaceIcon::Globe);
        assert_eq!(updated.color, WorkspaceColor::Green);
        assert_eq!(updated.root, root);
        assert!(!updated.invalid_root);
        assert_eq!(updated.canvas.terminals().len(), 1);
        assert!(update_workspace(
            &mut store,
            workspace.id,
            "Invalido".into(),
            WorkspaceIcon::Star,
            WorkspaceColor::Brass,
            PathBuf::from("/nao/existe"),
        )
        .is_err());
    }
}

#[cfg(test)]
mod card_gap_tests {
    use super::*;

    #[test]
    fn adjacent_origin_leaves_one_pixel_between_cards() {
        let last = Bounds::new(
            Point::new(px(10.0), px(20.0)),
            Size::new(px(CARD_WIDTH), px(CARD_HEIGHT)),
        );
        let next = adjacent_origin(last);
        assert_eq!((f32::from(next.x), f32::from(next.y)), (10.0 + CARD_WIDTH + 1.0, 20.0));
    }
}
