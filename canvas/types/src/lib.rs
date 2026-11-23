//! Framework-free types shared by the canvas host and its terminal cards.
//!
//! [`CanvasState`] tracks terminals, selection and id allocation; the host owns
//! the framework-specific entities separately.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn default_next_id() -> u64 {
    1
}

/// Identifier of a terminal on the canvas.
pub type TerminalId = u64;

/// Identifier of a workspace on the canvas.
pub type WorkspaceId = u64;

/// Saved pixel geometry of a canvas card, independent of the UI framework.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CardBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Persisted text block: its content and its canvas geometry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoteState {
    pub id: u64,
    pub text: String,
    pub bounds: CardBounds,
}

/// Logical state of a single terminal card.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerminalState {
    pub id: TerminalId,
    pub title: String,
    pub exited: bool,
    /// Absent in configs written before geometry was persisted.
    #[serde(default)]
    pub bounds: Option<CardBounds>,
}

/// Framework-free canvas state: terminals, the selected terminal and id allocation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanvasState {
    #[serde(default)]
    terminals: Vec<TerminalState>,
    #[serde(default)]
    notes: Vec<NoteState>,
    #[serde(default)]
    selected: Option<TerminalId>,
    #[serde(default = "default_next_id")]
    next_id: TerminalId,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self::new()
    }
}
impl CanvasState {
    pub fn new() -> Self {
        Self {
            terminals: Vec::new(),
            notes: Vec::new(),
            selected: None,
            next_id: 1,
        }
    }

    pub fn add(&mut self) -> TerminalState {
        let id = self.next_id;
        self.next_id += 1;
        let terminal = TerminalState {
            id,
            title: format!("Terminal {id}"),
            exited: false,
            bounds: None,
        };
        self.terminals.push(terminal.clone());
        self.selected = Some(id);
        terminal
    }

    pub fn remove(&mut self, id: TerminalId) -> Option<TerminalState> {
        let index = self.terminals.iter().position(|terminal| terminal.id == id)?;
        let removed = self.terminals.remove(index);
        if self.selected == Some(id) {
            self.selected = None;
        }
        Some(removed)
    }

    pub fn select(&mut self, id: TerminalId) {
        self.selected = Some(id);
    }

    pub fn selected(&self) -> Option<TerminalId> {
        self.selected
    }

    pub fn mark_exited(&mut self, id: TerminalId) {
        self.set_terminal_exited(id, true);
    }

    pub fn set_terminal_exited(&mut self, id: TerminalId, exited: bool) {
        if let Some(terminal) = self.terminals.iter_mut().find(|terminal| terminal.id == id) {
            terminal.exited = exited;
        }
    }

    /// Record a terminal's saved geometry; returns true when it changed.
    pub fn set_terminal_bounds(&mut self, id: TerminalId, bounds: CardBounds) -> bool {
        let Some(terminal) = self.terminals.iter_mut().find(|terminal| terminal.id == id) else {
            return false;
        };
        if terminal.bounds == Some(bounds) {
            return false;
        }
        terminal.bounds = Some(bounds);
        true
    }

    pub fn terminals(&self) -> &[TerminalState] {
        &self.terminals
    }

    /// Allocate a note from the same id space as terminals.
    pub fn add_note(&mut self, text: String, bounds: CardBounds) -> NoteState {
        let id = self.next_id;
        self.next_id += 1;
        let note = NoteState { id, text, bounds };
        self.notes.push(note.clone());
        note
    }

    pub fn set_note_text(&mut self, id: u64, text: String) -> bool {
        let Some(note) = self.notes.iter_mut().find(|note| note.id == id) else {
            return false;
        };
        if note.text == text {
            return false;
        }
        note.text = text;
        true
    }

    pub fn set_note_bounds(&mut self, id: u64, bounds: CardBounds) -> bool {
        let Some(note) = self.notes.iter_mut().find(|note| note.id == id) else {
            return false;
        };
        if note.bounds == bounds {
            return false;
        }
        note.bounds = bounds;
        true
    }

    pub fn remove_note(&mut self, id: u64) -> Option<NoteState> {
        let index = self.notes.iter().position(|note| note.id == id)?;
        Some(self.notes.remove(index))
    }

    pub fn notes(&self) -> &[NoteState] {
        &self.notes
    }

    /// Forget the live selection without discarding the saved layout: unloading
    /// ends the running shells, but clicking the workspace restores its cards.
    pub fn unload(&mut self) {
        self.selected = None;
    }


    /// Keep the shared allocator above every id found in a restored config.
    pub fn reserve_id_above(&mut self, id: u64) {
        self.next_id = self.next_id.max(id.saturating_add(1));
    }

    pub fn next_id(&self) -> u64 {
        self.next_id
    }
}

/// One of the fixed icons available to a workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceIcon {
    Star,
    Folder,
    Bolt,
    Code,
    Globe,
}

/// Accent chosen for a workspace; kept framework-free and persisted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceColor {
    Brass,
    Steel,
    Red,
    Green,
}

impl Default for WorkspaceColor {
    fn default() -> Self {
        Self::Brass
    }
}

/// Persisted state of one workspace, including its own canvas.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub id: WorkspaceId,
    pub name: String,
    pub icon: WorkspaceIcon,
    pub root: PathBuf,
    #[serde(default)]
    pub color: WorkspaceColor,
    #[serde(default)]
    pub invalid_root: bool,
    #[serde(default)]
    pub canvas: CanvasState,
    /// True while saved cards have no live entities (startup or after "Descarregar").
    #[serde(default)]
    pub unloaded: bool,
}

impl WorkspaceState {
    /// Mark the workspace unloaded, keeping its saved cards for the next click.
    pub fn unload(&mut self) {
        self.canvas.unload();
        self.unloaded = true;
    }

    /// Used at startup: the saved cards exist, but no live entity has been created.
    pub fn mark_unloaded(&mut self) {
        self.unloaded = true;
    }

    /// A new terminal or note brings the workspace back to life.
    pub fn mark_loaded(&mut self) {
        self.unloaded = false;
    }

    /// Apply the values edited in the shared workspace form.
    pub fn update_details(
        &mut self,
        name: String,
        icon: WorkspaceIcon,
        color: WorkspaceColor,
        root: PathBuf,
    ) {
        self.name = name;
        self.icon = icon;
        self.color = color;
        self.root = root;
        self.invalid_root = false;
    }
}

/// All workspaces plus the current selection and the id allocator.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceStore {
    #[serde(default)]
    workspaces: Vec<WorkspaceState>,
    #[serde(default)]
    active: Option<WorkspaceId>,
    #[serde(default = "default_next_id")]
    next_id: WorkspaceId,
}

impl WorkspaceStore {
    pub fn new() -> Self {
        Self {
            workspaces: Vec::new(),
            active: None,
            next_id: 1,
        }
    }

    pub fn add(&mut self, name: String, icon: WorkspaceIcon, root: PathBuf) -> WorkspaceState {
        self.add_with_color(name, icon, WorkspaceColor::Brass, root)
    }

    pub fn add_with_color(
        &mut self,
        name: String,
        icon: WorkspaceIcon,
        color: WorkspaceColor,
        root: PathBuf,
    ) -> WorkspaceState {
        let id = self.next_id;
        self.next_id += 1;
        let workspace = WorkspaceState {
            id,
            name,
            icon,
            root,
            invalid_root: false,
            color,
            canvas: CanvasState::new(),
            unloaded: true,
        };
        self.workspaces.push(workspace.clone());
        self.active = Some(id);
        workspace
    }

    pub fn remove(&mut self, id: WorkspaceId) -> Option<WorkspaceState> {
        let index = self.workspaces.iter().position(|workspace| workspace.id == id)?;
        let removed = self.workspaces.remove(index);
        if self.active == Some(id) {
            self.active = self.workspaces.first().map(|workspace| workspace.id);
        }
        Some(removed)
    }

    pub fn select(&mut self, id: WorkspaceId) {
        if self.workspaces.iter().any(|workspace| workspace.id == id) {
            self.active = Some(id);
        }
    }

    /// Startup keeps every saved layout but drops all live cards.
    pub fn mark_all_unloaded(&mut self) {
        for workspace in &mut self.workspaces {
            workspace.mark_unloaded();
        }
    }

    pub fn active(&self) -> Option<WorkspaceId> {
        self.active
    }

    pub fn active_mut(&mut self) -> Option<&mut WorkspaceState> {
        let id = self.active?;
        self.workspaces.iter_mut().find(|workspace| workspace.id == id)
    }

    /// Mutable access to a workspace by id, including inactive ones.
    pub fn workspace_mut(&mut self, id: WorkspaceId) -> Option<&mut WorkspaceState> {
        self.workspaces.iter_mut().find(|workspace| workspace.id == id)
    }

    pub fn workspaces(&self) -> &[WorkspaceState] {
        &self.workspaces
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_generates_sequential_ids_and_selects() {
        let mut state = CanvasState::new();
        let a = state.add();
        let b = state.add();
        assert_eq!((a.id, b.id), (1, 2));
        assert_eq!((a.title.as_str(), b.title.as_str()), ("Terminal 1", "Terminal 2"));
        assert_eq!(state.selected(), Some(2));
    }

    #[test]
    fn remove_clears_selection_only_when_removing_selected() {
        let mut state = CanvasState::new();
        let a = state.add();
        let _b = state.add();
        state.select(a.id);
        state.remove(a.id);
        assert_eq!(state.selected(), None);
        assert_eq!(state.terminals().len(), 1);
    }

    #[test]
    fn mark_exited_preserves_terminal_and_output_flag() {
        let mut state = CanvasState::new();
        let a = state.add();
        state.mark_exited(a.id);
        assert!(state.terminals()[0].exited);
        assert_eq!(state.terminals().len(), 1);
    }

    #[test]
    fn notes_keep_text_and_bounds_through_mutation_and_removal() {
        let mut state = CanvasState::new();
        let bounds = CardBounds {
            x: 12.5,
            y: -8.0,
            width: 320.0,
            height: 220.0,
        };
        let note = state.add_note("primeira linha\nsegunda linha".into(), bounds);
        assert_eq!(note.bounds, bounds);
        assert_eq!(state.notes()[0].text, "primeira linha\nsegunda linha");

        assert!(state.set_note_text(note.id, "texto novo".into()));
        assert!(!state.set_note_text(note.id, "texto novo".into()));
        let moved = CardBounds { x: 40.0, ..bounds };
        assert!(state.set_note_bounds(note.id, moved));
        assert_eq!(state.notes()[0].bounds, moved);

        assert_eq!(state.remove_note(note.id).map(|note| note.text), Some("texto novo".into()));
        assert!(state.notes().is_empty());
        assert_eq!(state.remove_note(note.id), None);
    }

    #[test]
    fn terminal_and_note_ids_share_one_allocator() {
        let mut state = CanvasState::new();
        let terminal = state.add();
        let note = state.add_note("oi".into(), CardBounds {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        });
        let next = state.add();

        assert_eq!((terminal.id, note.id, next.id), (1, 2, 3));
        assert_eq!((terminal.bounds, note.id), (None, 2));

        let mut restored = CanvasState::new();
        restored.reserve_id_above(17);
        assert_eq!(restored.next_id(), 18);
        let after = restored.add_note("a".into(), note.bounds);
        assert_eq!(after.id, 18);
    }

    #[test]
    fn terminal_bounds_are_settable_and_exited_flag_is_reversible() {
        let mut state = CanvasState::new();
        let terminal = state.add();
        let bounds = CardBounds {
            x: 1.0,
            y: 2.0,
            width: 640.0,
            height: 400.0,
        };
        assert!(state.set_terminal_bounds(terminal.id, bounds));
        assert!(!state.set_terminal_bounds(terminal.id, bounds));
        assert_eq!(state.terminals()[0].bounds, Some(bounds));
        state.mark_exited(terminal.id);
        state.set_terminal_exited(terminal.id, false);
        assert!(!state.terminals()[0].exited);
    }
}

#[cfg(test)]
mod workspace_tests {
    use super::*;
    use std::path::PathBuf;

    fn assert_serde<T: serde::Serialize + serde::de::DeserializeOwned>() {}

    #[test]
    fn add_generates_sequential_ids_and_activates_new_workspace() {
        let mut store = WorkspaceStore::new();
        assert_eq!(store.active(), None);

        let a = store.add("AURI".into(), WorkspaceIcon::Star, PathBuf::from("/tmp/auri"));
        let b = store.add("Repo".into(), WorkspaceIcon::Code, PathBuf::from("/tmp/repo"));

        assert_eq!((a.id, b.id), (1, 2));
        assert_eq!(store.active(), Some(b.id));
        assert_eq!(store.workspaces().len(), 2);
        assert_eq!(a.icon, WorkspaceIcon::Star);
        assert_eq!(b.root, PathBuf::from("/tmp/repo"));
        assert!(!a.invalid_root);
        assert!(a.unloaded);
        assert!(a.canvas.terminals().is_empty());
    }

    #[test]
    fn removing_active_workspace_activates_first_remaining_then_none() {
        let mut store = WorkspaceStore::new();
        let a = store.add("A".into(), WorkspaceIcon::Star, PathBuf::from("/a"));
        let b = store.add("B".into(), WorkspaceIcon::Bolt, PathBuf::from("/b"));

        assert_eq!(store.remove(b.id).map(|ws| ws.id), Some(b.id));
        assert_eq!(store.active(), Some(a.id));
        assert_eq!(store.remove(a.id).map(|ws| ws.id), Some(a.id));
        assert_eq!(store.active(), None);
        assert!(store.workspaces().is_empty());
        assert_eq!(store.remove(a.id), None);
    }

    #[test]
    fn removing_inactive_workspace_keeps_selection_and_active_mut_works() {
        let mut store = WorkspaceStore::new();
        let a = store.add("A".into(), WorkspaceIcon::Star, PathBuf::from("/a"));
        let b = store.add("B".into(), WorkspaceIcon::Globe, PathBuf::from("/b"));
        store.select(a.id);

        assert_eq!(store.remove(b.id).map(|ws| ws.id), Some(b.id));
        assert_eq!(store.active(), Some(a.id));
        store.active_mut().expect("active workspace").invalid_root = true;
        assert!(store.workspaces()[0].invalid_root);
        assert_eq!(store.remove(999), None);
    }

    #[test]
    fn invalid_root_and_store_are_serializable() {
        assert_serde::<WorkspaceIcon>();
        assert_serde::<WorkspaceState>();
        assert_serde::<WorkspaceStore>();

        let mut state = WorkspaceState {
            id: 7,
            name: "Quebrado".into(),
            icon: WorkspaceIcon::Folder,
            root: PathBuf::from("/nao/existe"),
            color: WorkspaceColor::Red,
            invalid_root: true,
            canvas: CanvasState::new(),
            unloaded: false,
        };
        state.canvas.add();
        assert!(state.invalid_root);
        assert_eq!(state.canvas.terminals().len(), 1);
    }

    #[test]
    fn unloading_keeps_saved_layout_and_marking_loaded_clears_the_flag() {
        let mut store = WorkspaceStore::new();
        let mut workspace = store.add("A".into(), WorkspaceIcon::Star, PathBuf::from("/a"));
        workspace.mark_loaded();
        let terminal = workspace.canvas.add();
        let bounds = CardBounds {
            x: 5.0,
            y: 6.0,
            width: 10.0,
            height: 10.0,
        };
        workspace.canvas.set_terminal_bounds(terminal.id, bounds);
        let note = workspace.canvas.add_note("nota".into(), bounds);
        let next_id = workspace.canvas.next_id();

        workspace.unload();

        assert!(workspace.unloaded);
        assert_eq!(workspace.canvas.selected(), None);
        assert_eq!(workspace.canvas.terminals()[0].bounds, Some(bounds));
        assert_eq!(workspace.canvas.notes()[0].text, "nota");
        assert_eq!(workspace.canvas.next_id(), next_id);

        workspace.mark_loaded();
        assert!(!workspace.unloaded);
        assert_eq!(note.bounds, bounds);
    }

    #[test]
    fn new_workspaces_start_unloaded_and_startup_marks_them_unloaded() {
        let mut store = WorkspaceStore::new();
        let first = store.add("A".into(), WorkspaceIcon::Star, PathBuf::from("/a"));
        let second = store.add("B".into(), WorkspaceIcon::Bolt, PathBuf::from("/b"));
        assert!(first.unloaded);
        assert!(store.workspaces()[1].unloaded);

        store.workspace_mut(second.id).unwrap().mark_loaded();
        assert!(!store.workspaces()[1].unloaded);

        store.mark_all_unloaded();

        assert!(store.workspaces().iter().all(|workspace| workspace.unloaded));
        assert_eq!(store.active(), Some(second.id));
    }

    #[test]
    fn mark_all_unloaded_preserves_saved_layout() {
        let mut store = WorkspaceStore::new();
        let workspace = store.add("A".into(), WorkspaceIcon::Star, PathBuf::from("/a"));
        let canvas = &mut store.workspace_mut(workspace.id).unwrap().canvas;
        canvas.add();
        canvas.add_note("nota".into(), CardBounds {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        });

        store.mark_all_unloaded();

        let saved = &store.workspaces()[0].canvas;
        assert_eq!(saved.terminals().len(), 1);
        assert_eq!(saved.notes()[0].text, "nota");
    }

    #[test]
    fn update_details_replaces_form_fields() {
        let mut store = WorkspaceStore::new();
        let mut workspace = store.add("Antigo".into(), WorkspaceIcon::Star, PathBuf::from("/a"));
        workspace.invalid_root = true;

        workspace.update_details(
            "Novo".into(),
            WorkspaceIcon::Globe,
            WorkspaceColor::Green,
            PathBuf::from("/b"),
        );

        assert_eq!(workspace.name, "Novo");
        assert_eq!(workspace.icon, WorkspaceIcon::Globe);
        assert_eq!(workspace.color, WorkspaceColor::Green);
        assert_eq!(workspace.root, PathBuf::from("/b"));
        assert!(!workspace.invalid_root);
    }
}
