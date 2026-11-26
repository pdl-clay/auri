//! GPUI presentation for the agent chat view.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use canvas_types::AgentKind;
use gpui_terminal::TerminalView;

use auro::gpui::{
    AnyElement, App, ClipboardItem, Context, Entity, IntoElement, ParentElement, Render,
    ScrollHandle, SharedString, Styled, WeakEntity, Window, div, px, relative, rgb,
};
use auro::prelude::*;
use gpui_component::{
    button::{Button, ButtonVariant, ButtonVariants as _},
    input::{Input, InputState},
    text::TextView,
};

use crate::card::{SharedPtyWriter, write_shared};
use crate::chat::{
    AgentLiveness, ChatFormat, ChatMessage, ChatQueue, ChatRole, ChatStatus, TranscriptEvent,
    can_send, normalize_message, snapshot_response,
};
use crate::chat_reader::TranscriptTail;
use crate::theme::*;

pub struct ChatView {
    pub input: Entity<InputState>,
    messages: Vec<ChatMessage>,
    status: Option<ChatStatus>,
    scroll: ScrollHandle,
    session: Option<PathBuf>,
    session_generation: u64,
    queue: ChatQueue,
    pending: Vec<String>,
    sender: Option<(AgentKind, SharedPtyWriter)>,
    liveness: AgentLiveness,
    exited: bool,
    terminal: Option<WeakEntity<TerminalView>>,
    last_snapshot: Vec<String>,
    snapshot_base: Vec<String>,
    last_change: Instant,
    awaiting_response: bool,
    snapshot_message: Option<usize>,
}

impl ChatView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Mensagem para o agente")
                .auto_grow(CHAT_INPUT_MIN_ROWS, CHAT_INPUT_MAX_ROWS)
        });
        Self {
            input,
            messages: Vec::new(),
            status: None,
            scroll: ScrollHandle::new(),
            session: None,
            session_generation: 0,
            queue: ChatQueue::default(),
            pending: Vec::new(),
            sender: None,
            liveness: AgentLiveness::Agent,
            exited: false,
            terminal: None,
            last_snapshot: Vec::new(),
            snapshot_base: Vec::new(),
            last_change: Instant::now(),
            awaiting_response: false,
            snapshot_message: None,
        }
    }

    pub fn set_messages(&mut self, messages: Vec<ChatMessage>, cx: &mut Context<Self>) {
        self.messages = messages;
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    pub fn push_message(&mut self, message: ChatMessage, cx: &mut Context<Self>) {
        self.messages.push(message);
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    pub fn set_info(&mut self, text: String, cx: &mut Context<Self>) {
        self.set_messages(
            vec![ChatMessage {
                role: ChatRole::Info,
                text,
                format: ChatFormat::Plain,
            }],
            cx,
        );
    }

    pub fn set_status(&mut self, status: Option<ChatStatus>, cx: &mut Context<Self>) {
        self.status = status;
        cx.notify();
    }

    pub fn set_sender(&mut self, kind: AgentKind, writer: SharedPtyWriter, cx: &mut Context<Self>) {
        self.sender = Some((kind, writer));
        self.try_send(cx);
        cx.notify();
    }

    pub fn set_terminal(&mut self, terminal: WeakEntity<TerminalView>, cx: &mut Context<Self>) {
        if self.terminal.is_some() {
            return;
        }
        self.terminal = Some(terminal.clone());
        let chat = cx.entity().downgrade();
        cx.spawn(async move |_, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let Some(entity) = chat.upgrade() else { break };
            let Some(terminal) = terminal.upgrade() else { break };
            let Ok(lines) = cx.update(|cx| terminal.read(cx).visible_lines()) else {
                continue;
            };
            let alive = entity
                .update(cx, |chat, cx| {
                    chat.observe_terminal(lines, cx);
                    true
                })
                .unwrap_or(false);
            if !alive {
                break;
            }
        })
        .detach();
    }

    pub fn set_liveness(&mut self, liveness: AgentLiveness, cx: &mut Context<Self>) {
        if self.liveness != liveness {
            self.liveness = liveness;
            cx.notify();
        }
    }

    pub fn set_exited(&mut self, exited: bool, cx: &mut Context<Self>) {
        if self.exited != exited {
            self.exited = exited;
            cx.notify();
        }
    }

    fn can_send_now(&self) -> bool {
        can_send(self.liveness, self.exited, "x")
    }

    fn observe_terminal(&mut self, lines: Vec<String>, cx: &mut Context<Self>) {
        let non_omp = self
            .sender
            .as_ref()
            .is_some_and(|(kind, _)| *kind != AgentKind::Omp);
        if !non_omp || self.session.is_some() {
            return;
        }
        if lines != self.last_snapshot {
            self.last_snapshot = lines;
            self.last_change = Instant::now();
            return;
        }
        if !self.awaiting_response
            || self.last_change.elapsed() < Duration::from_millis(900)
        {
            return;
        }
        let Some(text) = snapshot_response(&self.snapshot_base, &self.last_snapshot) else {
            self.awaiting_response = false;
            return;
        };
        if let Some(index) = self.snapshot_message {
            self.messages[index].text = text;
        } else {
            self.messages.push(ChatMessage {
                role: ChatRole::Agent,
                text,
                format: ChatFormat::Plain,
            });
            self.snapshot_message = Some(self.messages.len() - 1);
        }
        self.awaiting_response = false;
        self.status = None;
        self.try_send(cx);
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    /// Point the chat at an OMP session and poll new JSONL lines.
    pub fn set_session(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.session == path {
            return;
        }
        self.session = path.clone();
        self.session_generation += 1;
        let generation = self.session_generation;
        self.messages.clear();
        self.status = None;
        self.queue = ChatQueue::default();
        self.pending.clear();

        let Some(path) = path else {
            cx.notify();
            return;
        };
        let chat = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let (mut tail, initial) = cx
                .background_executor()
                .spawn(async move {
                    let mut tail = TranscriptTail::new(path, 200);
                    let events = tail.read_initial();
                    (tail, events)
                })
                .await;
            if let Some(entity) = chat.upgrade() {
                let _ = entity.update(cx, |chat, cx| {
                    if chat.session_generation == generation
                        && let Ok(events) = initial
                    {
                        chat.apply_events(events, cx);
                    }
                });
            }
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                let (next, result) = cx
                    .background_executor()
                    .spawn(async move {
                        let events = tail.read_appended();
                        (tail, events)
                    })
                    .await;
                tail = next;
                let Some(entity) = chat.upgrade() else { break };
                let alive = entity
                    .update(cx, |chat, cx| {
                        if chat.session_generation != generation {
                            return false;
                        }
                        if let Ok(events) = result {
                            chat.apply_events(events, cx);
                        }
                        true
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    pub fn enqueue(&mut self, text: String, is_skill: bool, cx: &mut Context<Self>) {
        let text = text.trim().to_owned();
        if !can_send(self.liveness, self.exited, &text) {
            return;
        }
        self.queue.enqueue(text.clone(), is_skill);
        self.pending.push(text);
        self.try_send(cx);
        cx.notify();
    }

    pub fn agent_finished(&mut self, cx: &mut Context<Self>) {
        self.status = None;
        self.try_send(cx);
        cx.notify();
    }

    fn try_send(&mut self, cx: &mut Context<Self>) {
        if self.status.is_some() {
            return;
        }
        let Some(pending) = self.queue.front().cloned() else {
            return;
        };
        let Some((kind, writer)) = self.sender.clone() else {
            return;
        };
        let bytes = send_bytes(kind, &pending.text);
        if write_shared(&writer, &bytes).is_err() {
            return;
        }
        self.queue.remove_front();
        if !self.pending.is_empty() {
            self.pending.remove(0);
        }
        if kind != AgentKind::Omp {
            self.snapshot_base = self.last_snapshot.clone();
            self.awaiting_response = true;
            self.snapshot_message = None;
            self.last_change = Instant::now();
        }
        self.messages.push(ChatMessage {
            role: ChatRole::User,
            text: pending.text,
            format: ChatFormat::Markdown,
        });
        self.status = Some(ChatStatus::Thinking);
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    fn apply_events(&mut self, events: Vec<TranscriptEvent>, cx: &mut Context<Self>) {
        if events.is_empty() {
            return;
        }
        for event in events {
            match event {
                TranscriptEvent::User(text) => {
                    let normalized = normalize_message(&text);
                    let duplicate = self
                        .messages
                        .iter()
                        .rev()
                        .take(12)
                        .any(|message| {
                            message.role == ChatRole::User
                                && normalize_message(&message.text) == normalized
                        });
                    if !duplicate {
                        self.messages.push(ChatMessage {
                            role: ChatRole::User,
                            text,
                            format: ChatFormat::Markdown,
                        });
                    }
                }
                TranscriptEvent::AgentFinal(text) => {
                    self.messages.push(ChatMessage {
                        role: ChatRole::Agent,
                        text,
                        format: ChatFormat::Markdown,
                    });
                    self.status = None;
                    self.try_send(cx);
                }
                TranscriptEvent::Status(status) => {
                    if matches!(status, ChatStatus::Exited) {
                        self.queue = ChatQueue::default();
                        self.pending.clear();
                    }
                    self.status = Some(status);
                }
            }
        }
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    pub fn focus_input(&self, window: &mut Window, cx: &mut App) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn value(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    fn render_message(
        &self,
        index: usize,
        message: &ChatMessage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selector = format!("chat-message-{index}");
        let content: AnyElement = match message.format {
            ChatFormat::Markdown => TextView::markdown(
                SharedString::from(format!("chat-md-{index}")),
                message.text.clone(),
                window,
                cx,
            )
            .selectable(true)
            .text_size(px(CHAT_TEXT_SIZE))
            .text_color(rgb(COLOR_FOREGROUND))
            .code_block_actions(|code, _, _cx| {
                let text = code.code();
                Button::new(code_button_id(&text))
                    .label("Copiar")
                    .with_variant(ButtonVariant::Ghost)
                    .h(px(CHAT_COPY_HEIGHT))
                    .rounded(px(RADIUS_CHIP))
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
                    })
            })
            .into_any_element(),
            ChatFormat::Plain => div()
                .text_size(px(CHAT_TEXT_SIZE))
                .text_color(rgb(COLOR_FOREGROUND))
                .child(message.text.clone())
                .into_any_element(),
        };

        let row = div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector)
            .flex()
            .w_full();

        match message.role {
            ChatRole::Info => row
                .child(
                    div()
                        .w_full()
                        .rounded(px(CHAT_BUBBLE_RADIUS))
                        .border_1()
                        .border_color(rgb(COLOR_BORDER))
                        .bg(rgb(COLOR_CHAT_INFO_BG))
                        .px(px(CHAT_BUBBLE_PADDING))
                        .py(px(SPACE_TIGHT))
                        .text_size(px(CHAT_STATUS_FONT))
                        .text_color(rgb(COLOR_MUTED))
                        .child(message.text.clone()),
                )
                .into_any_element(),
            ChatRole::User => row
                .justify_end()
                .child(
                    div()
                        .max_w(relative(CHAT_BUBBLE_WIDTH))
                        .rounded(px(CHAT_BUBBLE_RADIUS))
                        .border_1()
                        .border_color(rgb(COLOR_CHAT_USER_BORDER))
                        .bg(rgb(COLOR_CHAT_USER_BG))
                        .p(px(CHAT_BUBBLE_PADDING))
                        .child(content),
                )
                .into_any_element(),
            ChatRole::Agent => row
                .justify_start()
                .child(
                    div()
                        .max_w(relative(CHAT_BUBBLE_WIDTH))
                        .rounded(px(CHAT_BUBBLE_RADIUS))
                        .border_1()
                        .border_color(rgb(COLOR_BORDER))
                        .bg(rgb(COLOR_CHAT_AGENT_BG))
                        .p(px(CHAT_BUBBLE_PADDING))
                        .child(content),
                )
                .into_any_element(),
        }
    }
}

fn code_button_id(code: &str) -> SharedString {
    let mut hasher = DefaultHasher::new();
    code.hash(&mut hasher);
    SharedString::from(format!("chat-copy-{:x}", hasher.finish()))
}

pub fn send_bytes(kind: AgentKind, text: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    if kind == AgentKind::Omp && text.contains('\n') {
        bytes.extend_from_slice(b"\x1b[200~");
        bytes.extend_from_slice(text.as_bytes());
        bytes.extend_from_slice(b"\x1b[201~\r");
    } else {
        bytes.extend_from_slice(text.as_bytes());
        bytes.push(b'\r');
    }
    bytes
}

impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut all_messages = self.messages.clone();
        all_messages.extend(self.pending.iter().map(|text| ChatMessage {
            role: ChatRole::User,
            text: format!("{text}\n\n_na fila_"),
            format: ChatFormat::Markdown,
        }));
        let messages: Vec<AnyElement> = all_messages
            .iter()
            .enumerate()
            .map(|(index, message)| self.render_message(index, message, window, cx))
            .collect();
        let input_enabled = self.can_send_now();

        div()
            .id("chat-view")
            .debug_selector(|| "chat-view".to_string())
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(COLOR_BASE))
            .text_color(rgb(COLOR_FOREGROUND))
            .child(
                div()
                    .id("chat-log")
                    .debug_selector(|| "chat-log".to_string())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .gap(px(CHAT_BUBBLE_GAP))
                    .p(px(SPACE_WIDE))
                    .children(messages),
            )
            .when(!input_enabled, |this| {
                this.child(
                    div()
                        .id("chat-liveness")
                        .debug_selector(|| "chat-liveness".to_string())
                        .px(px(SPACE_WIDE))
                        .pb(px(SPACE_TIGHT))
                        .text_size(px(CHAT_STATUS_FONT))
                        .text_color(rgb(COLOR_DANGER))
                        .child("use o terminal"),
                )
            })
            .when_some(self.status.clone(), |this, status| {
                this.child(
                    div()
                        .id("chat-status")
                        .debug_selector(|| "chat-status".to_string())
                        .px(px(SPACE_WIDE))
                        .pb(px(SPACE_TIGHT))
                        .text_size(px(CHAT_STATUS_FONT))
                        .text_color(rgb(COLOR_MUTED))
                        .child(status.label()),
                )
            })
            .child(
                div()
                    .id("chat-input")
                    .debug_selector(|| "chat-input".to_string())
                    .px(px(SPACE_WIDE))
                    .pb(px(SPACE_WIDE))
                    .child(Input::new(&self.input).w_full().disabled(!input_enabled)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cx() -> auro::gpui::TestAppContext {
        let cx = auro::gpui::TestAppContext::single();
        cx.update(|cx| gpui_component::init(cx));
        cx
    }

    #[test]
    fn omp_multiline_send_uses_bracketed_paste_and_cr() {
        assert_eq!(
            send_bytes(AgentKind::Omp, "linha 1\nlinha 2"),
            b"\x1b[200~linha 1\nlinha 2\x1b[201~\r"
        );
        assert_eq!(send_bytes(AgentKind::Omp, "oi"), b"oi\r");
        assert_eq!(send_bytes(AgentKind::Claude, "oi"), b"oi\r");
    }

    #[test]
    fn chat_loads_omp_history_from_a_session_file() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(file, r#"{{"type":"message","message":{{"role":"user","content":[{{"type":"text","text":"oi"}}]}}}}"#).unwrap();
        writeln!(file, r#"{{"type":"message","message":{{"role":"assistant","content":[{{"type":"text","text":"olá"}}]}}}}"#).unwrap();
        drop(file);

        let mut cx = test_cx();
        let (view, vcx) = cx.add_window_view(|window, cx| ChatView::new(window, cx));
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.set_session(Some(path), cx);
            });
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("chat-message-0").is_some());
        assert!(vcx.debug_bounds("chat-message-1").is_some());
        assert!(vcx.debug_bounds("chat-status").is_none());
    }

    #[test]
    fn queued_message_is_sent_only_after_the_turn_ends() {
        use std::io::Write;
        use std::sync::{Arc, Mutex};

        #[derive(Clone)]
        struct Writer(Arc<Mutex<Vec<u8>>>);
        impl Write for Writer {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let written = Arc::new(Mutex::new(Vec::new()));
        let writer: crate::card::SharedPtyWriter =
            Arc::new(Mutex::new(Box::new(Writer(written.clone()))));

        let mut cx = test_cx();
        let (view, vcx) = cx.add_window_view(|window, cx| ChatView::new(window, cx));
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.set_sender(AgentKind::Omp, writer, cx);
                chat.enqueue("primeira".into(), false, cx);
                chat.enqueue("segunda".into(), false, cx);
            });
        });
        vcx.run_until_parked();
        let sent = String::from_utf8(written.lock().unwrap().clone()).unwrap();
        assert!(sent.contains("primeira"));
        assert!(!sent.contains("segunda"));

        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| chat.agent_finished(cx));
        });
        vcx.run_until_parked();
        let sent = String::from_utf8(written.lock().unwrap().clone()).unwrap();
        assert!(sent.contains("segunda"));
    }

    #[test]
    fn unknown_liveness_blocks_the_composer() {
        let mut cx = test_cx();
        let (view, vcx) = cx.add_window_view(|window, cx| ChatView::new(window, cx));
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.set_liveness(AgentLiveness::Unknown, cx);
            });
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("chat-liveness").is_some());
    }

    #[test]
    fn chat_view_renders_messages_and_composer() {
        let mut cx = test_cx();
        let (view, vcx) = cx.add_window_view(|window, cx| ChatView::new(window, cx));
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.set_info(
                    "Histórico não estruturado. Envie uma mensagem.".into(),
                    cx,
                );
                chat.push_message(
                    ChatMessage {
                        role: ChatRole::Agent,
                        text: "Olá".into(),
                        format: ChatFormat::Markdown,
                    },
                    cx,
                );
            });
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("chat-message-1").is_some());
        assert!(vcx.debug_bounds("chat-input").is_some());
    }
}
