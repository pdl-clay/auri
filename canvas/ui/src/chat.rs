//! Pure chat-transcript types and OMP JSONL parsing.

use serde_json::Value;
use std::collections::VecDeque;
use std::path::PathBuf;

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

    pub fn confirm(&mut self, text: &str) -> bool {
        let wanted = normalize_message(text);
        let Some(index) = self
            .pending
            .iter()
            .position(|pending| normalize_message(&pending.text) == wanted)
        else {
            return false;
        };
        self.pending.remove(index);
        true
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentLiveness {
    Agent,
    Idle,
    Unknown,
}

/// Gate for chat sends: a live foreground agent, not exited, non-empty text.
pub fn can_send(liveness: AgentLiveness, exited: bool, text: &str) -> bool {
    !exited && !text.trim().is_empty() && liveness == AgentLiveness::Agent
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
    Prompt,
    Exited,
}

impl ChatStatus {
    pub fn label(&self) -> String {
        match self {
            Self::Thinking => "pensando…".into(),
            Self::Tool(name) => format!("chamando ferramenta {name}"),
            Self::Writing => "escrevendo…".into(),
            Self::Prompt => "responda no terminal".into(),
            Self::Exited => "encerrado".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranscriptEvent {
    User(String),
    AgentFinal(String),
    Status(ChatStatus),
}

/// Parse one JSONL line from an OMP session. Unknown or malformed lines are
/// ignored so a long session can still be read after a schema change.
pub fn parse_transcript_line(line: &str) -> Option<TranscriptEvent> {
    let value: Value = serde_json::from_str(line).ok()?;
    match value.get("type")?.as_str()? {
        "message" => parse_message(value.get("message")?),
        "custom" => parse_custom(&value),
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
        "assistant" => parse_assistant(message.get("content")),
        "toolResult" => Some(TranscriptEvent::Status(ChatStatus::Writing)),
        _ => None,
    }
}

fn parse_assistant(content: Option<&Value>) -> Option<TranscriptEvent> {
    let blocks = content?.as_array()?;
    let tool = blocks.iter().rev().find_map(|block| {
        if block.get("type")?.as_str()? != "toolCall" {
            return None;
        }
        block
            .get("name")
            .or_else(|| block.get("toolName"))
            .and_then(Value::as_str)
    });
    if let Some(name) = tool {
        return Some(TranscriptEvent::Status(if name == "ask" {
            ChatStatus::Prompt
        } else {
            ChatStatus::Tool(name.to_owned())
        }));
    }
    let text = text_blocks(content);
    if !text.is_empty() {
        return Some(TranscriptEvent::AgentFinal(text));
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
                .get("data")?
                .get("toolName")?
                .as_str()?;
            Some(TranscriptEvent::Status(if name == "ask" {
                ChatStatus::Prompt
            } else {
                ChatStatus::Tool(name.to_owned())
            }))
        }
        "session_exit" => Some(TranscriptEvent::Status(ChatStatus::Exited)),
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn user_text_becomes_a_user_bubble() {
        let line = r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"oi"}]}}"#;
        assert_eq!(
            parse_transcript_line(line),
            Some(TranscriptEvent::User("oi".into()))
        );
    }

    #[test]
    fn tool_result_marks_writing_and_exit_marks_exited() {
        let result = r#"{"type":"message","message":{"role":"toolResult","content":[{"type":"text","text":"ok"}]}}"#;
        let exit = r#"{"type":"custom","customType":"session_exit","data":{}}"#;
        assert_eq!(
            parse_transcript_line(result),
            Some(TranscriptEvent::Status(ChatStatus::Writing))
        );
        assert_eq!(
            parse_transcript_line(exit),
            Some(TranscriptEvent::Status(ChatStatus::Exited))
        );
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
    fn unknown_liveness_disables_chat_sending() {
        assert!(!can_send(AgentLiveness::Unknown, false, "oi"));
        assert!(!can_send(AgentLiveness::Agent, true, "oi"));
        assert!(!can_send(AgentLiveness::Agent, false, "   "));
        assert!(can_send(AgentLiveness::Agent, false, "oi"));
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
