# Cross-workspace connections Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an administrator draw a connection to an agent, terminal or note in another workspace, then let both sides send and receive `auri ask` messages across it, with inert proxy cards and rope-popover removal.

**Architecture:** Extend the existing `Connection` record with optional workspace ids so old local edges still load. Resolve qualified addresses as `id@workspace`, check permission against global edges, and load an unloaded target workspace in the background before delivering bytes. Render one inert proxy card per remote endpoint in each workspace; use the existing rope painter and popover for the visual link and removal.

**Tech Stack:** Rust 2024, GPUI `=0.2.2` via `auro = "=0.1.0-alpha.15"`, `gpui-component = "=0.5.1"`, serde/serde_json, existing GPUI test-support harness.

**Spec:** `docs/superpowers/specs/2026-11-25-cross-workspace-connections-design.md`

## Global Constraints

- Keep GPUI pinned at `=0.2.2` through Auro; do not declare `gpui` directly.
- Old local connections must deserialize without a migration step; local behaviour stays unchanged.
- Workspace ids are stable; workspace names are display labels and may change.
- A cross-workspace edge may only be created from a selected administrator agent card.
- Connected normal agents may send and receive; no CLI verb may create or remove a connection.
- Auto-loading a peer workspace must not change `WorkspaceStore::active()`.
- The proxy card is inert: no click, hover action, focus target or selection state.
- All visual dimensions and colours live in `canvas/ui/src/theme.rs`.
- Every bug fix or behaviour change needs a deterministic test; UI/gesture states additionally need a debugger-visible element for tests.

## Review Focus

- Duplicate numeric ids across workspaces: `4@auro` must resolve to Auro's card 4, never Auri's card 4.
- Unloaded peer workspace: `auri ask` loads it in the background, waits for its writer, and leaves the active workspace untouched.
- Normal non-admin peer: connected normal agent can send/receive, but cannot create a new cross-workspace edge.
- Renamed or deleted workspace: old name-qualified addresses fail clearly; deletion removes cross-workspace edges referencing that workspace.
- Inert proxy: clicking it must not select, open, or change the canvas; only the rope popover removes the edge.

---

### Task 1: Workspace-qualified connection model

**Files:**
- Modify: `canvas/types/src/lib.rs` (`Connection`, `CanvasState`, `WorkspaceStore`, workspace tests)
- Test: `canvas/types/src/lib.rs` `mod workspace_tests`

**Interfaces:**
- Consumes: existing `Connection { from, to }`, `CanvasState::connections()`, `WorkspaceStore::workspaces()`.
- Produces: `Connection::from_workspace`, `Connection::to_workspace` optional fields; `Connection::is_cross_workspace(&self) -> bool`; `WorkspaceStore::connected_across(&self, from_ws: WorkspaceId, from: ConnectionTarget, to_ws: WorkspaceId, to: ConnectionTarget) -> bool`; `WorkspaceStore::drop_edges_for_workspace(&mut self, id: WorkspaceId)`.

- [ ] **Step 1: Write failing serde and lookup tests**

```rust
#[test]
fn old_local_connection_still_loads_without_workspace_ids() {
    let json = r#"{"from":{"Card":1},"to":{"Note":2}}"#;
    let edge: Connection = serde_json::from_str(json).unwrap();
    assert!(!edge.is_cross_workspace());
}

#[test]
fn cross_workspace_edge_round_trips_and_is_found_from_both_sides() {
    let edge = Connection { from: ConnectionTarget::Card(1), to: ConnectionTarget::Note(2),
        from_workspace: Some(1), to_workspace: Some(2) };
    assert!(edge.is_cross_workspace());
    // build a two-workspace store, insert the edge into workspace 1, then assert
    // connected_across(1, Card(1), 2, Note(2)) and the reverse are both true.
}
```

- [ ] **Step 2: Run `cargo test -p canvas-types --lib` and confirm the tests fail because the fields and helpers do not exist**

- [ ] **Step 3: Implement the optional workspace fields with `#[serde(default, skip_serializing_if = "Option::is_none")]`, the two helpers, and removal cleanup on workspace deletion**

Keep `Connection` `Copy`; both new fields are `Option<WorkspaceId>`. `connected_across` treats `None` as the workspace that stores the edge.

- [ ] **Step 4: Add the Review Focus test for duplicate ids: edges with the same local id in different workspaces must not match**

- [ ] **Step 5: Run `cargo test -p canvas-types --lib`; expected PASS**

- [ ] **Step 6: Commit with message `feat(types): workspace-qualified connections`**

### Task 2: Qualified address resolution and cross-workspace permission

**Files:**
- Modify: `canvas/ui/src/canvas.rs` (`resolve_target`, `dispatch`, `answer_check`, list rendering)
- Modify: `canvas/skills/auri/SKILL.md`
- Test: `canvas/ui/src/canvas.rs` `mod connect_tests`; `canvas/ui/src/regressions.rs`

**Interfaces:**
- Consumes: `WorkspaceStore::connected_across` from Task 1.
- Produces: `Canvas::resolve_qualified_target(&WorkspaceStore, &str) -> Option<(WorkspaceId, ConnectionTarget)>`; cross-workspace `auri from Nome <4@auro>` header.

- [ ] **Step 1: Write failing tests for `4@2`, `4@auro`, `Name @ Workspace`, local id fallback, and ambiguous workspace name**

```rust
#[test]
fn qualified_address_resolves_the_named_workspace_not_the_local_id() {
    // two workspaces both contain card 4
    assert_eq!(Canvas::resolve_qualified_target(&store, "4@auro"), Some((ws2, ConnectionTarget::Card(4))));
}
```

- [ ] **Step 2: Run the targeted tests and confirm they fail**

- [ ] **Step 3: Implement `resolve_qualified_target`, update `dispatch`'s `visible` closure to use `connected_across`, and format list lines with ` @ workspace` for remote endpoints**

- [ ] **Step 4: Update the managed skill: qualified addresses, `auri list` examples with `@`, and sender id parsing from `<4@auro>`**

- [ ] **Step 5: Add the Review Focus test for a connected normal peer: it can list and ask across, but an unconnected peer is refused**

- [ ] **Step 6: Run `cargo test -p canvas-ui --lib`; expected PASS**

- [ ] **Step 7: Commit with message `feat(canvas): resolve cross-workspace agent addresses`**

### Task 3: Background load and cross-workspace delivery

**Files:**
- Modify: `canvas/ui/src/canvas.rs` (`begin_workspace_load`, `load_workspace`, `handle_request`, `AgentRegistry`)
- Test: `canvas/ui/src/canvas.rs` `mod connect_tests`, `canvas/ui/src/regressions.rs`

**Interfaces:**
- Consumes: `Canvas::resolve_qualified_target` from Task 2.
- Produces: `Canvas::load_peer_workspace(&mut self, id: WorkspaceId, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String>`; it must not call `store.select`.

- [ ] **Step 1: Write a failing test that an ask to an unloaded peer workspace loads its cards, writes to its PTY, and leaves `store.active()` unchanged**

- [ ] **Step 2: Run it and confirm it fails**

- [ ] **Step 3: Factor the card/note rebuilding part of `load_workspace` into a helper that takes `activate: bool`; keep the existing public `load_workspace` behaviour**

- [ ] **Step 4: Implement `load_peer_workspace` with `activate = false`, then call it from `handle_request` when the resolved target workspace is unloaded**

- [ ] **Step 5: Add the Review Focus test for startup race: bytes are written only after the target writer is registered**

- [ ] **Step 6: Run `cargo test -p canvas-ui --lib`; expected PASS**

- [ ] **Step 7: Commit with message `feat(canvas): load peer workspace for cross-workspace ask`**

### Task 4: Picker, proxy card and rope removal

**Files:**
- Modify: `canvas/ui/src/theme.rs` (proxy dimensions and label colour tokens)
- Modify: `canvas/ui/src/canvas.rs` (connect gesture, proxy render, edge painter, popover removal)
- Test: `canvas/ui/src/canvas.rs` `mod connect_tests`; `canvas/ui/src/regressions.rs`

**Interfaces:**
- Consumes: qualified connection model and loader from Tasks 1–3.
- Produces: debugger selectors `connection-workspace-picker`, `connection-target-<workspace-id>`, `proxy-card-<workspace-id>-<id>`.

- [ ] **Step 1: Write failing tests for the admin gate, workspace picker, and inert proxy card using real GPUI dispatch and debug bounds**

- [ ] **Step 2: Run the tests and confirm they fail**

- [ ] **Step 3: Add the workspace step to the connect gesture: only an administrator source gets the cross-workspace option; choose workspace, then endpoint**

- [ ] **Step 4: Render one inert proxy card per remote endpoint in each workspace using the compact card shape from the spec, including the rope and pulse already used for local edges**

- [ ] **Step 5: Reuse the existing rope popover so pressing a cross-workspace rope shows both endpoints and the remove action; removal updates both workspaces**

- [ ] **Step 6: Add the Review Focus test that clicking the proxy changes no selection and opens nothing**

- [ ] **Step 7: Run `cargo test -p canvas-ui --lib`; expected PASS**

- [ ] **Step 8: Commit with message `feat(canvas): render cross-workspace proxy connections`**

### Task 5: End-to-end conversation, rename/delete cleanup and documentation

**Files:**
- Modify: `CHANGELOG.md`
- Test: `canvas/ui/src/regressions.rs` (two-workspace harness), `canvas/types/src/lib.rs` cleanup tests

**Interfaces:**
- Consumes: all previous tasks.
- Produces: a deterministic test that two agents in different workspaces exchange `auri ask` messages and ask-back through a cross-workspace edge.

- [ ] **Step 1: Write a failing test harness with workspace A admin agent and workspace B normal agent connected across; assert ask writes to B's PTY, B's reply reaches A, and active workspace does not change**

- [ ] **Step 2: Run it and confirm it fails**

- [ ] **Step 3: Implement any missing wiring only; do not add new architecture**

- [ ] **Step 4: Add the Review Focus tests for workspace rename (old qualified name rejected, `id@workspace-id` still resolves) and workspace delete (cross edges removed from other workspaces)**

- [ ] **Step 5: Update CHANGELOG with the cross-workspace connection feature**

- [ ] **Step 6: Run `cargo test --workspace`; expected PASS**

- [ ] **Step 7: Commit with message `test: verify cross-workspace agent conversations`**
