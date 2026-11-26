//! Pure chat-transcript types and OMP JSONL parsing.

use serde_json::Value;

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
}
