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

/// Logical state of a single terminal card.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalState {
    pub id: TerminalId,
    pub title: String,
    pub exited: bool,
}

/// Framework-free canvas state: terminals, the selected terminal and id allocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasState {
    #[serde(default)]
    terminals: Vec<TerminalState>,
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
        if let Some(terminal) = self.terminals.iter_mut().find(|terminal| terminal.id == id) {
            terminal.exited = true;
        }
    }

    pub fn terminals(&self) -> &[TerminalState] {
        &self.terminals
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
}

/// All workspaces plus the current selection and the id allocator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
        };
        state.canvas.add();
        assert!(state.invalid_root);
        assert_eq!(state.canvas.terminals().len(), 1);
    }
}
