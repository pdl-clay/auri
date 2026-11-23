//! Line-JSON wire protocol shared by the `auri` CLI and the canvas host.
//!
//! One request is one JSON object on one line, tagged by `verb`. The CLI adds
//! `token` (the calling card's secret) and `json` (the requested output mode)
//! to that same object; [`decode`] ignores those transport fields, while
//! [`decode_line`] returns them to the host. Responses use the same framing.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// One requested operation. Targets are the strings `auri list` prints: a
/// canonical workspace-local id first, then a display name the host resolves.
/// `Ask::raw` holds the exact bytes to send, after `--raw` escape decoding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "snake_case")]
pub enum Request {
    List,
    Ask {
        target: String,
        raw: Vec<u8>,
    },
    Check {
        target: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lines: Option<u32>,
    },
    NoteRead {
        note: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        count: Option<u32>,
    },
    NoteWrite {
        note: String,
        text: String,
    },
    NoteEdit {
        note: String,
        old: String,
        new: String,
    },
    NoteCreate {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    NoteDelete {
        note: String,
    },
    TerminalNew {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
    },
    TerminalRemove {
        target: String,
    },
    AgentNew {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
    },
    AgentRemove {
        target: String,
    },
    AgentRole {
        target: String,
        admin: bool,
    },
    Skill,
}

/// Result of one request. `Ok` carries human- or JSON-formatted output;
/// `Err` carries the reason the host refused the request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Response {
    Ok(String),
    Err(String),
}

/// A request plus the transport fields the CLI puts on the same line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestLine {
    pub token: String,
    pub json: bool,
    pub request: Request,
}

/// Why a line could not be decoded.
#[derive(Debug)]
pub enum Error {
    Json(serde_json::Error),
    MissingField(&'static str),
    InvalidEscape { offset: usize, sequence: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(e) => write!(f, "{e}"),
            Self::MissingField(name) => write!(f, "missing `{name}` in request line"),
            Self::InvalidEscape { offset, sequence } => {
                write!(f, "invalid escape `{sequence}` at byte {offset}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(e) => Some(e),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// Encode a bare request (no transport fields), for tests and in-process use.
pub fn encode(request: &Request) -> String {
    serde_json::to_string(request).expect("Request always serializes to an object")
}

/// Decode a request line, ignoring transport fields such as `token` and `json`.
pub fn decode(line: &str) -> Result<Request, Error> {
    Ok(serde_json::from_str(line)?)
}

/// Encode the exact line the CLI sends: verb, args, token and output mode.
pub fn encode_line(line: &RequestLine) -> String {
    let mut value =
        serde_json::to_value(&line.request).expect("Request always serializes to an object");
    let object = value
        .as_object_mut()
        .expect("internally tagged Request serializes to an object");
    object.insert("token".to_owned(), json!(line.token));
    object.insert("json".to_owned(), json!(line.json));
    serde_json::to_string(object).expect("request line always serializes")
}

/// Decode the CLI's line, including its token and output mode.
pub fn decode_line(line: &str) -> Result<RequestLine, Error> {
    let value: Value = serde_json::from_str(line)?;
    let token = value
        .get("token")
        .and_then(Value::as_str)
        .ok_or(Error::MissingField("token"))?
        .to_owned();
    let json = value.get("json").and_then(Value::as_bool).unwrap_or(false);
    let request = serde_json::from_value(value)?;
    Ok(RequestLine {
        token,
        json,
        request,
    })
}

pub fn encode_response(response: &Response) -> String {
    serde_json::to_string(response).expect("Response always serializes to an object")
}

pub fn decode_response(line: &str) -> Result<Response, Error> {
    Ok(serde_json::from_str(line)?)
}

/// Turn a CLI `--raw` argument into the exact bytes sent to the target PTY.
///
/// Supported escapes: `\n`, `\t`, `\r`, `\e` (ESC), `\\` and `\xNN`.
/// Anything else is an error rather than a silently mangled keystroke.
pub fn decode_escapes(input: &str) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::with_capacity(input.len());
    let mut chars = input.chars();
    let mut offset = 0usize;
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            let mut buffer = [0u8; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
            offset += ch.len_utf8();
            continue;
        }
        let start = offset;
        offset += 1;
        let escaped = chars.next().ok_or_else(|| Error::InvalidEscape {
            offset: start,
            sequence: "\\".to_owned(),
        })?;
        offset += escaped.len_utf8();
        match escaped {
            'n' => bytes.push(b'\n'),
            't' => bytes.push(b'\t'),
            'r' => bytes.push(b'\r'),
            'e' => bytes.push(0x1b),
            '\\' => bytes.push(b'\\'),
            'x' => {
                let high = chars.next().ok_or_else(|| Error::InvalidEscape {
                    offset: start,
                    sequence: "\\x".to_owned(),
                })?;
                let low = chars.next().ok_or_else(|| Error::InvalidEscape {
                    offset: start,
                    sequence: "\\x".to_owned(),
                })?;
                offset += high.len_utf8() + low.len_utf8();
                let (high, low) =
                    hex_digit(high)
                        .zip(hex_digit(low))
                        .ok_or_else(|| Error::InvalidEscape {
                            offset: start,
                            sequence: format!("\\x{high}{low}"),
                        })?;
                bytes.push(high << 4 | low);
            }
            other => {
                return Err(Error::InvalidEscape {
                    offset: start,
                    sequence: format!("\\{other}"),
                });
            }
        }
    }
    Ok(bytes)
}

fn hex_digit(ch: char) -> Option<u8> {
    match ch {
        '0'..='9' => Some(ch as u8 - b'0'),
        'a'..='f' => Some(ch as u8 - b'a' + 10),
        'A'..='F' => Some(ch as u8 - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_requests() -> Vec<Request> {
        vec![
            Request::List,
            Request::Ask {
                target: "card:7".into(),
                raw: b"hello".to_vec(),
            },
            Request::Check {
                target: "card:7".into(),
                lines: Some(12),
            },
            Request::NoteRead {
                note: "note:3".into(),
                start: Some(2),
                count: Some(5),
            },
            Request::NoteWrite {
                note: "note:3".into(),
                text: "new text".into(),
            },
            Request::NoteEdit {
                note: "note:3".into(),
                old: "old".into(),
                new: "new".into(),
            },
            Request::NoteCreate {
                text: Some("first".into()),
                name: Some("Ideas".into()),
            },
            Request::NoteDelete {
                note: "note:3".into(),
            },
            Request::TerminalNew {
                name: Some("shell".into()),
                command: Some("bash".into()),
            },
            Request::TerminalRemove {
                target: "card:7".into(),
            },
            Request::AgentNew {
                name: "worker".into(),
                command: None,
            },
            Request::AgentRemove {
                target: "card:7".into(),
            },
            Request::AgentRole {
                target: "card:7".into(),
                admin: true,
            },
            Request::Skill,
        ]
    }

    #[test]
    fn round_trips_every_request_variant() {
        let requests = all_requests();
        for request in requests {
            let line = encode(&request);
            assert!(
                !line.contains('\n'),
                "line JSON must stay on one line: {line}"
            );
            assert_eq!(decode(&line).unwrap(), request, "line: {line}");
        }
    }

    #[test]
    fn uses_a_verb_tag_on_a_single_json_object() {
        let line = encode(&Request::List);
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["verb"], "list");
    }

    #[test]
    fn rejects_unknown_verbs_and_malformed_json() {
        for bad in [
            r#"{"verb":"fly"}"#,
            r#"{"verb":"ask","target":"card:7"}"#,
            "{",
            "",
            "not json",
            r#"{"verb":"list"} trailing"#,
        ] {
            assert!(decode(bad).is_err(), "accepted invalid line: {bad}");
        }
    }

    #[test]
    fn raw_escape_strings_decode_to_exact_bytes() {
        let bytes = decode_escapes(r"go\nnext\tcol\eesc\x1b\x03\\end").unwrap();
        assert_eq!(bytes, b"go\nnext\tcol\x1besc\x1b\x03\\end");
    }

    #[test]
    fn raw_payload_round_trips_to_the_exact_cli_bytes() {
        let raw = decode_escapes(r"line\n\t\e\x03").unwrap();
        let request = Request::Ask {
            target: "card:7".into(),
            raw: raw.clone(),
        };
        let decoded = decode(&encode(&request)).unwrap();
        match decoded {
            Request::Ask { target, raw: got } => {
                assert_eq!(target, "card:7");
                assert_eq!(got, raw);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn rejects_invalid_escape_sequences() {
        assert!(decode_escapes(r"bad\q").is_err());
        assert!(decode_escapes(r"bad\xZZ").is_err());
        assert!(decode_escapes(r"bad\x0").is_err());
    }

    #[test]
    fn line_envelope_carries_token_and_json_flag() {
        let line = RequestLine {
            token: "tok".into(),
            json: true,
            request: Request::List,
        };
        let encoded = encode_line(&line);
        assert_eq!(decode_line(&encoded).unwrap(), line);
        assert_eq!(decode(&encoded).unwrap(), Request::List);
        assert!(decode_line(&encode(&Request::List)).is_err());
    }

    #[test]
    fn response_wire_round_trips() {
        for response in [
            Response::Ok("done".into()),
            Response::Err("permission_denied".into()),
        ] {
            let line = encode_response(&response);
            assert!(
                !line.contains('\n'),
                "line JSON must stay on one line: {line}"
            );
            assert_eq!(decode_response(&line).unwrap(), response);
        }
        assert!(decode_response("{").is_err());
        assert!(decode_response(r#"{"oops":"x"}"#).is_err());
    }
}
