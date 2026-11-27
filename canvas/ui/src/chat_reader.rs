//! Incremental reader for OMP session JSONL files.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use crate::chat::{TranscriptEvent, assistant_context_tokens, parse_transcript_line};

pub struct TranscriptTail {
    pub path: PathBuf,
    pub offset: u64,
    pub partial: String,
    pub context_tokens: Option<u64>,
    pub reset: bool,
    limit: usize,
}

impl TranscriptTail {
    pub fn new(path: PathBuf, limit: usize) -> Self {
        Self {
            path,
            offset: 0,
            partial: String::new(),
            context_tokens: None,
            reset: false,
            limit,
        }
    }

    pub fn read_initial(&mut self) -> io::Result<Vec<TranscriptEvent>> {
        let bytes = std::fs::read(&self.path)?;
        self.offset = bytes.len() as u64;
        let text = String::from_utf8_lossy(&bytes);
        let (mut events, partial, tokens) = parse_complete_lines(&text);
        self.partial = partial;
        self.context_tokens = tokens;
        if self.limit > 0 && events.len() > self.limit {
            events.drain(0..events.len() - self.limit);
        }
        Ok(events)
    }

    pub fn read_appended(&mut self) -> io::Result<Vec<TranscriptEvent>> {
        let len = std::fs::metadata(&self.path)?.len();
        if len < self.offset {
            self.offset = 0;
            self.partial.clear();
            self.reset = true;
        }
        if len == self.offset {
            return Ok(Vec::new());
        }

        let start = self.offset;
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        self.offset = start + bytes.len() as u64;

        let mut text = std::mem::take(&mut self.partial);
        text.push_str(&String::from_utf8_lossy(&bytes));
        let (events, partial, tokens) = parse_complete_lines(&text);
        self.partial = partial;
        if tokens.is_some() {
            self.context_tokens = tokens;
        }
        Ok(events)
    }
}

/// The model selected in the CLI for this session, independent of the event
/// window kept by `read_initial`. The last `model_change` wins; assistant
/// messages carry provider/model too and cover sessions whose model never
/// changed after the first turn.
///
/// Scans backward in chunks so a 40 MB transcript costs only the tail that
/// holds the last marker.
pub fn current_session_model(path: &Path) -> Option<String> {
    const CHUNK: u64 = 64 * 1024;
    let mut file = File::open(path).ok()?;
    let mut end = file.metadata().ok()?.len();
    let mut carry: Vec<u8> = Vec::new();
    while end > 0 {
        let start = end.saturating_sub(CHUNK);
        file.seek(SeekFrom::Start(start)).ok()?;
        let mut chunk = vec![0u8; (end - start) as usize];
        file.read_exact(&mut chunk).ok()?;
        chunk.extend_from_slice(&carry);
        let mut lines: Vec<&[u8]> = chunk.split(|byte| *byte == b'\n').collect();
        // When the chunk starts mid-line, its first fragment belongs to the
        // previous chunk; reconstruct it there instead of parsing it here.
        carry = if start > 0 && !lines.is_empty() {
            lines.remove(0).to_vec()
        } else {
            Vec::new()
        };
        for line in lines.iter().rev() {
            if let Some(model) = model_from_session_line(line) {
                return Some(model);
            }
        }
        end = start;
    }
    None
}

fn model_from_session_line(line: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(line).ok()?;
    if !text.contains("model_change") && !text.contains("assistant") {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    match value.get("type")?.as_str()? {
        "model_change" => value.get("model")?.as_str().map(str::to_owned),
        "message" => {
            let message = value.get("message")?;
            if message.get("role")?.as_str()? != "assistant" {
                return None;
            }
            let model = message.get("model")?.as_str()?;
            match message.get("provider").and_then(serde_json::Value::as_str) {
                Some(provider) if !model.contains('/') => Some(format!("{provider}/{model}")),
                _ => Some(model.to_owned()),
            }
        }
        _ => None,
    }
}
fn parse_complete_lines(text: &str) -> (Vec<TranscriptEvent>, String, Option<u64>) {
    let mut events = Vec::new();
    let mut context_tokens = None;
    let mut start = 0;
    for (index, byte) in text.as_bytes().iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }
        let line = text[start..index].trim_end_matches('\r');
        if let Some(tokens) = assistant_context_tokens(line) {
            context_tokens = Some(tokens);
        }
        if let Some(event) = parse_transcript_line(line) {
            events.push(event);
        }
        start = index + 1;
    }
    (events, text[start..].to_owned(), context_tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const USER_LINE: &str = r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"oi"}]}}"#;

    #[test]
    fn initial_read_returns_events_and_remembers_offset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, format!("{USER_LINE}\n")).unwrap();
        let mut tail = TranscriptTail::new(path, 200);
        let events = tail.read_initial().unwrap();
        assert_eq!(events, vec![TranscriptEvent::User("oi".into())]);
        assert!(tail.offset > 0);
    }

    #[test]
    fn appended_read_keeps_partial_line_until_complete() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, "").unwrap();
        let mut tail = TranscriptTail::new(path.clone(), 200);
        assert!(tail.read_initial().unwrap().is_empty());

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        write!(file, "{}", &USER_LINE[..USER_LINE.len() / 2]).unwrap();
        file.flush().unwrap();
        assert!(tail.read_appended().unwrap().is_empty());

        writeln!(file, "{}", &USER_LINE[USER_LINE.len() / 2..]).unwrap();
        file.flush().unwrap();
        assert_eq!(
            tail.read_appended().unwrap(),
            vec![TranscriptEvent::User("oi".into())]
        );
    }

    #[test]
    fn shrink_resets_the_offset_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, format!("{USER_LINE}\n{USER_LINE}\n")).unwrap();
        let mut tail = TranscriptTail::new(path.clone(), 200);
        assert_eq!(tail.read_initial().unwrap().len(), 2);

        std::fs::write(&path, format!("{USER_LINE}\n")).unwrap();
        let events = tail.read_appended().unwrap();
        assert!(tail.reset, "a shrink must tell the view to clear its history");
        assert_eq!(events, vec![TranscriptEvent::User("oi".into())]);
    }

    #[test]
    fn context_usage_is_read_from_tool_call_assistant_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let tool_call = r#"{"type":"message","message":{"role":"assistant","stopReason":"toolUse","content":[{"type":"toolCall","name":"read_file"}],"usage":{"input":100,"output":24,"cacheRead":112000,"cacheWrite":0,"totalTokens":112124}}}"#;
        let aborted = r#"{"type":"message","message":{"role":"assistant","stopReason":"aborted","content":[],"usage":{"totalTokens":0}}}"#;
        std::fs::write(&path, format!("{tool_call}\n{aborted}\n")).unwrap();
        let mut tail = TranscriptTail::new(path.clone(), 200);
        tail.read_initial().unwrap();
        assert_eq!(
            tail.context_tokens,
            Some(112100),
            "the gauge must count prompt tokens, not the reply"
        );

        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{tool_call}").unwrap();
        writeln!(file, "{aborted}").unwrap();
        file.flush().unwrap();
        tail.read_appended().unwrap();
        assert_eq!(
            tail.context_tokens,
            Some(112100),
            "aborted zero usage must not clear the meter"
        );
    }

    #[test]
    fn current_model_uses_the_last_change_or_assistant_turn() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let body = [
            r#"{"type":"model_change","model":"p/old"}"#,
            r#"{"type":"message","message":{"role":"assistant","provider":"p","model":"old","stopReason":"stop","content":[{"type":"text","text":"oi"}]}}"#,
        ]
        .join("\n");
        std::fs::write(&path, format!("{body}\n")).unwrap();
        assert_eq!(current_session_model(&path).as_deref(), Some("p/old"));

        let body = [
            r#"{"type":"message","message":{"role":"assistant","provider":"p","model":"flash","stopReason":"stop","content":[{"type":"text","text":"oi"}]}}"#,
            r#"{"type":"model_change","model":"q/new"}"#,
        ]
        .join("\n");
        std::fs::write(&path, format!("{body}\n")).unwrap();
        assert_eq!(current_session_model(&path).as_deref(), Some("q/new"));
    }

    #[test]
    fn current_model_falls_back_to_an_assistant_provider_and_model() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let line = r#"{"type":"message","message":{"role":"assistant","provider":"opencode-go","model":"deepseek-v4.1-flash","stopReason":"stop","content":[{"type":"text","text":"oi"}]}}"#;
        std::fs::write(&path, format!("{line}\n")).unwrap();
        assert_eq!(
            current_session_model(&path).as_deref(),
            Some("opencode-go/deepseek-v4.1-flash")
        );
    }

    #[test]
    fn current_model_scans_past_the_chunk_window() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let mut body = String::from(r#"{"type":"model_change","model":"p/ancient"}"#);
        body.push('\n');
        for index in 0..3000 {
            body.push_str(&format!(
                r#"{{"type":"message","message":{{"role":"user","content":[{{"type":"text","text":"padding {index}"}}]}}}}"#
            ));
            body.push('\n');
        }
        assert!(body.len() > 64 * 1024);
        std::fs::write(&path, body).unwrap();
        assert_eq!(
            current_session_model(&path).as_deref(),
            Some("p/ancient"),
            "the scan must reach a model_change outside the default event window"
        );
    }
}
