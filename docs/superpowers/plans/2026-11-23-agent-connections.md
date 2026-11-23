# Agent Connections Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build user-authorized connections between agent terminals, notes and terminals, with normal/admin roles, a local CLI for agents, and dashed connection lines on the canvas.

**Architecture:** Connection edges and the admin bit are persisted in `CanvasState`; a pure permission layer checks them for every CLI request. A per-PTY token authorizes a small line-JSON IPC to the Rust host; the CLI is a thin client and the host owns all canvas/PTY state. The UI adds a connection gesture in the right island and paints edges as dashed lines.

**Tech Stack:** Rust 2024, gpui/auro, portable-pty, serde, Unix domain sockets, `clap` for the new CLI crate, existing `cargo test`.

**Spec:** `docs/superpowers/specs/2026-11-23-agent-connections-design.md`

## Global Constraints

- Only agent cards call the CLI; plain terminals are targets only, with no token and no role.
- Edges are bidirectional, per workspace, and persisted with `#[serde(default)]`.
- Only the user draws edges, except admin-created cards, which start connected to the creator with `admin: false`.
- Admin role changes take effect on the next request without restarting the agent.
- No A2A/HTTP server and no MCP server in this iteration; no message history on canvas.
- Canonical CLI addresses are workspace-local ids printed by `auri list`; names are display labels.
- `ask` writes a sender header, the prompt text, then a separate Enter byte (`\r`).
- All visual tokens stay in `canvas/ui/src/theme.rs`; no inline colours or magic style numbers in components.
- Every task ends with its own tests and a commit using the eval `commit` namespace (never `git add`/`git commit`).
- Automated regression coverage is mandatory: every fixed behaviour gets a
  code test, and every visible canvas state gets a golden-image check (Task 9).

## Review Focus

- A CLI agent may not submit a prompt because its TUI expects a different Enter byte — `ask` smoke test in Task 5.
- A demoted admin with a live token must be denied on the next request — permission test in Task 2.
- Removing a card or note must leave no dangling edges or unreachable targets — data tests in Task 1.
- Configs written before this feature must load with no connections and no admins — serde test in Task 1.
- An admin-created agent must be normal and connected to its creator — creation test in Task 5.

---

### Task 1: Persist roles and connections

**Files:**
- Modify: `canvas/types/src/lib.rs` (AgentState, CanvasState, removal methods, tests)
- Modify: `canvas/ui/src/config.rs` (tests for old config files)

**Interfaces:**
- Produces: `ConnectionTarget::{Card(TerminalId), Note(u64)}`, `Connection { from, to }`, `AgentState.admin: bool`, `CanvasState::{connections, connect, disconnect, connected, remove_connections_for}`.

- [ ] **Step 1: Write failing tests in `canvas/types/src/lib.rs`**

Assert: `connect` returns `false` for a duplicate or self-edge; `connected(a, b)` is true when the edge was added in either direction; `remove_agent`, `remove` and `remove_note` clear every touching edge; serde round-trip preserves connections and `admin`; a JSON line without the new fields deserializes with empty connections and `admin: false`.

- [ ] **Step 2: Run the tests**

Run: `cd canvas && cargo test -p canvas-types --lib`
Expected: FAIL because the types and methods do not exist.

- [ ] **Step 3: Add the types and methods**

Add `#[serde(default)] admin: bool` to `AgentState` and `#[serde(default)] connections: Vec<Connection>` to `CanvasState`. Implement `connect`/`disconnect` with duplicate and self checks, `connected` scanning both directions, and edge cleanup inside the existing removal methods. Keep `next_id` and selection logic untouched.

- [ ] **Step 4: Run the tests**

Run: `cd canvas && cargo test -p canvas-types --lib`
Expected: PASS, including the old-config test in `canvas/ui/src/config.rs` (`cargo test -p canvas-ui --lib`).

- [ ] **Step 5: Commit**

Use `commit.draft()` with `repo=".worktrees/canvas-terminals"` and commit only these two files.

---

### Task 2: Pure permission matrix

**Files:**
- Create: `canvas/ui/src/permissions.rs`
- Modify: `canvas/ui/src/lib.rs` (`pub mod permissions;`)

**Interfaces:**
- Consumes: `WorkspaceState`, `CanvasState::connected`, `AgentState.admin` from Task 1.
- Produces: `Principal { id: TerminalId, admin: bool }`, `Action` with variants `List`, `Ask { target, raw }`, `Check { target }`, `NoteRead { note }`, `NoteWrite { note }`, `NoteCreate`, `NoteDelete { note }`, `TerminalNew`, `TerminalRemove { target }`, `AgentNew`, `AgentRemove { target }`, `AgentRole { target }`; `Denied::{NoIdentity, NotAdmin, NotConnected}`; `authorize(workspace: &WorkspaceState, principal: Option<&Principal>, action: &Action) -> Result<(), Denied>`.

- [ ] **Step 1: Write failing tests in `permissions.rs`**

Cover: no principal → `NoIdentity`; normal connected to a note → `NoteWrite` ok, `NoteCreate` → `NotAdmin`; normal unconnected to an agent → `Ask` → `NotConnected`; admin unconnected target → `Ask`/`NoteWrite`/`TerminalRemove` ok, `AgentRole` ok; normal `AgentRole` → `NotAdmin`; role toggle from `admin: false` to `true` in the same workspace changes the next result.

- [ ] **Step 2: Run the tests**

Run: `cd canvas && cargo test -p canvas-ui --lib permissions`
Expected: FAIL because the module is missing.

- [ ] **Step 3: Implement `authorize`**

Admin actions are everything except `List`, `Ask`, `Check`, `NoteRead` and `NoteWrite`; those five require an edge to the target. `List` is allowed for any identity. Never infer authority from a name string.

- [ ] **Step 4: Run the tests**

Run: `cd canvas && cargo test -p canvas-ui --lib permissions`
Expected: PASS.

- [ ] **Step 5: Commit**

Commit `canvas/ui/src/permissions.rs` and `canvas/ui/src/lib.rs`.

---

### Task 3: Shared IPC protocol crate and CLI scaffold

**Files:**
- Create: `canvas/protocol/Cargo.toml`, `canvas/protocol/src/lib.rs`
- Modify: `canvas/Cargo.toml` (workspace member + package dependency)
- Create: `canvas/auri-cli/Cargo.toml`, `canvas/auri-cli/src/main.rs`
- Modify: `canvas/Cargo.toml` (workspace member)

**Interfaces:**
- Produces: `Request` enum mirroring `Action` plus `Skill`, `Response::{Ok(String), Err(String)}`, `encode(&Request) -> String`, `decode(&str) -> Result<Request, Error>`; CLI `auri` parses argv into `Request` and prints responses.

**Spec pins:** CLI verbs `list`, `ask [--raw]`, `check`, `note read|write|edit|create|delete`, `terminal new|remove`, `agent new|remove|role`, `skill`; `--json` on every verb.

- [ ] **Step 1: Write failing codec tests**

Assert round-trip for every `Request` variant, rejection of unknown verbs and malformed JSON, and that `--raw` escape strings (`\n`, `\t`, `\e`, `\xNN`) decode into the exact bytes the CLI will send.

- [ ] **Step 2: Run the tests**

Run: `cd canvas && cargo test -p auri-protocol --lib`
Expected: FAIL because the crate does not exist.

- [ ] **Step 3: Implement the protocol crate and CLI argument parsing**

Use newline-delimited serde JSON. CLI reads `AURI_SOCKET` and `AURI_TOKEN`, connects, sends one line, reads one line, prints it. `auri skill` prints the skill file path from `AURI_SKILL`. No real host needed yet.

- [ ] **Step 4: Run the tests**

Run: `cd canvas && cargo test -p auri-protocol --lib -p auri-cli`
Expected: PASS.

- [ ] **Step 5: Commit**

Commit the two new crates and the manifest changes.

---

### Task 4: Per-PTY token, env and host listener

**Files:**
- Modify: `canvas/ui/src/pty.rs`
- Modify: `canvas/ui/src/canvas.rs` (token registry, listener lifecycle)
- Modify: `canvas/ui/src/lib.rs` (start/stop listener)

**Interfaces:**
- Consumes: `encode`, `decode` and `Request` from Task 3; `authorize` from Task 2.
- Produces: `PtySession::spawn_shell_in_with_env(cols, rows, dir, env: &[(String, String)])`; `Canvas::register_agent_pty(id, writer: Arc<Mutex<Box<dyn Write + Send>>>)`; `Canvas::handle_request(token: &str, request: Request) -> Response`; token strings from a random generator.

- [ ] **Step 1: Write failing tests**

Assert: a spawned shell sees `AURI_SOCKET` and `AURI_TOKEN` in its environment; an unknown token yields `Response::Err("permission_denied")`; a removed card's token yields the same error; requesting from a plain terminal id yields `NoIdentity`.

- [ ] **Step 2: Run the tests**

Run: `cd canvas && cargo test -p canvas-ui --lib pty -- --nocapture`
Expected: FAIL for the missing env/registry functions.

- [ ] **Step 3: Implement token registry and listener**

One Unix socket under `XDG_RUNTIME_DIR`; each agent PTY gets a random token and env. Keep a `HashMap<Token, TerminalId>` in `Canvas`. Resolve token → workspace → principal; call `authorize` before dispatch. Revoke on card removal, workspace unload and app shutdown.

- [ ] **Step 4: Run the tests**

Run: `cd canvas && cargo test -p canvas-ui --lib`
Expected: PASS.

- [ ] **Step 5: Commit**

Commit the four modified files.

---

### Task 5: Host dispatch: list, ask, check and notes

**Files:**
- Modify: `canvas/ui/src/canvas.rs` (dispatch arms, target resolution, PTY write/screen reads)
- Modify: `canvas/ui/src/terminal_card.rs` if a public writer helper is needed (prefer `card.rs`)
- Modify: `canvas/ui/src/card.rs` (shared writer handle and `TerminalCard::write_input`)

**Interfaces:**
- Consumes: `Request`/`Response` and `authorize` from earlier tasks.
- Produces: `Canvas::resolve_target(workspace, name_or_id) -> Option<ConnectionTarget>`; `TerminalCard::write_input(&self, bytes: &[u8]) -> io::Result<()>`.

**Spec pins:** `ask` payload = sender header line + prompt text + `\r`; returns immediately. `check` returns the target's visible screen lines. Admin-created cards are connected to the creator and have `admin: false`.

- [ ] **Step 1: Write failing tests**

Assert: `ask` on a connected target writes the exact bytes `[header, prompt, \r]` to the shared writer; an unconnected `ask` returns `permission_denied` and writes nothing; `note write` changes the `NoteState` text; `terminal new`/`agent new` from an admin create a card with an edge to the caller and `admin: false`; `agent role --admin on` changes the stored bit.

- [ ] **Step 2: Run the tests**

Run: `cd canvas && cargo test -p canvas-ui --lib canvas`
Expected: FAIL for missing dispatch arms.

- [ ] **Step 3: Implement dispatch**

Resolve targets by canonical id first, then display name. Validate before writing anything. Use the existing note mutation and card spawn/remove paths; insert the creator edge on creation. Never call the shell with the request text.

- [ ] **Step 4: Run the tests and one live smoke**

Run `cargo test -p canvas-ui --lib`, then in the running dev app run `auri list`, `auri ask "<id>" "ping"` from one agent and confirm the text and submit arrive in the other terminal.

- [ ] **Step 5: Commit**

Commit the modified files.

---

### Task 6: Connection gesture and dashed lines

**Files:**
- Modify: `canvas/ui/src/canvas.rs` (mode state, island icon, target clicks, paint pass)
- Modify: `canvas/ui/src/card.rs`, `canvas/ui/src/text_card.rs` (target hooks)
- Modify: `canvas/ui/src/theme.rs` (`CONNECTION_*` tokens)

**Interfaces:**
- Consumes: `CanvasState::connect` from Task 1.
- Produces: `Canvas::begin_connect(source: TerminalId, cx)`, `complete_connect(target: ConnectionTarget, cx)`, `cancel_connect(cx)`.

- [ ] **Step 1: Write failing tests**

Assert: clicking the island icon with no selected card does nothing; with a selected card the mode stores the source; clicking the same card is a no-op; a second click inserts one edge; Esc and empty-canvas click clear the mode; duplicated edges are not inserted. Add a pure helper `connect_target(selected, clicked) -> Option<(from, to)>` for these assertions.

- [ ] **Step 2: Run the tests**

Run: `cd canvas && cargo test -p canvas-ui --lib canvas`
Expected: FAIL because the helper does not exist.

- [ ] **Step 3: Implement the UI**

Island icon uses the existing icon helper and an element id `island-connect`. Set crosshair cursor, highlight the source card, and route card/note mouse-down to `complete_connect` before their normal selection logic. Paint the dashed line inside the existing grid canvas element using world coordinates transformed by zoom/offset and the new theme tokens.

- [ ] **Step 4: Run the tests and capture the canvas**

Run `cargo test -p canvas-ui --lib`, focus the dev app, and take a screenshot showing a dashed line between two cards after the gesture.

- [ ] **Step 5: Commit**

Commit the four modified files.

---

### Task 7: Admin toggle in the edit dialog

**Files:**
- Modify: `canvas/ui/src/canvas.rs` (`AgentMenu`, `agent_edit_dialog`, save/cancel)
- Modify: `canvas/ui/src/theme.rs` only if the switch needs a new dimension token

**Interfaces:**
- Consumes: `AgentState.admin` and `Canvas::save_agent_edits` from earlier tasks.
- Produces: `AgentMenu { admin, original_admin }`; `select_agent_admin(&mut self, value: bool, cx)`.

**Spec pins:** toggle available at creation and any time afterwards; cancel restores the original value; effect on the next command.

- [ ] **Step 1: Write failing tests**

Assert: selecting the toggle updates only the form state; saving persists `admin: true` in `AgentState`; cancelling restores `original_admin`; a second edit can turn it off again.

- [ ] **Step 2: Run the tests**

Run: `cd canvas && cargo test -p canvas-ui --lib canvas`
Expected: FAIL for missing fields.

- [ ] **Step 3: Add the toggle**

Use the existing `gpui-component` switch/checkbox style; place it after "Agente" in `agent_edit_dialog`. Keep the dialog centred as it is now. Wire the toggle and cancel/save paths.

- [ ] **Step 4: Run the tests and UI smoke**

Run `cargo test -p canvas-ui --lib`, then right-click an agent → Editar, toggle, save, reopen and confirm the state. Verify the next CLI request follows the new role.

- [ ] **Step 5: Commit**

Commit the modified files.

---

### Task 8: Skill file, distribution and final end-to-end proof

**Files:**
- Create: `canvas/skills/auri/SKILL.md`
- Modify: `canvas/ui/src/canvas.rs` (copy skill to cache at launch, set `AURI_SKILL`)
- Modify: `canvas/ui/src/lib.rs` (startup hook)
- Modify: `docs/superpowers/specs/2026-11-23-agent-connections-design.md` only if verification found a correction

**Interfaces:**
- Consumes: `auri skill` from Task 3 and per-PTY env from Task 4.

**Spec pins:** the skill teaches `auri list` first, the ask-back command in every delegated prompt, note read/write, and that connections are user-controlled.

- [ ] **Step 1: Write a failing test**

Assert `auri skill` prints a file containing `auri list` and `auri ask`, and that the host writes it to a cache path before spawning agent cards.

- [ ] **Step 2: Run the test**

Run: `cd canvas && cargo test -p auri-cli -p canvas-ui --lib`
Expected: FAIL because the skill file does not exist.

- [ ] **Step 3: Add and distribute the skill**

Write the SKILL.md with the exact CLI surface and ask-back pattern. Host writes it once into an XDG cache directory and adds `AURI_SKILL` to agent PTY env. No writes inside user repositories.

- [ ] **Step 4: Full smoke**

With two agent cards and one note: connect by the island gesture, confirm dashed lines; from one agent run `list`, `ask`, `note read/write`; confirm an unconnected target is denied; toggle admin and create a terminal, confirm it is connected to the creator and normal; toggle back and confirm creation is denied; restart and confirm edges, roles and notes survive.

- [ ] **Step 5: Commit**

Commit all remaining files and the spec correction if any.

### Task 9: Regression suites — code and visual

**Files:**
- Create: `canvas/ui/src/regressions.rs` (test-only module, wired from `canvas/ui/src/lib.rs`)
- Create: `canvas/scripts/visual-regression.sh`
- Create: `canvas/tests/visual/fixture/auro-canvas/workspaces.json`
- Create (recorded once, then committed): `canvas/tests/visual/golden/empty-canvas.png`,
  `canvas/tests/visual/golden/connected-cards.png`,
  `canvas/tests/visual/golden/edit-dialog.png`

**Interfaces:**
- Consumes: every helper from Tasks 1–8; no new production API except a
  test-only `pub(crate) fn render_state_labels(&self) -> Vec<String>` on
  `Canvas` if the dialog/notice assertions need it.

**Spec pins:** the regressions cover the bugs fixed immediately before this
feature as well as its new rules: the edit dialog opens from the menu, the
FPS badge is gone, the empty-canvas notice appears only when no workspace is
loaded, connections disappear with their cards, admin-created cards are normal
and connected to the creator, and the dashed line is visible.

- [ ] **Step 1: Write failing code regression tests in `canvas/ui/src/regressions.rs`**

Test names and assertions:
  - `edit_dialog_opens_from_agent_menu` — simulating the menu click sets the
    edit form active and leaves no dismiss overlay.
  - `workspace_notice_follows_load_state` — notice present for no active
    workspace and for unloaded workspace, absent after load.
  - `fps_badge_is_not_rendered` — rendered canvas labels contain no FPS text.
  - `connection_dies_with_its_card` — removing either endpoint removes the edge.
  - `admin_created_card_is_connected_and_normal` — creator edge exists,
    `admin == false`.
  - `old_config_regression_loads_without_connections` — JSON from before this
    feature deserializes with empty edges and no admins.

- [ ] **Step 2: Write the failing visual script**

`canvas/scripts/visual-regression.sh --record|--check` launches the debug app
with the isolated fixture config and runtime dir, records the PID, finds the
matching window in `niri msg --json windows`, focuses it, waits for a stable
frame, captures with `grim`, crops the window rectangle with `magick`, and
compares against the golden with `magick compare -metric AE`. Threshold:
at most 0.5% differing pixels. `--record` writes the goldens once.
Fixture states: (1) unloaded workspace → empty-canvas notice and no FPS badge;
(2) two connected cards → dashed line present; (3) right-click → Editar →
centred edit dialog. The script must fail if the window or a golden is missing.

- [ ] **Step 3: Run both suites to verify they fail**

Run: `cd canvas && cargo test -p canvas-ui --lib regressions`
Run: `cd canvas && ./scripts/visual-regression.sh --check`
Expected: FAIL — test module and goldens do not exist yet.

- [ ] **Step 4: Implement the tests, record goldens, and verify**

Wire `regressions` into `lib.rs`, add the test-only label helper if needed,
create the fixture config with fixed card bounds, run `--record` once, then
confirm both commands pass. Review the recorded goldens as images before
committing them; a blank or wrong-state golden invalidates the suite.

- [ ] **Step 5: Commit**

Commit the test module, script, fixture and goldens together so the baseline
travels with the code it guards.
