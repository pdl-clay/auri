//! Pure chat types: OMP transcript parsing, queue and terminal fallback.

use serde_json::Value;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Agent,
    Info,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatFormat {
    Markdown,
    Plain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub text: String,
    pub format: ChatFormat,
    /// Images shown in the bubble; the wire text may still carry their paths.
    pub images: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingMessage {
    pub text: String,
    pub is_skill: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChatQueue {
    pending: VecDeque<PendingMessage>,
}

impl ChatQueue {
    pub fn enqueue(&mut self, text: String, is_skill: bool) {
        self.pending.push_back(PendingMessage {
            text: text.trim().to_owned(),
            is_skill,
        });
    }

    pub fn front(&self) -> Option<&PendingMessage> {
        self.pending.front()
    }


    pub fn remove_front(&mut self) {
        self.pending.pop_front();
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }
}

pub fn normalize_message(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Paths inserted into the composer, quoted when a terminal would split them.
pub fn attached_paths_text(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| {
            let text = path.to_string_lossy();
            if text.contains(char::is_whitespace) {
                format!("\"{}\"", text.replace('"', "\\\""))
            } else {
                text.into_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Text added to the terminal viewport since the previous snapshot.
pub fn snapshot_response(previous: &[String], current: &[String]) -> Option<String> {
    let common = previous
        .iter()
        .zip(current)
        .take_while(|(before, after)| before == after)
        .count();
    if common == current.len() {
        return None;
    }
    let text = current[common..].join("\n").trim().to_owned();
    (!text.is_empty()).then_some(text)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelInfo {
    pub selector: String,
    pub name: String,
    pub provider: String,
    pub context_window: Option<u64>,
}

/// What a `@` mention points at on the canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MentionKind {
    Terminal,
    Note,
    Agent,
}

/// One connected endpoint offered by the chat's `@` picker. `insert` is the
/// exact text placed in the composer, already in the form `auri list` uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanvasMention {
    pub kind: MentionKind,
    pub name: String,
    pub detail: String,
    pub insert: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskKind {
    Choice,
    Text,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AskOption {
    pub label: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AskQuestion {
    pub name: String,
    pub header: String,
    pub question: String,
    pub options: Vec<AskOption>,
    pub multi: bool,
    /// Options checked by default; the TUI starts its cursor on the first one.
    pub selected: Vec<usize>,
    pub kind: AskKind,
    /// A free-text field marked `required` must not be submitted empty.
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AskRequest {
    pub tool_call_id: String,
    pub questions: Vec<AskQuestion>,
}

#[derive(Default)]
struct AskBuilder {
    name: String,
    header: String,
    question: String,
    options: Vec<AskOption>,
    multi: bool,
    selected: Vec<usize>,
    kind: Option<AskKind>,
    required: bool,
}

#[derive(Default)]
struct PendingOption {
    value: String,
    checked: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AskCapture {
    None,
    Legend,
    Question,
}

fn clean_ask_text(text: &str) -> String {
    let decoded = text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parse the `html` argument of the ask tool. OMP constrains it to one form of
/// plain semantic HTML, so a small tolerant tokenizer is enough; unknown tags
/// are ignored and malformed input degrades to no questions.
pub fn parse_ask_html(html: &str) -> Vec<AskQuestion> {
    let mut questions = Vec::new();
    let mut builder: Option<AskBuilder> = None;
    let mut capture = AskCapture::None;
    let mut in_label = false;
    let mut label = String::new();
    let mut pending: Option<PendingOption> = None;
    let mut rest = html;

    fn builder_start(builder: &mut Option<AskBuilder>, capture: &mut AskCapture) {
        if builder.is_none() {
            *builder = Some(AskBuilder::default());
        }
        *capture = AskCapture::None;
    }

    fn flush_option(
        builder: &mut Option<AskBuilder>,
        pending: &mut Option<PendingOption>,
        label: &mut String,
    ) {
        let Some(option) = pending.take() else {
            label.clear();
            return;
        };
        let Some(builder) = builder.as_mut() else {
            label.clear();
            return;
        };
        let mut text = clean_ask_text(label);
        if text.is_empty() {
            text = clean_ask_text(&option.value);
        }
        if text.is_empty() {
            text = format!("Opção {}", builder.options.len() + 1);
        }
        let index = builder.options.len();
        builder.options.push(AskOption {
            label: text,
            description: None,
        });
        if option.checked {
            builder.selected.push(index);
        }
        label.clear();
    }

    fn flush_question(builder: &mut Option<AskBuilder>, questions: &mut Vec<AskQuestion>) {
        let Some(mut builder) = builder.take() else {
            return;
        };
        builder.header = clean_ask_text(&builder.header);
        builder.question = clean_ask_text(&builder.question);
        if builder.question.is_empty() {
            builder.question = builder.header.clone();
        }
        let kind = builder.kind.unwrap_or(AskKind::Choice);
        if kind == AskKind::Choice && builder.options.is_empty() {
            return;
        }
        if builder.name.is_empty() {
            builder.name = format!("question-{}", questions.len() + 1);
        }
        builder.selected.sort_unstable();
        builder.selected.dedup();
        questions.push(AskQuestion {
            name: builder.name,
            header: builder.header,
            question: builder.question,
            options: builder.options,
            multi: builder.multi,
            selected: builder.selected,
            kind,
            required: builder.required,
        });
    }

    fn append_text(
        text: &str,
        builder: &mut Option<AskBuilder>,
        capture: AskCapture,
        in_label: bool,
        label: &mut String,
        pending: bool,
    ) {
        if text.is_empty() {
            return;
        }
        if capture == AskCapture::Legend || capture == AskCapture::Question {
            if let Some(builder) = builder.as_mut() {
                if capture == AskCapture::Legend {
                    builder.header.push_str(text);
                } else {
                    builder.question.push_str(text);
                }
            }
            return;
        }
        if in_label || pending {
            label.push_str(text);
        }
    }

    while let Some(lt) = rest.find('<') {
        let text = &rest[..lt];
        append_text(
            text,
            &mut builder,
            capture,
            in_label,
            &mut label,
            pending.is_some(),
        );
        let Some(gt_rel) = rest[lt..].find('>') else {
            break;
        };
        let tag_end = lt + gt_rel;
        let raw_tag = rest[lt + 1..tag_end].trim();
        let closing = raw_tag.starts_with('/');
        let self_closing = raw_tag.ends_with('/');
        let inner = raw_tag
            .trim_start_matches('/')
            .trim_end_matches('/')
            .trim();
        let (name, attr_body) = match inner.split_once(char::is_whitespace) {
            Some((name, attrs)) => (name.to_ascii_lowercase(), attrs),
            None => (inner.to_ascii_lowercase(), ""),
        };
        match name.as_str() {
            "fieldset" if !closing => {
                flush_option(&mut builder, &mut pending, &mut label);
                flush_question(&mut builder, &mut questions);
                builder_start(&mut builder, &mut capture);
            }
            "fieldset" => {
                flush_option(&mut builder, &mut pending, &mut label);
                flush_question(&mut builder, &mut questions);
            }
            "legend" if !closing => {
                builder_start(&mut builder, &mut capture);
                capture = AskCapture::Legend;
            }
            "legend" => capture = AskCapture::None,
            "p" if !closing => {
                builder_start(&mut builder, &mut capture);
                capture = AskCapture::Question;
            }
            "p" => capture = AskCapture::None,
            "label" if !closing => {
                in_label = true;
                label.clear();
            }
            "label" => {
                flush_option(&mut builder, &mut pending, &mut label);
                in_label = false;
            }
            "br" => append_text(
                " ",
                &mut builder,
                capture,
                in_label,
                &mut label,
                pending.is_some(),
            ),
            "input" if !closing && !self_closing => {
                let attrs = parse_ask_attributes(attr_body);
                let kind = attrs
                    .get("type")
                    .map(|value| value.to_ascii_lowercase())
                    .unwrap_or_default();
                match kind.as_str() {
                    "radio" | "checkbox" => {
                        flush_option(&mut builder, &mut pending, &mut label);
                        builder_start(&mut builder, &mut capture);
                        if let Some(builder) = builder.as_mut() {
                            if builder.name.is_empty() {
                                if let Some(name) = attrs.get("name") {
                                    builder.name = name.clone();
                                }
                            }
                            if kind == "checkbox" {
                                builder.multi = true;
                            }
                            builder.kind = Some(AskKind::Choice);
                        }
                        pending = Some(PendingOption {
                            value: attrs.get("value").cloned().unwrap_or_default(),
                            checked: attrs.contains_key("checked"),
                        });
                    }
                    "hidden" | "submit" | "button" | "reset" | "image" | "file" => {}
                    _ => {
                        builder_start(&mut builder, &mut capture);
                        if let Some(builder) = builder.as_mut() {
                            builder.kind = Some(AskKind::Text);
                            builder.required |= attrs.contains_key("required");
                            if builder.question.is_empty() && !label.is_empty() {
                                builder.question = std::mem::take(&mut label);
                            }
                            if builder.name.is_empty() {
                                if let Some(name) = attrs.get("name") {
                                    builder.name = name.clone();
                                }
                            }
                            if builder.question.is_empty() {
                                if let Some(placeholder) = attrs.get("placeholder") {
                                    builder.question = placeholder.clone();
                                }
                            }
                        }
                    }
                }
            }
            "textarea" if !closing => {
                builder_start(&mut builder, &mut capture);
                if let Some(builder) = builder.as_mut() {
                    builder.kind = Some(AskKind::Text);
                    let attrs = parse_ask_attributes(attr_body);
                    builder.required |= attrs.contains_key("required");
                    if builder.question.is_empty() && !label.is_empty() {
                        builder.question = std::mem::take(&mut label);
                    }
                    if builder.name.is_empty() {
                        if let Some(name) = attrs.get("name") {
                            builder.name = name.clone();
                        }
                    }
                }
            }
            "option" | "select" => {
                builder_start(&mut builder, &mut capture);
            }
            _ => {}
        }
        rest = &rest[tag_end + 1..];
    }
    append_text(
        rest,
        &mut builder,
        capture,
        in_label,
        &mut label,
        pending.is_some(),
    );
    flush_option(&mut builder, &mut pending, &mut label);
    flush_question(&mut builder, &mut questions);
    questions
}

fn parse_ask_attributes(body: &str) -> std::collections::HashMap<String, String> {
    let mut attrs = std::collections::HashMap::new();
    let mut rest = body.trim();
    while !rest.is_empty() {
        let name_end = rest
            .char_indices()
            .find(|(_, ch)| ch.is_whitespace() || *ch == '=')
            .map(|(index, _)| index)
            .unwrap_or(rest.len());
        let name = rest[..name_end].trim().to_ascii_lowercase();
        rest = rest[name_end..].trim_start();
        let mut value = String::new();
        if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start();
            if let Some(quote) = after.chars().next().filter(|ch| *ch == '"' || *ch == '\'') {
                let after = &after[quote.len_utf8()..];
                if let Some(end) = after.find(quote) {
                    value = after[..end].to_owned();
                    rest = &after[end + quote.len_utf8()..];
                } else {
                    value = after.to_owned();
                    rest = "";
                }
            } else {
                let end = after
                    .char_indices()
                    .find(|(_, ch)| ch.is_whitespace())
                    .map(|(index, _)| index)
                    .unwrap_or(after.len());
                value = after[..end].to_owned();
                rest = &after[end..];
            }
        }
        if !name.is_empty() {
            attrs.insert(name, value);
        }
        rest = rest.trim_start();
    }
    attrs
}

fn parse_legacy_questions(questions: &[Value]) -> Vec<AskQuestion> {
    questions
        .iter()
        .enumerate()
        .filter_map(|(index, question)| {
            let text = question.get("question")?.as_str()?.to_owned();
            let header = question
                .get("header")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let name = question
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("question-{}", index + 1));
            let options: Vec<AskOption> = question
                .get("options")?
                .as_array()?
                .iter()
                .filter_map(|option| {
                    Some(AskOption {
                        label: option
                            .get("label")
                            .or_else(|| option.get("value"))?
                            .as_str()?
                            .to_owned(),
                        description: option
                            .get("description")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    })
                })
                .collect();
            Some(AskQuestion {
                name,
                header,
                question: text,
                multi: question.get("multi").and_then(Value::as_bool).unwrap_or(false),
                selected: Vec::new(),
                options,
                kind: AskKind::Choice,
                required: false,
            })
        })
        .collect()
}

fn parse_ask_call(block: &Value) -> Option<AskRequest> {
    if block.get("type")?.as_str()? != "toolCall" {
        return None;
    }
    let name = block
        .get("name")
        .or_else(|| block.get("toolName"))
        .and_then(Value::as_str)?;
    if name != "ask" {
        return None;
    }
    let arguments = block.get("arguments")?;
    let questions = if let Some(html) = arguments.get("html").and_then(Value::as_str) {
        parse_ask_html(html)
    } else if let Some(questions) = arguments.get("questions").and_then(Value::as_array) {
        parse_legacy_questions(questions)
    } else {
        Vec::new()
    };
    if questions.is_empty() {
        return None;
    }
    Some(AskRequest {
        tool_call_id: block
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("ask")
            .to_owned(),
        questions,
    })
}

fn format_ask_answers(message: &Value) -> Option<String> {
    let is_error = message.get("isError").and_then(Value::as_bool).unwrap_or(false);
    if is_error {
        return None;
    }
    if let Some(answers) = message
        .get("details")
        .and_then(|details| details.get("answers"))
        .and_then(Value::as_array)
    {
        let lines: Vec<String> = answers
            .iter()
            .filter_map(|answer| {
                let text = answer.get("answer")?.as_str()?.trim();
                if text.is_empty() {
                    return None;
                }
                let question = answer
                    .get("question")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                Some(if question.is_empty() {
                    text.to_owned()
                } else {
                    format!("{question}: {text}")
                })
            })
            .collect();
        if !lines.is_empty() {
            return Some(lines.join("\n"));
        }
    }
    let content = message.get("content")?;
    let text = content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| {
            (block.get("type")?.as_str()? == "text")
                .then(|| block.get("text")?.as_str())
                .flatten()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let text = text.trim();
    if text.is_empty() || text == "Ask input was cancelled" {
        return None;
    }
    Some(text.to_owned())
}

fn ask_navigation(from: usize, to: usize) -> Vec<u8> {
    let down = b"\x1b[B";
    let up = b"\x1b[A";
    if to >= from {
        down.repeat(to - from)
    } else {
        up.repeat(from - to)
    }
}

/// Bytes to drive OMP's ask dialog, one chunk per question plus a final
/// submit. The dialog moves its highlight with arrows, selects with Space,
/// advances fields with Tab and begins the submit row with Enter. A choice
/// question also has an implicit `Other (type your own)` row after its
/// options: navigating there and typing selects it, so a custom answer must
/// not be preceded by Space.
pub fn ask_sequences(
    request: &AskRequest,
    selections: &[Vec<usize>],
    custom: &[String],
) -> Vec<Vec<u8>> {
    let mut sequences = Vec::new();
    for (index, question) in request.questions.iter().enumerate() {
        let answer = single_line_answer(
            custom.get(index).map(String::as_str).unwrap_or(""),
        );
        if question.kind == AskKind::Text {
            if question.required && answer.trim().is_empty() {
                return Vec::new();
            }
            let mut bytes = answer.into_bytes();
            bytes.push(b'\t');
            sequences.push(bytes);
            continue;
        }
        let chosen = selections.get(index).cloned().unwrap_or_default();
        if chosen.is_empty() && answer.trim().is_empty() {
            return Vec::new();
        }
        let last = question.options.len().saturating_sub(1);
        let initial = question
            .selected
            .first()
            .copied()
            .unwrap_or(0)
            .min(last);
        if !answer.trim().is_empty() {
            let target = question.options.len();
            let mut bytes = ask_navigation(initial, target);
            bytes.extend_from_slice(answer.as_bytes());
            bytes.push(b'\t');
            sequences.push(bytes);
            continue;
        }
        if question.multi {
            let mut bytes = Vec::new();
            let mut cursor = initial;
            for option in 0..question.options.len() {
                let desired = chosen.contains(&option);
                let current = question.selected.contains(&option);
                if desired != current {
                    bytes.extend(ask_navigation(cursor, option));
                    bytes.push(b' ');
                    cursor = option;
                }
            }
            bytes.push(b'\t');
            sequences.push(bytes);
        } else {
            let target = chosen[0].min(last);
            let mut bytes = ask_navigation(initial, target);
            bytes.push(b' ');
            bytes.push(b'\t');
            sequences.push(bytes);
        }
    }
    sequences.push(vec![b'\r']);
    sequences
}

/// The TUI's Other editor is single line: a newline would submit and a tab
/// would leave the field before the answer arrived.
fn single_line_answer(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '\n' | '\r' | '\t' => ' ',
            other => other,
        })
        .filter(|character| !character.is_control())
        .collect()
}

/// Parse `omp models --json` into a flat, searchable catalog.
pub fn parse_models_json(body: &str) -> Vec<ModelInfo> {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    let Some(models) = value.get("models").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut models: Vec<ModelInfo> = models
        .iter()
        .filter_map(|model| {
            let selector = model.get("selector")?.as_str()?.trim();
            if selector.is_empty() {
                return None;
            }
            Some(ModelInfo {
                selector: selector.to_owned(),
                name: model
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(selector)
                    .to_owned(),
                provider: model
                    .get("provider")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                context_window: model.get("contextWindow").and_then(Value::as_u64),
            })
        })
        .collect();
    models.sort_by(|a, b| {
        a.provider
            .to_lowercase()
            .cmp(&b.provider.to_lowercase())
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    models.dedup_by(|a, b| a.selector == b.selector);
    models
}

/// Where the chat keeps data that is safe to recreate: the model catalog and
/// pasted image files. Lives under the XDG cache, never in the workspace
/// config, so deleting it only costs one cold fetch.
pub fn chat_cache_dir() -> PathBuf {
    chat_cache_dir_from(std::env::var_os("XDG_CACHE_HOME"), std::env::var_os("HOME"))
}

fn chat_cache_dir_from(xdg: Option<std::ffi::OsString>, home: Option<std::ffi::OsString>) -> PathBuf {
    let base = xdg
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            home.filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".cache"))
        })
        .unwrap_or_else(std::env::temp_dir);
    base.join("auro-canvas")
}

pub fn model_cache_path() -> PathBuf {
    chat_cache_dir().join("models.json")
}

/// The cached catalog is the raw `omp models --json` body. A damaged cache is
/// treated exactly like a missing one.
pub fn read_model_cache(path: &Path) -> Vec<ModelInfo> {
    std::fs::read_to_string(path)
        .map(|body| parse_models_json(&body))
        .unwrap_or_default()
}

/// Persist the exact CLI body so the next launch draws the menu immediately.
pub fn write_model_cache(path: &Path, body: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, body)
}

/// A fresh fetch replaces the cache only when it actually brought a model the
/// cache did not know; an unchanged CLI answer leaves the cached rows alone.
pub fn catalog_with_new_models(
    cached: &[ModelInfo],
    fresh: Vec<ModelInfo>,
) -> Option<Vec<ModelInfo>> {
    if fresh.is_empty() {
        return None;
    }
    let has_new = cached.is_empty() || fresh.iter().any(|model| !cached.contains(model));
    has_new.then_some(fresh)
}

pub fn filter_models<'a>(models: &'a [ModelInfo], query: &str) -> Vec<&'a ModelInfo> {
    let query = query.trim().to_lowercase();
    models
        .iter()
        .filter(|model| {
            query.is_empty()
                || model.name.to_lowercase().contains(&query)
                || model.provider.to_lowercase().contains(&query)
                || model.selector.to_lowercase().contains(&query)
        })
        .collect()
}

pub fn provider_label(provider: &str) -> String {
    match provider {
        "opencode-go" => "Open Code".into(),
        "commandcode" => "Command Code".into(),
        "9router" => "9Router".into(),
        "inception" => "Inception".into(),
        other => other.replace('-', " "),
    }
}

/// The `model_change` line is the authoritative current model; assistant
/// messages carry provider/model too but can repeat on every turn.
pub fn parse_model_change(value: &Value) -> Option<String> {
    let model = value.get("model")?.as_str()?.trim();
    (!model.is_empty()).then(|| model.to_owned())
}

pub fn discover_skills(roots: &[PathBuf]) -> Vec<SkillInfo> {
    let mut skills = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path().join("SKILL.md");
            if let Some(skill) = parse_skill(&path) {
                skills.push(skill);
            }
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

fn parse_skill(path: &std::path::Path) -> Option<SkillInfo> {
    let body = std::fs::read_to_string(path).ok()?;
    let mut lines = body.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut name = None;
    let mut description = String::new();
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'');
        match key.trim() {
            "name" => name = Some(value.to_owned()),
            "description" if !value.is_empty() && value != ">-" && value != "|" => {
                description = value.to_owned();
            }
            _ => {}
        }
    }
    Some(SkillInfo {
        name: name?,
        description,
        path: path.to_path_buf(),
    })
}

pub fn filter_skills<'a>(skills: &'a [SkillInfo], query: &str) -> Vec<&'a SkillInfo> {
    let query = query.to_lowercase();
    skills
        .iter()
        .filter(|skill| {
            query.is_empty()
                || skill.name.to_lowercase().contains(&query)
                || skill.description.to_lowercase().contains(&query)
        })
        .collect()
}

pub fn skill_command(name: &str) -> String {
    format!("/skill:{name} ")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatStatus {
    Thinking,
    Tool(String),
    Writing,
}

impl ChatStatus {
    pub fn label(&self) -> String {
        match self {
            Self::Thinking => "pensando…".into(),
            Self::Tool(name) if name == "ask" => "responda no terminal".into(),
            Self::Tool(name) => format!("usando {name}…"),
            Self::Writing => "escrevendo…".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranscriptEvent {
    User(String),
    AgentFinal {
        text: String,
        context_tokens: Option<u64>,
    },
    Model(String),
    Ask(AskRequest),
    AskAnswered { tool_call_id: String, text: String },
    Status(ChatStatus),
    TurnEnd,
}

/// Parse one JSONL line from an OMP session. Unknown or malformed lines are
/// ignored so a long session can still be read after a schema change.
pub fn parse_transcript_line(line: &str) -> Option<TranscriptEvent> {
    let value: Value = serde_json::from_str(line).ok()?;
    match value.get("type")?.as_str()? {
        "message" => parse_message(value.get("message")?),
        "custom" => parse_custom(&value),
        "model_change" => parse_model_change(&value).map(TranscriptEvent::Model),
        _ => None,
    }
}

fn parse_message(message: &Value) -> Option<TranscriptEvent> {
    let role = message.get("role")?.as_str()?;
    match role {
        "user" => {
            let text = text_blocks(message.get("content"));
            (!text.is_empty()).then_some(TranscriptEvent::User(text))
        }
        "assistant" => parse_assistant(message),
        "toolResult" => {
            if message.get("toolName").and_then(Value::as_str) == Some("ask") {
                let tool_call_id = message
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .unwrap_or("ask")
                    .to_owned();
                let text = format_ask_answers(message).unwrap_or_default();
                Some(TranscriptEvent::AskAnswered {
                    tool_call_id,
                    text,
                })
            } else {
                Some(TranscriptEvent::Status(ChatStatus::Writing))
            }
        }
        _ => None,
    }
}

fn parse_assistant(message: &Value) -> Option<TranscriptEvent> {
    let content = message.get("content");
    let blocks = content?.as_array()?;
    if let Some(ask) = blocks.iter().find_map(parse_ask_call) {
        return Some(TranscriptEvent::Ask(ask));
    }
    let stop_reason = message.get("stopReason").and_then(Value::as_str);
    let has_tool_call = blocks
        .iter()
        .any(|block| block.get("type").and_then(Value::as_str) == Some("toolCall"));
    let text = text_blocks(content);
    if has_tool_call {
        // Text in a tool-call message is intermediate narration ("vou ler o
        // arquivo"), never the answer. A turn that ends here was aborted.
        if stop_reason.is_some_and(|reason| reason != "toolUse") {
            return Some(TranscriptEvent::TurnEnd);
        }
        let name = blocks
            .iter()
            .rev()
            .find_map(|block| {
                (block.get("type")?.as_str()? == "toolCall")
                    .then(|| block.get("name").or_else(|| block.get("toolName")))
                    .flatten()
                    .and_then(Value::as_str)
            })
            .unwrap_or("ferramenta");
        return Some(TranscriptEvent::Status(ChatStatus::Tool(name.to_owned())));
    }
    if !text.is_empty() && stop_reason != Some("aborted") {
        let context_tokens = message.get("usage").and_then(context_usage_tokens);
        return Some(TranscriptEvent::AgentFinal {
            text,
            context_tokens,
        });
    }
    if stop_reason.is_some_and(|reason| reason != "toolUse") {
        return Some(TranscriptEvent::TurnEnd);
    }
    let thinking = blocks
        .iter()
        .any(|block| block.get("type").and_then(Value::as_str) == Some("thinking"));
    thinking.then_some(TranscriptEvent::Status(ChatStatus::Thinking))
}

fn parse_custom(value: &Value) -> Option<TranscriptEvent> {
    match value.get("customType")?.as_str()? {
        "tool_execution_start" => {
            let name = value
                .get("data")
                .and_then(|data| data.get("toolName"))
                .and_then(Value::as_str)
                .unwrap_or("ferramenta");
            Some(TranscriptEvent::Status(ChatStatus::Tool(name.to_owned())))
        }
        "session_exit" => Some(TranscriptEvent::TurnEnd),
        _ => None,
    }
}

fn text_blocks(content: Option<&Value>) -> String {
    let Some(content) = content else {
        return String::new();
    };
    if let Some(text) = content.as_str() {
        return text.trim().to_owned();
    }
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| {
            (block.get("type")?.as_str()? == "text")
                .then(|| block.get("text")?.as_str())
                .flatten()
        })
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// OMP keeps the model remembered for new sessions in its agent config.
pub fn omp_config_path() -> PathBuf {
    std::env::var_os("PI_CODING_AGENT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            home.join(".omp").join("agent")
        })
        .join("config.yml")
}

pub fn read_omp_default_model() -> Option<String> {
    read_omp_default_model_from(&omp_config_path())
}

pub fn read_omp_default_model_from(path: &Path) -> Option<String> {
    parse_omp_default_model(&std::fs::read_to_string(path).ok()?)
}

/// The `default` role in `modelRoles`, including its thinking suffix.
pub fn parse_omp_default_model(body: &str) -> Option<String> {
    let mut in_roles = false;
    for line in body.lines() {
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if indent == 0 {
            in_roles = trimmed.trim_end_matches('{').trim() == "modelRoles:";
            continue;
        }
        if !in_roles {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        if key.trim() != "default" {
            continue;
        }
        let value = value
            .split('#')
            .next()
            .unwrap_or(value)
            .trim()
            .trim_matches(|c| c == '"' || c == '\'');
        if !value.is_empty() {
            return Some(value.to_owned());
        }
    }
    None
}

/// The context footprint OMP itself reports: `contextTokens` when the provider
/// supplies it, otherwise the prompt tokens (input plus cache reads/writes).
/// `totalTokens` includes the reply, which is not part of the next request's
/// context, so it is only a fallback for older transcripts.
fn context_usage_tokens(usage: &Value) -> Option<u64> {
    let tokens = usage
        .get("contextTokens")
        .and_then(Value::as_u64)
        .or_else(|| {
            let input = usage.get("input")?.as_u64()?;
            let cache_read = usage.get("cacheRead").and_then(Value::as_u64).unwrap_or(0);
            let cache_write = usage.get("cacheWrite").and_then(Value::as_u64).unwrap_or(0);
            Some(input + cache_read + cache_write)
        })
        .or_else(|| usage.get("totalTokens").and_then(Value::as_u64))?;
    (tokens > 0).then_some(tokens)
}

/// Cumulative usage lives on every assistant reply, including tool turns.
pub fn assistant_context_tokens(line: &str) -> Option<u64> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("type")?.as_str()? != "message" {
        return None;
    }
    let message = value.get("message")?;
    if message.get("role")?.as_str()? != "assistant" {
        return None;
    }
    context_usage_tokens(message.get("usage")?)
}

/// Directory where omp keys its terminal breadcrumbs.
pub fn omp_breadcrumb_dir() -> PathBuf {
    std::env::var_os("PI_CODING_AGENT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            home.join(".omp").join("agent")
        })
        .join("terminal-sessions")
}

/// omp keys terminal breadcrumbs by TTY path: `/dev/pts/3` becomes `pts-3`.
pub fn terminal_session_key(tty: &Path) -> Option<String> {
    Some(tty.to_str()?.strip_prefix("/dev/")?.replace('/', "-"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OmpBreadcrumb {
    pub cwd: PathBuf,
    pub session: PathBuf,
}

/// Breadcrumb lines: session cwd, JSONL path, then optional metadata.
pub fn read_omp_breadcrumb(path: &Path) -> Option<OmpBreadcrumb> {
    let body = std::fs::read_to_string(path).ok()?;
    let mut lines = body.lines();
    let cwd = lines.next()?.trim();
    let saved = lines.next()?.trim();
    if cwd.is_empty() || saved.is_empty() {
        return None;
    }
    let session = PathBuf::from(saved);
    let session = if session.is_absolute() {
        session
    } else {
        Path::new(cwd).join(session)
    };
    Some(OmpBreadcrumb {
        cwd: PathBuf::from(cwd),
        session,
    })
}

/// Breadcrumb file for a PTY child. `omp` rewrites it when it opens or clears
/// a session, so watching it catches `/clear` and a new session file.
pub fn omp_breadcrumb_for_pid(pid: u32) -> Option<PathBuf> {
    let tty = std::fs::read_link(format!("/proc/{pid}/fd/0")).ok()?;
    let key = terminal_session_key(&tty)?;
    Some(omp_breadcrumb_dir().join(key))
}

/// A fresh prompt after `/clear` replaces the whole screen with a short one.
/// Ordinary scrolling shifts lines instead, so it must not clear the chat.
pub fn screen_cleared(previous: &[String], current: &[String]) -> bool {
    if previous.is_empty() || current.is_empty() {
        return false;
    }
    let before = previous.iter().filter(|line| !line.trim().is_empty()).count();
    let after = current.iter().filter(|line| !line.trim().is_empty()).count();
    if after * 2 >= before {
        return false;
    }
    // Either a brand-new prompt or the same prompt with the old conversation
    // collapsed below it. Scrolling shifts lines instead of truncating them.
    let common = previous
        .iter()
        .zip(current)
        .take_while(|(before, after)| before == after)
        .count();
    common == 0 || common == current.len()
}

fn is_chrome_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('╭')
        || trimmed.starts_with('╰')
        || trimmed.starts_with('│')
        || trimmed.starts_with('├')
        || trimmed.starts_with('─')
        || trimmed.starts_with('✔')
        || trimmed.starts_with('✘')
        || trimmed == "❯"
        || trimmed.starts_with("❯ ")
        || trimmed.chars().any(|char| ('\u{2800}'..='\u{28FF}').contains(&char))
        || trimmed.contains(" > ◕ ")
        || trimmed.contains("💬  CHAT:")
}

fn is_thinking_line(line: &str) -> bool {
    line.starts_with(' ') && !line.starts_with("  ") && !line.trim().is_empty()
}

/// A finished tool call leaves a timing or result line. Seeing one below the
/// candidate block means the turn is still in the tool loop, not an answer.
fn is_tool_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.contains('⏱')
        || trimmed.starts_with('✔')
        || trimmed.starts_with('✘')
}

/// Keep only the model's answer from the terminal viewport.
///
/// The answer is the last unindented block after the user's echo. Intermediate
/// narration ("vou ler o arquivo"), tool boxes, results and the status bar are
/// above or around it and must never become the bubble. Anchoring on the echo
/// also survives a scrolled screen, whose prefix no longer matches.
pub fn final_answer_text(
    previous: &[String],
    current: &[String],
    sent: Option<&str>,
) -> Option<String> {
    let echo = sent.map(normalize_message);
    let is_echo = |line: &str| {
        echo.as_ref().is_some_and(|sent| {
            let normalized = normalize_message(line);
            normalized == *sent || normalized.ends_with(sent)
        })
    };
    let anchor = current.iter().rposition(|line| is_echo(line));
    let window: &[String] = match anchor {
        Some(index) => &current[index + 1..],
        None => current,
    };
    if anchor.is_none() && snapshot_response(previous, current).is_none() {
        return None;
    }
    let mut start = None;
    for (index, raw) in window.iter().enumerate().rev() {
        let line = raw.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if is_tool_line(line) {
            // A tool ran after the last unindented text: the model has not
            // answered yet, it is only narrating between calls.
            return None;
        }
        if is_chrome_line(line) || is_thinking_line(line) {
            continue;
        }
        start = Some(index);
        break;
    }
    let start = start?;
    let mut answer: Vec<&str> = Vec::new();
    for raw in window[..=start].iter().rev() {
        let line = raw.trim_end();
        if is_chrome_line(line) || is_thinking_line(line) || is_echo(line) {
            break;
        }
        if line.trim().is_empty() && answer.is_empty() {
            continue;
        }
        answer.push(line);
    }
    answer.reverse();
    let answer = answer.join("\n").trim().to_owned();
    (!answer.is_empty()).then_some(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_answer_drops_thinking_tools_and_status() {
        let previous = vec!["❯".into()];
        let current = vec![
            "❯".into(),
            "oi".into(),
            " Vou investigar o output do terminal.".into(),
            "╭─── Buscando no projeto ─────╮".into(),
            "│ prowl search chat nativo   │".into(),
            "╰─── ⏱ 3.6s ─────────────────╯".into(),
            " ✔ Read .".into(),
            "Resposta principal do modelo.".into(),
            " ⠙ 18s > ◕ Hy4 flash Free".into(),
            "╰─".into(),
        ];
        assert_eq!(
            final_answer_text(&previous, &current, Some("oi")),
            Some("Resposta principal do modelo.".into())
        );
    }

    #[test]
    fn final_answer_without_tools_ignores_thinking_lines() {
        let previous = vec!["❯".into()];
        let current = vec![
            "❯".into(),
            "oi".into(),
            " Pensando em português.".into(),
            "Resposta final.".into(),
        ];
        assert_eq!(
            final_answer_text(&previous, &current, Some("oi")),
            Some("Resposta final.".into())
        );
    }

    #[test]
    fn final_answer_is_the_last_block_not_the_intermediate_narration() {
        let previous = vec!["❯".into()];
        let current = vec![
            "❯".into(),
            "revise o parser".into(),
            "Vou olhar o parser agora.".into(),
            "╭─── lendo parser.rs ─────╮".into(),
            "│ read parser.rs         │".into(),
            "╰─── ⏱ 0.4s ─────────────╯".into(),
            " ✔ Read parser.rs".into(),
            "Encontrei um caso de linha truncada.".into(),
            " ⠙ 18s > ◕ Hy4 flash Free".into(),
            "╰─".into(),
        ];
        assert_eq!(
            final_answer_text(&previous, &current, Some("revise o parser")),
            Some("Encontrei um caso de linha truncada.".into())
        );
    }

    #[test]
    fn final_answer_survives_a_scrolled_screen() {
        let previous = vec![
            "resposta antiga".into(),
            "mais linhas".into(),
            "e mais".into(),
        ];
        let current = vec![
            "resposta antiga".into(),
            "❯".into(),
            "nova pergunta".into(),
            "Vou olhar o parser agora.".into(),
            "╭─── lendo parser.rs ─────╮".into(),
            "│ read parser.rs         │".into(),
            "╰─── ⏱ 0.4s ─────────────╯".into(),
            " ✔ Read parser.rs".into(),
            "Resposta nova do modelo.".into(),
            " ⠙ 18s > ◕ Hy4 flash Free".into(),
            "╰─".into(),
        ];
        assert_eq!(
            final_answer_text(&previous, &current, Some("nova pergunta")),
            Some("Resposta nova do modelo.".into())
        );
    }

    #[test]
    fn final_answer_is_none_while_only_tools_ran() {
        let previous = vec!["❯".into()];
        let current = vec![
            "❯".into(),
            "revise o parser".into(),
            "Vou olhar o parser agora.".into(),
            "╭─── lendo parser.rs ─────╮".into(),
            "│ read parser.rs         │".into(),
            "╰─── ⏱ 0.4s ─────────────╯".into(),
            " ✔ Read parser.rs".into(),
            " ⠙ 18s > ◕ Hy4 flash Free".into(),
            "╰─".into(),
        ];
        assert_eq!(
            final_answer_text(&previous, &current, Some("revise o parser")),
            None
        );
    }

    #[test]
    fn context_usage_excludes_the_reply_tokens() {
        let line = r#"{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"ok"}],"usage":{"input":1000,"output":500,"cacheRead":9000,"cacheWrite":50,"totalTokens":10550}}}"#;
        assert_eq!(assistant_context_tokens(line), Some(10050));

        let with_override = r#"{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"ok"}],"usage":{"contextTokens":1234,"input":1000,"output":500,"cacheRead":9000,"totalTokens":10550}}}"#;
        assert_eq!(assistant_context_tokens(with_override), Some(1234));
    }

    #[test]
    fn final_assistant_message_becomes_one_agent_bubble() {
        let line = r#"{"type":"message","message":{"role":"assistant","content":[{"type":"thinking","thinking":"x"},{"type":"text","text":"Olá"},{"type":"text","text":"mundo"}],"usage":{"totalTokens":550000}}}"#;
        assert_eq!(
            parse_transcript_line(line),
            Some(TranscriptEvent::AgentFinal {
                text: "Olá\n\nmundo".into(),
                context_tokens: Some(550000),
            })
        );
    }

    #[test]
    fn intermediate_narration_with_a_tool_call_is_not_a_bubble() {
        let line = r#"{"type":"message","message":{"role":"assistant","stopReason":"toolUse","content":[{"type":"text","text":"Vou ler o arquivo."},{"type":"toolCall","name":"read_file"}]}}"#;
        assert_eq!(
            parse_transcript_line(line),
            Some(TranscriptEvent::Status(ChatStatus::Tool("read_file".into())))
        );
    }

    #[test]
    fn tool_execution_start_names_the_running_tool() {
        let line = r#"{"type":"custom","customType":"tool_execution_start","data":{"toolName":"prowl_find"}}"#;
        assert_eq!(
            parse_transcript_line(line),
            Some(TranscriptEvent::Status(ChatStatus::Tool("prowl_find".into())))
        );
        assert_eq!(ChatStatus::Tool("read".into()).label(), "usando read…");
        assert_eq!(
            ChatStatus::Tool("ask".into()).label(),
            "responda no terminal"
        );
    }

    #[test]
    fn aborted_tool_turn_ends_without_a_bubble() {
        let line = r#"{"type":"message","message":{"role":"assistant","stopReason":"aborted","content":[{"type":"thinking","thinking":"x"},{"type":"toolCall","name":"read_file"}]}}"#;
        assert_eq!(parse_transcript_line(line), Some(TranscriptEvent::TurnEnd));
    }

    #[test]
    fn aborted_partial_text_is_not_a_final_answer() {
        let line = r#"{"type":"message","message":{"role":"assistant","stopReason":"aborted","content":[{"type":"text","text":"Vou explic"}]}}"#;
        assert_eq!(parse_transcript_line(line), Some(TranscriptEvent::TurnEnd));
    }

    #[test]
    fn user_text_becomes_a_user_bubble() {
        let line = r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"oi"}]}}"#;
        assert_eq!(
            parse_transcript_line(line),
            Some(TranscriptEvent::User("oi".into()))
        );
    }

    #[test]
    fn unknown_and_truncated_records_are_ignored() {
        assert_eq!(parse_transcript_line(""), None);
        assert_eq!(parse_transcript_line("{not json"), None);
        assert_eq!(parse_transcript_line(r#"{"type":"model_usage"}"#), None);
    }

    #[test]
    fn omp_breadcrumb_resolves_the_session_path() {
        let dir = tempfile::tempdir().unwrap();
        let breadcrumb = dir.path().join("pts-9");
        std::fs::write(
            &breadcrumb,
            "/tmp/project\n/tmp/project/session.jsonl\ncwdstat 1 2\n",
        )
        .unwrap();
        let parsed = read_omp_breadcrumb(&breadcrumb).unwrap();
        assert_eq!(parsed.cwd, PathBuf::from("/tmp/project"));
        assert_eq!(parsed.session, PathBuf::from("/tmp/project/session.jsonl"));
        assert_eq!(
            terminal_session_key(Path::new("/dev/pts/9")),
            Some("pts-9".into())
        );
    }

    #[test]
    fn ask_html_becomes_questions_and_options() {
        let html = r#"<form>
          <fieldset>
            <legend>Arquivo</legend>
            <p>Qual arquivo devo editar?</p>
            <label><input type="radio" name="arquivo" value="main"> src/main.rs</label>
            <label><input type="radio" name="arquivo" value="lib" checked> src/lib.rs</label>
          </fieldset>
        </form>"#;
        let questions = parse_ask_html(html);
        assert_eq!(questions.len(), 1);
        let question = &questions[0];
        assert_eq!(question.name, "arquivo");
        assert_eq!(question.header, "Arquivo");
        assert_eq!(question.question, "Qual arquivo devo editar?");
        assert_eq!(question.options.len(), 2);
        assert_eq!(question.options[0].label, "src/main.rs");
        assert_eq!(question.options[1].label, "src/lib.rs");
        assert_eq!(question.selected, vec![1]);
        assert!(!question.multi);
        assert_eq!(question.kind, AskKind::Choice);
    }

    #[test]
    fn ask_tool_call_and_answer_are_parsed() {
        let call = r#"{"type":"message","message":{"role":"assistant","stopReason":"toolUse","content":[{"type":"toolCall","id":"c1","name":"ask","arguments":{"i":"perguntar","html":"<form><fieldset><legend>Arquivo</legend><p>Qual?</p><label><input type=\"radio\" name=\"arquivo\" value=\"main\"> src/main.rs</label><label><input type=\"radio\" name=\"arquivo\" value=\"lib\"> src/lib.rs</label></fieldset></form>"}}]}}"#;
        assert!(matches!(
            parse_transcript_line(call),
            Some(TranscriptEvent::Ask(AskRequest { tool_call_id, questions })) if tool_call_id == "c1" && questions.len() == 1
        ));
        let result = r#"{"type":"message","message":{"role":"toolResult","toolCallId":"c1","toolName":"ask","content":[{"type":"text","text":"User answers"}],"details":{"answers":[{"name":"arquivo","question":"Qual?","answer":"src/main.rs"}]}}}"#;
        assert_eq!(
            parse_transcript_line(result),
            Some(TranscriptEvent::AskAnswered {
                tool_call_id: "c1".into(),
                text: "Qual?: src/main.rs".into()
            })
        );
    }

    #[test]
    fn ask_sequences_follow_the_tui_cursor() {
        let single = AskRequest {
            tool_call_id: "c1".into(),
            questions: vec![AskQuestion {
                name: "arquivo".into(),
                header: "Arquivo".into(),
                question: "Qual?".into(),
                options: vec![
                    AskOption {
                        label: "main".into(),
                        description: None,
                    },
                    AskOption {
                        label: "lib".into(),
                        description: None,
                    },
                ],
                multi: false,
                selected: vec![0],
                kind: AskKind::Choice,
                required: false,
            }],
        };
        assert_eq!(
            ask_sequences(&single, &[vec![1]], &[]),
            vec![b"\x1b[B \t".to_vec(), b"\r".to_vec()]
        );
        assert_eq!(
            ask_sequences(&single, &[vec![0]], &[]),
            vec![b" \t".to_vec(), b"\r".to_vec()]
        );

        let multi = AskRequest {
            tool_call_id: "c2".into(),
            questions: vec![AskQuestion {
                name: "auth".into(),
                header: "Auth".into(),
                question: "Quais?".into(),
                options: vec![
                    AskOption {
                        label: "password".into(),
                        description: None,
                    },
                    AskOption {
                        label: "oauth".into(),
                        description: None,
                    },
                    AskOption {
                        label: "magic".into(),
                        description: None,
                    },
                ],
                multi: true,
                selected: vec![0],
                kind: AskKind::Choice,
                required: false,
            }],
        };
        // Toggle option 2, advance to Submit, then confirm.
        assert_eq!(
            ask_sequences(&multi, &[vec![0, 2]], &[]),
            vec![b"\x1b[B\x1b[B \t".to_vec(), b"\r".to_vec()]
        );
    }

    #[test]
    fn ask_sequences_send_a_custom_other_answer() {
        let request = AskRequest {
            tool_call_id: "c1".into(),
            questions: vec![AskQuestion {
                name: "arquivo".into(),
                header: "Arquivo".into(),
                question: "Qual?".into(),
                options: vec![
                    AskOption {
                        label: "main".into(),
                        description: None,
                    },
                    AskOption {
                        label: "lib".into(),
                        description: None,
                    },
                ],
                multi: false,
                selected: vec![],
                kind: AskKind::Choice,
                required: false,
            }],
        };
        // The TUI appends its own Other row after the two options. Typing
        // there selects it; a Space would insert a literal space instead.
        assert_eq!(
            ask_sequences(&request, &[vec![]], &["minha resposta".into()]),
            vec![b"\x1b[B\x1b[Bminha resposta\t".to_vec(), b"\r".to_vec()]
        );
        // A newline would submit the TUI's single-line editor early.
        assert_eq!(
            ask_sequences(&request, &[vec![]], &["linha 1\nlinha 2".into()]),
            vec![b"\x1b[B\x1b[Blinha 1 linha 2\t".to_vec(), b"\r".to_vec()]
        );
    }

    #[test]
    fn ask_sequences_type_a_free_text_question() {
        let request = AskRequest {
            tool_call_id: "c1".into(),
            questions: vec![AskQuestion {
                name: "nome".into(),
                header: "Nome".into(),
                question: "Seu nome".into(),
                options: Vec::new(),
                multi: false,
                selected: Vec::new(),
                kind: AskKind::Text,
                required: true,
            }],
        };
        assert_eq!(
            ask_sequences(&request, &[vec![]], &["PDL".into()]),
            vec![b"PDL\t".to_vec(), b"\r".to_vec()]
        );
        assert!(ask_sequences(&request, &[vec![]], &["  ".into()]).is_empty());
    }

    #[test]
    fn ask_html_keeps_a_text_field_label_and_required() {
        let html = r#"<form><fieldset><legend>Nome</legend>
            <label>Seu nome ou apelido <input type="text" name="quem" placeholder="ex.: PDL" required></label>
        </fieldset></form>"#;
        let question = &parse_ask_html(html)[0];
        assert_eq!(question.kind, AskKind::Text);
        assert_eq!(question.question, "Seu nome ou apelido");
        assert_eq!(question.name, "quem");
        assert!(question.required);
    }

    #[test]
    fn model_catalog_and_filter_come_from_the_cli_shape() {
        let body = r#"{"models":[
            {"selector":"p/b","name":"B","provider":"p"},
            {"selector":"p/a","name":"A","provider":"p","contextWindow":128000},
            {"selector":"p/a","name":"A duplicate","provider":"p"}
        ]}"#;
        let models = parse_models_json(body);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].selector, "p/a");
        assert_eq!(models[0].context_window, Some(128000));
        assert_eq!(filter_models(&models, "b")[0].selector, "p/b");
        assert!(filter_models(&models, "zzz").is_empty());
    }

    #[test]
    fn omp_default_model_is_read_from_model_roles() {
        let body = "composer:\n  shape: band\nmodelRoles:\n  default: \"opencode-go/hy4-flash-free:xhigh\"\n  reasoning: high\ndefaultThinkingLevel: auto\n";
        assert_eq!(
            parse_omp_default_model(body).as_deref(),
            Some("opencode-go/hy4-flash-free:xhigh")
        );
        assert_eq!(parse_omp_default_model("modelRoles:\n  fast: p/a\n"), None);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yml");
        std::fs::write(&path, body).unwrap();
        assert_eq!(
            read_omp_default_model_from(&path).as_deref(),
            Some("opencode-go/hy4-flash-free:xhigh")
        );
    }

    #[test]
    fn catalog_replaces_the_cache_only_when_a_model_is_new() {
        let body = r#"{"models":[{"selector":"p/a","name":"A","provider":"p"}]}"#;
        let cached = parse_models_json(body);
        assert!(catalog_with_new_models(&cached, parse_models_json(body)).is_none());
        assert!(catalog_with_new_models(&cached, Vec::new()).is_none());
        let newer = r#"{"models":[
            {"selector":"p/a","name":"A","provider":"p"},
            {"selector":"p/b","name":"B","provider":"p"}
        ]}"#;
        assert!(catalog_with_new_models(&cached, parse_models_json(newer)).is_some());
        assert!(catalog_with_new_models(&[], parse_models_json(body)).is_some());
    }

    #[test]
    fn model_cache_round_trips_the_cli_body() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache/models.json");
        let body = r#"{"models":[{"selector":"p/a","name":"A","provider":"p"}]}"#;
        write_model_cache(&path, body).unwrap();
        assert_eq!(read_model_cache(&path)[0].selector, "p/a");
        std::fs::write(&path, "not json").unwrap();
        assert!(read_model_cache(&path).is_empty());
    }

    #[test]
    fn chat_cache_dir_honours_xdg_cache_home() {
        assert_eq!(
            chat_cache_dir_from(
                Some(std::ffi::OsString::from("/tmp/xdg-cache")),
                Some(std::ffi::OsString::from("/home/user")),
            ),
            PathBuf::from("/tmp/xdg-cache/auro-canvas")
        );
        assert_eq!(
            chat_cache_dir_from(None, Some(std::ffi::OsString::from("/home/user"))),
            PathBuf::from("/home/user/.cache/auro-canvas")
        );
    }

    #[test]
    fn model_change_updates_the_session_model() {
        let line = r#"{"type":"model_change","model":"opencode-go/deepseek-v4.1-flash"}"#;
        assert_eq!(
            parse_transcript_line(line),
            Some(TranscriptEvent::Model(
                "opencode-go/deepseek-v4.1-flash".into()
            ))
        );
    }

    #[test]
    fn attached_paths_text_quotes_paths_with_spaces() {
        let paths = vec![
            PathBuf::from("/tmp/relatorio final.pdf"),
            PathBuf::from("/tmp/foto.png"),
        ];
        assert_eq!(
            attached_paths_text(&paths),
            "\"/tmp/relatorio final.pdf\" /tmp/foto.png"
        );
    }
    #[test]
    fn screen_cleared_recognizes_a_short_new_banner() {
        let before: Vec<String> = (0..12).map(|index| format!("linha {index}")).collect();
        let after = vec![
            "sessão nova".into(),
            "modelo: x".into(),
            "$ omp".into(),
        ];
        assert!(screen_cleared(&before, &after));
    }

    #[test]
    fn screen_cleared_recognizes_a_fresh_prompt() {
        let before = vec![
            "$ omp".into(),
            "oi".into(),
            "pensando…".into(),
            "resposta longa".into(),
            "mais uma linha".into(),
        ];
        let after = vec!["$ omp".into()];
        assert!(screen_cleared(&before, &after));
    }

    #[test]
    fn screen_cleared_ignores_ordinary_scrolling() {
        let before = vec!["a".into(), "b".into(), "c".into(), "d".into()];
        let after = vec!["b".into(), "c".into(), "d".into(), "e".into()];
        assert!(!screen_cleared(&before, &after));
    }


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
    fn skill_messages_are_marked_on_enqueue() {
        let mut q = ChatQueue::default();
        q.enqueue("/skill:tdd".into(), true);
        assert!(q.front().unwrap().is_skill);
    }

    #[test]
    fn snapshot_response_returns_only_new_terminal_lines() {
        let previous = vec!["$ omp".into(), "pronto".into()];
        let current = vec!["$ omp".into(), "pronto".into(), "resposta".into()];
        assert_eq!(
            snapshot_response(&previous, &current),
            Some("resposta".into())
        );
    }

    #[test]
    fn snapshot_response_is_none_when_output_did_not_change() {
        let lines = vec!["a".into(), "b".into()];
        assert_eq!(snapshot_response(&lines, &lines), None);
    }


    #[test]
    fn discovers_skill_name_and_description_from_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let skill = dir.path().join("tdd");
        std::fs::create_dir(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: tdd\ndescription: Test-first\n---\n",
        )
        .unwrap();
        let found = discover_skills(&[dir.path().to_path_buf()]);
        assert_eq!(found[0].name, "tdd");
        assert_eq!(found[0].description, "Test-first");
        assert_eq!(skill_command(&found[0].name), "/skill:tdd ");
    }

    #[test]
    fn filter_matches_name_and_description() {
        let skills = vec![
            SkillInfo {
                name: "tdd".into(),
                description: "Test-first".into(),
                path: PathBuf::from("/skills/tdd/SKILL.md"),
            },
            SkillInfo {
                name: "docs".into(),
                description: "Write documentation".into(),
                path: PathBuf::from("/skills/docs/SKILL.md"),
            },
        ];
        assert_eq!(filter_skills(&skills, "TEST").len(), 1);
        assert_eq!(filter_skills(&skills, "tdd")[0].name, "tdd");
        assert_eq!(filter_skills(&skills, "").len(), 2);
        assert!(filter_skills(&skills, "not-found").is_empty());
    }
}
