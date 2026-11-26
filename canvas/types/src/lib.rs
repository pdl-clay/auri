//! Framework-free types shared by the canvas host and its terminal cards.
//!
//! [`CanvasState`] tracks terminals, selection and id allocation; the host owns
//! the framework-specific entities separately.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

fn default_next_id() -> u64 {
    1
}

/// Identifier of a terminal on the canvas.
pub type TerminalId = u64;

/// Identifier of a workspace on the canvas.
pub type WorkspaceId = u64;

/// A user-drawn edge endpoint: a card (terminal or agent) or a note.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionTarget {
    Card(TerminalId),
    Note(u64),
}

/// One stored edge. Edges are bidirectional; either endpoint can be a card
/// or a note. `from` accepts the old integer card form when loading configs.
/// Workspace ids qualify the endpoints; `None` means the storing workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Connection {
    pub from: ConnectionTarget,
    pub to: ConnectionTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_workspace: Option<WorkspaceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_workspace: Option<WorkspaceId>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LegacyConnectionFrom {
    Card(TerminalId),
    Target(ConnectionTarget),
}

fn deserialize_connection_from<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<ConnectionTarget, D::Error> {
    match LegacyConnectionFrom::deserialize(deserializer)? {
        LegacyConnectionFrom::Card(id) => Ok(ConnectionTarget::Card(id)),
        LegacyConnectionFrom::Target(target) => Ok(target),
    }
}

impl<'de> Deserialize<'de> for Connection {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(deserialize_with = "deserialize_connection_from")]
            from: ConnectionTarget,
            to: ConnectionTarget,
            #[serde(default)]
            from_workspace: Option<WorkspaceId>,
            #[serde(default)]
            to_workspace: Option<WorkspaceId>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(Self {
            from: raw.from,
            to: raw.to,
            from_workspace: raw.from_workspace,
            to_workspace: raw.to_workspace,
        })
    }
}

impl Connection {
    /// Build a local edge: both endpoints belong to the workspace storing it.
    pub fn local(from: ConnectionTarget, to: ConnectionTarget) -> Self {
        Self {
            from,
            to,
            from_workspace: None,
            to_workspace: None,
        }
    }

    /// True when both endpoints are explicitly qualified with different
    /// workspace ids. One-sided qualification cannot be classified without the
    /// storing workspace, so it is conservatively reported as local here. When
    /// the storing workspace is known, `WorkspaceStore::connected_across` is the
    /// authoritative check: it resolves `None` to that workspace.
    pub fn is_cross_workspace(&self) -> bool {
        match (self.from_workspace, self.to_workspace) {
            (Some(from), Some(to)) => from != to,
            _ => false,
        }
    }
}

/// Resolve an omitted qualifier to the workspace that stores the edge.
fn endpoint_workspace(endpoint: Option<WorkspaceId>, owner: WorkspaceId) -> WorkspaceId {
    endpoint.unwrap_or(owner)
}

/// A local lookup only matches edges whose two endpoints resolve to `owner`.
fn edge_links_in(
    edge: &Connection,
    owner: WorkspaceId,
    a: ConnectionTarget,
    b: ConnectionTarget,
) -> bool {
    endpoint_workspace(edge.from_workspace, owner) == owner
        && endpoint_workspace(edge.to_workspace, owner) == owner
        && edge_links(edge, a, b)
}

/// Whether an edge touches `target` in `target_ws`. `owner` is the workspace
/// storing the edge, so an omitted qualifier resolves to it, not to the caller.
fn edge_touches(
    edge: &Connection,
    owner: WorkspaceId,
    target_ws: WorkspaceId,
    target: ConnectionTarget,
) -> bool {
    (endpoint_workspace(edge.from_workspace, owner) == target_ws && edge.from == target)
        || (endpoint_workspace(edge.to_workspace, owner) == target_ws && edge.to == target)
}

fn edge_links(edge: &Connection, a: ConnectionTarget, b: ConnectionTarget) -> bool {
    (edge.from == a && edge.to == b) || (edge.from == b && edge.to == a)
}

/// Whether two edges join the same workspace-qualified endpoints, in either
/// direction. Workspace ids qualify the target, so two cards with the same
/// numeric id in different workspaces are different edges.
fn qualified_edge_links(a: &Connection, b: &Connection) -> bool {
    let forward = a.from == b.from
        && a.to == b.to
        && a.from_workspace == b.from_workspace
        && a.to_workspace == b.to_workspace;
    let backward = a.from == b.to
        && a.to == b.from
        && a.from_workspace == b.to_workspace
        && a.to_workspace == b.from_workspace;
    forward || backward
}

/// Saved pixel geometry of a canvas card, independent of the UI framework.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CardBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// World-space centre of a proxy card in the workspace that displays it.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProxyPlacement {
    pub x: f32,
    pub y: f32,
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
        Self::Omp,
        Self::OpenCode,
        Self::Claude,
        Self::Codex,
        Self::Pi,
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
            Self::Omp => "omp --continue".into(),
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
    /// Administrator cards can manage workspace cards through the CLI.
    #[serde(default)]
    pub admin: bool,
    /// Exact omp session JSONL resumed by this card, when one is known.
    #[serde(default)]
    pub session: Option<String>,
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
    connections: Vec<Connection>,
    /// World-space positions for proxies the user has moved; absent means the
    /// proxy keeps following the remote endpoint.
    #[serde(default)]
    proxy_positions: HashMap<String, ProxyPlacement>,
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
            connections: Vec::new(),
            proxy_positions: HashMap::new(),
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

    pub fn remove(&mut self, owner: WorkspaceId, id: TerminalId) -> Option<TerminalState> {
        let index = self.terminals.iter().position(|terminal| terminal.id == id)?;
        let removed = self.terminals.remove(index);
        self.remove_connections_for(owner, owner, ConnectionTarget::Card(id));
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

    pub fn remove_note(&mut self, owner: WorkspaceId, id: u64) -> Option<NoteState> {
        let index = self.notes.iter().position(|note| note.id == id)?;
        let removed = self.notes.remove(index);
        self.remove_connections_for(owner, owner, ConnectionTarget::Note(id));
        Some(removed)
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
            admin: false,
            session: None,
        };
        self.agents.push(agent.clone());
        self.reindex_titles();
        self.selected = Some(id);
        agent
    }

    pub fn remove_agent(&mut self, owner: WorkspaceId, id: TerminalId) -> Option<AgentState> {
        let index = self.agents.iter().position(|agent| agent.id == id)?;
        let removed = self.agents.remove(index);
        self.remove_connections_for(owner, owner, ConnectionTarget::Card(id));
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

    /// Remember the omp session resumed by this card so a reload reopens it.
    pub fn set_agent_session(&mut self, id: TerminalId, session: String) -> bool {
        let Some(agent) = self.agents.iter_mut().find(|agent| agent.id == id) else {
            return false;
        };
        if agent.session.as_deref() == Some(session.as_str()) {
            return false;
        }
        agent.session = Some(session);
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

    /// The stored edges, in insertion order.
    pub fn connections(&self) -> &[Connection] {
        &self.connections
    }

    pub fn proxy_position(&self, key: &str) -> Option<ProxyPlacement> {
        self.proxy_positions.get(key).copied()
    }

    /// Record a user-moved proxy; returns false when it did not move.
    pub fn set_proxy_position(&mut self, key: String, position: ProxyPlacement) -> bool {
        if self.proxy_positions.get(&key) == Some(&position) {
            return false;
        }
        self.proxy_positions.insert(key, position);
        true
    }

    /// Add one edge between a card and another card or note. Self edges and
    /// duplicates (including the reverse direction) are ignored.
    pub fn connect(&mut self, from: TerminalId, to: ConnectionTarget) -> bool {
        self.connect_between(ConnectionTarget::Card(from), to)
    }

    /// Add one edge starting at a sticky note.
    pub fn connect_note(&mut self, note: u64, to: ConnectionTarget) -> bool {
        self.connect_between(ConnectionTarget::Note(note), to)
    }

    /// Generic edge insertion; both endpoints may be cards or notes.
    pub fn connect_between(&mut self, from: ConnectionTarget, to: ConnectionTarget) -> bool {
        if from == to || self.connections.iter().any(|edge| edge_links(edge, from, to)) {
            return false;
        }
        self.connections.push(Connection::local(from, to));
        true
    }

    /// Add a workspace-qualified edge. Self edges and duplicates (including
    /// the reverse direction) are ignored. The caller owns workspace ids;
    /// `None` still means "the workspace storing this edge" when reading it.
    pub fn connect_qualified(&mut self, edge: Connection) -> bool {
        if edge.from == edge.to && edge.from_workspace == edge.to_workspace {
            return false;
        }
        if self.connections.iter().any(|stored| qualified_edge_links(stored, &edge)) {
            return false;
        }
        self.connections.push(edge);
        true
    }

    /// Remove exactly this workspace-qualified edge, in either direction.
    /// Unlike `disconnect_between`, an id in another workspace never matches.
    pub fn disconnect_exact(&mut self, edge: Connection) -> bool {
        let before = self.connections.len();
        self.connections
            .retain(|stored| !qualified_edge_links(stored, &edge));
        self.connections.len() != before
    }

    /// Remove an edge in either direction; false when no such edge existed.
    pub fn disconnect(&mut self, from: TerminalId, to: ConnectionTarget) -> bool {
        self.disconnect_between(ConnectionTarget::Card(from), to)
    }

    /// Remove an edge whose source is a note.
    pub fn disconnect_note(&mut self, note: u64, to: ConnectionTarget) -> bool {
        self.disconnect_between(ConnectionTarget::Note(note), to)
    }

    pub fn disconnect_between(&mut self, from: ConnectionTarget, to: ConnectionTarget) -> bool {
        let before = self.connections.len();
        self.connections.retain(|edge| !edge_links(edge, from, to));
        self.connections.len() != before
    }

    /// True when a local edge joins the two endpoints, regardless of direction.
    /// A qualified endpoint in another workspace never matches a local id.
    pub fn connected(&self, owner: WorkspaceId, a: TerminalId, b: ConnectionTarget) -> bool {
        self.connected_between(owner, ConnectionTarget::Card(a), b)
    }

    /// True when a local note joins the given endpoint.
    pub fn connected_note(&self, owner: WorkspaceId, note: u64, b: ConnectionTarget) -> bool {
        self.connected_between(owner, ConnectionTarget::Note(note), b)
    }

    pub fn connected_between(
        &self,
        owner: WorkspaceId,
        a: ConnectionTarget,
        b: ConnectionTarget,
    ) -> bool {
        self.connections.iter().any(|edge| edge_links_in(edge, owner, a, b))
    }

    /// Drop every stored edge touching `target` in `target_ws`. `owner` is the
    /// workspace storing these edges; `None` qualifiers resolve to it.
    pub fn remove_connections_for(
        &mut self,
        owner: WorkspaceId,
        target_ws: WorkspaceId,
        target: ConnectionTarget,
    ) {
        self.connections
            .retain(|edge| !edge_touches(edge, owner, target_ws, target));
    }

    /// Change a card's role; the next CLI request observes the new value.
    pub fn set_agent_admin(&mut self, id: TerminalId, admin: bool) -> bool {
        let Some(agent) = self.agents.iter_mut().find(|agent| agent.id == id) else {
            return false;
        };
        if agent.admin == admin {
            return false;
        }
        agent.admin = admin;
        true
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
        self.drop_edges_for_workspace(id);
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

    /// Remove one card (agent or terminal) from `workspace` and drop every
    /// edge touching it, including copies of the edge stored in other workspaces.
    pub fn remove_card(&mut self, workspace: WorkspaceId, id: TerminalId) -> bool {
        let Some(owner) = self.workspace_mut(workspace) else {
            return false;
        };
        let removed = if owner.canvas.agent(id).is_some() {
            owner.canvas.remove_agent(workspace, id).is_some()
        } else {
            owner.canvas.remove(workspace, id).is_some()
        };
        if removed {
            for stored in &mut self.workspaces {
                stored
                    .canvas
                    .remove_connections_for(stored.id, workspace, ConnectionTarget::Card(id));
            }
        }
        removed
    }

    /// Remove one note from `workspace` with the same cross-workspace cleanup.
    pub fn remove_note(&mut self, workspace: WorkspaceId, id: u64) -> Option<NoteState> {
        let removed = self.workspace_mut(workspace)?.canvas.remove_note(workspace, id)?;
        for stored in &mut self.workspaces {
            stored
                .canvas
                .remove_connections_for(stored.id, workspace, ConnectionTarget::Note(id));
        }
        Some(removed)
    }

    /// True when an edge between the two qualified endpoints exists in any
    /// workspace. An edge's missing workspace id means the workspace storing it.
    pub fn connected_across(
        &self,
        from_ws: WorkspaceId,
        from: ConnectionTarget,
        to_ws: WorkspaceId,
        to: ConnectionTarget,
    ) -> bool {
        self.workspaces.iter().any(|workspace| {
            let owner = workspace.id;
            workspace.canvas.connections.iter().any(|edge| {
                let edge_from = edge.from_workspace.unwrap_or(owner);
                let edge_to = edge.to_workspace.unwrap_or(owner);
                let forward = edge_from == from_ws
                    && edge_to == to_ws
                    && edge.from == from
                    && edge.to == to;
                let backward = edge_from == to_ws
                    && edge_to == from_ws
                    && edge.from == to
                    && edge.to == from;
                forward || backward
            })
        })
    }

    /// Drop edges that reference `id` from the other workspaces so deleting
    /// a workspace leaves no dangling cross-workspace connections behind.
    pub fn drop_edges_for_workspace(&mut self, id: WorkspaceId) {
        for workspace in &mut self.workspaces {
            if workspace.id == id {
                continue;
            }
            let owner = workspace.id;
            workspace.canvas.connections.retain(|edge| {
                edge.from_workspace.unwrap_or(owner) != id
                    && edge.to_workspace.unwrap_or(owner) != id
            });
        }
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

        state.remove(1, second.id);
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

        state.remove_agent(1, first.id);
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
        state.remove(1, a.id);
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

        assert_eq!(state.remove_note(1, note.id).map(|note| note.text), Some("texto novo".into()));
        assert!(state.notes().is_empty());
        assert_eq!(state.remove_note(1, note.id), None);
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
            (AgentKind::Omp, "OMP", "omp --continue"),
            (AgentKind::OpenCode, "Open Code", "opencode"),
            (AgentKind::Claude, "Claude", "claude"),
            (AgentKind::Codex, "Codex", "codex"),
            (AgentKind::Pi, "Pi", "pi"),
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

        assert_eq!(state.remove_agent(1, agent.id), Some(saved.clone()));
        assert_eq!(state.selected(), None);
        assert!(state.agents().is_empty());
        assert!(state.remove_agent(1, agent.id).is_none());
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
        assert_eq!(decoded.agents()[0].session, None);
        assert_eq!(decoded.next_id(), 9);
    }

    #[test]
    fn connections_are_bidirectional_and_reject_duplicates_and_self_edges() {
        let mut state = CanvasState::new();
        let bounds = CardBounds { x: 0.0, y: 0.0, width: 640.0, height: 400.0 };
        let a = state.add_agent(AgentKind::Claude, bounds);
        let b = state.add_agent(AgentKind::Codex, bounds);
        let note = state.add_note("oi".into(), bounds);

        assert!(state.connect(a.id, ConnectionTarget::Card(b.id)));
        assert!(!state.connect(a.id, ConnectionTarget::Card(b.id)), "duplicate edge");
        assert!(
            !state.connect(b.id, ConnectionTarget::Card(a.id)),
            "reverse edge is the same edge"
        );
        assert!(state.connected(1, a.id, ConnectionTarget::Card(b.id)));
        assert!(state.connected(1, b.id, ConnectionTarget::Card(a.id)));
        assert!(!state.connect(a.id, ConnectionTarget::Card(a.id)), "self edge");

        assert!(state.connect(a.id, ConnectionTarget::Note(note.id)));
        assert!(state.connected(1, a.id, ConnectionTarget::Note(note.id)));
        assert!(!state.connect(a.id, ConnectionTarget::Note(note.id)), "duplicate note edge");
        assert_eq!(state.connections().len(), 2);

        assert!(state.disconnect(b.id, ConnectionTarget::Card(a.id)), "reverse disconnect");
        assert!(!state.connected(1, a.id, ConnectionTarget::Card(b.id)));
        assert!(!state.disconnect(a.id, ConnectionTarget::Card(b.id)));
    }

    #[test]
    fn removing_cards_and_notes_removes_all_touching_edges() {
        let mut state = CanvasState::new();
        let bounds = CardBounds { x: 0.0, y: 0.0, width: 640.0, height: 400.0 };
        let a = state.add_agent(AgentKind::Claude, bounds);
        let b = state.add_agent(AgentKind::Codex, bounds);
        let terminal = state.add();
        let note = state.add_note("oi".into(), bounds);

        assert!(state.connect(a.id, ConnectionTarget::Card(b.id)));
        assert!(state.connect(terminal.id, ConnectionTarget::Card(a.id)));
        assert!(state.connect(a.id, ConnectionTarget::Note(note.id)));
        assert!(state.connect(b.id, ConnectionTarget::Note(note.id)));
        assert!(state.connect(terminal.id, ConnectionTarget::Note(note.id)));
        assert_eq!(state.connections().len(), 5);

        assert!(state.remove_agent(1, a.id).is_some());
        assert_eq!(
            state.connections(),
            &[
                Connection {
                    from: ConnectionTarget::Card(b.id),
                    to: ConnectionTarget::Note(note.id),
                    from_workspace: None,
                    to_workspace: None,
                },
                Connection {
                    from: ConnectionTarget::Card(terminal.id),
                    to: ConnectionTarget::Note(note.id),
                    from_workspace: None,
                    to_workspace: None,
                },
            ]
        );

        assert!(state.remove(1, terminal.id).is_some());
        assert_eq!(
            state.connections(),
            &[Connection {
                from: ConnectionTarget::Card(b.id),
                to: ConnectionTarget::Note(note.id),
                from_workspace: None,
                to_workspace: None,
            }]
        );

        assert!(state.remove_note(1, note.id).is_some());
        assert!(state.connections().is_empty());
    }

    #[test]
    fn connections_and_admin_survive_serde_round_trip() {
        let mut state = CanvasState::new();
        let bounds = CardBounds { x: 1.0, y: 2.0, width: 640.0, height: 400.0 };
        let agent = state.add_agent(AgentKind::Omp, bounds);
        let note = state.add_note("texto".into(), bounds);
        assert!(state.set_agent_admin(agent.id, true));
        assert!(state.connect(agent.id, ConnectionTarget::Note(note.id)));

        let encoded = serde_json::to_string(&state).unwrap();
        let decoded: CanvasState = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.connections(), state.connections());
        assert!(decoded.agent(agent.id).unwrap().admin);
        assert_eq!(decoded.next_id(), state.next_id());
        assert_eq!(decoded.selected(), state.selected());
    }

    #[test]
    fn notes_can_be_edge_sources_and_touch_either_endpoint_on_removal() {
        let mut state = CanvasState::new();
        let bounds = CardBounds { x: 0.0, y: 0.0, width: 320.0, height: 200.0 };
        let card = state.add_agent(AgentKind::Claude, bounds);
        let note = state.add_note("origem".into(), bounds);

        assert!(state.connect_note(note.id, ConnectionTarget::Card(card.id)));
        assert!(state.connected_note(1, note.id, ConnectionTarget::Card(card.id)));
        assert!(state.connected(1, card.id, ConnectionTarget::Note(note.id)));
        assert!(!state.connect_note(note.id, ConnectionTarget::Card(card.id)));
        assert!(!state.connect_note(note.id, ConnectionTarget::Note(note.id)));

        assert!(state.remove_note(1, note.id).is_some());
        assert!(state.connections().is_empty());
    }

    #[test]
    fn legacy_integer_connection_source_still_loads() {
        let json = r#"{"terminals":[],"notes":[],"agents":[],"selected":null,"next_id":4,"connections":[{"from":2,"to":{"Note":7}}]}"#;
        let state: CanvasState = serde_json::from_str(json).unwrap();
        assert_eq!(state.connections().len(), 1);
        assert!(state.connected(1, 2, ConnectionTarget::Note(7)));
    }

    #[test]
    fn old_canvas_json_without_connections_or_admin_still_loads() {
        let json = r#"{"terminals":[],"notes":[],"agents":[{"id":3,"kind":"Claude","name":"Claude 1","bounds":{"x":0.0,"y":0.0,"width":640.0,"height":400.0}}],"selected":3,"next_id":4}"#;
        let state: CanvasState = serde_json::from_str(json).unwrap();
        assert!(state.connections().is_empty());
        assert_eq!(state.agents().len(), 1);
        assert!(!state.agent(3).unwrap().admin);
        assert_eq!(state.next_id(), 4);
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

    #[test]
    fn old_local_connection_still_loads_without_workspace_ids() {
        let json = r#"{"from":{"Card":1},"to":{"Note":2}}"#;
        let edge: Connection = serde_json::from_str(json).unwrap();
        assert!(!edge.is_cross_workspace());

        let encoded = serde_json::to_string(&edge).unwrap();
        assert!(!encoded.contains("workspace"));
        assert_eq!(serde_json::from_str::<Connection>(&encoded).unwrap(), edge);
        let local = Connection::local(ConnectionTarget::Card(1), ConnectionTarget::Note(2));
        assert_eq!(local, edge);
        assert_eq!(serde_json::to_string(&local).unwrap(), json);
    }

    #[test]
    fn cross_workspace_edge_round_trips_and_is_found_from_both_sides() {
        let edge = Connection {
            from: ConnectionTarget::Card(1),
            to: ConnectionTarget::Note(2),
            from_workspace: Some(1),
            to_workspace: Some(2),
        };
        assert!(edge.is_cross_workspace());

        let encoded = serde_json::to_string(&edge).unwrap();
        assert!(encoded.contains("\"from_workspace\":1"));
        assert!(encoded.contains("\"to_workspace\":2"));
        assert_eq!(serde_json::from_str::<Connection>(&encoded).unwrap(), edge);

        let mut store = WorkspaceStore::new();
        let a = store.add("A".into(), WorkspaceIcon::Star, PathBuf::from("/a"));
        let b = store.add("B".into(), WorkspaceIcon::Bolt, PathBuf::from("/b"));
        assert_eq!((a.id, b.id), (1, 2));
        store.workspace_mut(a.id).unwrap().canvas.connections.push(edge);

        assert!(store.connected_across(
            a.id,
            ConnectionTarget::Card(1),
            b.id,
            ConnectionTarget::Note(2),
        ));
        assert!(store.connected_across(
            b.id,
            ConnectionTarget::Note(2),
            a.id,
            ConnectionTarget::Card(1),
        ));
    }

    #[test]
    fn connect_qualified_rejects_self_edges_but_keeps_cross_workspace_ids() {
        let mut state = CanvasState::new();
        let same = Connection {
            from: ConnectionTarget::Card(4),
            to: ConnectionTarget::Card(4),
            from_workspace: Some(1),
            to_workspace: Some(1),
        };
        assert!(!state.connect_qualified(same));
        assert!(state.connections().is_empty());

        let cross = Connection {
            from: ConnectionTarget::Card(4),
            to: ConnectionTarget::Card(4),
            from_workspace: Some(1),
            to_workspace: Some(2),
        };
        assert!(state.connect_qualified(cross), "same id in two workspaces is not a self edge");
        assert_eq!(state.connections(), &[cross]);
    }

    #[test]
    fn connect_qualified_ignores_a_reverse_duplicate() {
        let mut state = CanvasState::new();
        let edge = Connection {
            from: ConnectionTarget::Card(1),
            to: ConnectionTarget::Note(2),
            from_workspace: Some(1),
            to_workspace: Some(2),
        };
        assert!(state.connect_qualified(edge));
        let reverse = Connection {
            from: edge.to,
            to: edge.from,
            from_workspace: edge.to_workspace,
            to_workspace: edge.from_workspace,
        };
        assert!(!state.connect_qualified(reverse));
        assert_eq!(state.connections().len(), 1);
        assert_eq!(state.connections()[0], edge);
    }

    #[test]
    fn disconnect_exact_keeps_an_identical_none_qualified_local_edge() {
        let mut state = CanvasState::new();
        let local = Connection::local(ConnectionTarget::Card(1), ConnectionTarget::Card(2));
        let qualified = Connection {
            from: local.from,
            to: local.to,
            from_workspace: Some(1),
            to_workspace: Some(1),
        };
        assert!(state.connect_qualified(local));
        assert!(state.connect_qualified(qualified));
        assert_eq!(state.connections().len(), 2);

        assert!(state.disconnect_exact(qualified));
        assert_eq!(state.connections(), &[local], "only the qualified edge is removed");
        assert!(!state.disconnect_exact(qualified), "removal is not repeated");
    }

    #[test]
    fn deleting_a_workspace_drops_its_edges_from_other_workspaces() {
        let mut store = WorkspaceStore::new();
        let a = store.add("A".into(), WorkspaceIcon::Star, PathBuf::from("/a"));
        let b = store.add("B".into(), WorkspaceIcon::Bolt, PathBuf::from("/b"));
        let c = store.add("C".into(), WorkspaceIcon::Globe, PathBuf::from("/c"));
        let ab = Connection {
            from: ConnectionTarget::Card(1),
            to: ConnectionTarget::Note(2),
            from_workspace: Some(a.id),
            to_workspace: Some(b.id),
        };
        let bc = Connection {
            from: ConnectionTarget::Note(3),
            to: ConnectionTarget::Card(4),
            from_workspace: Some(b.id),
            to_workspace: Some(c.id),
        };
        let local = Connection {
            from: ConnectionTarget::Card(5),
            to: ConnectionTarget::Note(6),
            from_workspace: None,
            to_workspace: None,
        };
        store.workspace_mut(a.id).unwrap().canvas.connections.extend([ab, local]);
        store.workspace_mut(c.id).unwrap().canvas.connections.push(bc);

        assert_eq!(store.remove(b.id).map(|workspace| workspace.id), Some(b.id));

        assert!(!store.connected_across(
            a.id,
            ConnectionTarget::Card(1),
            b.id,
            ConnectionTarget::Note(2),
        ));
        assert!(!store.connected_across(
            b.id,
            ConnectionTarget::Note(3),
            c.id,
            ConnectionTarget::Card(4),
        ));
        assert!(store.connected_across(
            a.id,
            ConnectionTarget::Card(5),
            a.id,
            ConnectionTarget::Note(6),
        ));
        assert_eq!(store.workspace_mut(a.id).unwrap().canvas.connections().len(), 1);
    }

    #[test]
    fn deleting_a_colliding_local_id_keeps_the_cross_workspace_edge() {
        let json = r#"{
            "workspaces": [
                {"id":1,"name":"A","icon":"Star","root":"/a","canvas":{
                    "agents":[{"id":1,"kind":"Claude","name":"Admin","admin":true,
                        "bounds":{"x":0.0,"y":0.0,"width":10.0,"height":10.0}}],
                    "terminals":[{"id":4,"title":"Unrelated","exited":false,
                        "bounds":{"x":0.0,"y":0.0,"width":10.0,"height":10.0}}],
                    "notes":[{"id":2,"text":"local","bounds":{"x":0.0,"y":0.0,"width":10.0,"height":10.0}}],
                    "connections":[
                        {"from":{"Card":1},"to":{"Card":4},"from_workspace":1,"to_workspace":2},
                        {"from":{"Card":1},"to":{"Note":2},"from_workspace":1,"to_workspace":2}
                    ],"next_id":5}},
                {"id":2,"name":"B","icon":"Bolt","root":"/b","canvas":{
                    "terminals":[{"id":4,"title":"Peer","exited":false,
                        "bounds":{"x":0.0,"y":0.0,"width":10.0,"height":10.0}}],
                    "notes":[{"id":2,"text":"peer","bounds":{"x":0.0,"y":0.0,"width":10.0,"height":10.0}}],
                    "next_id":5}}
            ],"active":1,"next_id":3
        }"#;
        let mut store: WorkspaceStore = serde_json::from_str(json).expect("two-workspace store");
        let card_edge = (
            ConnectionTarget::Card(1),
            ConnectionTarget::Card(4),
        );
        let note_edge = (
            ConnectionTarget::Card(1),
            ConnectionTarget::Note(2),
        );
        assert!(store.connected_across(1, card_edge.0, 2, card_edge.1));
        assert!(store.connected_across(1, note_edge.0, 2, note_edge.1));
        // Neither cross edge is a local edge in the workspace that stores it.
        assert!(!store.workspace_mut(1).unwrap().canvas.connected(1, 1, card_edge.1));
        assert!(!store.workspace_mut(1).unwrap().canvas.connected_note(1, 2, note_edge.0));

        assert!(store.remove_card(1, 4), "A's unrelated local card 4 exists");
        assert!(store.connected_across(1, card_edge.0, 2, card_edge.1), "B's card 4 is still connected");
        assert!(store.remove_note(1, 2).is_some(), "A's unrelated local note 2 exists");
        assert!(store.connected_across(1, note_edge.0, 2, note_edge.1), "B's note 2 is still connected");

        assert!(store.remove_card(2, 4), "deleting the peer endpoint removes its edge");
        assert!(!store.connected_across(1, card_edge.0, 2, card_edge.1));
        assert!(store.remove_note(2, 2).is_some());
        assert!(!store.connected_across(1, note_edge.0, 2, note_edge.1));
    }

    #[test]
    fn duplicate_local_ids_do_not_satisfy_a_qualified_lookup() {
        let mut store = WorkspaceStore::new();
        let a = store.add("A".into(), WorkspaceIcon::Star, PathBuf::from("/a"));
        let b = store.add("B".into(), WorkspaceIcon::Bolt, PathBuf::from("/b"));
        for id in [a.id, b.id] {
            let canvas = &mut store.workspace_mut(id).unwrap().canvas;
            for _ in 0..4 {
                canvas.add();
            }
        }
        let qualified = Connection {
            from: ConnectionTarget::Card(4),
            to: ConnectionTarget::Card(3),
            from_workspace: Some(a.id),
            to_workspace: Some(b.id),
        };
        store.workspace_mut(a.id).unwrap().canvas.connections.push(qualified);
        assert!(store.workspace_mut(b.id).unwrap().canvas.connect(4, ConnectionTarget::Card(3)));

        assert!(store.connected_across(
            a.id,
            ConnectionTarget::Card(4),
            b.id,
            ConnectionTarget::Card(3),
        ));
        assert!(store.connected_across(
            b.id,
            ConnectionTarget::Card(3),
            a.id,
            ConnectionTarget::Card(4),
        ));
        // B's local edge has the same numeric ids but different qualified endpoints.
        assert!(!store.connected_across(
            b.id,
            ConnectionTarget::Card(4),
            a.id,
            ConnectionTarget::Card(3),
        ));
        assert!(!store.connected_across(
            a.id,
            ConnectionTarget::Card(4),
            a.id,
            ConnectionTarget::Card(3),
        ));
        assert!(store.connected_across(
            b.id,
            ConnectionTarget::Card(4),
            b.id,
            ConnectionTarget::Card(3),
        ));
    }

    #[test]
    fn one_sided_workspace_ids_are_local_to_the_helper_but_resolve_in_store() {
        let mut store = WorkspaceStore::new();
        let a = store.add("A".into(), WorkspaceIcon::Star, PathBuf::from("/a"));
        let b = store.add("B".into(), WorkspaceIcon::Bolt, PathBuf::from("/b"));

        let from_qualified = Connection {
            from: ConnectionTarget::Card(1),
            to: ConnectionTarget::Note(2),
            from_workspace: Some(a.id),
            to_workspace: None,
        };
        let to_qualified = Connection {
            from: ConnectionTarget::Card(3),
            to: ConnectionTarget::Note(4),
            from_workspace: None,
            to_workspace: Some(a.id),
        };
        assert!(!from_qualified.is_cross_workspace());
        assert!(!to_qualified.is_cross_workspace());

        let canvas = &mut store.workspace_mut(b.id).unwrap().canvas;
        canvas.connections.push(from_qualified);
        canvas.connections.push(to_qualified);

        // Stored in B, each unqualified end resolves to B, so both edges are
        // cross-workspace under the store's authoritative lookup.
        assert!(store.connected_across(
            a.id,
            ConnectionTarget::Card(1),
            b.id,
            ConnectionTarget::Note(2),
        ));
        assert!(store.connected_across(
            b.id,
            ConnectionTarget::Card(3),
            a.id,
            ConnectionTarget::Note(4),
        ));
    }

    #[test]
    fn renaming_a_workspace_keeps_cross_edges_bound_to_ids() {
        let mut store = WorkspaceStore::new();
        let a = store.add("A".into(), WorkspaceIcon::Star, PathBuf::from("/a"));
        let b = store.add("B".into(), WorkspaceIcon::Bolt, PathBuf::from("/b"));
        let edge = Connection {
            from: ConnectionTarget::Card(1),
            to: ConnectionTarget::Card(4),
            from_workspace: Some(a.id),
            to_workspace: Some(b.id),
        };
        store.workspace_mut(a.id).unwrap().canvas.connections.push(edge);
        store.workspace_mut(b.id).unwrap().update_details(
            "B renomeado".into(),
            WorkspaceIcon::Bolt,
            WorkspaceColor::Brass,
            PathBuf::from("/b"),
        );

        assert_eq!(store.workspaces()[1].name, "B renomeado");
        assert!(store.connected_across(
            a.id,
            ConnectionTarget::Card(1),
            b.id,
            ConnectionTarget::Card(4),
        ), "a rename must not detach an edge stored by workspace id");
        let encoded = serde_json::to_string(&store).expect("renamed store saves");
        assert!(encoded.contains("\"to_workspace\":2"));
        assert_eq!(
            serde_json::from_str::<WorkspaceStore>(&encoded).expect("renamed store reloads"),
            store
        );
    }

    #[test]
    fn deleting_a_workspace_drops_one_sided_qualified_edges_from_other_workspaces() {
        let mut store = WorkspaceStore::new();
        let a = store.add("A".into(), WorkspaceIcon::Star, PathBuf::from("/a"));
        let b = store.add("B".into(), WorkspaceIcon::Bolt, PathBuf::from("/b"));
        let c = store.add("C".into(), WorkspaceIcon::Globe, PathBuf::from("/c"));
        let to_deleted = Connection {
            from: ConnectionTarget::Card(1),
            to: ConnectionTarget::Card(2),
            from_workspace: None,
            to_workspace: Some(b.id),
        };
        let from_deleted = Connection {
            from: ConnectionTarget::Card(3),
            to: ConnectionTarget::Note(4),
            from_workspace: Some(b.id),
            to_workspace: None,
        };
        let local = Connection::local(ConnectionTarget::Card(5), ConnectionTarget::Note(6));
        store.workspace_mut(a.id).unwrap().canvas.connections.extend([to_deleted, local]);
        store.workspace_mut(c.id).unwrap().canvas.connections.push(from_deleted);

        assert_eq!(store.remove(b.id).map(|workspace| workspace.id), Some(b.id));

        assert_eq!(store.workspace_mut(a.id).unwrap().canvas.connections(), &[local]);
        assert!(store.workspace_mut(c.id).unwrap().canvas.connections().is_empty());
    }
}
