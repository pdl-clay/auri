//! Incremental reader for OMP session JSONL files.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::chat::{TranscriptEvent, parse_transcript_line};

pub struct TranscriptTail {
    pub path: PathBuf,
    pub offset: u64,
    pub partial: String,
    pub reset: bool,
    limit: usize,
}

impl TranscriptTail {
    pub fn new(path: PathBuf, limit: usize) -> Self {
        Self {
            path,
            offset: 0,
            partial: String::new(),
            reset: false,
            limit,
        }
    }

    pub fn read_initial(&mut self) -> Result<Vec<TranscriptEvent>, String> {
        let bytes = std::fs::read(&self.path).map_err(|error| error.to_string())?;
        self.offset = bytes.len() as u64;
        let text = String::from_utf8_lossy(&bytes);
        let (mut events, partial) = parse_complete_lines(&text);
        self.partial = partial;
        if self.limit > 0 && events.len() > self.limit {
            events.drain(0..events.len() - self.limit);
        }
        Ok(events)
    }

    pub fn read_appended(&mut self) -> Result<Vec<TranscriptEvent>, String> {
        let len = std::fs::metadata(&self.path)
            .map_err(|error| error.to_string())?
            .len();
        if len < self.offset {
            self.offset = 0;
            self.partial.clear();
            self.reset = true;
        }
        if len == self.offset {
            return Ok(Vec::new());
        }

        let start = self.offset;
        let mut file = File::open(&self.path).map_err(|error| error.to_string())?;
        file.seek(SeekFrom::Start(start))
            .map_err(|error| error.to_string())?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        self.offset = start + bytes.len() as u64;

        let mut text = std::mem::take(&mut self.partial);
        text.push_str(&String::from_utf8_lossy(&bytes));
        let (events, partial) = parse_complete_lines(&text);
        self.partial = partial;
        Ok(events)
    }
}

pub fn read_events(path: &Path, limit: usize) -> Result<Vec<TranscriptEvent>, String> {
    TranscriptTail::new(path.to_path_buf(), limit).read_initial()
}

fn parse_complete_lines(text: &str) -> (Vec<TranscriptEvent>, String) {
    let mut events = Vec::new();
    let mut start = 0;
    for (index, byte) in text.as_bytes().iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }
        let line = text[start..index].trim_end_matches('\r');
        if let Some(event) = parse_transcript_line(line) {
            events.push(event);
        }
        start = index + 1;
    }
    (events, text[start..].to_owned())
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
}
