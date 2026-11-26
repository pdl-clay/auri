# Agent Chat View Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a persisted chat presentation to every agent card, with OMP history/status from the session JSONL, a queued composer, markdown/code-copy rendering, and a terminal-snapshot fallback for other agents.

**Architecture:** The terminal stays mounted and keeps owning the PTY. A `ChatView` entity renders above the terminal body when `AgentView::Chat` is selected. Pure modules parse OMP JSONL, derive statuses, manage the FIFO, and discover skills; the card and canvas only wire entities, persistence and focus.

**Tech Stack:** Rust 2024, GPUI 0.2.2, gpui-component 0.5.1 (`TextView`, `InputState`, `Button`, `ScrollHandle`), canvas-types serde model, existing PTY/card infrastructure.

**Spec:** `docs/superpowers/specs/2026-11-26-agent-chat-view-design.md`

## Global Constraints

- Every visual token (colors, radii, spacing, font sizes, component dimensions) lives in `canvas/ui/src/theme.rs`; components contain no hex or magic style numbers.
- The terminal view and PTY are never recreated when switching views; chat is an overlay.
- Do not modify `canvas/vendor/gpui-terminal` or `canvas/vendor/gpui-component` in this plan.
- Do not add new external crates; `TextView`, `InputState`, `ScrollHandle` and `Button` already exist.
- OMP session parsing is tolerant: unknown, malformed, truncated or non-UTF-8 records are ignored, never panicked on.
- All file I/O for transcripts runs off the UI thread.
- Tests are written first and must fail before implementation, per task.
- Commit after every task. Run the task’s targeted test before committing, and `cargo test --workspace` before the final task commit.
- Use the existing debug selector pattern for UI tests: `.debug_selector(|| "...".to_string())`.
- Do not change the terminal card behavior, keybindings, connection gestures or workspace persistence semantics.

## Review Focus

- OMP session missing, unreadable, truncated mid-line, or replaced/truncated while tailing: the chat must show an informative state, not a blank or a panic.
- Shell surviving after the agent exits: chat must not send text that becomes a shell command; input must disable when liveness is unknown.
- OMP `ask`/permission prompt blocking the turn: queue must hold and the UI must point to the terminal.
- Queue ordering/duplication around `/skill:` and messages echoed late by the JSONL: one bubble per send, original order.
- Long answers, long histories and code blocks: no UI-thread blocking, no unbounded growth, copy works, scroll stays usable.

---

### Task 1: Persist `AgentView` per agent card

**Files:**
- Modify: `canvas/types/src/lib.rs` (add `AgentView`, add field to `AgentState`, add `CanvasState::set_agent_view`, update constructors/tests)
- Test: `canvas/types/src/lib.rs` (test module)

**Interfaces:**
- Produces:
  - `pub enum AgentView { Terminal, Chat }`, `Default = Terminal`, serde snake_case.
  - `AgentState.view: AgentView` with `#[serde(default)]`.
  - `pub fn CanvasState::set_agent_view(&mut self, id: TerminalId, view: AgentView) -> bool`.
- Later tasks read `AgentState.view` when spawning and call `set_agent_view` from the canvas.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn old_agent_config_defaults_to_terminal_view() {
    let json = r#"{"workspaces":[{"id":1,"name":"Auri","icon":"Star","root":"/tmp","canvas":{"agents":[{"id":1,"kind":"Omp","name":"OMP","bounds":{"x":0.0,"y":0.0,"width":300.0,"height":200.0}}]}}]}"#;
    let store: WorkspaceStore = serde_json::from_str(json).unwrap();
    assert_eq!(store.workspaces()[0].canvas.agents()[0].view, AgentView::Terminal);
}

#[test]
fn agent_view_roundtrips_chat() {
    let mut store = WorkspaceStore::new();
    let workspace = store.add("A".into(), WorkspaceIcon::Star, std::env::temp_dir());
    let agent = workspace.canvas.add_agent(AgentKind::Omp, bounds());
    assert!(store.workspace_mut(workspace.id).unwrap().canvas.set_agent_view(agent.id, AgentView::Chat));
    let json = serde_json::to_string(&store).unwrap();
    let decoded: WorkspaceStore = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.workspaces()[0].canvas.agents()[0].view, AgentView::Chat);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p canvas-types --lib`
Expected: FAIL because `AgentView`/`view`/`set_agent_view` do not exist.

- [ ] **Step 3: Implement the enum, field and mutator**

Add the enum above `AgentState`; add `#[serde(default)] pub view: AgentView`; initialize `view: AgentView::Terminal` in every `AgentState` constructor; add `CanvasState::set_agent_view`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p canvas-types --lib`
Expected: PASS, including existing type tests.

- [ ] **Step 5: Commit**

```bash
git add canvas/types/src/lib.rs
git commit -m "feat(types): persist per-agent view mode"
```

---

### Task 2: Parse OMP transcript records into chat events

**Files:**
- Create: `canvas/ui/src/chat.rs`
- Modify: `canvas/ui/src/lib.rs` (`mod chat;`)
- Test: `canvas/ui/src/chat.rs` (`mod tests`)

**Interfaces:**
- Produces:
```rust
pub enum ChatRole { User, Agent, Info }
pub enum ChatFormat { Markdown, Plain }
pub struct ChatMessage { pub role: ChatRole, pub text: String, pub format: ChatFormat }

pub enum ChatStatus {
    Thinking,
    Tool(String),
    Writing,
    Prompt,
    Exited,
}
impl ChatStatus { pub fn label(&self) -> String; }

pub enum TranscriptEvent {
    User(String),
    AgentFinal(String),
    Status(ChatStatus),
}
pub fn parse_transcript_line(line: &str) -> Option<TranscriptEvent>;
```
- Later tasks append `ChatMessage`/`ChatStatus` to `ChatView`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn final_assistant_message_becomes_one_agent_bubble() {
    let line = r#"{"type":"message","message":{"role":"assistant","content":[{"type":"thinking","thinking":"x"},{"type":"text","text":"Olá"},{"type":"text","text":"mundo"}]}}"#;
    assert_eq!(
        parse_transcript_line(line),
        Some(TranscriptEvent::AgentFinal("Olá\n\nmundo".into()))
    );
}

#[test]
fn tool_call_becomes_status_not_bubble() {
    let line = r#"{"type":"message","message":{"role":"assistant","content":[{"type":"thinking","thinking":"x"},{"type":"toolCall","name":"read_file"}]}}"#;
    assert_eq!(
        parse_transcript_line(line),
        Some(TranscriptEvent::Status(ChatStatus::Tool("read_file".into())))
    );
}

#[test]
fn ask_tool_requests_the_terminal() {
    let line = r#"{"type":"custom","customType":"tool_execution_start","data":{"toolName":"ask"}}"#;
    assert_eq!(
        parse_transcript_line(line),
        Some(TranscriptEvent::Status(ChatStatus::Prompt))
    );
}

#[test]
fn unknown_and_truncated_records_are_ignored() {
    assert_eq!(parse_transcript_line(""), None);
    assert_eq!(parse_transcript_line("{not json"), None);
    assert_eq!(parse_transcript_line(r#"{"type":"model_usage"}"#), None);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p canvas-ui --lib chat::tests -v`
Expected: FAIL because `chat` module does not exist.

- [ ] **Step 3: Implement the parser**

Use `serde_json::Value`. Rules:
- `message/user` with text blocks → `User(joined text)`.
- `message/assistant` with any `toolCall` → `Status(Tool(last toolCall.name))`; `name == "ask"` → `Prompt`.
- `message/assistant` with thinking only → `Status(Thinking)`.
- `message/assistant` with text and no toolCall → `AgentFinal(joined text)`.
- `message/assistant` without text and without toolCall → `None`.
- `custom/tool_execution_start` → `Status(Tool(data.toolName))`; `toolName == "ask"` → `Prompt`.
- `message/toolResult` → `Status(Writing)`.
- `custom/session_exit` → `Status(Exited)`.
- Everything else → `None`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p canvas-ui --lib chat::tests -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add canvas/ui/src/chat.rs canvas/ui/src/lib.rs
git commit -m "feat(chat): parse OMP transcript events"
```

---

### Task 3: FIFO queue with confirmation and deduplication

**Files:**
- Modify: `canvas/ui/src/chat.rs`
- Test: `canvas/ui/src/chat.rs` (`mod tests`)

**Interfaces:**
- Produces:
```rust
pub struct PendingMessage { pub text: String, pub is_skill: bool }
pub struct ChatQueue { /* VecDeque<PendingMessage> */ }
impl ChatQueue {
    pub fn enqueue(&mut self, text: String, is_skill: bool);
    pub fn front(&self) -> Option<&PendingMessage>;
    pub fn confirm(&mut self, text: &str) -> bool;
    pub fn remove_front(&mut self);
    pub fn is_empty(&self) -> bool;
    pub fn len(&self) -> usize;
}
pub fn normalize_message(text: &str) -> String;
```
- Later `ChatView` uses `front` to decide the next send and `confirm` when JSONL echoes it.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn queue_keeps_fifo_order() {
    let mut q = ChatQueue::default();
    q.enqueue("primeira".into(), false);
    q.enqueue("segunda".into(), false);
    assert_eq!(q.front().unwrap().text, "primeira");
    q.remove_front();
    assert_eq!(q.front().unwrap().text, "segunda");
}

#[test]
fn confirm_removes_the_matching_normalized_message() {
    let mut q = ChatQueue::default();
    q.enqueue("  Olá   mundo  ".into(), false);
    q.enqueue("segunda".into(), false);
    assert!(q.confirm("Olá mundo"));
    assert_eq!(q.front().unwrap().text, "segunda");
    assert!(!q.confirm("não existe"));
}

#[test]
fn skill_messages_are_marked_and_confirm_immediately_on_send() {
    let mut q = ChatQueue::default();
    q.enqueue("/skill:tdd".into(), true);
    assert!(q.front().unwrap().is_skill);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p canvas-ui --lib chat::tests -v`
Expected: FAIL because `ChatQueue` does not exist.

- [ ] **Step 3: Implement `ChatQueue` and `normalize_message`**

`normalize_message` collapses whitespace and trims. `confirm` matches the first pending whose normalized text equals the argument, removes it, returns true; if no match, false. `confirm` must not reorder the rest.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p canvas-ui --lib chat::tests -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add canvas/ui/src/chat.rs
git commit -m "feat(chat): add FIFO queue with confirmation"
```

---

### Task 4: Read and tail an OMP JSONL file

**Files:**
- Create: `canvas/ui/src/chat_reader.rs`
- Modify: `canvas/ui/src/lib.rs` (`mod chat_reader;`)
- Test: `canvas/ui/src/chat_reader.rs` (`mod tests`)

**Interfaces:**
- Produces:
```rust
pub struct TranscriptTail {
    pub path: PathBuf,
    pub offset: u64,
    pub partial: String,
}
impl TranscriptTail {
    pub fn new(path: PathBuf, limit: usize) -> Self;
    pub fn read_initial(&mut self) -> Result<Vec<TranscriptEvent>, String>;
    pub fn read_appended(&mut self) -> Result<Vec<TranscriptEvent>, String>;
}
pub fn read_events(path: &std::path::Path, limit: usize) -> Result<Vec<TranscriptEvent>, String>;
```
- `read_initial` reads the file, caps the returned events to the last `limit` messages, and stores the end offset.
- `read_appended` reads bytes after `offset`, keeps an incomplete last line in `partial`, and resets offset if the file shrank.

- [ ] **Step 1: Write the failing tests**

Use `tempfile::tempdir()` and write JSONL lines with `std::fs::write` / `OpenOptions::append`.

```rust
#[test]
fn initial_read_returns_events_and_remembers_offset() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    std::fs::write(&path, format!("{}\n", USER_LINE)).unwrap();
    let mut tail = TranscriptTail::new(path, 200);
    let events = tail.read_initial().unwrap();
    assert_eq!(events, vec![TranscriptEvent::User("oi".into())]);
    assert!(tail.offset > 0);
}

#[test]
fn appended_read_keeps_partial_line_until_complete() {
    // write half a line, call read_appended -> empty; append the rest + '\n'
    // call read_appended -> the event appears.
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p canvas-ui --lib chat_reader::tests -v`
Expected: FAIL because the module does not exist.

- [ ] **Step 3: Implement the reader**

Use `std::fs::File`, `Read`, `seek`, `metadata().len()`. Parse complete lines with `parse_transcript_line`; ignore empty/parse failures. Never block the UI thread from this module; it is called from a background task by later tasks.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p canvas-ui --lib chat_reader::tests -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add canvas/ui/src/chat_reader.rs canvas/ui/src/lib.rs
git commit -m "feat(chat): tail OMP transcript files"
```

---

### Task 5: Static chat view with markdown, code copy and composer

**Files:**
- Create: `canvas/ui/src/chat_view.rs`
- Modify: `canvas/ui/src/lib.rs` (`mod chat_view;`)
- Modify: `canvas/ui/src/theme.rs` (chat tokens)
- Test: `canvas/ui/src/chat_view.rs` (`mod tests`)

**Interfaces:**
- Produces:
```rust
pub struct ChatView {
    pub input: Entity<InputState>,
    /* messages, status, scroll handle, queue, callbacks */
}
impl ChatView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self;
    pub fn set_messages(&mut self, messages: Vec<ChatMessage>, cx: &mut Context<Self>);
    pub fn push_message(&mut self, message: ChatMessage, cx: &mut Context<Self>);
    pub fn set_status(&mut self, status: Option<ChatStatus>, cx: &mut Context<Self>);
    pub fn set_info(&mut self, text: String, cx: &mut Context<Self>);
    pub fn focus_input(&self, window: &mut Window);
    pub fn value(&self, cx: &App) -> String;
}
```
- `ChatView` owns only presentation; sending is wired in Task 7.

- [ ] **Step 1: Write the failing UI test**

```rust
#[test]
fn chat_view_renders_messages_and_composer() {
    let mut cx = cross_test_cx();
    let (view, vcx) = cx.add_window_view(|window, cx| ChatView::new(window, cx));
    view.update(&mut cx, |chat, cx| {
        chat.set_info("Histórico não estruturado. Envie uma mensagem.".into(), cx);
        chat.push_message(ChatMessage { role: ChatRole::Agent, text: "Olá".into(), format: ChatFormat::Markdown }, cx);
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("chat-message-1").is_some());
    assert!(vcx.debug_bounds("chat-input").is_some());
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p canvas-ui --lib chat_view::tests -v`
Expected: FAIL because `ChatView` does not exist.

- [ ] **Step 3: Implement the view**

Render a `flex_col` with:
- a scrollable message area (`.track_scroll(&self.scroll)` + `.overflow_y_scroll()`);
- one element per message with debug selector `chat-message-{index}`;
- agent/user messages using `TextView::markdown` with `.selectable(true)` and `.code_block_actions` returning a `Button` labeled “Copiar” that writes `code.code()` to the clipboard;
- status element with selector `chat-status`;
- `Input::new(&self.input)` with selector `chat-input`.
Add all new colors/sizes to `theme.rs` and use them here. Use `InputState::new(window, cx).placeholder("Mensagem para o agente")`.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p canvas-ui --lib chat_view::tests -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add canvas/ui/src/chat_view.rs canvas/ui/src/lib.rs canvas/ui/src/theme.rs
git commit -m "feat(chat): add markdown chat view"
```

---

### Task 6: Card toggle, overlay and persisted view

**Files:**
- Modify: `canvas/ui/src/card.rs`
- Modify: `canvas/ui/src/canvas.rs`
- Test: `canvas/ui/src/regressions.rs`

**Interfaces:**
- Produces:
  - `TerminalCard.view: AgentView`
  - `pub fn TerminalCard::set_view(&mut self, view: AgentView, cx: &mut Context<Self>)`
  - `pub fn Canvas::set_agent_view(&mut self, workspace: WorkspaceId, id: TerminalId, view: AgentView, cx: &mut Context<Self>)`
- Consumes: `AgentView`/`AgentState.view` from Task 1.
- Later tasks add `chat: Option<Entity<ChatView>>` and session wiring here.

- [ ] **Step 1: Write the failing regression tests**

```rust
#[test]
fn agent_view_toggle_persists() {
    // build Canvas, add OMP agent, call canvas.set_agent_view(..., AgentView::Chat)
    // assert store agent view == Chat and debug_bounds("terminal-toggle") exists
}

#[test]
fn agent_view_reopens_saved_mode() {
    // store with agent view Chat; build Canvas; assert debug_bounds("chat-view") is some
    // and debug_bounds("terminal-body") is still present behind it
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p canvas-ui --lib regressions::agent_view -v`
Expected: FAIL: no toggle/view.

- [ ] **Step 3: Implement toggle and overlay**

In `TerminalCard`:
- store `view: AgentView`; initialize from the state passed by the canvas;
- render the terminal body as today (`terminal-body` debug selector);
- when `view == Chat`, render a placeholder overlay child `chat-view` that will become the `ChatView` entity in Task 7;
- render the toggle overlay at top-right with selector `chat-toggle` (terminal mode) or `terminal-toggle` (chat mode); on click calls `Canvas::set_agent_view`;
- keep header drag/close untouched.

In `Canvas`:
- `set_agent_view` updates the workspace state, calls `finish_store_change`, and tells the card to `set_view`;
- `spawn_agent`/`spawn_cli_card` pass the saved `AgentState.view` to the card.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p canvas-ui --lib regressions::agent_view -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add canvas/ui/src/card.rs canvas/ui/src/canvas.rs canvas/ui/src/regressions.rs
git commit -m "feat(card): toggle terminal and chat views"
```

---

### Task 7: Wire OMP history, statuses, queue and send adapter

**Files:**
- Modify: `canvas/ui/src/chat_view.rs`
- Modify: `canvas/ui/src/card.rs`
- Modify: `canvas/ui/src/canvas.rs`
- Test: `canvas/ui/src/regressions.rs`, `canvas/ui/src/chat_view.rs`

**Interfaces:**
- Produces:
```rust
pub fn ChatView::set_session(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>);
pub fn ChatView::enqueue(&mut self, text: String, is_skill: bool, cx: &mut Context<Self>);
pub fn ChatView::agent_finished(&mut self, cx: &mut Context<Self>);
pub fn send_bytes(kind: AgentKind, text: &str) -> Vec<u8>;
```
- Consumes: `TranscriptTail`, `parse_transcript_line`, `ChatQueue`, `set_agent_session`.
- `TerminalCard` passes the session path and provides a `SharedPtyWriter` to the chat send callback.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn omp_multiline_send_uses_bracketed_paste_and_cr() {
    assert_eq!(
        send_bytes(AgentKind::Omp, "linha 1\nlinha 2"),
        b"\x1b[200~linha 1\nlinha 2\x1b[201~\r"
    );
    assert_eq!(send_bytes(AgentKind::Claude, "oi"), b"oi\r");
}

#[test]
fn chat_loads_omp_history_from_a_session_file() {
    // temp JSONL with one user + one final assistant; set_session; run_until_parked;
    // assert two bubbles and no status.
}

#[test]
fn queued_message_is_sent_only_after_the_turn_ends() {
    // enqueue two; assert only first write; call agent_finished; assert second write.
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p canvas-ui --lib chat_view::tests -v`
Run: `cargo test -p canvas-ui --lib regressions::chat_loads_omp_history_from_a_session_file -v`
Expected: FAIL: no session/queue wiring.

- [ ] **Step 3: Implement the wiring**

- `ChatView::set_session` starts a background poll (150 ms) reading `TranscriptTail`.
- Apply `TranscriptEvent::User`/`AgentFinal` as messages; `Status` updates the status line; `Prompt` sets “responda no terminal” and holds the queue.
- `enqueue` pushes to `ChatQueue`; when idle, take `front`, confirm immediately for `is_skill`, write via the card’s writer, and mark the bubble “na fila” until `confirm`.
- `agent_finished` clears status, advances the queue, and starts the next send.
- `send_bytes` is the per-agent adapter from the spec.
- `TerminalCard` calls `set_session` when created or when `track_omp_session` discovers a path; update both the store and the card.
- `track_omp_session` must call `card.set_session(...)` in addition to `set_agent_session`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p canvas-ui --lib chat_view::tests -v`
Run: `cargo test -p canvas-ui --lib regressions::chat_loads_omp_history_from_a_session_file -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add canvas/ui/src/chat_view.rs canvas/ui/src/card.rs canvas/ui/src/canvas.rs canvas/ui/src/regressions.rs
git commit -m "feat(chat): load OMP history and queue sends"
```

---

### Task 8: Non-OMP info state, snapshot response and liveness guard

**Files:**
- Modify: `canvas/ui/src/chat.rs` (pure snapshot helper)
- Modify: `canvas/ui/src/chat_view.rs`
- Modify: `canvas/ui/src/card.rs`
- Test: `canvas/ui/src/chat.rs`, `canvas/ui/src/chat_view.rs`

**Interfaces:**
- Produces:
```rust
pub fn snapshot_response(previous: &[String], current: &[String]) -> Option<String>;
pub enum AgentLiveness { Agent, Idle, Unknown }
pub fn foreground_liveness(pid: Option<u32>) -> AgentLiveness;
pub fn can_send(liveness: AgentLiveness, exited: bool, text: &str) -> bool;
```
- `snapshot_response` returns the text added since the previous snapshot, trimmed; `None` if nothing new.
- `foreground_liveness` is a best-effort check of the PTY foreground process; `Unknown` disables sending.
- `can_send` is the pure gate: non-empty text, not exited, and `AgentLiveness::Agent`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn snapshot_response_returns_only_new_terminal_lines() {
    let previous = vec!["$ omp".into(), "pronto".into()];
    let current = vec!["$ omp".into(), "pronto".into(), "resposta".into()];
    assert_eq!(snapshot_response(&previous, &current), Some("resposta".into()));
}

#[test]
fn snapshot_response_is_none_when_output_did_not_change() {
    let lines = vec!["a".into(), "b".into()];
    assert_eq!(snapshot_response(&lines, &lines), None);
}

#[test]
fn unknown_liveness_disables_chat_sending() {
    assert!(!can_send(AgentLiveness::Unknown, false, "oi"));
    assert!(!can_send(AgentLiveness::Agent, true, "oi"));
    assert!(!can_send(AgentLiveness::Agent, false, "   "));
    assert!(can_send(AgentLiveness::Agent, false, "oi"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p canvas-ui --lib chat::tests -v`
Run: `cargo test -p canvas-ui --lib chat_view::tests -v`
Expected: FAIL: helpers do not exist.

- [ ] **Step 3: Implement**

- Add the info bubble when `ChatView` has no OMP session and the card kind is not OMP.
- Poll `terminal.visible_lines(cx)` at ~150 ms; when output changes and then stays stable for ~900 ms, call `snapshot_response` and update one agent bubble.
- Add `ChatView::set_liveness`; render “use o terminal” and disable send when `Unknown`/`Idle`.
- `TerminalCard` computes `foreground_liveness(self._pty.pid())`; if it cannot verify, returns `Unknown`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p canvas-ui --lib chat::tests -v`
Run: `cargo test -p canvas-ui --lib chat_view::tests -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add canvas/ui/src/chat.rs canvas/ui/src/chat_view.rs canvas/ui/src/card.rs
git commit -m "feat(chat): support non-OMP fallback and liveness guard"
```

---

### Task 9: Slash skill picker

**Files:**
- Modify: `canvas/ui/src/chat.rs` (discovery/filter)
- Modify: `canvas/ui/src/chat_view.rs` (popover and insertion)
- Test: `canvas/ui/src/chat.rs`, `canvas/ui/src/chat_view.rs`

**Interfaces:**
- Produces:
```rust
pub struct SkillInfo { pub name: String, pub description: String, pub path: PathBuf }
pub fn discover_skills(roots: &[PathBuf]) -> Vec<SkillInfo>;
pub fn filter_skills<'a>(skills: &'a [SkillInfo], query: &str) -> Vec<&'a SkillInfo>;
pub fn skill_command(name: &str) -> String; // "/skill:{name} "
```
- Later `ChatView` inserts `skill_command` into the input.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn discovers_skill_name_and_description_from_frontmatter() {
    let dir = tempfile::tempdir().unwrap();
    let skill = dir.path().join("tdd");
    std::fs::create_dir(&skill).unwrap();
    std::fs::write(skill.join("SKILL.md"), "---\nname: tdd\ndescription: Test-first\n---\n").unwrap();
    let found = discover_skills(&[dir.path().to_path_buf()]);
    assert_eq!(found[0].name, "tdd");
    assert_eq!(found[0].description, "Test-first");
    assert_eq!(skill_command(&found[0].name), "/skill:tdd ");
}

#[test]
fn filter_matches_name_and_description() {
    // two skills; query "test" matches one by description; query "tdd" matches by name.
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p canvas-ui --lib chat::tests::discovers_skill_name_and_description_from_frontmatter -v`
Expected: FAIL: discovery helpers absent.

- [ ] **Step 3: Implement**

- Scan `~/.agents/skills/*/SKILL.md`, `<workspace>/.agents/skills/*/SKILL.md`, and `~/.omp/skills/*/SKILL.md`; skip missing dirs.
- Parse `name:` and `description:` from the YAML frontmatter without a YAML crate.
- In `ChatView`, show a popover above the input when the input starts with `/`; render names/descriptions; on click insert `skill_command(name)` into the input and keep focus.
- Empty state: “Nenhuma skill encontrada”.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p canvas-ui --lib chat::tests -v`
Run: `cargo test -p canvas-ui --lib chat_view::tests -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add canvas/ui/src/chat.rs canvas/ui/src/chat_view.rs
git commit -m "feat(chat): add slash skill picker"
```

---

### Task 10: Visual fixture, goldens and whole-suite verification

**Files:**
- Create: `canvas/tests/visual/fixture/agent-chat/workspaces.json`
- Create: `canvas/tests/visual/fixture/agent-chat/session.jsonl`
- Create: `canvas/scripts/visual-chat.sh`
- Modify: `canvas/ui/src/regressions.rs` (any remaining selectors)
- Test: whole workspace + visual script

**Interfaces:**
- Consumes all previous tasks.
- Produces a deterministic capture of the chat view with markdown, code block and status.

- [ ] **Step 1: Add a failing visual state**

Create a fixture workspace with one OMP agent whose `view` is `"chat"` and whose `session` path will be rewritten by the script to a temp copy of `session.jsonl`. Add `scripts/visual-chat.sh` modeled on `visual-regression.sh` with `STATES=(agent-chat agent-chat-info)`.

Run: `VISUAL_OUTPUT=HDMI-A-1 ./scripts/visual-chat.sh --check`
Expected: FAIL because the goldens do not exist.

- [ ] **Step 2: Record the goldens**

Run: `FORCE_GOLDENS=1 VISUAL_OUTPUT=HDMI-A-1 ./scripts/visual-chat.sh --record`
Expected: writes `agent-chat.png` and `agent-chat-info.png`.

- [ ] **Step 3: Inspect the captures**

Open both images and verify: user bubble right, agent bubble left, code block copy button visible, status line visible in `agent-chat`; info state shows the explanatory bubble and disabled/active input correctly.

- [ ] **Step 4: Run the full checks**

Run: `cargo test --workspace`
Expected: PASS, 0 failed.

Run: `VISUAL_OUTPUT=HDMI-A-1 ./scripts/visual-chat.sh --check`
Expected: both states within the 0.5% tolerance.

Run: `VISUAL_OUTPUT=HDMI-A-1 ./scripts/visual-regression.sh --check`
Expected: existing 8 states still within tolerance.

- [ ] **Step 5: Commit**

```bash
git add canvas/tests/visual/fixture/agent-chat canvas/tests/visual/golden canvas/scripts/visual-chat.sh canvas/ui/src/regressions.rs
git commit -m "test(chat): add visual chat goldens"
```

---

## Self-Review

- **Spec coverage:** persisted view (Task 1, 6), OMP parser/status (2), queue/dedupe (3), tail (4), chat UI/markdown/copy (5), toggle/overlay (6), OMP history/send (7), non-OMP/liveness (8), skills (9), visual/full verification (10).
- **Step scan:** each task starts with a failing test, has a target command, a minimal implementation instruction, a passing command, and a commit.
- **Type consistency:** `AgentView`, `ChatMessage`, `ChatStatus`, `TranscriptEvent`, `ChatQueue`, `TranscriptTail`, `ChatView`, `SkillInfo` are introduced before later tasks consume them.
- **Review focus:** session failure, shell liveness, interactive prompts, dedupe and long output each have a task line and a test.
- **Proportion:** code blocks are signatures and tests, not full implementations.
