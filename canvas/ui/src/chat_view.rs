//! GPUI presentation for the agent chat view.

use std::collections::VecDeque;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use canvas_types::AgentKind;
use gpui_terminal::TerminalView;

use auro::gpui::{
    Animation, AnimationExt, AnyElement, App, ClipboardEntry, ClipboardItem, Context, ElementId,
    Entity, Focusable, ImageFormat, ImageSource, IntoElement, KeystrokeEvent, MouseButton,
    ObjectFit, ParentElement, PathBuilder, PathPromptOptions, Pixels, Point, Render, ScrollHandle,
    SharedString, StyleRefinement, Styled, Subscription, WeakEntity, Window, canvas, div, img, px,
    relative, rgb, svg,
};
use auro::prelude::*;
use gpui_component::{
    button::{Button, ButtonVariant, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
    text::{TextView, TextViewStyle},
};

use crate::card::{SharedPtyWriter, write_shared};
use crate::chat::{
    AskKind, AskRequest, CanvasMention, ChatFormat, ChatMessage, ChatQueue, ChatRole, ChatStatus,
    ModelInfo, SkillInfo, TranscriptEvent, ask_sequences, attached_paths_text,
    catalog_with_new_models, chat_cache_dir, discover_skills, filter_models, filter_skills,
    final_answer_text, model_cache_path, normalize_message, omp_config_path, parse_models_json,
    provider_label, read_model_cache, read_omp_breadcrumb, read_omp_default_model,
    read_omp_default_model_from, screen_cleared, skill_command, snapshot_response,
    write_model_cache,
};
use crate::chat_reader::{TranscriptTail, current_session_model};
use crate::theme::*;

pub struct ActiveAsk {
    request: AskRequest,
    selections: Vec<Vec<usize>>,
    /// Free-text answer for the question's implicit "Other" row.
    other: Vec<String>,
    /// Answer for a question that is a real `<input type=text>` field.
    text: Vec<String>,
    /// Which free-text field is currently open in the inline editor.
    edit: Option<AskEdit>,
    sending: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AskEdit {
    Other(usize),
    Text(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Attachment {
    path: PathBuf,
    image: bool,
}

/// One row of the slash picker: a chat command or an agent skill.
#[derive(Clone)]
struct SlashEntry {
    name: String,
    description: String,
    command: String,
}

/// Commands owned by the chat itself, listed before skills. They are written
/// to the agent PTY verbatim, so OMP's own slash command handles them.
const CHAT_COMMANDS: [(&str, &str); 1] =
    [("new", "Nova sessão para este cartão")];
const CHAT_HISTORY_LIMIT: usize = 200;

/// Commands are handled by the CLI and never appear as transcript messages,
/// so they must not enter the outbound match queue.
fn is_command_text(text: &str) -> bool {
    let text = text.trim();
    text.starts_with("/skill:") || text == "/new"
}

pub struct ChatView {
    pub input: Entity<InputState>,
    messages: Vec<ChatMessage>,
    status: Option<ChatStatus>,
    ask: Option<ActiveAsk>,
    scroll: ScrollHandle,
    queue: ChatQueue,
    pending: Vec<String>,
    outbound: VecDeque<String>,
    kind: Option<AgentKind>,
    session: Option<PathBuf>,
    session_generation: u64,
    breadcrumb: Option<PathBuf>,
    watching_breadcrumb: bool,
    session_changed: Option<Rc<dyn Fn(PathBuf, &mut App)>>,
    sender: Option<(AgentKind, SharedPtyWriter)>,
    default_model: Option<String>,
    current_model: Option<String>,
    /// True once the current model came from the session or the user's picker;
    /// reopening the chat must not replace it with the CLI default.
    model_from_session: bool,
    models: Vec<ModelInfo>,
    models_loading: bool,
    models_loaded: bool,
    models_error: Option<String>,
    model_menu_open: bool,
    model_switching: bool,
    selected_provider: Option<String>,
    model_query: Entity<InputState>,
    _model_query_subscription: Subscription,
    context_tokens: Option<u64>,
    context_flash: Option<Instant>,
    zoom: f32,
    exited: bool,
    terminal: Option<WeakEntity<TerminalView>>,
    last_snapshot: Vec<String>,
    snapshot_base: Vec<String>,
    last_change: Instant,
    quiet_ms: u64,
    awaiting_response: bool,
    observed_output: bool,
    last_sent: Option<String>,
    snapshot_message: Option<usize>,
    skills: Vec<SkillInfo>,
    skill_selection: usize,
    skill_scroll: ScrollHandle,
    /// Live connected endpoints supplied by the owning canvas card.
    mention_provider: Option<Rc<dyn Fn(&App) -> Vec<CanvasMention>>>,
    mention_selection: usize,
    /// Sent prompts, oldest first. `history_cursor == None` means the live
    /// draft; the extra slot at `history.len()` is the empty input.
    history: Vec<String>,
    history_cursor: Option<usize>,
    history_restore: Option<String>,
    ask_input: Entity<InputState>,
    attachments: Vec<Attachment>,
    _input_subscription: Subscription,
    _ask_input_subscription: Subscription,
    _key_interceptor: Subscription,
    /// Test-only override so reopening can be exercised with a temp config.
    #[cfg(test)]
    config_path: Option<PathBuf>,
}

impl ChatView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Mensagem para o agente")
                .multi_line(true)
        });
        let input_subscription = cx.subscribe_in(
            &input,
            window,
            |this, state, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let value = state.read(cx).value().to_string();
                    if this.history_restore.as_deref() == Some(value.as_str()) {
                        this.history_restore = None;
                    } else {
                        this.history_cursor = None;
                        this.history_restore = None;
                    }
                    this.skill_selection = 0;
                    this.mention_selection = 0;
                    cx.notify()
                }
                InputEvent::PressEnter { secondary: false } => this.send_composed(window, cx),
                _ => {}
            },
        );
        let ask_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Digite sua resposta")
        });
        let ask_input_subscription = cx.subscribe_in(
            &ask_input,
            window,
            |this, state, event: &InputEvent, _, cx| match event {
                InputEvent::Change => {
                    this.sync_ask_edit(state.read(cx).value().to_string(), cx)
                }
                InputEvent::PressEnter { secondary: false } => this.send_ask(cx),
                _ => {}
            },
        );
        let model_query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Buscar modelo")
                .multi_line(true)
        });
        let model_query_subscription = cx.subscribe_in(
            &model_query,
            window,
            |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );
        // Key bindings run before `on_key_down`, so the picker must see the
        // keystroke at the interceptor stage; otherwise the Input's Tab and
        // arrow bindings consume it first.
        let chat = cx.entity().downgrade();
        let key_interceptor = cx.intercept_keystrokes(move |event, window, cx| {
            let Some(chat) = chat.upgrade() else {
                return;
            };
            let handled = chat
                .update(cx, |chat, cx| chat.on_chat_keystroke(event, window, cx));
            if handled {
                cx.stop_propagation();
            }
        });
        let cached_models = read_model_cache(&model_cache_path());
        let models_loaded = !cached_models.is_empty();
        let default_model = read_omp_default_model();
        Self {
            input,
            _input_subscription: input_subscription,
            _ask_input_subscription: ask_input_subscription,
            ask_input,
            attachments: Vec::new(),
            _key_interceptor: key_interceptor,
            #[cfg(test)]
            config_path: None,
            messages: Vec::new(),
            status: None,
            ask: None,
            scroll: ScrollHandle::new(),
            queue: ChatQueue::default(),
            pending: Vec::new(),
            outbound: VecDeque::new(),
            kind: None,
            session: None,
            session_generation: 0,
            breadcrumb: None,
            watching_breadcrumb: false,
            session_changed: None,
            sender: None,
            default_model: default_model.clone(),
            current_model: default_model.clone(),
            model_from_session: false,
            models: cached_models,
            models_loading: false,
            models_loaded,
            models_error: None,
            model_menu_open: false,
            model_switching: false,
            selected_provider: default_model
                .as_ref()
                .and_then(|model| model.split('/').next())
                .map(str::to_owned),
            model_query,
            _model_query_subscription: model_query_subscription,
            // Known window + zero usage still draws an empty gauge; hiding the
            // whole component made a fresh session look like a missing feature.
            context_tokens: Some(0),
            context_flash: None,
            zoom: 1.0,
            exited: false,
            terminal: None,
            last_snapshot: Vec::new(),
            snapshot_base: Vec::new(),
            last_change: Instant::now(),
            quiet_ms: 900,
            awaiting_response: false,
            observed_output: false,
            last_sent: None,
            snapshot_message: None,
            skills: Vec::new(),
            skill_selection: 0,
            skill_scroll: ScrollHandle::new(),
            mention_provider: None,
            mention_selection: 0,
            history: Vec::new(),
            history_cursor: None,
            history_restore: None,
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
                images: Vec::new(),
            }],
            cx,
        );
    }

    pub fn set_status(&mut self, status: Option<ChatStatus>, cx: &mut Context<Self>) {
        self.status = status;
        cx.notify();
    }

    pub fn set_sender(&mut self, kind: AgentKind, writer: SharedPtyWriter, cx: &mut Context<Self>) {
        self.kind = Some(kind);
        self.sender = Some((kind, writer));
        self.try_send(cx);
        cx.notify();
    }

    fn z(&self, value: f32) -> Pixels {
        px(value * self.zoom)
    }

    pub fn set_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        if (self.zoom - zoom).abs() > 0.005 {
            let previous = self.zoom.max(0.01);
            let offset = self.scroll.offset();
            let max = self.scroll.max_offset();
            let at_bottom = f32::from(offset.y) + f32::from(max.height) <= 1.0;
            self.zoom = zoom;
            if at_bottom {
                self.scroll.scroll_to_bottom();
            } else {
                let factor = zoom / previous;
                self.scroll.set_offset(Point::new(
                    px(f32::from(offset.x) * factor),
                    px(f32::from(offset.y) * factor),
                ));
            }
            cx.notify();
        }
    }

    fn model_display_name(&self) -> String {
        let Some(current) = &self.current_model else {
            return "Modelo".into();
        };
        let base = current.split(':').next().unwrap_or(current);
        self.models
            .iter()
            .find(|model| model.selector == base)
            .map(|model| model.name.clone())
            .unwrap_or_else(|| base.rsplit('/').next().unwrap_or(base).to_owned())
    }

    fn note_context_usage(&mut self, tokens: u64, cx: &mut Context<Self>) {
        let previous = self.context_tokens.replace(tokens);
        if let Some(previous) = previous {
            if previous > 0 && tokens.saturating_mul(10) < previous.saturating_mul(6) {
                self.context_flash = Some(Instant::now());
                let chat = cx.entity().downgrade();
                cx.spawn(async move |_, cx| {
                    cx.background_executor()
                        .timer(Duration::from_secs(2))
                        .await;
                    let Some(chat) = chat.upgrade() else { return };
                    let _ = chat.update(cx, |chat, cx| {
                        chat.context_flash = None;
                        cx.notify();
                    });
                })
                .detach();
            }
        }
    }

    fn shows_context_meter(&self) -> bool {
        self.kind == Some(AgentKind::Omp) || self.session.is_some()
    }

    fn context_window(&self) -> Option<u64> {
        let current = self.current_model.as_deref()?;
        self.models
            .iter()
            .find(|model| model_is_current(model, Some(current)))
            .and_then(|model| model.context_window)
            .filter(|window| *window > 0)
    }

    fn context_ratio(&self) -> Option<f32> {
        if !self.shows_context_meter() {
            return None;
        }
        let tokens = self.context_tokens? as f32;
        let window = self.context_window()? as f32;
        Some((tokens / window).clamp(0.0, 1.0))
    }

    /// Ring gauge with the percentage inside it, right of the composer input.
    /// Twenty-eight radial bars: filled up to the current usage, colored along
    /// a green → gold → red gradient.
    fn context_meter(&self) -> Option<AnyElement> {
        let ratio = self.context_ratio()?;
        let percent = (ratio * 100.0).round() as u32;
        let zoom = self.zoom;
        let compacting = self.context_flash.is_some();
        let bars = CHAT_CONTEXT_BARS;
        let filled = if ratio > 0.0 {
            ((ratio * bars as f32).round() as usize).max(1).min(bars)
        } else {
            0
        };
        Some(
            div()
                .id("chat-context-meter")
                .debug_selector(|| "chat-context-meter".to_string())
                .relative()
                .w(self.z(CHAT_CONTEXT_SIZE))
                .h(self.z(CHAT_CONTEXT_SIZE))
                .when(compacting, |this| this.opacity(0.35))
                .child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, window, _| {
                            let origin = bounds.origin;
                            let size = f32::from(bounds.size.width);
                            let center = Point::new(
                                px(f32::from(origin.x) + size / 2.0),
                                px(f32::from(origin.y) + size / 2.0),
                            );
                            let bar_length = CHAT_CONTEXT_BAR_LENGTH * zoom;
                            let radius = size / 2.0 - bar_length / 2.0 - 1.0;
                            let angle_step = std::f32::consts::TAU / bars as f32;
                            for index in 0..bars {
                                let angle =
                                    -std::f32::consts::FRAC_PI_2 + angle_step * index as f32;
                                let inner = radius - bar_length / 2.0;
                                let outer = radius + bar_length / 2.0;
                                let mut bar =
                                    PathBuilder::stroke(px(CHAT_CONTEXT_BAR_WIDTH * zoom));
                                bar.move_to(Point::new(
                                    px(f32::from(center.x) + inner * angle.cos()),
                                    px(f32::from(center.y) + inner * angle.sin()),
                                ));
                                bar.line_to(Point::new(
                                    px(f32::from(center.x) + outer * angle.cos()),
                                    px(f32::from(center.y) + outer * angle.sin()),
                                ));
                                let color = if index < filled {
                                    context_color((index as f32 + 0.5) / bars as f32)
                                } else {
                                    COLOR_BORDER
                                };
                                if let Ok(path) = bar.build() {
                                    window.paint_path(path, rgb(color));
                                }
                            }
                        },
                    )
                    .absolute()
                    .left(px(0.))
                    .top(px(0.))
                    .size_full(),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(0.))
                        .top(px(0.))
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(self.z(CHAT_CONTEXT_FONT))
                        .text_color(rgb(COLOR_FOREGROUND))
                        .child(format!("{}%", percent)),
                )
                .into_any_element(),
        )
    }

    fn load_models(&mut self, force: bool, cx: &mut Context<Self>) {
        if self.models_loading || (self.models_loaded && !force) {
            return;
        }
        self.models_loading = true;
        if self.models.is_empty() {
            self.models_error = None;
        }
        let chat = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let output = cx
                .background_executor()
                .spawn(async move { omp_models_output() })
                .await;
            let Some(entity) = chat.upgrade() else { return };
            let _ = entity.update(cx, |chat, cx| {
                chat.models_loading = false;
                match output {
                    Ok(output) if output.status.success() => {
                        let body = String::from_utf8_lossy(&output.stdout).into_owned();
                        let parsed = parse_models_json(&body);
                        if !parsed.is_empty() {
                            // Draw the cached rows immediately; only a catalog
                            // that brought a new model replaces them.
                            if catalog_with_new_models(&chat.models, parsed.clone()).is_some() {
                                chat.models = parsed;
                                let _ = write_model_cache(&model_cache_path(), &body);
                            }
                            chat.models_loaded = true;
                            chat.models_error = None;
                            let current_provider = chat
                                .current_model
                                .as_ref()
                                .and_then(|model| model.split('/').next())
                                .map(str::to_owned);
                            let provider = current_provider
                                .filter(|provider| {
                                    chat.models
                                        .iter()
                                        .any(|model| &model.provider == provider)
                                })
                                .or_else(|| chat.models.first().map(|model| model.provider.clone()));
                            if chat.selected_provider.as_ref().is_none_or(|selected| {
                                !chat.models.iter().any(|model| &model.provider == selected)
                            }) {
                                chat.selected_provider = provider;
                            }
                        } else if chat.models.is_empty() {
                            chat.models_error = Some("Nenhum modelo disponível.".into());
                        }
                    }
                    Ok(output) => {
                        if chat.models.is_empty() {
                            let error = String::from_utf8_lossy(&output.stderr);
                            chat.models_error =
                                Some(format!("omp models falhou: {}", error.trim()));
                        }
                    }
                    Err(error) => {
                        if chat.models.is_empty() {
                            chat.models_error =
                                Some(format!("Não consegui listar modelos: {error}"));
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Adopt the model OMP would use for a fresh session. Called again every
    /// time the chat is opened: a long-lived ChatView must not keep the value
    /// read when its entity was constructed, because the CLI can change its
    /// default while the app is running.
    fn adopt_default_model(&mut self, model: Option<String>, cx: &mut Context<Self>) {
        let Some(model) = model else { return };
        if self.default_model.as_deref() == Some(model.as_str()) {
            return;
        }
        let previous = self.default_model.replace(model.clone());
        let replaceable = self.current_model.is_none()
            || (!self.model_from_session
                && self.current_model.as_deref() == previous.as_deref());
        if replaceable {
            self.selected_provider = model.split('/').next().map(str::to_owned);
            self.current_model = Some(model);
            self.model_from_session = false;
            cx.notify();
        }
    }

    fn refresh_default_model_from(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.adopt_default_model(read_omp_default_model_from(path), cx);
    }

    fn needs_model_catalog(&self) -> bool {
        self.models.is_empty()
            || self.current_model.as_deref().is_some_and(|model| {
                !self
                    .models
                    .iter()
                    .any(|entry| model_is_current(entry, Some(model)))
            })
            || (self.current_model.is_some() && self.context_window().is_none())
    }

    /// Re-read the CLI's default and refresh a catalog that cannot size the
    /// current model. Runs on every transition to the chat presentation.
    pub fn refresh_for_open(&mut self, cx: &mut Context<Self>) {
        #[cfg(test)]
        let path = self.config_path.clone().unwrap_or_else(omp_config_path);
        #[cfg(not(test))]
        let path = omp_config_path();
        self.refresh_default_model_from(&path, cx);
        if self.needs_model_catalog() {
            self.load_models(true, cx);
        }
    }

    /// Test hook: point the open-refresh at a temp OMP config.
    #[cfg(test)]
    pub(crate) fn use_config_path(&mut self, path: PathBuf) {
        self.config_path = Some(path);
    }

    #[cfg(test)]
    pub(crate) fn model_for_test(&self) -> Option<String> {
        self.current_model.clone()
    }

    /// First open of an OMP card without a cached catalog. With a cache, the
    /// chip already has names and only a menu click refreshes it.
    pub fn prefetch_models(&mut self, cx: &mut Context<Self>) {
        self.refresh_for_open(cx);
    }

    fn open_model_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ask.is_some() || self.model_switching {
            return;
        }
        self.model_menu_open = true;
        if self.selected_provider.is_none() {
            self.selected_provider = self
                .current_model
                .as_ref()
                .and_then(|model| model.split('/').next())
                .map(str::to_owned)
                .filter(|provider| {
                    self.models
                        .iter()
                        .any(|model| &model.provider == provider)
                })
                .or_else(|| self.models.first().map(|model| model.provider.clone()));
        }
        self.load_models(true, cx);
        self.model_query.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn close_model_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.model_menu_open = false;
        self.model_query.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.focus_input(window, cx);
        cx.notify();
    }

    fn select_model(
        &mut self,
        selector: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ask.is_some() || self.model_switching {
            return;
        }
        let Some((_, writer)) = self.sender.clone() else {
            return;
        };
        let sequences = model_switch_sequence(&selector);
        self.current_model = Some(selector.clone());
        self.model_from_session = true;
        self.selected_provider = selector.split('/').next().map(str::to_owned);
        self.model_menu_open = false;
        self.model_switching = true;
        self.model_query.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        let chat = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            for sequence in sequences {
                let _ = write_shared(&writer, &sequence);
                cx.background_executor()
                    .timer(Duration::from_millis(180))
                    .await;
            }
            cx.background_executor()
                .timer(Duration::from_millis(2500))
                .await;
            if let Some(entity) = chat.upgrade() {
                let _ = entity.update(cx, |chat, cx| {
                    if chat.model_switching {
                        chat.model_switching = false;
                        cx.notify();
                    }
                });
            }
        })
        .detach();
        cx.notify();
    }

    /// Point the chat at the OMP session JSONL. `breadcrumb` is the terminal
    /// breadcrumb omp rewrites on `/clear` or a new session; it is what lets
    /// the chat follow the process to the new file.
    pub fn set_session(
        &mut self,
        session: Option<PathBuf>,
        breadcrumb: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        self.breadcrumb = breadcrumb.clone();
        self.watch_breadcrumb(breadcrumb, cx);
        if self.session == session {
            cx.notify();
            return;
        }
        let clear_queue = self.session.is_some();
        self.restart_session(session, clear_queue, cx);
    }

    /// The card uses this to persist a session discovered after `/clear` or a
    /// new start, so reopening the card resumes the same transcript.
    pub fn on_session_change(
        &mut self,
        handler: Rc<dyn Fn(PathBuf, &mut App)>,
        _cx: &mut Context<Self>,
    ) {
        self.session_changed = Some(handler);
    }

    fn watch_breadcrumb(&mut self, breadcrumb: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.watching_breadcrumb || breadcrumb.is_none() {
            return;
        }
        self.watching_breadcrumb = true;
        let chat = cx.entity().downgrade();
        cx.spawn(async move |_, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(400))
                .await;
            let Some(entity) = chat.upgrade() else { break };
            let Some(breadcrumb) = entity
                .update(cx, |chat, _| chat.breadcrumb.clone())
                .ok()
                .flatten()
            else {
                continue;
            };
            let found = cx
                .background_executor()
                .spawn(async move {
                    read_omp_breadcrumb(&breadcrumb).map(|breadcrumb| breadcrumb.session)
                })
                .await;
            let Some(found) = found else { continue };
            let _ = entity.update(cx, |chat, cx| {
                if chat.session.as_ref() == Some(&found) {
                    return;
                }
                chat.restart_session(Some(found.clone()), chat.session.is_some(), cx);
                if let Some(handler) = chat.session_changed.clone() {
                    handler(found, cx);
                }
            });
        })
        .detach();
    }

    fn restart_session(
        &mut self,
        path: Option<PathBuf>,
        clear_queue: bool,
        cx: &mut Context<Self>,
    ) {
        self.session = path.clone();
        self.session_generation += 1;
        let generation = self.session_generation;
        self.current_model = self.default_model.clone();
        self.model_from_session = false;
        self.context_tokens = Some(0);
        self.context_flash = None;
        self.model_switching = false;
        self.selected_provider = self
            .default_model
            .as_ref()
            .and_then(|model| model.split('/').next())
            .map(str::to_owned);
        self.ask = None;
        self.messages.clear();
        self.status = None;
        self.awaiting_response = false;
        self.observed_output = false;
        self.snapshot_message = None;
        if clear_queue {
            self.queue = ChatQueue::default();
            self.pending.clear();
            self.outbound.clear();
        }
        if let Some(path) = path.as_ref() {
            // The history window keeps only the newest events, so the last
            // model_change may be outside it. The scan runs in the background
            // and never overrides a model that the live tail already applied.
            let model_chat = cx.entity().downgrade();
            let model_path = path.clone();
            cx.spawn(async move |_, cx| {
                let model = cx
                    .background_executor()
                    .spawn(async move { current_session_model(&model_path) })
                    .await;
                let Some(model) = model else { return };
                let Some(entity) = model_chat.upgrade() else { return };
                let _ = entity.update(cx, |chat, cx| {
                    let default = chat.default_model.clone();
                    let replaceable = chat.current_model.as_deref().is_none()
                        || chat.current_model.as_deref() == default.as_deref();
                    if chat.session_generation == generation && replaceable {
                        chat.selected_provider = model.split('/').next().map(str::to_owned);
                        chat.current_model = Some(model);
                        chat.model_from_session = true;
                        cx.notify();
                    }
                });
            })
            .detach();
        }
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
            let mut priming = initial.is_err();
            let tokens = tail.context_tokens;
            if let Some(entity) = chat.upgrade() {
                let _ = entity.update(cx, |chat, cx| {
                    if chat.session_generation != generation {
                        return;
                    }
                    match initial {
                        Ok(events) if events.is_empty() => chat.set_info(
                            "A sessão deste cartão está vazia. Envie uma mensagem para começar.".into(),
                            cx,
                        ),
                        Ok(events) => chat.apply_history(events, cx),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            // omp may announce the session before creating the
                            // file; the first append will load the history.
                        }
                        Err(error) => chat.set_info(
                            format!("Não consegui ler a sessão deste cartão: {error}"),
                            cx,
                        ),
                    }
                    if let Some(tokens) = tokens {
                        chat.note_context_usage(tokens, cx);
                    }
                });
            }
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                let (next, result, reset) = cx
                    .background_executor()
                    .spawn(async move {
                        let events = tail.read_appended();
                        let reset = tail.reset;
                        tail.reset = false;
                        (tail, events, reset)
                    })
                    .await;
                tail = next;
                let tokens = tail.context_tokens;
                let Some(entity) = chat.upgrade() else { break };
                let alive = entity
                    .update(cx, |chat, cx| {
                        if chat.session_generation != generation {
                            return false;
                        }
                        if reset {
                            chat.messages.clear();
                            chat.status = None;
                        }
                        if let Ok(events) = result {
                            if priming {
                                priming = false;
                                chat.messages.clear();
                                chat.status = None;
                                chat.apply_history(events, cx);
                            } else {
                                chat.apply_events(events, cx);
                            }
                        }
                        if let Some(tokens) = tokens {
                            chat.note_context_usage(tokens, cx);
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

    fn apply_history(&mut self, events: Vec<TranscriptEvent>, cx: &mut Context<Self>) {
        for event in events {
            match event {
                TranscriptEvent::User(text) => {
                    self.outbound_matches(&text);
                    self.messages.push(ChatMessage {
                        role: ChatRole::User,
                        text,
                        format: ChatFormat::Markdown,
                        images: Vec::new(),
                    });
                }
                TranscriptEvent::AgentFinal { text, context_tokens } => {
                    self.messages.push(ChatMessage {
                        role: ChatRole::Agent,
                        text,
                        format: ChatFormat::Markdown,
                        images: Vec::new(),
                    });
                    if let Some(tokens) = context_tokens {
                        self.note_context_usage(tokens, cx);
                    }
                }
                TranscriptEvent::Model(model) => {
                    self.model_switching = false;
                    self.selected_provider = model
                        .split('/')
                        .next()
                        .map(str::to_owned)
                        .or(self.selected_provider.take());
                    self.current_model = Some(model);
                    self.model_from_session = true;
                }
                TranscriptEvent::Ask(request) => self.start_ask(request, cx),
                TranscriptEvent::AskAnswered { text, .. } => {
                    self.ask = None;
                    if !text.is_empty() {
                        self.messages.push(ChatMessage {
                            role: ChatRole::User,
                            text,
                            format: ChatFormat::Markdown,
                            images: Vec::new(),
                        });
                    }
                }
                TranscriptEvent::Status(_) | TranscriptEvent::TurnEnd => {}
            }
        }
        self.status = None;
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
                    if !self.outbound_matches(&text) {
                        self.messages.push(ChatMessage {
                            role: ChatRole::User,
                            text,
                            format: ChatFormat::Markdown,
                            images: Vec::new(),
                        });
                    }
                }
                TranscriptEvent::AgentFinal { text, context_tokens } => {
                    self.messages.push(ChatMessage {
                        role: ChatRole::Agent,
                        text,
                        format: ChatFormat::Markdown,
                        images: Vec::new(),
                    });
                    if let Some(tokens) = context_tokens {
                        self.note_context_usage(tokens, cx);
                    }
                    self.status = None;
                    self.try_send(cx);
                }
                TranscriptEvent::Status(status) => self.status = Some(status),
                TranscriptEvent::Model(model) => {
                    self.model_switching = false;
                    self.selected_provider = model
                        .split('/')
                        .next()
                        .map(str::to_owned)
                        .or(self.selected_provider.take());
                    self.current_model = Some(model);
                    self.model_from_session = true;
                }
                TranscriptEvent::Ask(request) => self.start_ask(request, cx),
                TranscriptEvent::AskAnswered { text, .. } => {
                    self.ask = None;
                    if !text.is_empty() {
                        self.messages.push(ChatMessage {
                            role: ChatRole::User,
                            text,
                            format: ChatFormat::Markdown,
                            images: Vec::new(),
                        });
                    }
                    self.status = Some(ChatStatus::Writing);
                }
                TranscriptEvent::TurnEnd => {
                    self.ask = None;
                    self.status = None;
                    self.try_send(cx);
                }
            }
        }
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    fn outbound_matches(&mut self, text: &str) -> bool {
        let normalized = normalize_message(text);
        if self.outbound.front() == Some(&normalized) {
            self.outbound.pop_front();
            true
        } else {
            false
        }
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


    pub fn set_skill_roots(&mut self, roots: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.skills = discover_skills(&roots);
        cx.notify();
    }

    /// Connected canvas endpoints for `@`; the card supplies a closure that
    /// reads the live store, so connecting/disconnecting updates the picker.
    pub fn set_mention_provider(
        &mut self,
        provider: Rc<dyn Fn(&App) -> Vec<CanvasMention>>,
        cx: &mut Context<Self>,
    ) {
        self.mention_provider = Some(provider);
        cx.notify();
    }

    /// The text after the last `@`, while it is still a single token.
    fn mention_query(&self, cx: &App) -> Option<String> {
        if self.model_menu_open || self.ask.is_some() {
            return None;
        }
        let value = self.input.read(cx).value().to_string();
        let at = value.rfind('@')?;
        let query = &value[at + 1..];
        if query.chars().any(char::is_whitespace) {
            return None;
        }
        Some(query.to_owned())
    }

    fn mention_picker_open(&self, cx: &App) -> bool {
        self.mention_provider.is_some() && self.mention_query(cx).is_some()
    }

    fn visible_mentions(&self, cx: &App) -> Vec<CanvasMention> {
        let Some(query) = self.mention_query(cx) else {
            return Vec::new();
        };
        let Some(provider) = self.mention_provider.as_ref() else {
            return Vec::new();
        };
        let lowered = query.to_lowercase();
        let mut mentions: Vec<CanvasMention> = provider(cx)
            .into_iter()
            .filter(|mention| {
                lowered.is_empty()
                    || mention.name.to_lowercase().contains(&lowered)
                    || mention.detail.to_lowercase().contains(&lowered)
            })
            .collect();
        mentions.truncate(CHAT_SKILLS_VISIBLE);
        mentions
    }

    /// Replace the `@query` tail with the chosen mention and keep composing.
    fn choose_mention(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(mention) = self.visible_mentions(cx).get(index).cloned() else {
            return false;
        };
        let value = self.input.read(cx).value().to_string();
        let Some(at) = value.rfind('@') else {
            return false;
        };
        let next = format!("{}{} ", &value[..at], mention.insert);
        let next = if next.trim_start().starts_with("/skill:") {
            next
        } else {
            format!("/skill:auri {next}")
        };
        self.input.update(cx, |input, cx| {
            input.set_value(next, window, cx);
            input.focus(window, cx);
        });
        self.mention_selection = 0;
        cx.notify();
        true
    }

    fn move_mention_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.visible_mentions(cx).len();
        if count == 0 {
            return;
        }
        let current = self.mention_selection.min(count - 1) as isize;
        self.mention_selection = (current + delta).clamp(0, count as isize - 1) as usize;
        self.skill_scroll.scroll_to_item(self.mention_selection);
        cx.notify();
    }

    fn mention_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.mention_query(cx)?;
        self.mention_provider.as_ref()?;
        let mentions = self.visible_mentions(cx);
        let selected = self
            .mention_selection
            .min(mentions.len().saturating_sub(1));
        let mut items: Vec<AnyElement> = mentions
            .iter()
            .enumerate()
            .map(|(index, mention)| {
                let name = mention.name.clone();
                let detail = mention.detail.clone();
                let highlighted = index == selected;
                div()
                    .id(("chat-mention", index))
                    .debug_selector(move || format!("chat-mention-{index}"))
                    .flex()
                    .flex_col()
                    .gap(self.z(CHAT_SKILL_GAP))
                    .px(self.z(SPACE_NORMAL))
                    .py(self.z(SPACE_TIGHT))
                    .rounded(self.z(RADIUS_CHIP))
                    .when(highlighted, |row| row.bg(rgb(COLOR_ELEVATED)))
                    .hover(|style| style.bg(rgb(COLOR_ELEVATED)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.choose_mention(index, window, cx);
                    }))
                    .child(
                        div()
                            .text_size(self.z(FONT_CARD_TITLE))
                            .text_color(rgb(if highlighted {
                                COLOR_ACCENT
                            } else {
                                COLOR_FOREGROUND
                            }))
                            .child(name),
                    )
                    .child(
                        div()
                            .text_size(self.z(CHAT_STATUS_FONT))
                            .text_color(rgb(COLOR_MUTED))
                            .child(detail),
                    )
                    .into_any_element()
            })
            .collect();
        if items.is_empty() {
            items.push(
                div()
                    .px(self.z(SPACE_NORMAL))
                    .py(self.z(SPACE_TIGHT))
                    .text_size(self.z(CHAT_STATUS_FONT))
                    .text_color(rgb(COLOR_MUTED))
                    .child("Nenhum item conectado")
                    .into_any_element(),
            );
        }
        Some(
            div()
                .id("chat-mentions")
                .debug_selector(|| "chat-mentions".to_string())
                .mx(self.z(SPACE_WIDE))
                .mb(self.z(SPACE_TIGHT))
                .max_h(self.z(CHAT_SKILLS_MAX_HEIGHT))
                .overflow_y_scroll()
                .track_scroll(&self.skill_scroll)
                .flex()
                .flex_col()
                .p(self.z(SPACE_TIGHT))
                .rounded(self.z(RADIUS_CONTROL))
                .border_1()
                .border_color(rgb(COLOR_BORDER))
                .bg(rgb(COLOR_PANEL))
                .children(items)
                .into_any_element(),
        )
    }

    /// The composer text is a skill query when it starts with `/` and holds no
    /// space or newline: that is exactly when the picker is on screen.
    fn skill_query(&self, cx: &App) -> Option<String> {
        let value = self.input.read(cx).value().to_string();
        let query = value.strip_prefix('/')?;
        if query.contains(' ') || query.contains('\n') {
            return None;
        }
        Some(query.to_owned())
    }

    /// The model search field is also a child of this view; its arrow keys
    /// must not move a picker that is not on screen.
    fn skill_picker_open(&self, cx: &App) -> bool {
        !self.model_menu_open && self.skill_query(cx).is_some()
    }

    fn visible_slash_entries(&self, cx: &App) -> Vec<SlashEntry> {
        let Some(query) = self.skill_query(cx) else {
            return Vec::new();
        };
        let lowered = query.to_lowercase();
        let mut entries: Vec<SlashEntry> = CHAT_COMMANDS
            .iter()
            .filter(|(name, _)| lowered.is_empty() || name.contains(&lowered))
            .map(|(name, description)| SlashEntry {
                name: (*name).to_owned(),
                description: (*description).to_owned(),
                command: format!("/{name}"),
            })
            .collect();
        entries.extend(filter_skills(&self.skills, &query).into_iter().map(|skill| {
            SlashEntry {
                name: skill.name.clone(),
                description: skill.description.clone(),
                command: skill_command(&skill.name),
            }
        }));
        entries.truncate(CHAT_SKILLS_VISIBLE);
        entries
    }

    /// Insert the highlighted slash row. Tab with no prior arrow press takes
    /// the first row because the selection starts at zero.
    fn choose_slash_entry(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(entry) = self
            .visible_slash_entries(cx)
            .get(index)
            .cloned()
        else {
            return false;
        };
        self.input.update(cx, |input, cx| {
            input.set_value(entry.command, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
        true
    }

    fn move_skill_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.visible_slash_entries(cx).len();
        if count == 0 {
            return;
        }
        let current = self.skill_selection.min(count - 1) as isize;
        self.skill_selection = (current + delta).clamp(0, count as isize - 1) as usize;
        self.skill_scroll.scroll_to_item(self.skill_selection);
        cx.notify();
    }

    /// Interceptor-stage keys for composer features that key bindings would
    /// otherwise consume first: Escape pauses a working turn, an image on the
    /// clipboard becomes an attachment, the pickers own Tab/↑/↓/Enter while
    /// open, and ↑/↓ recall sent prompts when no picker is visible.
    fn on_chat_keystroke(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if event.keystroke.key == "escape" && self.is_working() {
            self.stop(cx);
            return true;
        }
        if self.paste_clipboard_image(event, window, cx) {
            return true;
        }
        if self.on_picker_keystroke(event, window, cx) {
            return true;
        }
        self.on_history_keystroke(event, window, cx)
    }

    /// ↑/↓ walk sent prompts; the slot after the last prompt is the empty
    /// input, so Down always ends on a blank composer. The Input keeps the
    /// arrows inside a multi-line draft.
    fn on_history_keystroke(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.input.read(cx).focus_handle(cx).is_focused(window) {
            return false;
        }
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform || modifiers.shift {
            return false;
        }
        if self.model_menu_open
            || self.ask.is_some()
            || self.skill_picker_open(cx)
            || self.mention_picker_open(cx)
        {
            return false;
        }
        match event.keystroke.key.as_str() {
            "up" => {
                if self.history.is_empty() {
                    return false;
                }
                if self.history_cursor.is_none() {
                    let input = self.input.read(cx);
                    let value = input.value().to_string();
                    let cursor = input.cursor();
                    if value
                        .find('\n')
                        .is_some_and(|first_newline| cursor > first_newline)
                    {
                        return false;
                    }
                }
                self.recall_history(-1, window, cx)
            }
            "down" if self.history_cursor.is_some() => self.recall_history(1, window, cx),
            _ => false,
        }
    }

    fn recall_history(
        &mut self,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let len = self.history.len();
        if len == 0 {
            return false;
        }
        let current = self.history_cursor.unwrap_or(len);
        let next = if delta < 0 {
            current.saturating_sub(1)
        } else {
            (current + 1).min(len)
        };
        let value = if next == len {
            String::new()
        } else {
            self.history[next].clone()
        };
        self.history_cursor = (next != len).then_some(next);
        self.history_restore = Some(value.clone());
        self.input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.focus(window, cx);
        });
        true
    }

    /// Ctrl/Cmd+V with an image on the clipboard: write it to the chat cache
    /// and insert its path, exactly like a picked attachment. Text paste keeps
    /// flowing to the Input component.
    fn paste_clipboard_image(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let modifiers = event.keystroke.modifiers;
        let paste = event.keystroke.key == "v"
            && (modifiers.control || modifiers.platform)
            && !modifiers.alt
            && !modifiers.shift;
        if !paste || !self.input.read(cx).focus_handle(cx).is_focused(window) {
            return false;
        }
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let Some(image) = item.entries().iter().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(image.clone()),
            _ => None,
        }) else {
            return false;
        };
        let extension = image_extension(image.format);
        let path = chat_cache_dir()
            .join("paste")
            .join(format!("{}.{extension}", image.id()));
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::write(&path, &image.bytes).is_ok() {
            self.attach_paths(vec![path], cx);
        }
        true
    }

    fn on_picker_keystroke(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.input.read(cx).focus_handle(cx).is_focused(window) {
            return false;
        }
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform {
            return false;
        }
        if self.mention_picker_open(cx) {
            return match event.keystroke.key.as_str() {
                "tab" if !modifiers.shift => {
                    let index = self.mention_selection;
                    self.choose_mention(index, window, cx)
                }
                "enter" if !modifiers.shift => {
                    let count = self.visible_mentions(cx).len();
                    if count == 0 {
                        // Nothing matched: let the composer send the text.
                        return false;
                    }
                    let index = self.mention_selection.min(count - 1);
                    self.choose_mention(index, window, cx)
                }
                "down" if !modifiers.shift => {
                    self.move_mention_selection(1, cx);
                    true
                }
                "up" if !modifiers.shift => {
                    self.move_mention_selection(-1, cx);
                    true
                }
                _ => false,
            };
        }
        if !self.skill_picker_open(cx) {
            return false;
        }
        match event.keystroke.key.as_str() {
            "tab" if !modifiers.shift => {
                let index = self.skill_selection;
                self.choose_slash_entry(index, window, cx)
            }
            "enter" if !modifiers.shift => {
                let count = self.visible_slash_entries(cx).len();
                if count == 0 {
                    // No row: let the composer send a manual slash command
                    // such as /clear exactly as typed.
                    return false;
                }
                let index = self.skill_selection.min(count - 1);
                if self.choose_slash_entry(index, window, cx) {
                    self.send_composed(window, cx);
                    true
                } else {
                    false
                }
            }
            "down" if !modifiers.shift => {
                self.move_skill_selection(1, cx);
                true
            }
            "up" if !modifiers.shift => {
                self.move_skill_selection(-1, cx);
                true
            }
            _ => false,
        }
    }

    pub fn attach_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Anexar arquivos".into()),
        });
        let window_handle = window.window_handle();
        let chat = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            if paths.is_empty() {
                return;
            }
            let _ = cx.update_window(window_handle, |_, _, cx| {
                if let Some(chat) = chat.upgrade() {
                    chat.update(cx, |chat, cx| chat.attach_paths(paths, cx));
                }
            });
        })
        .detach();
    }

    /// Add attachment tiles. Paths stay out of the composer and are appended
    /// only when the message is sent.
    fn attach_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let mut fresh = Vec::new();
        for path in paths {
            if self.attachments.iter().any(|attachment| attachment.path == path) {
                continue;
            }
            let image = is_previewable_image(&path);
            self.attachments.push(Attachment {
                path: path.clone(),
                image,
            });
            fresh.push(path);
        }
        if !fresh.is_empty() {
            cx.notify();
        }
    }

    fn remove_attachment(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.attachments.len() {
            self.attachments.remove(index);
            cx.notify();
        }
    }

    pub fn set_exited(&mut self, exited: bool, cx: &mut Context<Self>) {
        if self.exited != exited {
            self.exited = exited;
            if exited {
                self.queue = ChatQueue::default();
                self.pending.clear();
                self.outbound.clear();
                self.ask = None;
                self.model_switching = false;
                self.status = None;
                self.awaiting_response = false;
                self.observed_output = false;
            }
            cx.notify();
        }
    }

    fn can_send_now(&self) -> bool {
        !self.exited
    }

    fn is_working(&self) -> bool {
        self.ask.is_some()
            || self.status.is_some()
            || (self.awaiting_response && self.session.is_none())
    }

    /// Interrupt the running turn. OMP binds Escape to `app.interrupt`; the
    /// other CLIs accept the usual terminal interrupt.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        let Some((kind, writer)) = self.sender.clone() else {
            return;
        };
        if write_shared(&writer, &stop_bytes(kind)).is_err() {
            return;
        }
        self.queue = ChatQueue::default();
        self.pending.clear();
        self.outbound.clear();
        self.ask = None;
        self.status = None;
        self.awaiting_response = false;
        self.observed_output = false;
        cx.notify();
    }

    fn start_ask(&mut self, request: AskRequest, cx: &mut Context<Self>) {
        let selections = request
            .questions
            .iter()
            .map(|question| {
                if question.kind != AskKind::Choice {
                    Vec::new()
                } else if question.multi {
                    question.selected.clone()
                } else {
                    vec![question.selected.first().copied().unwrap_or(0)]
                }
            })
            .collect();
        let count = request.questions.len();
        self.ask = Some(ActiveAsk {
            request,
            selections,
            other: vec![String::new(); count],
            text: vec![String::new(); count],
            edit: None,
            sending: false,
        });
        self.status = None;
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    fn toggle_ask_option(&mut self, question: usize, option: usize, cx: &mut Context<Self>) {
        let multi = self
            .ask
            .as_ref()
            .and_then(|ask| ask.request.questions.get(question))
            .map(|question| question.multi)
            .unwrap_or(false);
        let Some(ask) = self.ask.as_mut() else {
            return;
        };
        if ask
            .request
            .questions
            .get(question)
            .is_none_or(|question| option >= question.options.len())
        {
            return;
        }
        let selected = &mut ask.selections[question];
        if multi {
            if let Some(position) = selected.iter().position(|index| *index == option) {
                selected.remove(position);
            } else {
                selected.push(option);
            }
        } else {
            *selected = vec![option];
            if let Some(other) = ask.other.get_mut(question) {
                other.clear();
            }
            if ask.edit == Some(AskEdit::Other(question)) {
                ask.edit = None;
            }
        }
        cx.notify();
    }

    /// Open the inline editor for a question's Other row or free-text field.
    fn open_ask_edit(&mut self, target: AskEdit, window: &mut Window, cx: &mut Context<Self>) {
        let value = {
            let Some(ask) = self.ask.as_mut() else {
                return;
            };
            if ask.sending || ask.edit == Some(target) {
                return;
            }
            ask.edit = Some(target);
            match target {
                AskEdit::Other(index) => ask.other.get(index).cloned().unwrap_or_default(),
                AskEdit::Text(index) => ask.text.get(index).cloned().unwrap_or_default(),
            }
        };
        self.ask_input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    /// Mirror the inline editor into the active question. A single-choice
    /// question's custom text replaces its option, exactly like the TUI.
    fn sync_ask_edit(&mut self, value: String, cx: &mut Context<Self>) {
        let Some(ask) = self.ask.as_mut() else {
            return;
        };
        match ask.edit {
            Some(AskEdit::Other(index)) => {
                if let Some(slot) = ask.other.get_mut(index) {
                    *slot = value.clone();
                }
                let single = ask
                    .request
                    .questions
                    .get(index)
                    .is_some_and(|question| !question.multi);
                if single && !value.trim().is_empty() {
                    if let Some(selected) = ask.selections.get_mut(index) {
                        selected.clear();
                    }
                }
            }
            Some(AskEdit::Text(index)) => {
                if let Some(slot) = ask.text.get_mut(index) {
                    *slot = value;
                }
            }
            None => {}
        }
        if !ask.sending {
            cx.notify();
        }
    }

    fn ask_ready(&self) -> bool {
        let Some(ask) = &self.ask else {
            return false;
        };
        !ask.sending
            && !ask.request.questions.is_empty()
            && ask
                .request
                .questions
                .iter()
                .enumerate()
                .all(|(index, question)| match question.kind {
                    AskKind::Choice => {
                        ask.selections
                            .get(index)
                            .is_some_and(|selected| !selected.is_empty())
                            || ask
                                .other
                                .get(index)
                                .is_some_and(|answer| !answer.trim().is_empty())
                    }
                    AskKind::Text => {
                        !question.required
                            || ask
                                .text
                                .get(index)
                                .is_some_and(|answer| !answer.trim().is_empty())
                    }
                })
    }

    fn send_ask(&mut self, cx: &mut Context<Self>) {
        if !self.ask_ready() {
            return;
        }
        let Some((_, writer)) = self.sender.clone() else {
            return;
        };
        let Some(ask) = self.ask.as_mut() else {
            return;
        };
        let custom: Vec<String> = ask
            .request
            .questions
            .iter()
            .enumerate()
            .map(|(index, question)| match question.kind {
                AskKind::Choice => ask.other.get(index).cloned().unwrap_or_default(),
                AskKind::Text => ask.text.get(index).cloned().unwrap_or_default(),
            })
            .collect();
        let sequences = ask_sequences(&ask.request, &ask.selections, &custom);
        if sequences.is_empty() {
            return;
        }
        ask.sending = true;
        cx.spawn(async move |_, cx| {
            for sequence in sequences {
                let _ = write_shared(&writer, &sequence);
                cx.background_executor()
                    .timer(Duration::from_millis(180))
                    .await;
            }
        })
        .detach();
        cx.notify();
    }

    fn observe_terminal(&mut self, lines: Vec<String>, cx: &mut Context<Self>) {
        // OMP has a structured transcript. Reading its screen is what made
        // tool calls and intermediate narration leak into the chat.
        if self.session.is_some() || self.kind == Some(AgentKind::Omp) {
            return;
        }
        if lines.is_empty() {
            return;
        }
        let slash_clear = snapshot_response(&self.last_snapshot, &lines)
            .is_some_and(|text| text.lines().any(|line| line.trim_end().ends_with("/clear")));
        if slash_clear || (screen_cleared(&self.last_snapshot, &lines) && !self.awaiting_response) {
            self.reset_from_terminal(lines, cx);
            return;
        }
        if lines != self.last_snapshot {
            self.last_snapshot = lines;
            self.last_change = Instant::now();
            if self.awaiting_response {
                self.observed_output = true;
                self.status = Some(ChatStatus::Writing);
                cx.notify();
            }
            return;
        }
        if !self.awaiting_response
            || !self.observed_output
            || self.last_change.elapsed() < Duration::from_millis(self.quiet_ms)
        {
            return;
        }
        let Some(text) =
            final_answer_text(&self.snapshot_base, &self.last_snapshot, self.last_sent.as_deref())
        else {
            return;
        };
        if !text.is_empty() {
            if let Some(index) = self.snapshot_message {
                self.messages[index].text = text;
            } else {
                self.messages.push(ChatMessage {
                    role: ChatRole::Agent,
                    text,
                    format: ChatFormat::Plain,
                    images: Vec::new(),
                });
                self.snapshot_message = Some(self.messages.len() - 1);
            }
        }
        self.awaiting_response = false;
        self.observed_output = false;
        self.status = None;
        self.try_send(cx);
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    fn reset_from_terminal(&mut self, lines: Vec<String>, cx: &mut Context<Self>) {
        self.messages.clear();
        self.status = None;
        self.ask = None;
        self.queue = ChatQueue::default();
        self.pending.clear();
        self.awaiting_response = false;
        self.observed_output = false;
        self.snapshot_message = None;
        self.snapshot_base = lines.clone();
        self.last_snapshot = lines;
        self.last_change = Instant::now();
        cx.notify();
    }


    pub fn enqueue(&mut self, text: String, is_skill: bool, cx: &mut Context<Self>) {
        let text = text.trim().to_owned();
        if self.exited || self.ask.is_some() || text.is_empty() {
            return;
        }
        self.queue.enqueue(text.clone(), is_skill);
        self.pending.push(text);
        self.try_send(cx);
        cx.notify();
    }

    pub fn agent_finished(&mut self, cx: &mut Context<Self>) {
        self.status = None;
        self.ask = None;
        self.try_send(cx);
        cx.notify();
    }

    fn try_send(&mut self, cx: &mut Context<Self>) {
        if self.ask.is_some() || self.status.is_some() {
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
        let structured = self.session.is_some() || self.kind == Some(AgentKind::Omp);
        if structured {
            if !pending.is_skill {
                self.outbound.push_back(normalize_message(&pending.text));
            }
        } else {
            self.snapshot_base = self.last_snapshot.clone();
            self.last_sent = Some(pending.text.clone());
            self.awaiting_response = true;
            self.observed_output = false;
            self.snapshot_message = None;
            self.last_change = Instant::now();
        }
        self.messages.push(ChatMessage {
            role: ChatRole::User,
            text: pending.text,
            format: ChatFormat::Markdown,
            images: Vec::new(),
        });
        self.status = Some(ChatStatus::Thinking);
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    /// Enter and the send button both send the typed text with attachment
    /// paths appended; the paths are never shown in the composer.
    fn send_composed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.exited || self.ask.is_some() {
            return;
        }
        let typed = self.input.read(cx).value().to_string();
        let is_skill = is_command_text(&typed);
        let images: Vec<PathBuf> = self
            .attachments
            .iter()
            .filter(|attachment| attachment.image)
            .map(|attachment| attachment.path.clone())
            .collect();
        let paths = attached_paths_text(
            &self
                .attachments
                .iter()
                .map(|attachment| attachment.path.clone())
                .collect::<Vec<_>>(),
        );
        let text = match (typed.trim().is_empty(), paths.is_empty()) {
            (true, true) => return,
            (true, false) => paths,
            (false, true) => typed.clone(),
            (false, false) => format!("{typed} {paths}"),
        };
        let prompt = typed.trim();
        if !prompt.is_empty()
            && self.history.last().map(String::as_str) != Some(prompt)
        {
            self.history.push(prompt.to_owned());
            if self.history.len() > CHAT_HISTORY_LIMIT {
                self.history.remove(0);
            }
        }
        self.history_cursor = None;
        self.history_restore = None;
        let before = self.messages.len();
        self.input.update(cx, |input, cx| input.set_value("", window, cx));
        self.enqueue(text, is_skill, cx);
        if self.messages.len() == before + 1 {
            if let Some(message) = self.messages.last_mut() {
                message.text = typed;
                message.images = images;
            }
        }
        self.attachments.clear();
        cx.notify();
    }


    pub fn focus_input(&self, window: &mut Window, cx: &mut App) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn value(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    /// Rounded preview tiles above the composer. Images show the file itself;
    /// documents show a file logo and the extension.
    fn attachments_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.attachments.is_empty() {
            return None;
        }
        let tiles: Vec<AnyElement> = self
            .attachments
            .iter()
            .enumerate()
            .map(|(index, attachment)| {
                let path = attachment.path.clone();
                let preview: AnyElement = if attachment.image {
                    img(ImageSource::from(path.as_path()))
                        .w_full()
                        .h_full()
                        .object_fit(ObjectFit::Cover)
                        .into_any_element()
                } else {
                    let icon = attachment_icon(&path);
                    let label = attachment_label(&path);
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(self.z(2.0))
                        .child(
                            svg()
                                .path(icon)
                                .w(self.z(CHAT_ATTACH_PREVIEW_ICON))
                                .h(self.z(CHAT_ATTACH_PREVIEW_ICON))
                                .text_color(rgb(COLOR_MUTED)),
                        )
                        .child(
                            div()
                                .text_size(self.z(CHAT_ATTACH_LABEL_FONT))
                                .text_color(rgb(COLOR_MUTED))
                                .child(label),
                        )
                        .into_any_element()
                };
                div()
                    .id(("chat-attachment", index))
                    .debug_selector(move || format!("chat-attachment-{index}"))
                    .relative()
                    .w(self.z(CHAT_ATTACH_PREVIEW))
                    .h(self.z(CHAT_ATTACH_PREVIEW))
                    .rounded(self.z(RADIUS_CONTROL))
                    .overflow_hidden()
                    .bg(rgb(COLOR_ELEVATED))
                    .child(preview)
                    .child(
                        div()
                            .id(("chat-attachment-remove", index))
                            .debug_selector(move || format!("chat-attachment-remove-{index}"))
                            .absolute()
                            .top(self.z(2.0))
                            .right(self.z(2.0))
                            .w(self.z(CHAT_ATTACH_REMOVE))
                            .h(self.z(CHAT_ATTACH_REMOVE))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(self.z(RADIUS_PILL))
                            .bg(rgb(COLOR_BASE))
                            .text_size(self.z(CHAT_ATTACH_REMOVE_ICON))
                            .text_color(rgb(COLOR_MUTED))
                            .hover(|style| style.text_color(rgb(COLOR_FOREGROUND)))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.remove_attachment(index, cx)
                            }))
                            .child("×"),
                    )
                    .into_any_element()
            })
            .collect();
        Some(
            div()
                .id("chat-attachments")
                .debug_selector(|| "chat-attachments".to_string())
                .flex()
                .flex_wrap()
                .gap(self.z(SPACE_NORMAL))
                .px(self.z(SPACE_WIDE))
                .pb(self.z(SPACE_TIGHT))
                .children(tiles)
                .into_any_element(),
        )
    }

    fn render_message(
        &self,
        index: usize,
        message: &ChatMessage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selector = format!("chat-message-{index}");
        let zoom = self.zoom;
        let content: AnyElement = match message.format {
            ChatFormat::Markdown => TextView::markdown(
                SharedString::from(format!(
                    "chat-md-{index}-{}",
                    (self.zoom * 100.0).round() as i32
                )),
                message.text.clone(),
                window,
                cx,
            )
            .style(
                TextViewStyle::default().code_block(
                    StyleRefinement::default()
                        .p(self.z(CHAT_CODE_PADDING))
                        .pr(self.z(CHAT_COPY_RESERVE))
                        .rounded(self.z(RADIUS_CONTROL))
                        .text_size(self.z(CHAT_CODE_FONT))
                        .whitespace_nowrap(),
                ),
            )
            .selectable(true)
            .text_size(self.z(CHAT_TEXT_SIZE))
            .text_color(rgb(COLOR_FOREGROUND))
            .code_block_actions(move |code, _, _cx| {
                let text = code.code();
                Button::new(code_button_id(&text))
                    .tooltip("Copiar")
                    .child(
                        svg()
                            .path("copy.svg")
                            .w(px(ICON_SIZE * zoom))
                            .h(px(ICON_SIZE * zoom))
                            .text_color(rgb(COLOR_MUTED)),
                    )
                    .with_variant(ButtonVariant::Ghost)
                    .w(px(CHAT_COPY_HEIGHT * zoom))
                    .h(px(CHAT_COPY_HEIGHT * zoom))
                    .p_0()
                    .rounded(px(RADIUS_CHIP * zoom))
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
                    })
            })
            .into_any_element(),
            ChatFormat::Plain => div()
                .text_size(self.z(CHAT_TEXT_SIZE))
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
                        .py(self.z(SPACE_TIGHT))
                        .text_size(self.z(CHAT_STATUS_FONT))
                        .text_color(rgb(COLOR_MUTED))
                        .child(message.text.clone()),
                )
                .into_any_element(),
            ChatRole::User => row
                .justify_end()
                .child(
                    div()
                        .relative()
                        .max_w(relative(CHAT_BUBBLE_WIDTH))
                        .rounded(self.z(CHAT_BUBBLE_RADIUS))
                        .rounded_tr(self.z(CHAT_BUBBLE_TAIL_RADIUS))
                        .bg(rgb(COLOR_CHAT_USER_BG))
                        .p(self.z(CHAT_BUBBLE_PADDING))
                        .child(if message.images.is_empty() {
                            content
                        } else {
                            let tiles = message
                                .images
                                .iter()
                                .enumerate()
                                .map(|(tile_index, path)| {
                                    img(ImageSource::from(path.as_path()))
                                        .id(ElementId::Name(
                                            format!("chat-message-image-{index}-{tile_index}").into(),
                                        ))
                                        .debug_selector(move || {
                                            format!("chat-message-image-{index}-{tile_index}")
                                        })
                                        .w(self.z(CHAT_MESSAGE_IMAGE_W))
                                        .h(self.z(CHAT_MESSAGE_IMAGE_H))
                                        .rounded(self.z(RADIUS_CHIP))
                                        .object_fit(ObjectFit::Cover)
                                        .into_any_element()
                                })
                                .collect::<Vec<AnyElement>>();
                            div()
                                .flex()
                                .flex_col()
                                .gap(self.z(SPACE_TIGHT))
                                .children(tiles)
                                .child(content)
                                .into_any_element()
                        })
                        .child(
                            svg()
                                .path("bubble-tail-right.svg")
                                .absolute()
                                .top(self.z(0.0))
                                .right(self.z(-(CHAT_TAIL_WIDTH - CHAT_TAIL_OVERLAP)))
                                .w(self.z(CHAT_TAIL_WIDTH))
                                .h(self.z(CHAT_TAIL_HEIGHT))
                                .text_color(rgb(COLOR_CHAT_USER_BG)),
                        ),
                )
                .into_any_element(),
            ChatRole::Agent => row
                .justify_start()
                .child(
                    div()
                        .relative()
                        .max_w(relative(CHAT_BUBBLE_WIDTH))
                        .rounded(self.z(CHAT_BUBBLE_RADIUS))
                        .rounded_tl(self.z(CHAT_BUBBLE_TAIL_RADIUS))
                        .bg(rgb(COLOR_CHAT_AGENT_BG))
                        .p(self.z(CHAT_BUBBLE_PADDING))
                        .child(content)
                        .child(
                            svg()
                                .path("bubble-tail-left.svg")
                                .absolute()
                                .top(self.z(0.0))
                                .left(self.z(-(CHAT_TAIL_WIDTH - CHAT_TAIL_OVERLAP)))
                                .w(self.z(CHAT_TAIL_WIDTH))
                                .h(self.z(CHAT_TAIL_HEIGHT))
                                .text_color(rgb(COLOR_CHAT_AGENT_BG)),
                        ),
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

/// Run the model catalog through a login shell so `omp` resolves with the same
/// PATH the agent card gets. The app itself may be launched from a desktop
/// session without ~/.local/bin.
fn omp_models_output() -> std::io::Result<std::process::Output> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
    let omp = std::process::Command::new(&shell)
        .args(["-lc", "command -v omp"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .last()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "omp".to_owned());
    std::process::Command::new(omp)
        .args(["models", "--json"])
        .output()
}

/// One dot of the three-dot wave shown while the agent is working. The phase
/// offset makes the three dots travel as a wave instead of bouncing together.
fn wave_dot(index: usize, scale: f32) -> AnyElement {
    div()
        .relative()
        .w(px(CHAT_WAVE_DOT * scale))
        .h(px(CHAT_WAVE_DOT * scale))
        .rounded_full()
        .bg(rgb(COLOR_MUTED))
        .with_animation(
            ElementId::named_usize("chat-wave-dot", index),
            Animation::new(Duration::from_millis(CHAT_WAVE_DURATION_MS)).repeat(),
            move |dot, delta| {
                let phase = (delta + index as f32 / 3.0).fract();
                let offset = (phase * std::f32::consts::TAU).sin() * CHAT_WAVE_AMPLITUDE;
                dot.top(px(offset * scale))
            },
        )
        .into_any_element()
}

fn image_extension(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Webp => "webp",
        ImageFormat::Gif => "gif",
        ImageFormat::Svg => "svg",
        ImageFormat::Bmp => "bmp",
        ImageFormat::Tiff => "tiff",
    }
}

/// An image tall enough to preview directly. Huge files fall back to the file
/// tile so opening a chat cannot decode a 200 MB photo.
fn is_previewable_image(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };
    let image = matches!(
        extension.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "svg" | "tiff" | "tif"
    );
    image
        && std::fs::metadata(path)
            .is_ok_and(|metadata| metadata.len() <= CHAT_ATTACH_IMAGE_MAX_BYTES)
}

fn attachment_icon(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "c" | "h" | "cpp" | "hpp"
        | "java" | "sh" | "bash" | "zsh" | "fish" | "toml" | "json" | "yaml" | "yml"
        | "md" | "css" | "html" | "xml" | "sql" => "code.svg",
        _ => "file.svg",
    }
}

fn attachment_label(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|extension| {
            extension
                .to_ascii_uppercase()
                .chars()
                .take(5)
                .collect::<String>()
        })
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| "FILE".into())
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

/// Match the exact selector or the selector without its thinking suffix.
fn model_is_current(model: &ModelInfo, current: Option<&str>) -> bool {
    current.is_some_and(|current| {
        current == model.selector || current.split(':').next() == Some(model.selector.as_str())
    })
}

/// The provider list keeps the catalog order; inside the selected provider the
/// model the CLI is already using comes first. `sort_by_key` is stable, so the
/// rest of the list does not move.
fn order_models_current_first<'a>(
    mut models: Vec<&'a ModelInfo>,
    current: Option<&str>,
) -> Vec<&'a ModelInfo> {
    models.sort_by_key(|model| !model_is_current(model, current));
    models
}

/// OMP's `/model` is a command-palette action, not a chat line: writing
/// `/model ...` to the PTY becomes a user prompt. The reliable path is the
/// temporary-model picker (Alt+P), a filter, and Enter.
pub fn model_switch_sequence(selector: &str) -> Vec<Vec<u8>> {
    vec![
        vec![0x1b, b'p'],
        selector.as_bytes().to_vec(),
        vec![b'\r'],
    ]
}

/// Interrupt sequence per agent. OMP binds Escape to `app.interrupt`; the
/// other CLIs stop the running turn with the terminal interrupt.
pub fn stop_bytes(kind: AgentKind) -> Vec<u8> {
    match kind {
        AgentKind::Omp => vec![0x1b],
        _ => vec![0x03],
    }
}

impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut all_messages = self.messages.clone();
        all_messages.extend(self.pending.iter().map(|text| ChatMessage {
            role: ChatRole::User,
            text: format!("{text}\n\n_na fila_"),
            format: ChatFormat::Markdown,
            images: Vec::new(),
        }));
        let messages: Vec<AnyElement> = all_messages
            .iter()
            .enumerate()
            .map(|(index, message)| self.render_message(index, message, window, cx))
            .collect();
        let ask_bubble: Option<AnyElement> = self.ask.as_ref().map(|ask| {
            let mut questions: Vec<AnyElement> = Vec::new();
            for (question_index, question) in ask.request.questions.iter().enumerate() {
                let selected = ask.selections.get(question_index);
                let mut options: Vec<AnyElement> = Vec::new();
                for (option_index, option) in question.options.iter().enumerate() {
                    let is_selected =
                        selected.is_some_and(|selected| selected.contains(&option_index));
                    let marker = match (question.multi, is_selected) {
                        (true, true) => "☑",
                        (true, false) => "☐",
                        (false, true) => "◉",
                        (false, false) => "○",
                    };
                    options.push(
                        div()
                            .id(("chat-ask-option", question_index * 100 + option_index))
                            .debug_selector(move || {
                                format!("chat-ask-option-{question_index}-{option_index}")
                            })
                            .flex()
                            .items_start()
                            .gap(self.z(SPACE_NORMAL))
                            .px(self.z(SPACE_TIGHT))
                            .py(self.z(SPACE_TIGHT))
                            .rounded(self.z(RADIUS_CHIP))
                            .when(is_selected, |row| row.bg(rgb(COLOR_CHAT_ASK_OPTION)))
                            .hover(|row| row.bg(rgb(COLOR_CHAT_ASK_OPTION_HOVER)))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_ask_option(question_index, option_index, cx)
                            }))
                            .child(
                                div()
                                    .text_size(self.z(CHAT_STATUS_LABEL_FONT))
                                    .text_color(if is_selected {
                                        rgb(COLOR_ACCENT)
                                    } else {
                                        rgb(COLOR_MUTED)
                                    })
                                    .child(marker),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .text_size(self.z(CHAT_TEXT_SIZE))
                                            .text_color(rgb(COLOR_FOREGROUND))
                                            .child(option.label.clone()),
                                    )
                                    .when_some(option.description.clone(), |this, description| {
                                        this.child(
                                            div()
                                                .text_size(self.z(CHAT_STATUS_FONT))
                                                .text_color(rgb(COLOR_MUTED))
                                                .child(description),
                                        )
                                    }),
                            )
                            .into_any_element(),
                    );
                }
                let mut body = div().flex().flex_col().gap(self.z(SPACE_TIGHT));
                if !question.header.is_empty() {
                    body = body.child(
                        div()
                            .text_size(self.z(CHAT_STATUS_FONT))
                            .text_color(rgb(COLOR_MUTED))
                            .child(question.header.clone()),
                    );
                }
                body = body.child(
                    div()
                        .text_size(self.z(CHAT_TEXT_SIZE))
                        .text_color(rgb(COLOR_FOREGROUND))
                        .child(question.question.clone()),
                );
                if question.kind == AskKind::Text {
                    let editing = ask.edit == Some(AskEdit::Text(question_index));
                    let answer = ask.text.get(question_index).cloned().unwrap_or_default();
                    let empty = answer.trim().is_empty();
                    let mut row = div()
                        .id(("chat-ask-text", question_index))
                        .debug_selector(move || format!("chat-ask-text-{question_index}"))
                        .flex()
                        .w_full()
                        .items_center()
                        .gap(self.z(SPACE_NORMAL))
                        .px(self.z(SPACE_TIGHT))
                        .py(self.z(SPACE_TIGHT))
                        .rounded(self.z(RADIUS_CHIP))
                        .when(!empty || editing, |row| row.bg(rgb(COLOR_CHAT_ASK_OPTION)))
                        .hover(|row| row.bg(rgb(COLOR_CHAT_ASK_OPTION_HOVER)))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_ask_edit(AskEdit::Text(question_index), window, cx)
                        }));
                    if editing {
                        row = row.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&self.ask_input).w_full().rounded(self.z(RADIUS_INPUT))),
                        );
                    } else {
                        row = row.child(
                            div()
                                .text_size(self.z(CHAT_TEXT_SIZE))
                                .text_color(rgb(if empty {
                                    COLOR_MUTED
                                } else {
                                    COLOR_FOREGROUND
                                }))
                                .child(if empty {
                                    "Digite sua resposta…".to_string()
                                } else {
                                    answer
                                }),
                        );
                    }
                    body = body.child(row);
                    if question.required && empty {
                        body = body.child(
                            div()
                                .text_size(self.z(CHAT_STATUS_FONT))
                                .text_color(rgb(COLOR_MUTED))
                                .child("Obrigatório"),
                        );
                    }
                } else {
                    body = body.children(options);
                    let other_text = ask
                        .other
                        .get(question_index)
                        .cloned()
                        .unwrap_or_default();
                    let other_chosen = !other_text.trim().is_empty();
                    let editing = ask.edit == Some(AskEdit::Other(question_index));
                    let marker = match (question.multi, other_chosen) {
                        (true, true) => "☑",
                        (true, false) => "☐",
                        (false, true) => "◉",
                        (false, false) => "○",
                    };
                    let mut row = div()
                        .id(("chat-ask-other", question_index))
                        .debug_selector(move || format!("chat-ask-other-{question_index}"))
                        .flex()
                        .w_full()
                        .items_center()
                        .gap(self.z(SPACE_NORMAL))
                        .px(self.z(SPACE_TIGHT))
                        .py(self.z(SPACE_TIGHT))
                        .rounded(self.z(RADIUS_CHIP))
                        .when(other_chosen || editing, |row| {
                            row.bg(rgb(COLOR_CHAT_ASK_OPTION))
                        })
                        .hover(|row| row.bg(rgb(COLOR_CHAT_ASK_OPTION_HOVER)))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_ask_edit(AskEdit::Other(question_index), window, cx)
                        }))
                        .child(
                            div()
                                .text_size(self.z(CHAT_STATUS_LABEL_FONT))
                                .text_color(if other_chosen {
                                    rgb(COLOR_ACCENT)
                                } else {
                                    rgb(COLOR_MUTED)
                                })
                                .child(marker),
                        );
                    if editing {
                        row = row.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&self.ask_input).w_full().rounded(self.z(RADIUS_INPUT))),
                        );
                    } else {
                        row = row.child(
                            div()
                                .text_size(self.z(CHAT_TEXT_SIZE))
                                .text_color(rgb(if other_chosen {
                                    COLOR_FOREGROUND
                                } else {
                                    COLOR_MUTED
                                }))
                                .child(if other_chosen {
                                    other_text
                                } else {
                                    "Outra resposta…".to_string()
                                }),
                        );
                    }
                    body = body.child(row);
                }
                questions.push(body.into_any_element());
            }
            let ready = self.ask_ready();
            let sending = ask.sending;
            div()
                .id("chat-ask")
                .debug_selector(|| "chat-ask".to_string())
                .flex()
                .w_full()
                .justify_start()
                .child(
                    div()
                        .relative()
                        .max_w(relative(CHAT_BUBBLE_WIDTH))
                        .rounded(self.z(CHAT_BUBBLE_RADIUS))
                        .rounded_tl(self.z(CHAT_BUBBLE_TAIL_RADIUS))
                        .bg(rgb(COLOR_CHAT_AGENT_BG))
                        .p(self.z(CHAT_BUBBLE_PADDING))
                        .flex()
                        .flex_col()
                        .gap(self.z(SPACE_NORMAL))
                        .child(
                            div()
                                .text_size(self.z(CHAT_STATUS_FONT))
                                .text_color(rgb(COLOR_MUTED))
                                .child("Pergunta do agente"),
                        )
                        .children(questions)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .pt(self.z(SPACE_TIGHT))
                                .child(
                                    div()
                                        .text_size(self.z(CHAT_STATUS_FONT))
                                        .text_color(rgb(COLOR_MUTED))
                                        .child(if sending { "Enviando…" } else { "" }),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(self.z(SPACE_NORMAL))
                                        .child(
                                            div()
                                                .id("chat-ask-cancel")
                                                .debug_selector(|| {
                                                    "chat-ask-cancel".to_string()
                                                })
                                                .px(self.z(SPACE_WIDE))
                                                .py(self.z(SPACE_TIGHT))
                                                .rounded(self.z(RADIUS_PILL))
                                                .bg(rgb(COLOR_ELEVATED))
                                                .text_size(self.z(CHAT_STATUS_LABEL_FONT))
                                                .text_color(rgb(COLOR_MUTED))
                                                .hover(|style| {
                                                    style.text_color(rgb(COLOR_FOREGROUND))
                                                })
                                                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                    cx.stop_propagation()
                                                })
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.stop(cx)
                                                }))
                                                .child("Cancelar"),
                                        )
                                        .child(
                                            div()
                                                .id("chat-ask-send")
                                                .debug_selector(|| {
                                                    "chat-ask-send".to_string()
                                                })
                                                .px(self.z(SPACE_WIDE))
                                                .py(self.z(SPACE_TIGHT))
                                                .rounded(self.z(RADIUS_PILL))
                                                .text_size(self.z(CHAT_STATUS_LABEL_FONT))
                                                .when(ready && !sending, |button| {
                                                    button
                                                        .bg(rgb(COLOR_ACCENT))
                                                        .text_color(rgb(COLOR_BASE))
                                                        .on_click(cx.listener(
                                                            |this, _, _, cx| this.send_ask(cx),
                                                        ))
                                                })
                                                .when(!ready || sending, |button| {
                                                    button
                                                        .bg(rgb(COLOR_ELEVATED))
                                                        .text_color(rgb(COLOR_MUTED))
                                                })
                                                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                    cx.stop_propagation()
                                                })
                                                .child(if sending { "Enviando…" } else { "Enviar" }),
                                        ),
                                ),
                        )
                        .child(
                            svg()
                                .path("bubble-tail-left.svg")
                                .absolute()
                                .top(self.z(0.0))
                                .left(self.z(-(CHAT_TAIL_WIDTH - CHAT_TAIL_OVERLAP)))
                                .w(self.z(CHAT_TAIL_WIDTH))
                                .h(self.z(CHAT_TAIL_HEIGHT))
                                .text_color(rgb(COLOR_CHAT_AGENT_BG)),
                        ),
                )
                .into_any_element()
        });
        let input_enabled = self.can_send_now() && self.ask.is_none() && !self.model_switching;
        let working = self.is_working();
        let status_bubble: Option<AnyElement> = self.status.as_ref().map(|status| {
            div()
                .id("chat-status-bubble")
                .debug_selector(|| "chat-status-bubble".to_string())
                .flex()
                .w_full()
                .justify_start()
                .child(
                    div()
                        .id("chat-status")
                        .debug_selector(|| "chat-status".to_string())
                        .relative()
                        .flex()
                        .items_center()
                        .gap(self.z(CHAT_STATUS_GAP))
                        .max_w(relative(CHAT_BUBBLE_WIDTH))
                        .rounded(self.z(CHAT_BUBBLE_RADIUS))
                        .rounded_tl(self.z(CHAT_BUBBLE_TAIL_RADIUS))
                        .bg(rgb(COLOR_CHAT_AGENT_BG))
                        .p(self.z(CHAT_STATUS_PADDING))
                        .child(
                            div()
                                .id("chat-wave")
                                .debug_selector(|| "chat-wave".to_string())
                                .flex()
                                .items_center()
                                .gap(self.z(CHAT_WAVE_GAP))
                                .h(self.z(CHAT_WAVE_HEIGHT))
                                .children((0..3).map(|index| wave_dot(index, self.zoom))),
                        )
                        .child(
                            div()
                                .text_size(self.z(CHAT_STATUS_LABEL_FONT))
                                .text_color(rgb(COLOR_MUTED))
                                .child(status.label()),
                        )
                        .child(
                            svg()
                                .path("bubble-tail-left.svg")
                                .absolute()
                                .top(self.z(0.0))
                                .left(self.z(-(CHAT_TAIL_WIDTH - CHAT_TAIL_OVERLAP)))
                                .w(self.z(CHAT_TAIL_WIDTH))
                                .h(self.z(CHAT_TAIL_HEIGHT))
                                .text_color(rgb(COLOR_CHAT_AGENT_BG)),
                        ),
                )
                .into_any_element()
        });
        let show_model_selector = self.kind == Some(AgentKind::Omp) || self.session.is_some();
        let model_name = self.model_display_name();
        let model_menu: Option<AnyElement> = if self.model_menu_open && show_model_selector {
            let query = self.model_query.read(cx).value().to_string();
            let current = self.current_model.clone();
            let has_catalog = !self.models.is_empty();
            let mut providers: Vec<(String, usize)> = Vec::new();
            for model in &self.models {
                if let Some((_, count)) = providers
                    .iter_mut()
                    .find(|(provider, _)| provider == &model.provider)
                {
                    *count += 1;
                } else {
                    providers.push((model.provider.clone(), 1));
                }
            }
            providers.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
            let current_provider = current
                .as_ref()
                .and_then(|model| model.split('/').next())
                .map(str::to_owned);
            let selected_provider = self
                .selected_provider
                .clone()
                .filter(|provider| providers.iter().any(|(name, _)| name == provider))
                .or_else(|| {
                    current_provider
                        .clone()
                        .filter(|provider| {
                            providers.iter().any(|(name, _)| name == provider)
                        })
                })
                .or_else(|| providers.first().map(|(provider, _)| provider.clone()));
            let mut visible = filter_models(&self.models, &query);
            if let Some(provider) = &selected_provider {
                visible.retain(|model| &model.provider == provider);
            }
            let visible = order_models_current_first(visible, current.as_deref());
            let model_rows: Vec<AnyElement> = visible
                .into_iter()
                .take(80)
                .enumerate()
                .map(|(index, model)| {
                    let selector = model.selector.clone();
                    let is_current = model_is_current(model, current.as_deref());
                    div()
                        .id(("chat-model-row", index))
                        .debug_selector(move || format!("chat-model-row-{index}"))
                        .flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .gap(self.z(SPACE_NORMAL))
                        .px(self.z(SPACE_NORMAL))
                        .py(self.z(SPACE_TIGHT))
                        .rounded(self.z(RADIUS_CHIP))
                        .when(is_current, |row| row.bg(rgb(COLOR_ELEVATED)))
                        .hover(|row| row.bg(rgb(COLOR_ELEVATED)))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select_model(selector.clone(), window, cx)
                        }))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                            .flex_1()
                                .child(
                                    div()
                                        .text_size(self.z(CHAT_MODEL_NAME_FONT))
                                        .text_color(rgb(COLOR_FOREGROUND))
                                        .child(model.name.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(self.z(CHAT_MODEL_PROVIDER_FONT))
                                        .text_color(rgb(COLOR_MUTED))
                                        .child(model.selector.clone()),
                                ),
                        )
                        .when(is_current, |row| {
                            row.child(
                                div()
                                    .debug_selector(|| "chat-model-current-badge".to_string())
                                    .text_size(self.z(CHAT_MODEL_PROVIDER_FONT))
                                    .text_color(rgb(COLOR_ACCENT))
                                    .child("atual"),
                            )
                        })
                        .into_any_element()
                })
                .collect();
            let provider_rows: Vec<AnyElement> = providers
                .iter()
                .enumerate()
                .map(|(index, (provider, count))| {
                    let provider_name = provider.clone();
                    let provider_for_click = provider.clone();
                    let is_selected = selected_provider.as_deref() == Some(provider.as_str());
                    div()
                        .id(("chat-model-provider", index))
                        .debug_selector(move || format!("chat-model-provider-{index}"))
                        .flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .gap(self.z(SPACE_TIGHT))
                        .px(self.z(SPACE_TIGHT))
                        .py(self.z(SPACE_TIGHT))
                        .rounded(self.z(RADIUS_CHIP))
                        .when(is_selected, |row| row.bg(rgb(COLOR_ELEVATED)))
                        .hover(|row| row.bg(rgb(COLOR_ELEVATED)))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected_provider = Some(provider_for_click.clone());
                            cx.notify();
                        }))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(self.z(SPACE_TIGHT))
                                .when(
                                    current_provider.as_deref() == Some(provider.as_str()),
                                    |row| {
                                        row.child(
                                            div()
                                                .id(("chat-model-provider-in-use", index))
                                                .debug_selector(move || {
                                                    format!("chat-model-provider-in-use-{index}")
                                                })
                                                .w(self.z(CHAT_MODEL_IN_USE_DOT))
                                                .h(self.z(CHAT_MODEL_IN_USE_DOT))
                                                .rounded_full()
                                                .bg(rgb(COLOR_ACCENT)),
                                        )
                                    },
                                )
                                .child(
                                    div()
                                        .text_size(self.z(CHAT_MODEL_PROVIDER_ROW_FONT))
                                        .text_color(rgb(if is_selected {
                                            COLOR_FOREGROUND
                                        } else {
                                            COLOR_MUTED
                                        }))
                                        .child(provider_label(&provider_name)),
                                ),
                        )
                        .child(
                            div()
                                .text_size(self.z(CHAT_MODEL_PROVIDER_COUNT_FONT))
                                .text_color(rgb(COLOR_MUTED))
                                .child(count.to_string()),
                        )
                        .into_any_element()
                })
                .collect();
            let body: AnyElement = if !has_catalog {
                if self.models_loading {
                    div()
                        .py(self.z(SPACE_NORMAL))
                        .text_size(self.z(CHAT_STATUS_FONT))
                        .text_color(rgb(COLOR_MUTED))
                        .child("Carregando modelos…")
                        .into_any_element()
                } else if let Some(error) = &self.models_error {
                    div()
                        .py(self.z(SPACE_NORMAL))
                        .text_size(self.z(CHAT_STATUS_FONT))
                        .text_color(rgb(COLOR_DANGER))
                        .child(error.clone())
                        .into_any_element()
                } else {
                    div()
                        .py(self.z(SPACE_NORMAL))
                        .text_size(self.z(CHAT_STATUS_FONT))
                        .text_color(rgb(COLOR_MUTED))
                        .child("Nenhum modelo disponível")
                        .into_any_element()
                }
            } else {
                let model_list: AnyElement = if model_rows.is_empty() {
                    div()
                        .py(self.z(SPACE_NORMAL))
                        .text_size(self.z(CHAT_STATUS_FONT))
                        .text_color(rgb(COLOR_MUTED))
                        .child("Nenhum modelo encontrado")
                        .into_any_element()
                } else {
                    div()
                        .id("chat-model-list")
                        .debug_selector(|| "chat-model-list".to_string())
                        .flex_1()
                        .min_w_0()
                        .w_full()
                        .min_h_0()
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap(self.z(CHAT_MODEL_ROW_GAP))
                        .children(model_rows)
                        .into_any_element()
                };
                let provider_title = selected_provider
                    .as_deref()
                    .map(provider_label)
                    .map(|provider| format!("Modelos · {provider}"))
                    .unwrap_or_else(|| "Modelos".to_owned());
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .w_full()
                    .gap(self.z(SPACE_NORMAL))
                    .child(
                        div()
                            .w(self.z(CHAT_MODEL_PROVIDER_WIDTH))
                            .h_full()
                            .flex()
                            .flex_col()
                            .gap(self.z(CHAT_MODEL_ROW_GAP))
                            .child(
                                div()
                                    .px(self.z(SPACE_TIGHT))
                                    .text_size(self.z(CHAT_MODEL_PROVIDER_COUNT_FONT))
                                    .text_color(rgb(COLOR_MUTED))
                                    .child("Provedores"),
                            )
                            .child(
                                div()
                                    .id("chat-model-providers")
                                    .debug_selector(|| "chat-model-providers".to_string())
                                    .flex_1()
                                    .w_full()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .flex()
                                    .flex_col()
                                    .gap(self.z(CHAT_MODEL_ROW_GAP))
                                    .children(provider_rows),
                            ),
                    )
                    .child(
                        div()
                            .w(self.z(CHAT_MODEL_DIVIDER_WIDTH))
                            .h_full()
                            .bg(rgb(COLOR_BORDER)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .flex_col()
                            .gap(self.z(CHAT_MODEL_ROW_GAP))
                            .child(
                                div()
                                    .px(self.z(SPACE_TIGHT))
                                    .text_size(self.z(CHAT_MODEL_PROVIDER_COUNT_FONT))
                                    .text_color(rgb(COLOR_MUTED))
                                    .child(provider_title),
                            )
                            .child(model_list),
                    )
                    .into_any_element()
            };
            Some(
                div()
                    .id("chat-model-overlay")
                    .debug_selector(|| "chat-model-overlay".to_string())
                    .absolute()
                    .top(self.z(0.0))
                    .left(self.z(0.0))
                    .right(self.z(0.0))
                    .bottom(self.z(0.0))
                    .child(
                        div()
                            .absolute()
                            .top(self.z(0.0))
                            .left(self.z(0.0))
                            .right(self.z(0.0))
                            .bottom(self.z(0.0))
                            .on_mouse_down(MouseButton::Left, cx.listener(
                                |this, _, window, cx| this.close_model_menu(window, cx),
                            )),
                    )
                    .child(
                        div()
                            .id("chat-model-menu")
                            .debug_selector(|| "chat-model-menu".to_string())
                            .absolute()
                            .top(self.z(CHAT_MODEL_BAR_HEIGHT))
                            .right(self.z(SPACE_WIDE))
                            .w(self.z(CHAT_MODEL_MENU_WIDTH))
                            .h(self.z(CHAT_MODEL_MENU_HEIGHT))
                            .rounded(self.z(RADIUS_CONTROL))
                            .border_1()
                            .border_color(rgb(COLOR_BORDER))
                            .bg(rgb(COLOR_PANEL))
                            .p(self.z(SPACE_NORMAL))
                            .flex()
                            .flex_col()
                            .gap(self.z(SPACE_TIGHT))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_size(self.z(FONT_CARD_TITLE))
                                            .text_color(rgb(COLOR_FOREGROUND))
                                            .child("Modelo"),
                                    )
                                    .when(self.models_loading, |header| {
                                        header.child(
                                            div()
                                                .text_size(self.z(CHAT_MODEL_PROVIDER_FONT))
                                                .text_color(rgb(COLOR_MUTED))
                                                .child("atualizando…"),
                                        )
                                    })
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(self.z(SPACE_TIGHT))
                                            .child(
                                                div()
                                                    .id("chat-model-refresh")
                                                    .debug_selector(|| {
                                                        "chat-model-refresh".to_string()
                                                    })
                                                    .px(self.z(SPACE_TIGHT))
                                                    .text_color(rgb(COLOR_MUTED))
                                                    .hover(|style| {
                                                        style.text_color(rgb(COLOR_FOREGROUND))
                                                    })
                                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                        cx.stop_propagation()
                                                    })
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.load_models(true, cx)
                                                    }))
                                                    .child("↻"),
                                            )
                                            .child(
                                                div()
                                                    .id("chat-model-close")
                                                    .debug_selector(|| {
                                                        "chat-model-close".to_string()
                                                    })
                                                    .px(self.z(SPACE_TIGHT))
                                                    .text_color(rgb(COLOR_MUTED))
                                                    .hover(|style| {
                                                        style.text_color(rgb(COLOR_FOREGROUND))
                                                    })
                                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                        cx.stop_propagation()
                                                    })
                                                    .on_click(cx.listener(|this, _, window, cx| {
                                                        this.close_model_menu(window, cx)
                                                    }))
                                                    .child("×"),
                                            ),
                                    ),
                            )
                            .child(
                                Input::new(&self.model_query)
                                    .w_full()
                                    .h(self.z(CHAT_ACTION_SIZE))
                                    .pt(self.z(2.5))
                                    .rounded(self.z(RADIUS_INPUT)),
                            )
                            .child(body),
                    )
                    .into_any_element(),
            )
        } else {
            None
        };
        let input_value = self.input.read(cx).value().to_string();
        let mention_panel = self.mention_panel(cx);
        let show_skills = mention_panel.is_none()
            && !self.model_menu_open
            && input_value.starts_with('/')
            && !input_value.contains(' ')
            && !input_value.contains('\n');
        let visible_entries = self.visible_slash_entries(cx);
        let selected_entry = self
            .skill_selection
            .min(visible_entries.len().saturating_sub(1));
        let skill_panel: Option<AnyElement> = if show_skills {
            let mut items: Vec<AnyElement> = visible_entries
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    let name = entry.name.clone();
                    let description = entry.description.clone();
                    let highlighted = index == selected_entry;
                    div()
                        .id(("chat-skill", index))
                        .debug_selector(move || format!("chat-skill-{index}"))
                        .flex()
                        .flex_col()
                        .gap(self.z(CHAT_SKILL_GAP))
                        .px(self.z(SPACE_NORMAL))
                        .py(self.z(SPACE_TIGHT))
                        .rounded(self.z(RADIUS_CHIP))
                        .when(highlighted, |row| row.bg(rgb(COLOR_ELEVATED)))
                        .hover(|style| style.bg(rgb(COLOR_ELEVATED)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.choose_slash_entry(index, window, cx);
                        }))
                        .child(
                            div()
                                .text_size(self.z(FONT_CARD_TITLE))
                                .text_color(rgb(if highlighted {
                                    COLOR_ACCENT
                                } else {
                                    COLOR_FOREGROUND
                                }))
                                .child(name),
                        )
                        .when(!description.is_empty(), |this| {
                            this.child(
                                div()
                                    .text_size(self.z(CHAT_STATUS_FONT))
                                    .text_color(rgb(COLOR_MUTED))
                                    .child(description),
                            )
                        })
                        .into_any_element()
                })
                .collect();
            if items.is_empty() {
                items.push(
                    div()
                        .px(self.z(SPACE_NORMAL))
                        .py(self.z(SPACE_TIGHT))
                        .text_size(self.z(CHAT_STATUS_FONT))
                        .text_color(rgb(COLOR_MUTED))
                        .child("Nenhum comando ou skill encontrado")
                        .into_any_element(),
                );
            }
            Some(
                div()
                    .id("chat-skills")
                    .debug_selector(|| "chat-skills".to_string())
                    .mx(self.z(SPACE_WIDE))
                    .mb(self.z(SPACE_TIGHT))
                    .max_h(self.z(CHAT_SKILLS_MAX_HEIGHT))
                    .overflow_y_scroll()
                    .track_scroll(&self.skill_scroll)
                    .flex()
                    .flex_col()
                    .p(self.z(SPACE_TIGHT))
                    .rounded(self.z(RADIUS_CONTROL))
                    .border_1()
                    .border_color(rgb(COLOR_BORDER))
                    .bg(rgb(COLOR_PANEL))
                    .children(items)
                    .into_any_element(),
            )
        } else {
            None
        };
        let attachment_panel = self.attachments_panel(cx);
        let context_meter = self.context_meter();

        div()
            .id("chat-view")
            .debug_selector(|| "chat-view".to_string())
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(COLOR_BASE))
            .text_size(self.z(CHAT_TEXT_SIZE))
            .text_color(rgb(COLOR_FOREGROUND))
            .when(show_model_selector, |this| {
                this.child(
                    div()
                        .id("chat-model-bar")
                        .debug_selector(|| "chat-model-bar".to_string())
                        .flex()
                        .items_center()
                        .justify_end()
                        .h(self.z(CHAT_MODEL_BAR_HEIGHT))
                        .px(self.z(SPACE_WIDE))
                        .child(
                            div()
                                .id("chat-model")
                                .debug_selector(|| "chat-model".to_string())
                                .flex()
                                .items_center()
                                .gap(self.z(SPACE_TIGHT))
                                .h(self.z(CHAT_MODEL_CHIP_HEIGHT))
                                .px(self.z(SPACE_NORMAL))
                                .rounded(self.z(RADIUS_PILL))
                                .bg(rgb(COLOR_ELEVATED))
                                .hover(|style| style.bg(rgb(COLOR_BORDER)))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation()
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if this.model_menu_open {
                                        this.close_model_menu(window, cx);
                                    } else {
                                        this.open_model_menu(window, cx);
                                    }
                                }))
                                .child(
                                    div()
                                        .text_size(self.z(CHAT_MODEL_FONT))
                                        .text_color(rgb(COLOR_MUTED))
                                        .child("modelo"),
                                )
                                .child(
                                    div()
                                        .text_size(self.z(CHAT_MODEL_NAME_FONT))
                                        .text_color(rgb(COLOR_FOREGROUND))
                                        .child(model_name),
                                )
                                .when(self.model_switching, |chip| {
                                    chip.child(
                                        div()
                                            .text_size(self.z(CHAT_MODEL_FONT))
                                            .text_color(rgb(COLOR_ACCENT))
                                            .child("trocando…"),
                                    )
                                })
                                .child(
                                    div()
                                        .text_size(self.z(CHAT_MODEL_FONT))
                                        .text_color(rgb(COLOR_MUTED))
                                        .child("▾"),
                                ),
                        ),
                )
            })
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
                    .gap(self.z(CHAT_BUBBLE_GAP))
                    .p(self.z(SPACE_WIDE))
                    .children(messages)
                    .when_some(ask_bubble, |log, bubble| log.child(bubble))
                    .when_some(status_bubble, |log, bubble| log.child(bubble)),
            )
            .when_some(mention_panel, |this, panel| this.child(panel))
            .when_some(skill_panel, |this, panel| this.child(panel))
            .when_some(attachment_panel, |this, panel| this.child(panel))
            .child(
                div()
                    .id("chat-composer")
                    .flex()
                    .items_center()
                    .gap_2()
                    .px(self.z(SPACE_WIDE))
                    .pb(self.z(SPACE_WIDE))
                    .child(
                        div()
                            .id("chat-attach")
                            .debug_selector(|| "chat-attach".to_string())
                            .w(self.z(CHAT_ACTION_SIZE))
                            .h(self.z(CHAT_ACTION_SIZE))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(self.z(RADIUS_PILL))
                            .bg(rgb(COLOR_ELEVATED))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.attach_files(window, cx)
                            }))
                            .child(
                                svg()
                                    .path("paperclip.svg")
                                    .w(self.z(CHAT_ATTACH_ICON))
                                    .h(self.z(CHAT_ATTACH_ICON))
                                    .text_color(rgb(COLOR_MUTED)),
                            ),
                    )
                    .child(
                        div()
                            .id("chat-input")
                            .debug_selector(|| "chat-input".to_string())
                            .flex_1()
                            .child(
                                Input::new(&self.input)
                                    .w_full()
                                    .h(self.z(CHAT_ACTION_SIZE))
                                    .pt(self.z(2.5))
                                    .rounded(self.z(RADIUS_INPUT))
                                    .text_size(self.z(CHAT_TEXT_SIZE))
                                    .disabled(!input_enabled),
                            ),
                    )
                    .when_some(context_meter, |row, meter| row.child(meter))
                    .child(
                        div()
                            .id("chat-send-stop")
                            .debug_selector(|| "chat-send-stop".to_string())
                            .relative()
                            .w(self.z(CHAT_ACTION_SIZE))
                            .h(self.z(CHAT_ACTION_SIZE))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(self.z(RADIUS_PILL))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if working {
                                    this.stop(cx);
                                } else {
                                    this.send_composed(window, cx);
                                }
                            }))
                            .child(
                                div()
                                    .absolute()
                                    .left(px(0.))
                                    .top(px(0.))
                                    .size_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        svg()
                                            .path("send.svg")
                                            .w(self.z(CHAT_STOP_ICON))
                                            .h(self.z(CHAT_STOP_ICON))
                                            .text_color(rgb(COLOR_BASE)),
                                    )
                                    .with_animation(
                                        ElementId::Name(
                                            format!("chat-send-icon-{working}").into(),
                                        ),
                                        Animation::new(Duration::from_millis(180)),
                                        move |layer, delta| {
                                            layer.opacity(if working { 1.0 - delta } else { delta })
                                        },
                                    ),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .left(px(0.))
                                    .top(px(0.))
                                    .size_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        svg()
                                            .path("pause.svg")
                                            .w(self.z(CHAT_STOP_ICON))
                                            .h(self.z(CHAT_STOP_ICON))
                                            .text_color(rgb(COLOR_CHAT_STOP_FG)),
                                    )
                                    .with_animation(
                                        ElementId::Name(
                                            format!("chat-pause-icon-{working}").into(),
                                        ),
                                        Animation::new(Duration::from_millis(180)),
                                        move |layer, delta| {
                                            layer.opacity(if working { delta } else { 1.0 - delta })
                                        },
                                    ),
                            )
                            .with_animation(
                                ElementId::Name(format!("chat-send-stop-bg-{working}").into()),
                                Animation::new(Duration::from_millis(180)),
                                move |button, delta| {
                                    let (from, to) = if working {
                                        (COLOR_ACCENT, COLOR_CHAT_STOP_BG)
                                    } else {
                                        (COLOR_CHAT_STOP_BG, COLOR_ACCENT)
                                    };
                                    button.bg(rgb(mix_color(from, to, delta)))
                                },
                            ),
                    )
            )
            .when_some(model_menu, |this, menu| this.child(menu))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{AskOption, AskQuestion};

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
        assert_eq!(stop_bytes(AgentKind::Omp), b"\x1b");
        assert_eq!(stop_bytes(AgentKind::Claude), b"\x03");
        assert_eq!(
            model_switch_sequence("opencode-go/hy4-flash-free"),
            vec![
                b"\x1bp".to_vec(),
                b"opencode-go/hy4-flash-free".to_vec(),
                b"\r".to_vec()
            ]
        );
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
    fn non_omp_waits_for_first_output_instead_of_abandoning() {
        let writer: crate::card::SharedPtyWriter = std::sync::Arc::new(
            std::sync::Mutex::new(Box::new(std::io::sink())),
        );
        let mut cx = test_cx();
        let (view, vcx) = cx.add_window_view(|window, cx| ChatView::new(window, cx));
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.set_sender(AgentKind::Claude, writer, cx);
                chat.enqueue("oi".into(), false, cx);
                chat.observe_terminal(Vec::new(), cx);
                assert!(chat.awaiting_response);
            });
        });
    }

    #[test]
    fn up_and_down_recall_sent_prompts_and_end_on_empty_input() {
        let writer: crate::card::SharedPtyWriter = std::sync::Arc::new(std::sync::Mutex::new(
            Box::new(std::io::sink()),
        ));
        let mut cx = test_cx();
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let captured = slot.clone();
        let (_, vcx) = cx.add_window_view(|window, cx| {
            let chat = cx.new(|cx| ChatView::new(window, cx));
            *captured.borrow_mut() = Some(chat.clone());
            gpui_component::Root::new(chat, window, cx)
        });
        let view = slot.borrow().clone().expect("the chat view");
        vcx.update(|window, cx| {
            view.update(cx, |chat, cx| {
                chat.set_sender(AgentKind::Omp, writer, cx);
                for text in ["primeira", "segunda"] {
                    chat.input.update(cx, |input, cx| {
                        input.set_value(text, window, cx)
                    });
                    chat.send_composed(window, cx);
                }
                chat.focus_input(window, cx);
            });
        });
        vcx.run_until_parked();

        vcx.simulate_keystrokes("up");
        assert_eq!(vcx.update(|_, cx| view.read(cx).value(cx)), "segunda");
        vcx.simulate_keystrokes("up");
        assert_eq!(vcx.update(|_, cx| view.read(cx).value(cx)), "primeira");
        vcx.simulate_keystrokes("up");
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).value(cx)),
            "primeira",
            "Up stops at the oldest prompt"
        );
        vcx.simulate_keystrokes("down");
        assert_eq!(vcx.update(|_, cx| view.read(cx).value(cx)), "segunda");
        vcx.simulate_keystrokes("down");
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).value(cx)),
            "",
            "the slot after the last prompt is the empty input"
        );
        vcx.simulate_keystrokes("down");
        assert_eq!(vcx.update(|_, cx| view.read(cx).value(cx)), "");
    }

    #[test]
    fn at_picker_lists_connected_mentions_and_inserts_them() {
        use crate::chat::MentionKind;
        use auro::gpui::Modifiers;

        let mentions = std::rc::Rc::new(|_: &App| {
            vec![
                CanvasMention {
                    kind: MentionKind::Terminal,
                    name: "Terminal 1".into(),
                    detail: "Terminal".into(),
                    insert: "@Terminal 1".into(),
                },
                CanvasMention {
                    kind: MentionKind::Note,
                    name: "primeira linha".into(),
                    detail: "Bloco de notas".into(),
                    insert: "@note #7".into(),
                },
            ]
        });
        let mut cx = test_cx();
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let captured = slot.clone();
        let (_, vcx) = cx.add_window_view(|window, cx| {
            let chat = cx.new(|cx| ChatView::new(window, cx));
            *captured.borrow_mut() = Some(chat.clone());
            gpui_component::Root::new(chat, window, cx)
        });
        let view = slot.borrow().clone().expect("the chat view");
        vcx.update(|window, cx| {
            view.update(cx, |chat, cx| {
                chat.set_mention_provider(mentions, cx);
                chat.input
                    .update(cx, |input, cx| input.set_value("veja @Ter", window, cx));
                chat.focus_input(window, cx);
            });
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("chat-mentions").is_some());
        let row = vcx.debug_bounds("chat-mention-0").expect("terminal mention");
        vcx.simulate_click(row.center(), Modifiers::none());
        vcx.run_until_parked();
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).value(cx)),
            "/skill:auri veja @Terminal 1 "
        );

        vcx.update(|window, cx| {
            view.update(cx, |chat, cx| {
                chat.input.update(cx, |input, cx| {
                    input.set_value("veja @pri", window, cx)
                });
                chat.focus_input(window, cx);
            });
        });
        vcx.run_until_parked();
        let row = vcx.debug_bounds("chat-mention-0").expect("note mention");
        vcx.simulate_click(row.center(), Modifiers::none());
        vcx.run_until_parked();
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).value(cx)),
            "/skill:auri veja @note #7 "
        );

        vcx.update(|window, cx| {
            view.update(cx, |chat, cx| {
                chat.input.update(cx, |input, cx| {
                    input.set_value("/skill:auri veja @pri", window, cx)
                });
                chat.focus_input(window, cx);
            });
        });
        vcx.run_until_parked();
        let row = vcx.debug_bounds("chat-mention-0").expect("note mention");
        vcx.simulate_click(row.center(), Modifiers::none());
        vcx.run_until_parked();
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).value(cx)),
            "/skill:auri veja @note #7 ",
            "an existing skill prefix must not be duplicated"
        );
    }

    #[test]
    fn slash_lists_new_and_skills_and_inserts_commands() {
        use auro::gpui::Modifiers;

        let dir = tempfile::tempdir().unwrap();
        let skill = dir.path().join("tdd");
        std::fs::create_dir(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: tdd\ndescription: Test-first\n---\n",
        )
        .unwrap();

        let mut cx = test_cx();
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let captured = slot.clone();
        let (_, vcx) = cx.add_window_view(|window, cx| {
            let chat = cx.new(|cx| ChatView::new(window, cx));
            *captured.borrow_mut() = Some(chat.clone());
            gpui_component::Root::new(chat, window, cx)
        });
        let view = slot.borrow().clone().expect("the chat view");
        vcx.update(|window, cx| {
            view.update(cx, |chat, cx| {
                chat.set_skill_roots(vec![dir.path().to_path_buf()], cx);
                chat.input
                    .update(cx, |input, cx| input.set_value("/", window, cx));
            });
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("chat-skills").is_some());
        let item = vcx.debug_bounds("chat-skill-0").expect("the new command row");
        vcx.simulate_click(item.center(), Modifiers::none());
        vcx.run_until_parked();
        assert_eq!(vcx.update(|_, cx| view.read(cx).value(cx)), "/new");

        vcx.update(|window, cx| {
            view.update(cx, |chat, cx| {
                chat.input
                    .update(cx, |input, cx| input.set_value("/tdd", window, cx));
            });
        });
        vcx.run_until_parked();
        let item = vcx.debug_bounds("chat-skill-0").expect("the tdd skill row");
        vcx.simulate_click(item.center(), Modifiers::none());
        vcx.run_until_parked();
        assert_eq!(vcx.update(|_, cx| view.read(cx).value(cx)), "/skill:tdd ");
    }


    #[test]
    fn tab_completes_the_picker_rows_and_arrows_move_the_selection() {
        let dir = tempfile::tempdir().unwrap();
        for (name, description) in
            [("alpha", "primeira"), ("beta", "segunda"), ("gamma", "terceira")]
        {
            let skill = dir.path().join(name);
            std::fs::create_dir(&skill).unwrap();
            std::fs::write(
                skill.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {description}\n---\n"),
            )
            .unwrap();
        }

        let mut cx = test_cx();
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let captured = slot.clone();
        let (_, vcx) = cx.add_window_view(|window, cx| {
            let chat = cx.new(|cx| ChatView::new(window, cx));
            *captured.borrow_mut() = Some(chat.clone());
            gpui_component::Root::new(chat, window, cx)
        });
        let view = slot.borrow().clone().expect("the chat view");
        vcx.update(|window, cx| {
            view.update(cx, |chat, cx| {
                chat.set_skill_roots(vec![dir.path().to_path_buf()], cx);
                chat.input
                    .update(cx, |input, cx| input.set_value("/", window, cx));
                chat.focus_input(window, cx);
            });
        });
        vcx.run_until_parked();

        // Tab without navigating takes the first row of the picker: /new.
        vcx.simulate_keystrokes("tab");
        let value = vcx.update(|_, cx| view.read(cx).value(cx));
        assert_eq!(value, "/new", "Tab must complete the new-session command");

        // ↓ moves the highlight past /new; Tab then inserts the first skill.
        vcx.update(|window, cx| {
            view.update(cx, |chat, cx| {
                chat.input
                    .update(cx, |input, cx| input.set_value("/", window, cx));
                chat.focus_input(window, cx);
            });
        });
        vcx.run_until_parked();
        vcx.simulate_keystrokes("down tab");
        let value = vcx.update(|_, cx| view.read(cx).value(cx));
        assert_eq!(
            value, "/skill:alpha ",
            "↓ must move to the first skill and Tab must take it"
        );

        // ↓↓ lands on beta.
        vcx.update(|window, cx| {
            view.update(cx, |chat, cx| {
                chat.input
                    .update(cx, |input, cx| input.set_value("/", window, cx));
                chat.focus_input(window, cx);
            });
        });
        vcx.run_until_parked();
        vcx.simulate_keystrokes("down down tab");
        let value = vcx.update(|_, cx| view.read(cx).value(cx));
        assert_eq!(
            value, "/skill:beta ",
            "↓↓ must move to the second skill and Tab must take it"
        );

        // ↓ and ↑ move the highlight in both directions: ↓↓ lands on beta,
        // ↑ returns to alpha.
        vcx.update(|window, cx| {
            view.update(cx, |chat, cx| {
                chat.input
                    .update(cx, |input, cx| input.set_value("/", window, cx));
                chat.focus_input(window, cx);
            });
        });
        vcx.run_until_parked();
        vcx.simulate_keystrokes("down down up tab");
        let value = vcx.update(|_, cx| view.read(cx).value(cx));
        assert_eq!(
            value, "/skill:alpha ",
            "↓/↑ must move the picker highlight in both directions"
        );
    }

    #[test]
    fn new_command_is_sent_verbatim_without_outbound_matching() {
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
        vcx.update(|window, cx| {
            view.update(cx, |chat, cx| {
                chat.set_sender(AgentKind::Omp, writer, cx);
                chat.input
                    .update(cx, |input, cx| input.set_value("/new", window, cx));
                chat.send_composed(window, cx);
            });
        });
        vcx.run_until_parked();

        let sent = String::from_utf8(written.lock().unwrap().clone()).unwrap();
        assert_eq!(sent, "/new\r", "the native slash command goes verbatim");
        assert!(
            vcx.update(|_, cx| view.read(cx).outbound.is_empty()),
            "commands are not transcript user messages"
        );
    }

    #[test]
    fn escape_pauses_a_working_turn() {
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
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let captured = slot.clone();
        let (_, vcx) = cx.add_window_view(|window, cx| {
            let chat = cx.new(|cx| ChatView::new(window, cx));
            *captured.borrow_mut() = Some(chat.clone());
            gpui_component::Root::new(chat, window, cx)
        });
        let view = slot.borrow().clone().expect("the chat view");
        vcx.update(|window, cx| {
            view.update(cx, |chat, cx| {
                chat.set_sender(AgentKind::Omp, writer, cx);
                chat.enqueue("oi".into(), false, cx);
                chat.focus_input(window, cx);
            });
        });
        vcx.run_until_parked();
        assert!(vcx.update(|_, cx| view.read(cx).is_working()));

        vcx.simulate_keystrokes("escape");
        vcx.run_until_parked();

        let bytes = written.lock().unwrap().clone();
        assert!(
            bytes.ends_with(&[0x1b]),
            "Escape in the chat must interrupt OMP"
        );
        assert!(
            !vcx.update(|_, cx| view.read(cx).is_working()),
            "Escape must leave the chat paused"
        );
    }

    #[test]
    fn enter_on_the_slash_picker_sends_the_highlighted_row() {
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
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let captured = slot.clone();
        let (_, vcx) = cx.add_window_view(|window, cx| {
            let chat = cx.new(|cx| ChatView::new(window, cx));
            *captured.borrow_mut() = Some(chat.clone());
            gpui_component::Root::new(chat, window, cx)
        });
        let view = slot.borrow().clone().expect("the chat view");
        vcx.update(|window, cx| {
            view.update(cx, |chat, cx| {
                chat.set_sender(AgentKind::Omp, writer, cx);
                chat.input
                    .update(cx, |input, cx| input.set_value("/", window, cx));
                chat.focus_input(window, cx);
            });
        });
        vcx.run_until_parked();

        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();

        let sent = String::from_utf8(written.lock().unwrap().clone()).unwrap();
        assert_eq!(
            sent, "/new\r",
            "Enter must fill and send the highlighted default row"
        );
        assert_eq!(vcx.update(|_, cx| view.read(cx).value(cx)), "");
        assert!(
            vcx.update(|_, cx| view.read(cx).outbound.is_empty()),
            "the command is not a transcript user message"
        );
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
                        images: Vec::new(),
                    },
                    cx,
                );
            });
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("chat-message-1").is_some());
        assert!(vcx.debug_bounds("chat-input").is_some());
    }

    #[test]
    fn omp_session_renders_only_final_answers() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(file, r#"{{"type":"model_change","model":"opencode-go/deepseek-v4.1-flash"}}"#).unwrap();
        writeln!(file, r#"{{"type":"message","message":{{"role":"user","content":[{{"type":"text","text":"oi"}}]}}}}"#).unwrap();
        writeln!(file, r#"{{"type":"message","message":{{"role":"assistant","stopReason":"toolUse","content":[{{"type":"text","text":"Vou procurar."}},{{"type":"toolCall","name":"prowl_find"}}]}}}}"#).unwrap();
        writeln!(file, r#"{{"type":"custom","customType":"tool_execution_start","data":{{"toolName":"prowl_find"}}}}"#).unwrap();
        writeln!(file, r#"{{"type":"message","message":{{"role":"assistant","stopReason":"stop","content":[{{"type":"text","text":"Olá, mundo."}}]}}}}"#).unwrap();
        drop(file);

        let mut cx = test_cx();
        let (view, vcx) = cx.add_window_view(|window, cx| ChatView::new(window, cx));
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.set_sender(
                    AgentKind::Omp,
                    std::sync::Arc::new(std::sync::Mutex::new(Box::new(std::io::sink()))),
                    cx,
                );
                chat.set_session(Some(path), None, cx);
            });
        });
        vcx.run_until_parked();
        let messages = vcx.update(|_, cx| view.read(cx).messages.clone());
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, ChatRole::User);
        assert_eq!(messages[0].text, "oi");
        assert_eq!(messages[1].role, ChatRole::Agent);
        assert_eq!(messages[1].text, "Olá, mundo.");
        assert!(
            vcx.update(|_, cx| view.read(cx).status.is_none()),
            "history must not leave a live status"
        );
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).current_model.clone()),
            Some("opencode-go/deepseek-v4.1-flash".into())
        );
    }

    #[test]
    fn switching_to_a_new_session_discards_the_old_chat() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.jsonl");
        let new = dir.path().join("new.jsonl");
        let mut file = std::fs::File::create(&old).unwrap();
        writeln!(file, r#"{{"type":"message","message":{{"role":"user","content":[{{"type":"text","text":"antiga"}}]}}}}"#).unwrap();
        drop(file);
        std::fs::File::create(&new).unwrap();

        let mut cx = test_cx();
        let (view, vcx) = cx.add_window_view(|window, cx| ChatView::new(window, cx));
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| chat.set_session(Some(old), None, cx));
        });
        vcx.run_until_parked();
        assert_eq!(vcx.update(|_, cx| view.read(cx).messages.len()), 1);

        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| chat.set_session(Some(new), None, cx));
        });
        vcx.run_until_parked();
        let messages = vcx.update(|_, cx| view.read(cx).messages.clone());
        assert!(
            messages.iter().all(|message| message.role == ChatRole::Info),
            "a new session must not keep the previous history"
        );
    }

    #[test]
    fn send_button_turns_into_pause_and_interrupts_omp() {
        use auro::gpui::Modifiers;
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
        vcx.run_until_parked();
        let idle = vcx.debug_bounds("chat-send-stop").expect("send button");
        assert!(
            (f32::from(idle.size.width) - 37.0).abs() < 1.5,
            "the composer action must be 37px wide"
        );

        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.set_sender(AgentKind::Omp, writer, cx);
                chat.enqueue("oi".into(), false, cx);
            });
        });
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("chat-send-stop").is_some(),
            "the same control must now act as pause"
        );

        let bounds = vcx.debug_bounds("chat-send-stop").unwrap();
        vcx.simulate_click(bounds.center(), Modifiers::none());
        vcx.run_until_parked();
        let bytes = written.lock().unwrap().clone();
        assert!(bytes.ends_with(&[0x1b]), "OMP must receive Escape");
        assert!(
            vcx.update(|_, cx| view.read(cx).status.is_none()),
            "stop must clear the live status"
        );
        assert!(
            !vcx.update(|_, cx| view.read(cx).is_working()),
            "stop must leave the agent idle"
        );
    }

    #[test]
    fn working_composer_puts_attach_left_pause_right_and_shows_wave() {
        let mut cx = test_cx();
        let (view, vcx) = cx.add_window_view(|window, cx| ChatView::new(window, cx));
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.set_status(Some(ChatStatus::Tool("read".into())), cx)
            });
        });
        vcx.run_until_parked();
        let attach = vcx.debug_bounds("chat-attach").expect("attach button");
        let input = vcx.debug_bounds("chat-input").expect("composer input");
        let stop = vcx.debug_bounds("chat-send-stop").expect("pause button");
        let bubble = vcx
            .debug_bounds("chat-status-bubble")
            .expect("status bubble");
        let attach_right = f32::from(attach.origin.x) + f32::from(attach.size.width);
        let input_left = f32::from(input.origin.x);
        let input_right = input_left + f32::from(input.size.width);
        let stop_left = f32::from(stop.origin.x);
        let bubble_bottom = f32::from(bubble.origin.y) + f32::from(bubble.size.height);
        assert!(attach_right <= input_left, "attach must sit left of the input");
        assert!(stop_left >= input_right, "pause must sit right of the input");
        assert!(
            bubble_bottom <= f32::from(input.origin.y),
            "the status must live in the chat log, not beside the input"
        );
        assert!(vcx.debug_bounds("chat-wave").is_some(), "the thinking wave must render");
        assert!(
            vcx.debug_bounds("chat-status").is_some(),
            "the status text must sit beside the wave"
        );
    }

    #[test]
    fn slash_clear_from_the_terminal_discards_previous_chat_messages() {
        let mut cx = test_cx();
        let (view, vcx) = cx.add_window_view(|window, cx| ChatView::new(window, cx));
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.push_message(
                    ChatMessage {
                        role: ChatRole::User,
                        text: "oi".into(),
                        format: ChatFormat::Plain,
                        images: Vec::new(),
                    },
                    cx,
                );
                chat.push_message(
                    ChatMessage {
                        role: ChatRole::Agent,
                        text: "Olá".into(),
                        format: ChatFormat::Plain,
                        images: Vec::new(),
                    },
                    cx,
                );
                chat.observe_terminal(vec!["$ omp".into()], cx);
                chat.observe_terminal(vec!["$ omp".into(), "/clear".into()], cx);
            });
        });
        let messages = vcx.update(|_, cx| view.read(cx).messages.clone());
        assert!(messages.is_empty(), "/clear must start an empty chat");
    }

    #[test]
    fn attach_button_is_rendered_in_the_composer() {
        let mut cx = test_cx();
        let (view, vcx) = cx.add_window_view(|window, cx| ChatView::new(window, cx));
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| chat.set_info("oi".into(), cx));
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("chat-attach").is_some());
    }

    #[test]
    fn terminal_clear_discards_previous_chat_messages() {
        let mut cx = test_cx();
        let (view, vcx) = cx.add_window_view(|window, cx| ChatView::new(window, cx));
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.push_message(
                    ChatMessage {
                        role: ChatRole::User,
                        text: "oi".into(),
                        format: ChatFormat::Plain,
                        images: Vec::new(),
                    },
                    cx,
                );
                chat.push_message(
                    ChatMessage {
                        role: ChatRole::Agent,
                        text: "Olá".into(),
                        format: ChatFormat::Plain,
                        images: Vec::new(),
                    },
                    cx,
                );
                chat.observe_terminal(
                    vec!["$ omp".into(), "oi".into(), "Olá".into(), "mais".into(), "linhas".into()],
                    cx,
                );
                chat.observe_terminal(vec!["$ omp".into()], cx);
            });
        });
        let messages = vcx.update(|_, cx| view.read(cx).messages.clone());
        assert!(messages.is_empty(), "a new prompt must start an empty chat");
    }

    #[test]
    fn current_model_moves_to_the_top_of_its_provider() {
        let models = vec![
            ModelInfo {
                selector: "p/a".into(),
                name: "A".into(),
                provider: "p".into(),
                context_window: None,
            },
            ModelInfo {
                selector: "p/b".into(),
                name: "B".into(),
                provider: "p".into(),
                context_window: None,
            },
            ModelInfo {
                selector: "p/c".into(),
                name: "C".into(),
                provider: "p".into(),
                context_window: None,
            },
        ];
        let ordered = order_models_current_first(models.iter().collect(), Some("p/c:max"));
        assert_eq!(
            ordered
                .iter()
                .map(|model| model.selector.as_str())
                .collect::<Vec<_>>(),
            vec!["p/c", "p/a", "p/b"],
            "the current model leads and the rest keeps catalog order"
        );
        let ordered = order_models_current_first(models.iter().collect(), None);
        assert_eq!(
            ordered
                .iter()
                .map(|model| model.selector.as_str())
                .collect::<Vec<_>>(),
            vec!["p/a", "p/b", "p/c"],
            "without a current model the catalog order is untouched"
        );
    }

    #[test]
    fn context_meter_appears_only_with_usage_and_window() {
        let mut cx = test_cx();
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let captured = slot.clone();
        let (_, vcx) = cx.add_window_view(|window, cx| {
            let chat = cx.new(|cx| ChatView::new(window, cx));
            *captured.borrow_mut() = Some(chat.clone());
            gpui_component::Root::new(chat, window, cx)
        });
        let view = slot.borrow().clone().expect("the chat view");
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.kind = Some(AgentKind::Omp);
                chat.models = vec![ModelInfo {
                    selector: "p/a".into(),
                    name: "A".into(),
                    provider: "p".into(),
                    context_window: Some(1_000_000),
                }];
                chat.current_model = Some("p/a".into());
                chat.context_tokens = Some(550_000);
                cx.notify();
            });
        });
        vcx.run_until_parked();
        let meter = vcx.debug_bounds("chat-context-meter").expect("context meter");
        assert!((f32::from(meter.size.width) - 41.0).abs() < 1.5);
        assert!((f32::from(meter.size.height) - 41.0).abs() < 1.5);
        let ratio = vcx.update(|_, cx| view.read(cx).context_ratio().unwrap());
        assert!((ratio - 0.55).abs() < 0.01);

        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.context_tokens = Some(0);
                cx.notify();
            });
        });
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("chat-context-meter").is_some(),
            "a known window with zero usage must still draw the empty gauge"
        );
        let ratio = vcx.update(|_, cx| view.read(cx).context_ratio().unwrap());
        assert_eq!(ratio, 0.0);

        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.models.clear();
                cx.notify();
            });
        });
        vcx.run_until_parked();
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).context_ratio()),
            None,
            "without a model window the gauge has no ratio to paint"
        );
    }

    #[test]
    fn a_session_without_usage_keeps_an_empty_meter() {
        let mut cx = test_cx();
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let captured = slot.clone();
        let (_, vcx) = cx.add_window_view(|window, cx| {
            let chat = cx.new(|cx| ChatView::new(window, cx));
            *captured.borrow_mut() = Some(chat.clone());
            gpui_component::Root::new(chat, window, cx)
        });
        let view = slot.borrow().clone().expect("the chat view");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"oi"}]}}"#,
                "\n",
                r#"{"type":"message","message":{"role":"assistant","stopReason":"aborted","content":[],"usage":{"totalTokens":0}}}"#,
                "\n"
            ),
        )
        .unwrap();
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.kind = Some(AgentKind::Omp);
                chat.default_model = Some("p/a".into());
                chat.models = vec![ModelInfo {
                    selector: "p/a".into(),
                    name: "A".into(),
                    provider: "p".into(),
                    context_window: Some(1_000_000),
                }];
                chat.set_session(Some(path.clone()), None, cx);
            });
        });
        vcx.run_until_parked();
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).context_tokens),
            Some(0),
            "an aborted-only session has zero context, not an unknown one"
        );
        assert!(
            vcx.debug_bounds("chat-context-meter").is_some(),
            "the gauge must be visible even before the first real usage"
        );
    }

    #[test]
    fn context_meter_tracks_live_usage_changes() {
        let mut cx = test_cx();
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let captured = slot.clone();
        let (_, vcx) = cx.add_window_view(|window, cx| {
            let chat = cx.new(|cx| ChatView::new(window, cx));
            *captured.borrow_mut() = Some(chat.clone());
            gpui_component::Root::new(chat, window, cx)
        });
        let view = slot.borrow().clone().expect("the chat view");
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.kind = Some(AgentKind::Omp);
                chat.models = vec![ModelInfo {
                    selector: "p/a".into(),
                    name: "A".into(),
                    provider: "p".into(),
                    context_window: Some(1_000_000),
                }];
                chat.current_model = Some("p/a".into());
                chat.context_tokens = Some(550_000);
                cx.notify();
            });
        });
        assert!(
            (vcx.update(|_, cx| view.read(cx).context_ratio().unwrap()) - 0.55).abs() < 0.001,
            "the fixture starts at 55%"
        );

        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| chat.note_context_usage(100_000, cx));
        });
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).context_ratio()),
            Some(0.1),
            "a new assistant usage must move the gauge instead of freezing it"
        );
    }

    #[test]
    fn catalog_refresh_is_needed_only_when_it_cannot_size_the_model() {
        let mut cx = test_cx();
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let captured = slot.clone();
        let (_, vcx) = cx.add_window_view(|window, cx| {
            let chat = cx.new(|cx| ChatView::new(window, cx));
            *captured.borrow_mut() = Some(chat.clone());
            gpui_component::Root::new(chat, window, cx)
        });
        let view = slot.borrow().clone().expect("the chat view");

        vcx.update(|_, cx| {
            view.update(cx, |chat, _| {
                chat.models.clear();
                chat.current_model = None;
            });
        });
        assert!(
            vcx.update(|_, cx| view.read(cx).needs_model_catalog()),
            "an empty catalog must be fetched"
        );

        vcx.update(|_, cx| {
            view.update(cx, |chat, _| {
                chat.models = vec![ModelInfo {
                    selector: "p/a".into(),
                    name: "A".into(),
                    provider: "p".into(),
                    context_window: Some(1_000_000),
                }];
                chat.current_model = Some("p/a".into());
            });
        });
        assert!(
            !vcx.update(|_, cx| view.read(cx).needs_model_catalog()),
            "a cached model with a window is enough"
        );

        vcx.update(|_, cx| {
            view.update(cx, |chat, _| chat.current_model = Some("q/other".into()));
        });
        assert!(
            vcx.update(|_, cx| view.read(cx).needs_model_catalog()),
            "a model missing from the cache must trigger a fetch"
        );

        vcx.update(|_, cx| {
            view.update(cx, |chat, _| {
                chat.models[0].context_window = None;
                chat.current_model = Some("p/a".into());
            });
        });
        assert!(
            vcx.update(|_, cx| view.read(cx).needs_model_catalog()),
            "a cached model without a window must trigger a fetch"
        );
    }

    #[test]
    fn reopening_the_chat_refreshes_the_cli_default_model() {
        let mut cx = test_cx();
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let captured = slot.clone();
        let (_, vcx) = cx.add_window_view(|window, cx| {
            let chat = cx.new(|cx| ChatView::new(window, cx));
            *captured.borrow_mut() = Some(chat.clone());
            gpui_component::Root::new(chat, window, cx)
        });
        let view = slot.borrow().clone().expect("the chat view");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "modelRoles:\n  default: q/new\n").unwrap();

        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.default_model = Some("p/old".into());
                chat.current_model = Some("p/old".into());
                chat.refresh_default_model_from(&path, cx);
            });
        });
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).current_model.clone()),
            Some("q/new".into()),
            "the model the CLI would load must replace the stale default"
        );

        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                // The session now owns q/new: a later config change must not
                // replace the model the CLI already loaded for that session.
                chat.model_from_session = true;
                chat.current_model = Some("q/new".into());
                std::fs::write(&path, "modelRoles:\n  default: t/other\n").unwrap();
                chat.refresh_default_model_from(&path, cx);
            });
        });
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).current_model.clone()),
            Some("q/new".into()),
            "a model owned by the session must survive a default refresh"
        );

        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.current_model = Some("r/session".into());
                std::fs::write(&path, "modelRoles:\n  default: s/newer\n").unwrap();
                chat.refresh_default_model_from(&path, cx);
            });
        });
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).current_model.clone()),
            Some("r/session".into()),
            "the session's own model still outranks the CLI default"
        );
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).default_model.clone()),
            Some("s/newer".into())
        );
    }

    #[test]
    fn default_model_survives_an_empty_session_and_yields_to_sessions_model() {
        let mut cx = test_cx();
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let captured = slot.clone();
        let (_, vcx) = cx.add_window_view(|window, cx| {
            let chat = cx.new(|cx| ChatView::new(window, cx));
            *captured.borrow_mut() = Some(chat.clone());
            gpui_component::Root::new(chat, window, cx)
        });
        let view = slot.borrow().clone().expect("the chat view");
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.default_model = Some("p/default".into());
                chat.restart_session(None, false, cx);
            });
        });
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).current_model.clone()),
            Some("p/default".into())
        );

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, "{\"type\":\"model_change\",\"model\":\"q/from-session\"}\n").unwrap();
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.set_session(Some(path.clone()), None, cx);
            });
        });
        vcx.run_until_parked();
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).current_model.clone()),
            Some("q/from-session".into()),
            "the session's own model outranks the CLI default"
        );
    }

    #[test]
    fn code_block_height_scales_with_zoom() {
        let mut cx = test_cx();
        let (view, vcx) = cx.add_window_view(|window, cx| ChatView::new(window, cx));
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.push_message(
                    ChatMessage {
                        role: ChatRole::Agent,
                        text: "```rust\nlet a = 1;\nlet b = 2;\nlet c = 3;\n```".into(),
                        format: ChatFormat::Markdown,
                        images: Vec::new(),
                    },
                    cx,
                );
            });
        });
        vcx.run_until_parked();
        let before = vcx
            .debug_bounds("chat-codeblock")
            .expect("the markdown code block");
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| chat.set_zoom(0.5, cx));
        });
        vcx.run_until_parked();
        let after = vcx
            .debug_bounds("chat-codeblock")
            .expect("the code block after zoom");
        assert!(
            f32::from(after.size.height) < f32::from(before.size.height) * 0.75,
            "the code block must shrink with zoom instead of keeping stale metrics: {before:?} -> {after:?}"
        );
    }

    #[test]
    fn code_block_style_changes_reach_the_text_view() {
        let small = TextViewStyle::default().code_block(
            StyleRefinement::default().text_size(px(CHAT_CODE_FONT)),
        );
        let large = TextViewStyle::default().code_block(
            StyleRefinement::default().text_size(px(CHAT_CODE_FONT * 2.0)),
        );
        assert!(
            small != large,
            "TextViewStyle must notice code_block changes, or zoom never reparses it"
        );
    }

    #[test]
    fn zoom_preserves_the_chat_scroll_position() {
        let mut cx = test_cx();
        let (view, vcx) = cx.add_window_view(|window, cx| ChatView::new(window, cx));
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                for index in 0..80 {
                    chat.push_message(
                        ChatMessage {
                            role: ChatRole::Agent,
                            text: format!("linha {index}"),
                            format: ChatFormat::Plain,
                            images: Vec::new(),
                        },
                        cx,
                    );
                }
            });
        });
        vcx.run_until_parked();
        let max = vcx.update(|_, cx| view.read(cx).scroll.max_offset().height);
        assert!(
            max > px(100.0),
            "the test chat must be tall enough to scroll: {max:?}"
        );
        let half = px(-f32::from(max) * 0.5);
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.scroll
                    .set_offset(Point::new(px(0.0), half));
                chat.set_zoom(0.5, cx);
            });
        });
        let offset = vcx.update(|_, cx| view.read(cx).scroll.offset().y);
        assert!(
            (f32::from(offset) - f32::from(half) * 0.5).abs() < 2.0,
            "zoom must keep the scroll position proportional: {half:?} -> {offset:?}"
        );
    }

    #[test]
    fn chat_sizes_follow_the_canvas_zoom() {
        let mut cx = test_cx();
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let captured = slot.clone();
        let (_, vcx) = cx.add_window_view(|window, cx| {
            let chat = cx.new(|cx| ChatView::new(window, cx));
            *captured.borrow_mut() = Some(chat.clone());
            gpui_component::Root::new(chat, window, cx)
        });
        let view = slot.borrow().clone().expect("the chat view");
        vcx.run_until_parked();
        let before = vcx.debug_bounds("chat-attach").expect("the attach button");
        let input_before = vcx.debug_bounds("chat-input").expect("the composer input");
        assert!(
            (f32::from(input_before.size.height) - 37.0).abs() < 1.5,
            "the composer input must be 37px tall: {input_before:?}"
        );
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| chat.set_zoom(0.5, cx));
        });
        vcx.run_until_parked();
        let after = vcx.debug_bounds("chat-attach").expect("the attach button");
        let input_after = vcx.debug_bounds("chat-input").expect("the composer input");
        assert!(
            (f32::from(input_before.size.height) * 0.5 - f32::from(input_after.size.height)).abs() < 1.5,
            "the input must scale with zoom like the other controls: {input_before:?} -> {input_after:?}"
        );
        assert!(
            (f32::from(before.size.width) * 0.5 - f32::from(after.size.width)).abs() < 1.5,
            "chat controls must scale with the canvas zoom: {before:?} -> {after:?}"
        );
    }

    #[test]
    fn model_selector_switches_through_the_tui_picker() {
        use auro::gpui::Modifiers;
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
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let captured = slot.clone();
        let (_, vcx) = cx.add_window_view(|window, cx| {
            let chat = cx.new(|cx| ChatView::new(window, cx));
            *captured.borrow_mut() = Some(chat.clone());
            gpui_component::Root::new(chat, window, cx)
        });
        let view = slot.borrow().clone().expect("the chat view");
        vcx.update(|window, cx| {
            view.update(cx, |chat, cx| {
                chat.set_sender(AgentKind::Omp, writer, cx);
                chat.models = vec![
                    ModelInfo {
                        selector: "commandcode/c1".into(),
                        name: "Command One".into(),
                        provider: "commandcode".into(),
                        context_window: None,
                    },
                    ModelInfo {
                        selector: "opencode-go/b".into(),
                        name: "Open B".into(),
                        provider: "opencode-go".into(),
                        context_window: None,
                    },
                    ModelInfo {
                        selector: "opencode-go/a".into(),
                        name: "Open A".into(),
                        provider: "opencode-go".into(),
                        context_window: None,
                    },
                ];
                chat.models_loaded = true;
                chat.models_loading = true;
                chat.current_model = Some("opencode-go/a".into());
                chat.open_model_menu(window, cx);
            });
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("chat-model-menu").is_some());
        assert!(vcx.debug_bounds("chat-model-providers").is_some());
        assert!(vcx.debug_bounds("chat-model-provider-1").is_some());
        assert!(
            vcx.debug_bounds("chat-model-provider-in-use-1").is_some(),
            "the provider that owns the current model wears the usage dot"
        );
        assert!(
            vcx.debug_bounds("chat-model-provider-in-use-0").is_none(),
            "the other provider must not wear the usage dot"
        );
        let badge = vcx
            .debug_bounds("chat-model-current-badge")
            .expect("the current model badge");
        let first = vcx
            .debug_bounds("chat-model-row-0")
            .expect("the first model row");
        assert!(
            badge.center().y >= first.origin.y
                && badge.center().y <= first.origin.y + first.size.height,
            "the current model must be the first row even when the catalog sorts another model ahead"
        );

        // The provider drawer switches the right-hand model list.
        let provider = vcx
            .debug_bounds("chat-model-provider-0")
            .expect("the commandcode provider");
        vcx.simulate_click(provider.center(), Modifiers::none());
        vcx.run_until_parked();
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).selected_provider.clone()),
            Some("commandcode".into())
        );

        let row = vcx
            .debug_bounds("chat-model-row-0")
            .expect("the commandcode model");
        vcx.simulate_click(row.center(), Modifiers::none());
        vcx.run_until_parked();
        vcx.executor().advance_clock(Duration::from_millis(600));
        vcx.run_until_parked();
        assert_eq!(
            written.lock().unwrap().as_slice(),
            b"\x1bpcommandcode/c1\r"
        );
        assert!(vcx.update(|_, cx| view.read(cx).model_switching));
        assert!(!vcx.update(|_, cx| view.read(cx).model_menu_open));

        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.apply_events(
                    vec![TranscriptEvent::Model("commandcode/c1".into())],
                    cx,
                )
            });
        });
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).current_model.clone()),
            Some("commandcode/c1".into())
        );
        assert!(!vcx.update(|_, cx| view.read(cx).model_switching));
    }

    #[test]
    fn ask_form_sends_the_selected_option_to_the_tui() {
        use auro::gpui::Modifiers;
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
                chat.start_ask(
                    AskRequest {
                        tool_call_id: "c1".into(),
                        questions: vec![AskQuestion {
                            name: "arquivo".into(),
                            header: "Arquivo".into(),
                            question: "Qual arquivo?".into(),
                            options: vec![
                                AskOption {
                                    label: "src/main.rs".into(),
                                    description: None,
                                },
                                AskOption {
                                    label: "src/lib.rs".into(),
                                    description: None,
                                },
                            ],
                            multi: false,
                            selected: vec![0],
                            kind: AskKind::Choice,
                            required: false,
                        }],
                    },
                    cx,
                );
            });
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("chat-ask").is_some());
        let second = vcx
            .debug_bounds("chat-ask-option-0-1")
            .expect("the second option");
        vcx.simulate_click(second.center(), Modifiers::none());
        vcx.run_until_parked();
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).ask.as_ref().unwrap().selections[0].clone()),
            vec![1]
        );
        let send = vcx
            .debug_bounds("chat-ask-send")
            .expect("the send answer button");
        vcx.simulate_click(send.center(), Modifiers::none());
        vcx.run_until_parked();
        vcx.executor().advance_clock(Duration::from_millis(250));
        vcx.run_until_parked();
        assert_eq!(written.lock().unwrap().as_slice(), b"\x1b[B \t\r");
    }

    #[test]
    fn pasting_an_image_attaches_preview_without_path() {
        use auro::gpui::{ClipboardItem, Image, ImageFormat};

        const PNG: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49,
            0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x01, 0x03,
            0x00, 0x00, 0x00, 0x25, 0xdb, 0x56, 0xca, 0x00, 0x00, 0x00, 0x03, 0x50, 0x4c,
            0x54, 0x45, 0xc9, 0xa2, 0x27, 0x00, 0x3f, 0xfb, 0x15, 0x00, 0x00, 0x00, 0x0a,
            0x49, 0x44, 0x41, 0x54, 0x08, 0xd7, 0x63, 0x60, 0x00, 0x00, 0x00, 0x02, 0x00,
            0x01, 0xe2, 0x21, 0xbc, 0x33, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44,
            0xae, 0x42, 0x60, 0x82,
        ];

        let mut cx = test_cx();
        cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(
            ImageFormat::Png,
            PNG.to_vec(),
        )));
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let captured = slot.clone();
        let (_, vcx) = cx.add_window_view(|window, cx| {
            let chat = cx.new(|cx| ChatView::new(window, cx));
            *captured.borrow_mut() = Some(chat.clone());
            gpui_component::Root::new(chat, window, cx)
        });
        let view = slot.borrow().clone().expect("the chat view");
        vcx.run_until_parked();
        let input = vcx.debug_bounds("chat-input").expect("the composer");
        vcx.simulate_click(input.center(), auro::gpui::Modifiers::none());
        vcx.run_until_parked();
        vcx.simulate_keystrokes("ctrl-v");
        vcx.run_until_parked();

        let (attachments, value) = vcx.update(|_, cx| {
            let chat = view.read(cx);
            (chat.attachments.clone(), chat.value(cx))
        });
        assert_eq!(attachments.len(), 1, "the clipboard image became an attachment");
        assert!(attachments[0].image);
        assert!(attachments[0].path.exists(), "the pasted bytes were written");
        assert!(!value.contains(&attachments[0].path.to_string_lossy().to_string()));
        let _ = std::fs::remove_file(&attachments[0].path);
    }

    #[test]
    fn other_answer_is_typed_and_sent_from_the_ask_form() {
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
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let captured = slot.clone();
        let (_, vcx) = cx.add_window_view(|window, cx| {
            let chat = cx.new(|cx| ChatView::new(window, cx));
            *captured.borrow_mut() = Some(chat.clone());
            gpui_component::Root::new(chat, window, cx)
        });
        let view = slot.borrow().clone().expect("the chat view");
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.set_sender(AgentKind::Omp, writer, cx);
                chat.start_ask(
                    AskRequest {
                        tool_call_id: "c1".into(),
                        questions: vec![AskQuestion {
                            name: "arquivo".into(),
                            header: "Arquivo".into(),
                            question: "Qual arquivo?".into(),
                            options: vec![
                                AskOption {
                                    label: "src/main.rs".into(),
                                    description: None,
                                },
                                AskOption {
                                    label: "src/lib.rs".into(),
                                    description: None,
                                },
                            ],
                            multi: false,
                            selected: vec![0],
                            kind: AskKind::Choice,
                            required: false,
                        }],
                    },
                    cx,
                );
            });
        });
        vcx.run_until_parked();
        let other = vcx
            .debug_bounds("chat-ask-other-0")
            .expect("the other answer row");
        vcx.simulate_click(other.center(), auro::gpui::Modifiers::none());
        vcx.run_until_parked();
        vcx.simulate_input("minha resposta");
        vcx.run_until_parked();
        assert_eq!(
            vcx.update(|_, cx| view.read(cx).ask.as_ref().unwrap().other[0].clone()),
            "minha resposta"
        );
        assert!(
            vcx.update(|_, cx| view.read(cx).ask.as_ref().unwrap().selections[0].is_empty()),
            "typing Other clears the single-choice option"
        );
        let send = vcx
            .debug_bounds("chat-ask-send")
            .expect("the send answer button");
        vcx.simulate_click(send.center(), auro::gpui::Modifiers::none());
        vcx.run_until_parked();
        vcx.executor().advance_clock(Duration::from_millis(250));
        vcx.run_until_parked();
        assert_eq!(
            written.lock().unwrap().as_slice(),
            b"\x1b[B\x1b[Bminha resposta\t\r"
        );
    }

    #[test]
    fn attachment_preview_renders_and_removes_tile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notas finais.txt");
        std::fs::write(&path, "conteudo").unwrap();

        let mut cx = test_cx();
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let captured = slot.clone();
        let (_, vcx) = cx.add_window_view(|window, cx| {
            let chat = cx.new(|cx| ChatView::new(window, cx));
            *captured.borrow_mut() = Some(chat.clone());
            gpui_component::Root::new(chat, window, cx)
        });
        let view = slot.borrow().clone().expect("the chat view");
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| chat.attach_paths(vec![path.clone()], cx));
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("chat-attachments").is_some());
        assert!(vcx.debug_bounds("chat-attachment-0").is_some());
        let value = vcx.update(|_, cx| view.read(cx).value(cx));
        assert!(!value.contains("notas finais.txt"), "the path must stay out of the composer");

        let remove = vcx
            .debug_bounds("chat-attachment-remove-0")
            .expect("the remove control");
        vcx.simulate_click(remove.center(), auro::gpui::Modifiers::none());
        vcx.run_until_parked();
        assert!(vcx.update(|_, cx| view.read(cx).attachments.is_empty()));
        let value = vcx.update(|_, cx| view.read(cx).value(cx));
        assert!(value.is_empty(), "removing the tile leaves the composer untouched");
    }

    #[test]
    fn attachment_paths_are_sent_without_entering_the_composer() {
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

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("imagem.png");
        let written = Arc::new(Mutex::new(Vec::new()));
        let writer: crate::card::SharedPtyWriter =
            Arc::new(Mutex::new(Box::new(Writer(written.clone()))));
        let mut cx = test_cx();
        let (view, vcx) = cx.add_window_view(|window, cx| ChatView::new(window, cx));
        vcx.update(|window, cx| {
            view.update(cx, |chat, cx| {
                chat.set_sender(AgentKind::Omp, writer, cx);
                chat.attachments.push(Attachment { path: path.clone(), image: true });
                chat.input.update(cx, |input, cx| input.set_value("oi", window, cx));
            });
        });
        vcx.run_until_parked();
        let send = vcx.debug_bounds("chat-send-stop").expect("send button");
        vcx.simulate_click(send.center(), auro::gpui::Modifiers::none());
        vcx.run_until_parked();
        let bytes = written.lock().unwrap().clone();
        let sent = String::from_utf8_lossy(&bytes);
        assert!(sent.contains("oi"), "typed text is sent: {sent}");
        assert!(sent.contains("imagem.png"), "attachment path is sent: {sent}");
        assert!(vcx.update(|_, cx| view.read(cx).value(cx).is_empty()));
        assert!(vcx.update(|_, cx| view.read(cx).attachments.is_empty()));
        let messages = vcx.update(|_, cx| view.read(cx).messages.clone());
        let user = messages.last().expect("user message");
        assert_eq!(user.text, "oi");
        assert!(!user.text.contains("imagem.png"));
        assert_eq!(user.images, vec![path]);
        assert!(vcx.debug_bounds("chat-message-image-0-0").is_some());
    }

    #[test]
    fn answered_ask_from_history_does_not_leave_a_form() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        let ask = serde_json::json!({
            "type": "message",
            "message": {
                "role": "assistant",
                "stopReason": "toolUse",
                "content": [{
                    "type": "toolCall",
                    "id": "c1",
                    "name": "ask",
                    "arguments": {
                        "html": "<form><fieldset><legend>Arquivo</legend><p>Qual?</p><label><input type=\"radio\" name=\"arquivo\" value=\"main\"> src/main.rs</label><label><input type=\"radio\" name=\"arquivo\" value=\"lib\"> src/lib.rs</label></fieldset></form>"
                    }
                }]
            }
        });
        writeln!(file, "{ask}").unwrap();
        let answered = serde_json::json!({
            "type": "message",
            "message": {
                "role": "toolResult",
                "toolCallId": "c1",
                "toolName": "ask",
                "content": [{"type": "text", "text": "User answers"}],
                "details": {"answers": [{"name": "arquivo", "question": "Qual?", "answer": "src/main.rs"}]}
            }
        });
        writeln!(file, "{answered}").unwrap();
        drop(file);

        let mut cx = test_cx();
        let (view, vcx) = cx.add_window_view(|window, cx| ChatView::new(window, cx));
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| {
                chat.set_session(Some(path), None, cx);
            });
        });
        vcx.run_until_parked();
        let messages = vcx.update(|_, cx| view.read(cx).messages.clone());
        assert!(vcx.update(|_, cx| view.read(cx).ask.is_none()));
        assert!(messages.iter().any(|message| {
            message.role == ChatRole::User && message.text == "Qual?: src/main.rs"
        }));
    }

    #[test]
    fn clicking_composer_focuses_input() {
        use auro::gpui::{AppContext, Focusable, Modifiers};
        use std::cell::RefCell;
        use std::rc::Rc;

        let mut cx = test_cx();
        let slot = Rc::new(RefCell::new(None));
        let captured = slot.clone();
        let (_, vcx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| ChatView::new(window, cx));
            *captured.borrow_mut() = Some(view.clone());
            gpui_component::Root::new(view, window, cx)
        });
        let view = slot.borrow().clone().expect("the chat view");
        vcx.update(|_, cx| {
            view.update(cx, |chat, cx| chat.set_info("oi".into(), cx));
        });
        vcx.run_until_parked();
        let bounds = vcx.debug_bounds("chat-input").expect("the composer");
        vcx.simulate_click(bounds.center(), Modifiers::none());
        vcx.run_until_parked();
        vcx.update(|window, cx| {
            let handle = view.read(cx).input.read(cx).focus_handle(cx);
            assert!(handle.is_focused(window), "composer input must take focus");
        });
    }
}
