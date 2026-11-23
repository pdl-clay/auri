# Agent connections, roles and local CLI — design

Date: 2026-11-23
Status: approved — ready for the implementation plan

## Intent

Turn the Auri canvas into a place where CLI agents can collaborate, but only
through relationships the user explicitly draws. There are two categories of
agent card: **normal** and **administrator**. A normal agent can read/write the
notes connected to it, send commands to the terminal connected to it, and send
messages to agents connected to it. An administrator can do all of that plus
create, edit and remove terminals, notes and agents. The user can always do
everything.

A connection exists only when the user creates it in the canvas. An agent
never connects itself, never reaches an unconnected card, and cannot message
another agent without an edge between them. Agent-to-agent messages stay
inside the terminals; the canvas does not show a message history.

## What the user decided

- Right-click an agent card → **Editar** → a toggle switches between *agente*
  and *administrador*. It can be on at creation time and toggled at any time
  afterwards, without recreating the terminal.
- To connect: select a card, click the connection icon in the right-hand
  island, then click another terminal, note or agent. The canvas draws a
  **dashed line** between the two cards.
- Communication is asynchronous. The sender includes, in the message itself,
  the command the receiver should run to reply when it finishes, and carries
  on working. The receiver replies later with `auri ask "Sender" "result"`.
  This is the `ask back` pattern from the Maestri skills.
- No chat bubbles, message log or any other representation of agent traffic
  on the canvas.
- Cards created by an administrator start **connected to the creator** and in
  **normal mode** (`admin: false`); the creator does not clone its admin role.

## Calls made in this design (not user decisions)

- An edge is **bidirectional**: both endpoints can use each other within their
  role permissions.
- Connections are stored per workspace in the persisted JSON and survive
  restart. They live inside `CanvasState` with `#[serde(default)]` so old
  config files keep loading.
- Connections are **within one workspace**. No cross-workspace edges in this
  iteration.
- `Esc` or a click on empty canvas cancels connection mode.
- A normal agent's `list` shows itself plus connected endpoints. An
  administrator's `list` shows every card and note in the workspace, because
  it has workspace-wide duties.
- Every agent process gets a private socket token through its PTY
  environment. Revoking a card revokes its token. No shared secret per
  workspace.


## Builder review resolutions

A fresh engineer reviewed the draft before planning. These were resolved from
the conversation or settled as design calls:

- **Only agent cards call the CLI.** Plain terminals are command targets and
  may be connection endpoints, but they have no `admin` field, no token and
  no CLI identity. `AgentState.admin` is the only role flag.
- **Canonical addresses are workspace-local ids**, as printed by `auri list`.
  Names are display labels, including duplicates and generated suffixes.
  Notes do not gain a name field in this iteration; `note 7` is an address.
- **`ask` writes an envelope**: one header line naming the sender, then the
  prompt text, then a separate Enter byte (`\r`). The receiver learns the
  sender's address from the header and from the ask-back command in the
  prompt. The smoke test verifies that the CLI agent actually submits.
- **Tokens are invalidated host-side** when the card is removed, the
  workspace is unloaded, or the app restarts. A role toggle does not revoke
  the token; the next request simply sees the new role. Copies of a token in
  child processes stop working when the host invalidates the card.
- **Only cards can start a connection** (plain terminal or agent), with card
  or note as target. Notes cannot start one; the island icon is gated on a
  selected card, as the user described.
- **The skill is part of this deliverable.** The user asked for skills that
  teach agents to use Auri, including the ask-back instruction. A CLI-only
  distribution without the skill does not meet the request.

## Goals

1. Any CLI agent can use the feature with no patch, adapter or model proxy: it
   runs a shell command and reads text.
2. Permissions follow the visible canvas graph and the role toggle, not a
   prompt the agent could ignore.
3. Changing the role takes effect on the next command; no restart.
4. The host remains the single owner of canvas state and PTYs. The CLI is a
   thin client, never writes the config file directly.

## Non-goals

- No A2A/HTTP server, Agent Cards, task lifecycle or push notification.
- No MCP server in the core. MCP can be added later as a thin adapter over the
  same CLI; it is not needed to make a shell command work.
- No auto-connect, no transitive reach through a third card, no broadcast.
- No message history on canvas, no scraping of the agent's internal state.
- No user confirmation prompt for every command. The user grants authority by
  drawing the edge and toggling the role.

## Why this shape

Research done for this design:

- **Maestri skills** (`maestri/SKILL.md`, `maestri-manager/SKILL.md`), read
  locally. Its `maestri` CLI runs inside the agent terminal and exposes
  `list`, `ask`, `ask --raw`, `check`, `note create/read/write/edit/delete`,
  `connect`, `recruit`, roles and bands. Authority comes from the canvas
  graph: without a connection the agent cannot see the other node. `ask --raw`
  injects raw text and escape sequences (including Enter and Ctrl-C) into the
  target terminal input, which is why it works with unrelated CLIs. The
  ask-back convention is explicitly recommended there for long tasks.
- **[agent-office](https://github.com/Mr-AK-AI/agent-office)** — terminal-native
  delegation for local CLI agents. Two verbs (`up`, `send`); `send` is
  `tmux send-keys` into a real agent terminal. No API proxy, no model adapter,
  no scraping. Confirms that typing into the target terminal is the simplest
  universal transport.
- **[swarmux](https://github.com/6missedcalls/swarmux)** — documents the
  "Enter Key Problem": an agent can compose a prompt but cannot press Enter in
  another terminal. Its fix is two distinct writes: literal text, then a
  separate Enter key. Auri already owns the PTY, so it can do the equivalent
  by writing bytes to the slave input, with the same separation.
- **[A2A](https://github.com/a2aproject/A2A)** — JSON-RPC over HTTP(S), Agent
  Cards, discovery, streaming and push notifications, designed for opaque
  agents on separate servers. Solves a different problem; too much machinery
  for local agents that share one process and one canvas.
- **[MCP server guide](https://modelcontextprotocol.io/docs/develop/build-server)** —
  stdio JSON-RPC with tools/resources/prompts. Good as an optional adapter for
  hosts that only speak MCP, but it does not itself define identity,
  permission or terminal input. The CLI is the primitive; MCP can wrap it.

## Architecture

```text
 agent CLI (inside PTY)          Auri host (Rust)
 ┌──────────────────┐            ┌──────────────────────────────────┐
 │ auri ask ...     │  socket    │ listener per PTY (token in env)  │
 │ auri note read   ├───────────►│ auth: role + edge from canvas    │
 │ auri list        │  line JSON │ dispatch to host command         │
 └──────────────────┘            │   ├─ write bytes to target PTY    │
                                 │   ├─ read screen via TerminalView │
                                 │   └─ mutate notes/cards via Canvas│
                                 └──────────────────────────────────┘
```

- **CLI**: a small Rust binary (`auri`) in this workspace, built with the app
  and copied to a cache directory at launch. The PTY environment carries
  `AURI_SOCKET` (absolute path) and `AURI_TOKEN` (random per card). No PATH
  mutation of the user's shell; the spawned shell gets the env through
  `PtySession::spawn_shell_in`, and the typed agent command inherits it.
- **Transport**: one Unix domain socket per running app, newline-delimited
  JSON requests/responses, one connection per request. The token selects the
  calling card. Local only. No writes to the app config from the CLI.
- **Host registry**: maps `TerminalId` → shared PTY writer handle and weak
  `TerminalView` / `TextCard` handles, kept next to the existing card maps in
  `Canvas`. The writer handle must be shared with `TerminalView` because the
  reader/writer are currently moved into it; add an `Arc<Mutex<...>>` handle
  retained by the host.
- **Identity**: agents and plain terminals share `TerminalId`; notes have
  `u64` ids. CLI names come from `AgentState.name` (and `TerminalState.title`
  for plain terminals); ids can also be addressed explicitly. Duplicate names
  get the same `(2)`, `(3)` suffix convention already used in the canvas, and
  `list` always prints the exact address to use.

## Data model

Add to `canvas/types/src/lib.rs`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionTarget {
    Card(TerminalId), // agent or plain terminal
    Note(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    pub from: TerminalId,
    pub to: ConnectionTarget,
}
```

`CanvasState` gains `#[serde(default)] connections: Vec<Connection>` and
`AgentState` gains `#[serde(default)] admin: bool`. The edge is stored once;
permission checks look at both directions. Removing a card or note removes
every edge that touches it. Old JSON files load with no connections and no
admins.

## UI changes

- **Island**: when a card is selected, show a connection icon beside the
  existing add/agent/note controls. Clicking it sets
  `Canvas.connecting_from = Some(selected_id)`, changes the cursor to
  crosshair, and highlights the source card.
- **Target pick**: `on_mouse_down` on `TerminalCard`, `TextCard` and the note
  card checks connection mode before its normal selection logic. On a valid
  target it inserts the edge, persists and exits mode. Clicking the same card
  is a no-op; a duplicate edge is a no-op.
- **Dashed line**: paint in the canvas element that already draws the grid,
  using card/note bounds transformed by the current zoom/offset. The line is
  drawn in world coordinates, so it moves with pan and zoom. Use short quads
  with a gap; tokens go in `theme.rs` (`CONNECTION_*`), never inline colours.
- **Edit dialog**: add the administrator toggle to `agent_edit_dialog`, bound
  to `AgentMenu.admin` / `original_admin`, using the existing Dialog switch or
  toggle component style from `gpui-component`. Saving writes
  `AgentState.admin`. Cancelling restores the original value.
- **No canvas chat UI.** Nothing renders message content.

## CLI surface

All commands are callable as `auri <verb> [args] [--json]`. Names in examples
are whatever `list` prints.

| Command | Normal | Admin | Effect |
|---|---|---|---|
| `auri list` | connected endpoints only | all cards/notes in workspace | names, ids, roles |
| `auri ask "Name" "prompt"` | if connected to a card | any card | writes prompt + Enter to that PTY; returns after send |
| `auri ask --raw "Name" "text\x03"` | if connected | any card | raw bytes, escapes like Maestri's `--raw` |
| `auri check "Name" [--lines N]` | if connected | any card | last visible screen lines of that terminal |
| `auri note read "Note" [start] [count]` | if connected | any note | text, optionally numbered |
| `auri note write "Note" "text"` | if connected | any note | replace content |
| `auri note edit "Note" "old" "new"` | if connected | any note | substring replace |
| `auri note create ["text"] [--name N]` | denied | allowed | creates note, connected to caller |
| `auri note delete "Note"` | denied | allowed | destructive, explicit only |
| `auri terminal new [--name N] [--command C]` | denied | allowed | spawns plain terminal, connected to caller |
| `auri terminal remove "Name"` | denied | allowed | removes card and its edges |
| `auri agent new "Name" [--command C]` | denied | allowed | spawns normal agent, connected to caller |
| `auri agent remove "Name"` | denied | allowed | removes card and its edges |
| `auri agent role "Name" --admin on\|off` | denied | allowed | same as the UI toggle |
| `auri connect ...` | denied | denied | user-only, no CLI verb |

`ask` always returns as soon as the bytes are queued. The skill installed with
the CLI teaches the ask-back pattern: include `auri ask "Caller" "<result>"`
in the delegated prompt, then carry on.

## Permission algorithm

For every request:

1. Resolve `AURI_TOKEN` to a live card in a loaded workspace. Unknown or
   revoked token → `permission_denied`.
2. For actions on a specific target: require an edge between caller and
   target, except for admin-only actions, which require `admin == true`.
3. An admin may act on any target in its workspace; a normal agent may not
   create/delete anything, cannot change roles, and cannot connect cards.
4. Never fall back to "allow if unsure". Errors name the missing connection
   or role so the user can fix it in the canvas.

The user is not an agent and is not subject to this layer: all existing UI
actions stay unchanged.

## Safety

- Random token per PTY, sent only through that PTY's environment; no token in
  the config file, logs or canvas state.
- Validate every command name and argument on the host. Write only the exact
  bytes requested to the target PTY; no shell interpolation by the host.
- Keep the existing destructive-action rules: `note delete` and card removal
  are admin-only and must be explicitly requested in the CLI call.
- Reject cross-workspace targets even if an id happens to exist elsewhere.
- Closing or unloading a workspace tears down its listener handles and
  invalidates all its tokens.

## Verification

- Unit tests: serde round-trip for old configs, edge removal on card/note
  deletion, permission matrix for each CLI verb (normal connected, normal
  unconnected, admin), duplicate-edge no-op, role toggle revert on cancel.
- Integration smoke in the running app: two agent cards + one note, connect
  them by the island gesture, confirm dashed lines, run `auri list` from one
  agent, send an `ask`, observe the text arriving in the other terminal,
  reply via `auri ask`, read/write the note, confirm an unconnected target is
  refused, toggle admin and confirm create/remove works, toggle back and
  confirm it stops working immediately.
- Check restart persistence: connections, roles and notes survive relaunch.
- Regression suites: a code suite that pins the edit-dialog opening, the
  empty-canvas notice, role/connection rules and old-config loading, and a
  golden-image visual suite for the empty canvas, the dashed connection line
  and the centred edit dialog. The visual suite also proves the FPS badge
  stays absent.

## Left to the builder

- Exact switch component and its token usage in the edit dialog.
- Socket framing details and reconnect/backoff in the CLI.
- How to copy/expose the `auri` binary at launch without polluting the
  user's PATH.
- Whether `check` returns plain text or JSON by default (both are exposed).
- Test harness choice for the CLI smoke test.

