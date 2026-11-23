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

/// One of the five SVGs bundled with the app. Custom agents pick one of these.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentIcon {
    Claude,
    Codex,
    Pi,
    Omp,
    OpenCode,
}

impl AgentIcon {
    pub const ALL: [Self; 5] = [
        Self::Claude,
        Self::Codex,
        Self::Pi,
        Self::Omp,
        Self::OpenCode,
    ];

    /// The built-in agent whose colours and mark this icon belongs to.
    pub fn kind(self) -> AgentKind {
        match self {
            Self::Claude => AgentKind::Claude,
            Self::Codex => AgentKind::Codex,
            Self::Pi => AgentKind::Pi,
            Self::Omp => AgentKind::Omp,
            Self::OpenCode => AgentKind::OpenCode,
        }
    }
}

impl Default for AgentIcon {
    fn default() -> Self {
        Self::Claude
    }
}

/// CLI agents that can own a canvas card. Custom agents are resolved by id
/// through the workspace store, where their name, command and icon live.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentKind {
    Claude,
    Codex,
    Pi,
    Omp,
    OpenCode,
    Custom(u64),
}

impl AgentKind {
    pub const ALL: [Self; 5] = [
        Self::Claude,
        Self::Codex,
        Self::Pi,
        Self::Omp,
        Self::OpenCode,
    ];

    /// Label shown in menus, cards and the edit dialog. Custom agents use
    /// their saved name when resolved through the store.
    pub fn label(self) -> String {
        match self {
            Self::Claude => "Claude".into(),
            Self::Codex => "Codex".into(),
            Self::Pi => "Pi".into(),
            Self::Omp => "OMP".into(),
            Self::OpenCode => "Open Code".into(),
            Self::Custom(id) => format!("Agente {id}"),
        }
    }

    /// Exact CLI command written to the PTY when the card is created.
    /// Custom agents use their saved command when resolved through the store.
    pub fn command(self) -> String {
        match self {
            Self::Claude => "claude".into(),
            Self::Codex => "codex".into(),
            Self::Pi => "pi".into(),
            Self::Omp => "omp".into(),
            Self::OpenCode => "opencode".into(),
            Self::Custom(_) => String::new(),
        }
    }

    /// Default mark; custom agents use the icon saved in the store.
    pub fn icon(self) -> AgentIcon {
        match self {
            Self::Claude => AgentIcon::Claude,
            Self::Codex => AgentIcon::Codex,
            Self::Pi => AgentIcon::Pi,
            Self::Omp => AgentIcon::Omp,
            Self::OpenCode => AgentIcon::OpenCode,
            Self::Custom(_) => AgentIcon::Claude,
        }
    }

    /// Stable id for UI element keys, including custom agents.
    pub fn id(self) -> u64 {
        match self {
            Self::Claude => 1,
            Self::Codex => 2,
            Self::Pi => 3,
            Self::Omp => 4,
            Self::OpenCode => 5,
            Self::Custom(id) => id.saturating_add(16),
        }
    }
}

impl Default for AgentKind {
    fn default() -> Self {
        Self::Claude
    }
}

/// A user-defined agent, persisted once and reusable in every workspace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomAgent {
    pub id: u64,
    pub name: String,
    pub command: String,
    pub icon: AgentIcon,
}

impl CustomAgent {
    pub fn kind(&self) -> AgentKind {
        AgentKind::Custom(self.id)
    }
}

/// Persisted AI agent card: the CLI it runs, its title and its canvas geometry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentState {
    pub id: TerminalId,
    pub kind: AgentKind,
    pub name: String,
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
    agents: Vec<AgentState>,
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

/// Generated labels are the only ones safe to renumber; user-renamed cards keep
/// their names. Custom agents always keep their saved name.
fn is_generated_agent_name(kind: AgentKind, name: &str) -> bool {
    if name == kind.label() {
        return true;
    }
    name.strip_prefix(&format!("{} ", kind.label()))
        .is_some_and(|suffix| suffix.parse::<usize>().is_ok())
}
impl CanvasState {
    pub fn new() -> Self {
        Self {
            terminals: Vec::new(),
            notes: Vec::new(),
            agents: Vec::new(),
            selected: None,
            next_id: 1,
        }
    }

    pub fn add(&mut self) -> TerminalState {
        let id = self.next_id;
        self.next_id += 1;
        let terminal = TerminalState {
            id,
            title: format!("Terminal {}", self.terminals.len() + 1),
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
        self.reindex_titles();
        if self.selected == Some(id) {
            self.selected = None;
        }
        Some(removed)
    }

    pub fn select(&mut self, id: TerminalId) {
        self.selected = Some(id);
    }

    /// Clear the selected terminal without touching the saved cards.
    pub fn clear_selection(&mut self) {
        self.selected = None;
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

    /// Allocate an agent from the same id space as terminals and notes.
    pub fn add_agent(&mut self, kind: AgentKind, bounds: CardBounds) -> AgentState {
        let number = self.agents.len() + 1;
        self.add_agent_named(format!("{} {number}", kind.label()), kind, bounds)
    }

    /// Allocate a card whose title comes from a custom agent's saved name.
    pub fn add_agent_named(
        &mut self,
        name: String,
        kind: AgentKind,
        bounds: CardBounds,
    ) -> AgentState {
        let name = if matches!(kind, AgentKind::Custom(_)) || !is_generated_agent_name(kind, &name) {
            name
        } else {
            format!("{} {}", kind.label(), self.agents.len() + 1)
        };
        let id = self.next_id;
        self.next_id += 1;
        let agent = AgentState {
            id,
            kind,
            name,
            bounds,
        };
        self.agents.push(agent.clone());
        self.reindex_titles();
        self.selected = Some(id);
        agent
    }

    pub fn remove_agent(&mut self, id: TerminalId) -> Option<AgentState> {
        let index = self.agents.iter().position(|agent| agent.id == id)?;
        let removed = self.agents.remove(index);
        self.reindex_titles();
        if self.selected == Some(id) {
            self.selected = None;
        }
        Some(removed)
    }

    pub fn agent(&self, id: TerminalId) -> Option<&AgentState> {
        self.agents.iter().find(|agent| agent.id == id)
    }

    pub fn agents(&self) -> &[AgentState] {
        &self.agents
    }

    /// Display numbers follow the current per-type lists, not the cumulative id.
    /// Internal and persisted ids stay untouched, so reloading or editing is safe.
    pub fn reindex_titles(&mut self) {
        for (index, terminal) in self.terminals.iter_mut().enumerate() {
            terminal.title = format!("Terminal {}", index + 1);
        }
        for (index, agent) in self.agents.iter_mut().enumerate() {
            if !matches!(agent.kind, AgentKind::Custom(_))
                && is_generated_agent_name(agent.kind, &agent.name)
            {
                agent.name = format!("{} {}", agent.kind.label(), index + 1);
            }
        }
    }

    pub fn rename_agent(&mut self, id: TerminalId, name: String) -> bool {
        let Some(agent) = self.agents.iter_mut().find(|agent| agent.id == id) else {
            return false;
        };
        if agent.name == name {
            return false;
        }
        agent.name = name;
        true
    }

    pub fn set_agent_kind(&mut self, id: TerminalId, kind: AgentKind) -> bool {
        let Some(agent) = self.agents.iter_mut().find(|agent| agent.id == id) else {
            return false;
        };
        if agent.kind == kind {
            return false;
        }
        agent.kind = kind;
        true
    }

    pub fn set_agent_bounds(&mut self, id: TerminalId, bounds: CardBounds) -> bool {
        let Some(agent) = self.agents.iter_mut().find(|agent| agent.id == id) else {
            return false;
        };
        if agent.bounds == bounds {
            return false;
        }
        agent.bounds = bounds;
        true
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

/// All workspaces plus the current selection and the id allocators.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceStore {
    #[serde(default)]
    workspaces: Vec<WorkspaceState>,
    #[serde(default)]
    active: Option<WorkspaceId>,
    #[serde(default = "default_next_id")]
    next_id: WorkspaceId,
    #[serde(default)]
    custom_agents: Vec<CustomAgent>,
    #[serde(default = "default_next_custom_id")]
    next_custom_id: u64,
}

fn default_next_custom_id() -> u64 {
    1
}

impl WorkspaceStore {
    pub fn new() -> Self {
        Self {
            workspaces: Vec::new(),
            active: None,
            next_id: 1,
            custom_agents: Vec::new(),
            next_custom_id: 1,
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

    pub fn custom_agents(&self) -> &[CustomAgent] {
        &self.custom_agents
    }

    pub fn custom_agent(&self, id: u64) -> Option<&CustomAgent> {
        self.custom_agents.iter().find(|agent| agent.id == id)
    }

    pub fn next_custom_id(&self) -> u64 {
        self.next_custom_id
    }

    /// Keep custom ids above workspace ids and every card allocator, so a
    /// custom agent can never be confused with a workspace, terminal or note.
    pub fn reserve_custom_id_above(&mut self, id: u64) {
        self.next_custom_id = self.next_custom_id.max(id.saturating_add(1));
    }

    /// Persist a reusable agent; its id comes from the dedicated allocator.
    pub fn add_custom_agent(
        &mut self,
        name: String,
        command: String,
        icon: AgentIcon,
    ) -> CustomAgent {
        let floor = self
            .workspaces
            .iter()
            .map(|workspace| workspace.canvas.next_id())
            .max()
            .unwrap_or(1)
            .max(self.next_id);
        let id = self.next_custom_id.max(floor);
        self.reserve_custom_id_above(id);
        let agent = CustomAgent {
            id,
            name,
            command,
            icon,
        };
        self.custom_agents.push(agent.clone());
        agent
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
    fn terminal_titles_follow_current_positions_not_ids() {
        let mut state = CanvasState::new();
        let first = state.add();
        let second = state.add();
        let third = state.add();
        assert_eq!(
            state.terminals().iter().map(|item| item.title.as_str()).collect::<Vec<_>>(),
            ["Terminal 1", "Terminal 2", "Terminal 3"]
        );

        state.remove(second.id);
        assert_eq!(
            state.terminals().iter().map(|item| item.title.as_str()).collect::<Vec<_>>(),
            ["Terminal 1", "Terminal 2"]
        );
        assert_eq!(state.terminals()[1].id, third.id);

        let reopened = state.add();
        assert_eq!(reopened.id, 4);
        assert_eq!(reopened.title, "Terminal 3");
        let _ = first;
    }

    #[test]
    fn agent_titles_follow_current_positions_and_keep_custom_names() {
        let mut state = CanvasState::new();
        let bounds = CardBounds { x: 0.0, y: 0.0, width: 10.0, height: 10.0 };
        let first = state.add_agent(AgentKind::Claude, bounds);
        let second = state.add_agent(AgentKind::Codex, bounds);
        assert_eq!((first.name.as_str(), second.name.as_str()), ("Claude 1", "Codex 2"));

        state.remove_agent(first.id);
        assert_eq!(state.agents()[0].name, "Codex 1");

        let custom = state.add_agent_named("Meu agente".into(), AgentKind::Custom(99), bounds);
        state.reindex_titles();
        assert_eq!(state.agents()[1].id, custom.id);
        assert_eq!(state.agents()[1].name, "Meu agente");
    }

    #[test]
    fn renumbering_keeps_user_chosen_agent_names() {
        let mut state = CanvasState::new();
        let bounds = CardBounds { x: 0.0, y: 0.0, width: 10.0, height: 10.0 };
        let first = state.add_agent(AgentKind::Claude, bounds);
        assert!(state.rename_agent(first.id, "Meu Claude".into()));
        state.add_agent(AgentKind::Codex, bounds);

        assert_eq!(state.agents()[0].name, "Meu Claude");
        assert_eq!(state.agents()[1].name, "Codex 2");
        state.reindex_titles();
        assert_eq!(state.agents()[0].name, "Meu Claude");
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
    fn clear_selection_drops_only_the_selected_terminal() {
        let mut state = CanvasState::new();
        let a = state.add();
        state.clear_selection();
        assert_eq!(state.selected(), None);
        assert_eq!(state.terminals()[0].id, a.id);
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

    #[test]
    fn agent_command_mapping_covers_all_five_agents() {
        let expected = [
            (AgentKind::Claude, "Claude", "claude"),
            (AgentKind::Codex, "Codex", "codex"),
            (AgentKind::Pi, "Pi", "pi"),
            (AgentKind::Omp, "OMP", "omp"),
            (AgentKind::OpenCode, "Open Code", "opencode"),
        ];
        assert_eq!(AgentKind::ALL.len(), expected.len());
        for (kind, (expected_kind, label, command)) in AgentKind::ALL.into_iter().zip(expected) {
            assert_eq!(kind, expected_kind);
            assert_eq!(kind.label(), label);
            assert_eq!(kind.command(), command);
        }
    }

    #[test]
    fn agents_share_the_terminal_and_note_allocator() {
        let mut state = CanvasState::new();
        let terminal = state.add();
        let bounds = CardBounds { x: 0.0, y: 0.0, width: 640.0, height: 400.0 };
        let agent = state.add_agent(AgentKind::Codex, bounds);
        let note = state.add_note("oi".into(), bounds);
        let last = state.add_agent(AgentKind::Pi, bounds);

        assert_eq!((terminal.id, agent.id, note.id, last.id), (1, 2, 3, 4));
        assert_eq!((agent.name.as_str(), last.name.as_str()), ("Codex 1", "Pi 2"));
        assert_eq!(state.selected(), Some(4));

        let mut restored = CanvasState::new();
        restored.reserve_id_above(last.id);
        assert_eq!(restored.add_agent(AgentKind::Claude, bounds).id, 5);
    }

    #[test]
    fn agent_name_kind_bounds_and_removal_are_persisted_mutations() {
        let mut state = CanvasState::new();
        let bounds = CardBounds { x: 4.0, y: 8.0, width: 640.0, height: 400.0 };
        let agent = state.add_agent(AgentKind::Claude, bounds);

        assert!(state.rename_agent(agent.id, "Meu Claude".into()));
        assert!(!state.rename_agent(agent.id, "Meu Claude".into()));
        assert!(state.set_agent_kind(agent.id, AgentKind::OpenCode));
        assert!(!state.set_agent_kind(agent.id, AgentKind::OpenCode));
        let moved = CardBounds { x: 40.0, ..bounds };
        assert!(state.set_agent_bounds(agent.id, moved));
        assert!(!state.set_agent_bounds(agent.id, moved));

        let saved = state.agent(agent.id).unwrap().clone();
        assert_eq!((saved.name.as_str(), saved.kind, saved.bounds), ("Meu Claude", AgentKind::OpenCode, moved));

        assert_eq!(state.remove_agent(agent.id), Some(saved.clone()));
        assert_eq!(state.selected(), None);
        assert!(state.agents().is_empty());
        assert!(state.remove_agent(agent.id).is_none());
    }

    #[test]
    fn agent_state_round_trips_and_old_canvas_without_agents_still_loads() {
        let bounds = CardBounds { x: 2.0, y: 3.0, width: 640.0, height: 400.0 };
        let json = r#"{"terminals":[],"notes":[],"selected":7,"next_id":8}"#;
        let mut state: CanvasState = serde_json::from_str(json).unwrap();
        assert!(state.agents().is_empty());

        let agent = state.add_agent(AgentKind::Omp, bounds);
        let encoded = serde_json::to_string(&state).unwrap();
        let decoded: CanvasState = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded.agents().len(), 1);
        assert_eq!(decoded.agents()[0], agent);
        assert_eq!(decoded.agents()[0].kind, AgentKind::Omp);
        assert_eq!(decoded.next_id(), 9);
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

    #[test]
    fn custom_agents_default_to_empty_and_round_trip() {
        let old = r#"{"workspaces":[],"active":null,"next_id":3}"#;
        let store: WorkspaceStore = serde_json::from_str(old).unwrap();
        assert!(store.custom_agents().is_empty());
        assert_eq!(store.next_custom_id(), 1);

        let mut store = WorkspaceStore::new();
        let agent = store.add_custom_agent(
            "Meu agente".into(),
            "opencode --model x".into(),
            AgentIcon::Pi,
        );
        let encoded = serde_json::to_string(&store).unwrap();
        let decoded: WorkspaceStore = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.custom_agents(), &[agent.clone()]);
        assert_eq!(decoded.custom_agent(agent.id).unwrap().icon, AgentIcon::Pi);
        assert_eq!(decoded.custom_agent(agent.id).unwrap().kind(), AgentKind::Custom(agent.id));
    }

    #[test]
    fn custom_ids_never_collide_with_workspaces_or_cards() {
        let mut store = WorkspaceStore::new();
        let workspace = store.add("A".into(), WorkspaceIcon::Star, PathBuf::from("/a"));
        let terminal = store.active_mut().unwrap().canvas.add();
        let note = store.active_mut().unwrap().canvas.add_note("n".into(), CardBounds {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        });

        let first = store.add_custom_agent("A".into(), "a".into(), AgentIcon::Claude);
        let second = store.add_custom_agent("B".into(), "b".into(), AgentIcon::Omp);
        assert_ne!(first.id, second.id);
        assert!(first.id > workspace.id);
        assert!(first.id > store.active().unwrap_or_default());
        for assigned in [terminal.id, note.id] {
            assert_ne!(first.id, assigned);
        }
        assert!(first.id >= store.workspaces()[0].canvas.next_id());
        assert_ne!(AgentKind::Custom(first.id).id(), AgentKind::Claude.id());

        let mut restored = WorkspaceStore::new();
        restored.reserve_custom_id_above(second.id);
        assert_eq!(restored.add_custom_agent("C".into(), "c".into(), AgentIcon::Pi).id, second.id + 1);
    }
}
