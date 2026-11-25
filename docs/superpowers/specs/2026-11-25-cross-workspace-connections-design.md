# Cross-workspace connections — design

Date: 2026-11-25
Status: draft — builder check pending, then owner approval

## Intent

Let agents, terminals and notes in different workspaces collaborate on work that
spans more than one repository. Today an Auri connection is strictly local: an
edge lives inside one workspace's `CanvasState`, and `complete_connect` only
resolves targets from the active workspace. The user wants to start the same
gesture from the island bolt or the right-click `Conectar`, choose another
workspace, choose an agent/terminal/note there, and get a live communication
path between the two.

The point is mutual work across repositories. Two agents must be able to send
and receive messages once connected. No chat panel is added: messages still
arrive as `auri from ...` envelopes in the target terminal, and the canvas only
shows the relationship and a pulse when a signal passes.

## What the user decided

- A connection may cross workspace boundaries. Agents, terminals and notes can
  be endpoints.
- The human owns the UI gesture: any selected agent, terminal or note can start
  a cross-workspace connection. CLI requests still need a stored edge and the
  normal permission matrix.
- The creation gesture is the existing one: press the island's bolt, or
  right-click a card and press `Conectar`, then choose another workspace and an
  item inside it.
- The remote endpoint is represented in the current canvas by a **proxy card**:
  a compact card with the remote agent's name and its workspace as subtitle,
  visually like the supplied Maestri example.
- Clicking the proxy card does nothing. It is a representation, not a control.
- Clicking the rope between the local agent and the proxy opens the existing
  connection popover, with the remove/cut action, like local edges.
- Once connected, a normal agent can **send and receive**. It may start a
  conversation, not merely answer.
- The same connection is represented symmetrically: opening the other
  workspace shows a proxy of the endpoint from the first workspace.
- If the remote workspace is unloaded, using the connection loads it
  automatically; the user should not have to click the workspace row first.

## Calls made in this design

These are mine, not decisions the user confirmed:

1. **Addresses.** `auri list` shows `agent 4 Oh My Pi #3 @ auro`. The canonical
   address accepted by `auri ask` and `auri check` is `4@auro`; a display name
   with `@workspace` also resolves. Workspace names must be unique in this
   slice; an ambiguous name is an error rather than a silent guess.
2. **Edge storage.** A cross-workspace edge carries workspace-qualified
   endpoints. Old local edges keep loading through the existing format. Both
   workspaces filter the same stored edge to render their side of it.
3. **Removal authority.** The user removes the edge from the rope popover in
   either workspace; there is no `auri connect` or `auri disconnect` CLI verb.
   Adding an edge remains a canvas gesture owned by the human; the source
   card's CLI role does not gate it.
4. **Auto-load failure.** If the target workspace cannot be loaded, the request
   fails with a typed error to the sender; no half-open connection is staged.
5. **Proxy inertness.** The proxy card has no click, hover action, focus target
   or selection state. This is a deliberate exception to the usual rule that a
   visible control should respond.
6. **No UI admin gate.** The drawer is the workspace selector for any selected
   source. Administrator remains a CLI permission, not a requirement to draw
   an edge. A cross edge is refused only when it would point at the source
   workspace itself or at a workspace that no longer exists.
7. **Auto-load semantics.** Load-on-demand does **not** switch the active
   workspace. It creates the target workspace's live PTYs in the background,
   waits for their writers to be registered, then delivers the bytes. Agents
   in an unloaded workspace restart fresh; that loss of in-memory conversation
   is inherent to Auri's existing unload/load model and is accepted here.
8. **Rename and delete.** Edges are stored by stable workspace id. Addresses
   resolve by workspace name at use time; if a workspace is renamed, old
   name-qualified addresses stop resolving while `id@workspace-id` remains a
   fallback. Deleting a workspace removes every cross-workspace edge that
   references it from the other workspaces.
9. **Envelope.** A cross-workspace message header becomes
   `auri from Nome <4@auro>`; the managed skill learns to parse the id inside
   `<...>` including `@workspace`. The explicit ask-back command remains the
   sanctioned reply path.
10. **Symmetry.** Both workspaces render the proxy from the same stored edge,
    and neither render switches the active workspace.

## Goals

1. Any selected agent, terminal or note can connect to an item in another
   workspace using the existing UI gesture.
2. Connected endpoints can exchange `auri ask` messages in both directions,
   including normal agents on either side.
3. Both workspaces show the relationship through a proxy card and a rope.
4. The connection survives restart and is removable from the rope popover.
5. Unloaded workspaces load on demand when a connected address is used.

## Non-goals

- No chat bubbles, message history or scraping of agent internal state.
- No broadcast, transitive relay through a third card, or auto-connect.
- No cross-machine or remote-API workspaces; all workspaces live in the same
  local app instance.
- No new CLI verb for creating or removing connections.
- No changes to how local-only connections behave.

## Anti-goals

- A proxy card that looks clickable but does nothing useful.
- A normal agent gaining reach into another workspace without a drawn edge.
- Silent id collisions when two workspaces contain the same numeric id.
- Carrying a token or credential across workspaces; each card keeps its own
  PTY token.

## Current shape to build on

- `canvas/skills/auri/SKILL.md` defines `list`, `ask`, `check`, notes and
  admin-only verbs, plus the ask-back convention.
- `canvas/types/src/lib.rs` owns `Connection`, `ConnectionTarget` and
  `CanvasState.connections`.
- `canvas/ui/src/canvas.rs` owns `complete_connect`, `complete_connect_at`,
  `visible` in dispatch, `resolve_target`, the connection painter, the pulse
  and the rope popover.
- `canvas/ui/src/permissions.rs` owns the role/permission matrix.
- `docs/superpowers/specs/2026-11-23-agent-connections-design.md` is the local
  connection contract this design extends.

## Interaction flow

1. The user selects an agent card and presses the bolt, or right-clicks a card
   and presses `Conectar`.
2. The target picker gains a workspace step: first choose a workspace, then an
   agent, terminal or note inside it. The active workspace remains the default.
3. On confirmation, the host records a workspace-qualified edge and renders a
   proxy card for the remote endpoint in the current canvas.
4. When the other workspace is opened, it renders its own proxy of the
   originating endpoint from the same edge.
5. `auri list` in each workspace includes connected remote endpoints with the
   `@workspace` suffix.
6. `auri ask "4@auro" "..."` resolves the target, loads that workspace if
   needed, writes the envelope to the target PTY and returns when queued. The
   receiver replies with its own ask-back command.
7. Pressing a cross-workspace rope opens the existing popover; removing the edge
   deletes it for both workspaces and drops both proxy cards.

## Verification

- Serde round-trip for old local edges and new workspace-qualified edges.
- Permission matrix: normal agent cannot create, but can ask a connected remote
  endpoint; unconnected remote endpoint is refused.
- Address resolution: local id, qualified id, qualified display name, duplicate
  workspace name rejection.
- Dispatch: ask/check to an unloaded workspace loads it before writing/reading.
- UI: proxy card is inert; rope popover removes the edge on both sides; pulse
  lights the cross-workspace rope.
- Persistence: restart keeps the edge and both proxies; deletion removes both.

## What is left to the builder

- Exact data shape and migration for workspace-qualified connections.
- Picker UI details and how the proxy card reuses the existing card renderer.
- Where the load-on-demand hook lives in the async request path.
- Test placement and any visual regression fixtures for the proxy card.
