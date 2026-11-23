//! The `auri` CLI: one argv → one line JSON request → one line JSON response.

use std::env;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;

use auri_protocol::{Request, RequestLine, Response, decode_escapes, decode_response, encode_line};

#[derive(Debug, PartialEq, Eq)]
struct Args {
    json: bool,
    request: Request,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err((code, message)) => {
            eprintln!("auri: {message}");
            ExitCode::from(code)
        }
    }
}

fn run() -> Result<(), (u8, String)> {
    let argv: Vec<String> = env::args().skip(1).collect();
    let Args { json, request } = parse_args(&argv).map_err(|error| (2, error))?;

    // `skill` is local: the host only puts the path in the agent's environment.
    if matches!(request, Request::Skill) {
        let path = env::var("AURI_SKILL").map_err(|_| (2, "AURI_SKILL is not set".to_owned()))?;
        println!("{path}");
        return Ok(());
    }

    let socket = env::var("AURI_SOCKET").map_err(|_| (2, "AURI_SOCKET is not set".to_owned()))?;
    let token = env::var("AURI_TOKEN").map_err(|_| (2, "AURI_TOKEN is not set".to_owned()))?;
    let stream = UnixStream::connect(socket)
        .map_err(|error| (1, format!("cannot connect to host: {error}")))?;

    let line = encode_line(&RequestLine {
        token,
        json,
        request,
    });
    let mut stream = BufReader::new(stream);
    writeln!(stream.get_mut(), "{line}")
        .map_err(|error| (1, format!("cannot send request: {error}")))?;

    let mut response_line = String::new();
    let read = stream
        .read_line(&mut response_line)
        .map_err(|error| (1, format!("cannot read response: {error}")))?;
    if read == 0 {
        return Err((1, "host closed the connection".to_owned()));
    }
    let response = decode_response(response_line.trim_end_matches(['\r', '\n']))
        .map_err(|error| (1, format!("invalid response: {error}")))?;
    match response {
        Response::Ok(text) => {
            println!("{text}");
            Ok(())
        }
        Response::Err(text) => Err((1, text)),
    }
}

/// Parse `auri <verb> [args] [--json]` into a request plus output mode.
fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut json = false;
    let mut raw = false;
    let mut lines = None;
    let mut name = None;
    let mut command = None;
    let mut admin = None;
    let mut positional: Vec<&str> = Vec::new();

    let mut iter = argv.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--raw" => raw = true,
            "--lines" => {
                lines = Some(parse_number(iter.next(), "--lines")?);
            }
            "--name" => {
                name = Some(iter.next().ok_or("--name needs a value")?.clone());
            }
            "--command" => {
                command = Some(iter.next().ok_or("--command needs a value")?.clone());
            }
            "--admin" => {
                let value = iter.next().ok_or("--admin needs on or off")?;
                admin = Some(match value.as_str() {
                    "on" => true,
                    "off" => false,
                    other => return Err(format!("--admin must be on|off, got `{other}`")),
                });
            }
            other if other.starts_with('-') => return Err(format!("unknown flag `{other}`")),
            other => positional.push(other),
        }
    }

    let request = match positional.as_slice() {
        ["list"] => Request::List,
        ["ask", target, prompt] => {
            let payload = if raw {
                decode_escapes(prompt).map_err(|error| format!("invalid --raw text: {error}"))?
            } else {
                prompt.as_bytes().to_vec()
            };
            Request::Ask {
                target: (*target).to_owned(),
                raw: payload,
            }
        }
        ["check", target] => Request::Check {
            target: (*target).to_owned(),
            lines,
        },
        ["note", "read", note] => Request::NoteRead {
            note: (*note).to_owned(),
            start: None,
            count: None,
        },
        ["note", "read", note, start] => Request::NoteRead {
            note: (*note).to_owned(),
            start: Some(parse_number_str(start, "start")?),
            count: None,
        },
        ["note", "read", note, start, count] => Request::NoteRead {
            note: (*note).to_owned(),
            start: Some(parse_number_str(start, "start")?),
            count: Some(parse_number_str(count, "count")?),
        },
        ["note", "write", note, text] => Request::NoteWrite {
            note: (*note).to_owned(),
            text: (*text).to_owned(),
        },
        ["note", "edit", note, old, new] => Request::NoteEdit {
            note: (*note).to_owned(),
            old: (*old).to_owned(),
            new: (*new).to_owned(),
        },
        ["note", "create"] => Request::NoteCreate { text: None, name },
        ["note", "create", text] => Request::NoteCreate {
            text: Some((*text).to_owned()),
            name,
        },
        ["note", "delete", note] => Request::NoteDelete {
            note: (*note).to_owned(),
        },
        ["terminal", "new"] => Request::TerminalNew { name, command },
        ["terminal", "remove", target] => Request::TerminalRemove {
            target: (*target).to_owned(),
        },
        ["agent", "new", new_name] => Request::AgentNew {
            name: (*new_name).to_owned(),
            command,
        },
        ["agent", "remove", target] => Request::AgentRemove {
            target: (*target).to_owned(),
        },
        ["agent", "role", target] => Request::AgentRole {
            target: (*target).to_owned(),
            admin: admin.ok_or("agent role needs --admin on|off")?,
        },
        ["skill"] => Request::Skill,
        _ => {
            return Err(format!(
                "unknown or incomplete command: {}",
                positional.join(" ")
            ));
        }
    };
    Ok(Args { json, request })
}

fn parse_number(value: Option<&String>, flag: &str) -> Result<u32, String> {
    let value = value.ok_or_else(|| format!("{flag} needs a number"))?;
    parse_number_str(value, flag)
}

fn parse_number_str(value: &str, label: &str) -> Result<u32, String> {
    value
        .parse()
        .map_err(|_| format!("{label} must be a non-negative integer, got `{value}`"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use auri_protocol::decode_escapes;

    fn cli(args: &[&str]) -> Args {
        let argv: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        parse_args(&argv).unwrap()
    }

    #[test]
    fn parses_list_with_json_flag_anywhere() {
        assert_eq!(
            cli(&["--json", "list"]),
            Args {
                json: true,
                request: Request::List,
            }
        );
        assert_eq!(
            cli(&["list", "--json"]),
            Args {
                json: true,
                request: Request::List,
            }
        );
    }

    #[test]
    fn parses_ask_with_and_without_raw_escapes() {
        assert_eq!(
            cli(&["ask", "card:7", r"go\n\x03"]),
            Args {
                json: false,
                request: Request::Ask {
                    target: "card:7".into(),
                    raw: br"go\n\x03".to_vec(),
                },
            }
        );
        assert_eq!(
            cli(&["ask", "--raw", "card:7", r"go\n\x03"]),
            Args {
                json: false,
                request: Request::Ask {
                    target: "card:7".into(),
                    raw: decode_escapes(r"go\n\x03").unwrap(),
                },
            }
        );
    }

    #[test]
    fn parses_check_with_optional_line_count() {
        assert_eq!(
            cli(&["check", "card:7", "--lines", "9"]),
            Args {
                json: false,
                request: Request::Check {
                    target: "card:7".into(),
                    lines: Some(9),
                },
            }
        );
        assert_eq!(
            cli(&["check", "card:7"]),
            Args {
                json: false,
                request: Request::Check {
                    target: "card:7".into(),
                    lines: None,
                },
            }
        );
    }

    #[test]
    fn parses_every_note_verb() {
        assert_eq!(
            cli(&["note", "read", "note:3", "2", "5"]),
            Args {
                json: false,
                request: Request::NoteRead {
                    note: "note:3".into(),
                    start: Some(2),
                    count: Some(5),
                },
            }
        );
        assert_eq!(
            cli(&["note", "write", "note:3", "hello world"]),
            Args {
                json: false,
                request: Request::NoteWrite {
                    note: "note:3".into(),
                    text: "hello world".into(),
                },
            }
        );
        assert_eq!(
            cli(&["note", "edit", "note:3", "old", "new"]),
            Args {
                json: false,
                request: Request::NoteEdit {
                    note: "note:3".into(),
                    old: "old".into(),
                    new: "new".into(),
                },
            }
        );
        assert_eq!(
            cli(&["note", "create", "--name", "Ideas", "first"]),
            Args {
                json: false,
                request: Request::NoteCreate {
                    text: Some("first".into()),
                    name: Some("Ideas".into()),
                },
            }
        );
        assert_eq!(
            cli(&["note", "create"]),
            Args {
                json: false,
                request: Request::NoteCreate {
                    text: None,
                    name: None,
                },
            }
        );
        assert_eq!(
            cli(&["note", "delete", "note:3"]),
            Args {
                json: false,
                request: Request::NoteDelete {
                    note: "note:3".into(),
                },
            }
        );
    }

    #[test]
    fn parses_every_card_verb() {
        assert_eq!(
            cli(&["terminal", "new", "--name", "shell", "--command", "bash"]),
            Args {
                json: false,
                request: Request::TerminalNew {
                    name: Some("shell".into()),
                    command: Some("bash".into()),
                },
            }
        );
        assert_eq!(
            cli(&["terminal", "remove", "card:7"]),
            Args {
                json: false,
                request: Request::TerminalRemove {
                    target: "card:7".into(),
                },
            }
        );
        assert_eq!(
            cli(&["agent", "new", "worker", "--command", "claude"]),
            Args {
                json: false,
                request: Request::AgentNew {
                    name: "worker".into(),
                    command: Some("claude".into()),
                },
            }
        );
        assert_eq!(
            cli(&["agent", "remove", "card:7"]),
            Args {
                json: false,
                request: Request::AgentRemove {
                    target: "card:7".into(),
                },
            }
        );
        assert_eq!(
            cli(&["agent", "role", "card:7", "--admin", "off"]),
            Args {
                json: false,
                request: Request::AgentRole {
                    target: "card:7".into(),
                    admin: false,
                },
            }
        );
    }

    #[test]
    fn parses_skill_without_a_socket() {
        assert_eq!(
            cli(&["skill", "--json"]),
            Args {
                json: true,
                request: Request::Skill,
            }
        );
    }

    #[test]
    fn rejects_invalid_argv() {
        for args in [
            vec![],
            vec!["fly"],
            vec!["note"],
            vec!["note", "frobnicate"],
            vec!["ask", "card:7"],
            vec!["check", "card:7", "--lines", "many"],
            vec!["agent", "role", "card:7", "--admin", "maybe"],
            vec!["list", "--bogus"],
        ] {
            let argv: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
            assert!(parse_args(&argv).is_err(), "accepted argv: {args:?}");
        }
    }
}
