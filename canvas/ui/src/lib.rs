//! Host-side view code for the PTY terminal canvas.

pub mod canvas;
pub mod card;
pub mod chat;
pub mod chat_reader;
pub mod chat_view;
pub mod config;
pub mod pty;
pub mod permissions;
pub mod theme;
pub mod text_card;
pub mod workspaces;
pub mod updater;

#[cfg(test)]
mod regressions;

use auro::{App, assets::EmbeddedAssets, gpui::AppContext};
use canvas_types::WorkspaceStore;

use crate::config::LoadOutcome;

use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use auri_protocol::{RequestLine, Response, decode_line, encode_response};

/// The host's answer to one CLI line. Request bodies are decoded before this
/// handler runs; a connection waits for exactly one response.
type RequestHandler = Arc<dyn Fn(RequestLine) -> Response + Send + Sync>;

fn socket_path_in(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join(format!("auri-{}.sock", std::process::id()))
}

/// One socket per running app instance, local to the user's runtime directory.
fn socket_path() -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(std::env::temp_dir);
    socket_path_in(&runtime)
}

/// Answer one connection. A malformed line is refused without shutting down
/// the listener, and the timeout keeps a silent client from pinning a thread.
fn serve(stream: &mut UnixStream, handler: &RequestHandler) {
    let result = (|| -> anyhow::Result<Response> {
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            anyhow::bail!("empty request line");
        }
        Ok(handler(decode_line(line.trim_end())?))
    })();
    let response = result.unwrap_or_else(|error| Response::Err(format!("bad_request: {error}")));
    let mut payload = encode_response(&response);
    payload.push('\n');
    let _ = stream.write_all(payload.as_bytes());
    let _ = stream.flush();
}

/// Owns the host's single Unix socket. Dropping it stops the accept loop and
/// unlinks the socket, so a stale path never outlives the app.
pub struct HostListener {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl HostListener {
    /// Bind the app socket under `XDG_RUNTIME_DIR` and start accepting.
    pub fn start(handler: RequestHandler) -> io::Result<Self> {
        Self::start_at(&socket_path(), handler)
    }

    /// Bind a specific socket path; `start` uses the app's runtime directory.
    pub fn start_at(path: &Path, handler: RequestHandler) -> io::Result<Self> {
        if path.exists() {
            // A crashed instance may have left a socket behind.
            fs::remove_file(path)?;
        }
        let listener = UnixListener::bind(path)?;
        listener.set_nonblocking(true)?;
        // XDG_RUNTIME_DIR is private already, but the /tmp fallback is not.
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_path = path.to_path_buf();
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => serve(&mut stream, &handler),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20));
                    }
                    Err(_) => break,
                }
            }
            let _ = fs::remove_file(&thread_path);
        });
        Ok(Self {
            path: path.to_path_buf(),
            stop,
            thread: Some(thread),
        })
    }

    /// Stop accepting and unlink the socket. Safe to call more than once.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = fs::remove_file(&self.path);
    }
}

impl Drop for HostListener {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Version shown in the drawer footer and checked against `Auro.toml`.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Runs the app with the terminal canvas as the `main` window's root view.
pub fn run() {
    let dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    match crate::updater::prepare(&dir) {
        Ok(true) => return,
        Ok(false) => {}
        Err(error) => eprintln!("auri updater unavailable: {error}"),
    }
    // Install before the first window (and therefore the first agent PTY).
    let skill = match canvas::install_skill() {
        Ok(path) => Some(path),
        Err(error) => {
            eprintln!("auri skill unavailable: {error}");
            None
        }
    };
    let mut store = WorkspaceStore::new();
    let load: LoadOutcome = config::load(&mut store);
    App::new()
        .assets(EmbeddedAssets::new(&[
            ("star.svg", include_bytes!("../../icons/star.svg")),
            ("folder.svg", include_bytes!("../../icons/folder.svg")),
            ("bolt.svg", include_bytes!("../../icons/bolt.svg")),
            ("connect.svg", include_bytes!("../../icons/connect.svg")),
            ("globe.svg", include_bytes!("../../icons/globe.svg")),
            ("plus.svg", include_bytes!("../../icons/plus.svg")),
            ("minus.svg", include_bytes!("../../icons/minus.svg")),
            ("map.svg", include_bytes!("../../icons/map.svg")),
            ("search.svg", include_bytes!("../../icons/search.svg")),
            ("x.svg", include_bytes!("../../icons/x.svg")),
            ("terminal.svg", include_bytes!("../../icons/terminal.svg")),
            ("chat.svg", include_bytes!("../../icons/chat.svg")),
            ("copy.svg", include_bytes!("../../icons/copy.svg")),
            ("trash.svg", include_bytes!("../../icons/trash.svg")),
            ("note.svg", include_bytes!("../../icons/note.svg")),
            ("unload.svg", include_bytes!("../../icons/unload.svg")),
            ("edit.svg", include_bytes!("../../icons/edit.svg")),
            ("file.svg", include_bytes!("../../icons/file.svg")),
            ("code.svg", include_bytes!("../../icons/code.svg")),
            ("agent-claude.svg", include_bytes!("../../icons/agent-claude.svg")),
            ("agent-codex.svg", include_bytes!("../../icons/agent-codex.svg")),
            ("agent-pi.svg", include_bytes!("../../icons/agent-pi.svg")),
            ("agent-omp.svg", include_bytes!("../../icons/agent-omp.svg")),
            ("agent-open-code.svg", include_bytes!("../../icons/agent-open-code.svg")),
            ("admin.svg", include_bytes!("../../icons/admin.svg")),
            ("member.svg", include_bytes!("../../icons/member.svg")),
            ("paperclip.svg", include_bytes!("../../icons/paperclip.svg")),
            ("pause.svg", include_bytes!("../../icons/pause.svg")),
            ("send.svg", include_bytes!("../../icons/send.svg")),
            (
                "bubble-tail-left.svg",
                include_bytes!("../../icons/bubble-tail-left.svg"),
            ),
            (
                "bubble-tail-right.svg",
                include_bytes!("../../icons/bubble-tail-right.svg"),
            ),
        ]))
        .setup(|cx, _| {
            gpui_component::init(cx);
            gpui_component::theme::Theme::change(
                gpui_component::theme::ThemeMode::Dark,
                None,
                cx,
            );
            let theme = gpui_component::theme::Theme::global_mut(cx);
            theme.radius = auro::gpui::px(crate::theme::RADIUS_CONTROL);
            theme.radius_lg = auro::gpui::px(crate::theme::RADIUS_DIALOG);
        })
        .root_with("main", move |window, cx| {
            let handle = window.window_handle();
            let view = cx.new(|cx| canvas::Canvas::new(store.clone(), load.clone(), handle, skill.clone(), cx));
            view.update(cx, |canvas, cx| canvas.bind_actions(cx));
            // Dev-only preview: show the update card without waiting for a signed release.
            #[cfg(debug_assertions)]
            if let Ok(version) = std::env::var("AURI_PREVIEW_UPDATE") {
                view.update(cx, |canvas, cx| canvas.set_update_available(version, cx));
            }
            view.update(cx, |canvas, cx| canvas.prepare_workspaces(cx));
            // A release published after the last boot check must still notify
            // the running app; applying it stays a boot-time decision.
            view.update(cx, |_, cx| {
                let canvas = view.downgrade();
                cx.spawn(async move |_, cx| {
                    loop {
                        cx.background_executor().timer(Duration::from_secs(3600)).await;
                        let found = cx
                            .background_executor()
                            .spawn(async { crate::updater::probe(Path::new(".")) })
                            .await;
                        if let Ok(Some(version)) = found {
                            let canvas = canvas.clone();
                            let _ = cx.update_window(handle, move |_, _, app| {
                                if let Some(entity) = canvas.upgrade() {
                                    entity.update(app, |canvas, cx| {
                                        canvas.set_update_available(version, cx)
                                    });
                                }
                            });
                        }
                    }
                })
                .detach();
            });
            // Requests arrive on the socket thread but must mutate the canvas
            // on the main thread; each connection waits for its own answer.
            let (sender, receiver) = mpsc::channel::<(RequestLine, mpsc::Sender<Response>)>();
            view.update(cx, |_, cx| {
                cx.spawn(async move |canvas, cx| {
                    let mut receiver = Some(receiver);
                    loop {
                        let rx = receiver.take().expect("receiver is restored every turn");
                        let (rx, incoming) = cx
                            .background_executor()
                            .spawn(async move {
                                let item = rx.recv().ok();
                                (rx, item)
                            })
                            .await;
                        receiver = Some(rx);
                        let Some((line, reply)) = incoming else {
                            break;
                        };
                        let answer = canvas
                            .upgrade()
                            .and_then(|entity| {
                                cx.update_window(handle, |_, window, app| {
                                    entity.update(app, |canvas, cx| {
                                        canvas.handle_request(&line.token, line.request, window, cx)
                                    })
                                })
                                .ok()
                            })
                            .unwrap_or_else(|| Response::Err("host_unavailable".into()));
                        let _ = reply.send(answer);
                    }
                })
                .detach();
            });
            let handler: RequestHandler = Arc::new(move |line| {
                let (reply, wait) = mpsc::channel();
                if sender.send((line, reply)).is_err() {
                    return Response::Err("host_unavailable".into());
                }
                wait.recv().unwrap_or_else(|_| Response::Err("host_unavailable".into()))
            });
            match HostListener::start(handler) {
                Ok(listener) => view.update(cx, |canvas, _| canvas.set_listener(listener)),
                Err(error) => eprintln!("auri host socket unavailable: {error}"),
            }
            cx.new(|cx| gpui_component::Root::new(view, window, cx))
        })
        .run();
}

#[cfg(test)]
mod listener_tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    use auri_protocol::{Request, RequestLine, decode_response, encode_line};

    #[test]
    fn socket_path_lives_under_the_runtime_directory() {
        let path = socket_path_in(Path::new("/run/user/1000"));
        assert_eq!(path.parent(), Some(Path::new("/run/user/1000")));
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("auri-"));
        assert!(name.ends_with(".sock"));
    }

    #[test]
    fn listener_answers_one_line_per_connection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auri.sock");
        let handler: RequestHandler = Arc::new(|line| Response::Ok(format!("{}:{}", line.token, line.json)));
        let mut listener = HostListener::start_at(&path, handler).unwrap();

        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);

        let mut stream = UnixStream::connect(&path).unwrap();
        let line = encode_line(&RequestLine {
            token: "abc".into(),
            json: false,
            request: Request::List,
        });
        writeln!(stream, "{line}").unwrap();
        let mut answer = String::new();
        BufReader::new(&stream).read_line(&mut answer).unwrap();
        assert_eq!(decode_response(answer.trim()).unwrap(), Response::Ok("abc:false".into()));

        listener.stop();
        assert!(!path.exists());
    }

    #[test]
    fn malformed_line_is_refused_without_stopping_the_listener() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auri.sock");
        let handler: RequestHandler = Arc::new(|_| Response::Ok("ok".into()));
        let mut listener = HostListener::start_at(&path, handler).unwrap();

        let mut bad = UnixStream::connect(&path).unwrap();
        writeln!(bad, "not json").unwrap();
        let mut answer = String::new();
        BufReader::new(&bad).read_line(&mut answer).unwrap();
        assert!(matches!(decode_response(answer.trim()).unwrap(), Response::Err(reason) if reason.starts_with("bad_request")));

        let mut good = UnixStream::connect(&path).unwrap();
        writeln!(good, "{}", encode_line(&RequestLine {
            token: "abc".into(),
            json: true,
            request: Request::List,
        })).unwrap();
        let mut answer = String::new();
        BufReader::new(&good).read_line(&mut answer).unwrap();
        assert_eq!(decode_response(answer.trim()).unwrap(), Response::Ok("ok".into()));
        listener.stop();
        assert!(!path.exists());
    }
}

