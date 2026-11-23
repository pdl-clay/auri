//! PTY lifecycle for canvas terminals: spawn a command into a pseudo-terminal,
//! expose its streams and tear down the whole process group.

use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use std::path::Path;
use portable_pty::{CommandBuilder, PtySize};


/// One frame at 60 fps: output produced inside this window is delivered as a
/// single read, so the terminal view notifies the UI once per frame instead of
/// once per 4 KB PTY chunk.
const OUTPUT_BATCH_INTERVAL: Duration = Duration::from_millis(16);

enum BatchEvent {
    Data(Vec<u8>),
    Flush,
    Done,
}

/// Coalesces PTY output before it reaches `TerminalView`.
pub struct BatchedReader {
    events: Receiver<BatchEvent>,
    pending: Vec<u8>,
    ready: bool,
}

impl BatchedReader {
    pub fn new(mut inner: Box<dyn Read + Send>) -> Self {
        let (tx, rx) = mpsc::sync_channel(4);

        let producer = tx.clone();
        thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            loop {
                match inner.read(&mut buffer) {
                    Ok(0) => {
                        let _ = producer.send(BatchEvent::Done);
                        break;
                    }
                    Ok(size) => {
                        if producer
                            .send(BatchEvent::Data(buffer[..size].to_vec()))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = producer.send(BatchEvent::Done);
                        break;
                    }
                }
            }
        });

        thread::spawn(move || loop {
            thread::sleep(OUTPUT_BATCH_INTERVAL);
            if tx.send(BatchEvent::Flush).is_err() {
                break;
            }
        });

        Self {
            events: rx,
            pending: Vec::new(),
            ready: false,
        }
    }
}

impl Read for BatchedReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.ready && !self.pending.is_empty() {
                let size = output.len().min(self.pending.len());
                output[..size].copy_from_slice(&self.pending[..size]);
                self.pending.drain(..size);
                if self.pending.is_empty() {
                    self.ready = false;
                }
                return Ok(size);
            }
            match self.events.recv() {
                Ok(BatchEvent::Data(bytes)) => self.pending.extend_from_slice(&bytes),
                Ok(BatchEvent::Flush) if !self.pending.is_empty() => self.ready = true,
                Ok(BatchEvent::Flush) => {}
                Ok(BatchEvent::Done) => {
                    if self.pending.is_empty() {
                        return Ok(0);
                    }
                    self.ready = true;
                }
                Err(_) => return Ok(0),
            }
        }
    }
}

/// Resolve a POSIX login shell; GUI sessions often have no `$SHELL`.
/// fish's login startup does not drive portable-pty correctly here, so bash is used.
fn default_shell() -> String {
    let resolve = |shell: String| {
        if shell.rsplit('/').next() == Some("fish") {
            "/bin/bash".into()
        } else {
            shell
        }
    };
    if let Ok(shell) = std::env::var("SHELL")
        && !shell.is_empty()
        && Path::new(&shell).exists()
    {
        return resolve(shell);
    }
    let user = std::env::var("USER").unwrap_or_default();
    if !user.is_empty()
        && let Ok(passwd) = std::fs::read_to_string("/etc/passwd")
    {
        for line in passwd.lines() {
            let mut fields = line.split(':');
            if fields.next() == Some(user.as_str())
                && let Some(shell) = fields.nth(5)
                && Path::new(shell).exists()
            {
                return resolve(shell.to_string());
            }
        }
    }
    "/bin/bash".into()
}
/// A process running on the slave side of a pty, with its master-side streams.
pub struct PtySession {
    /// Output from the slave side of the pty.
    pub reader: Box<dyn Read + Send>,
    /// Input to the slave side of the pty.
    pub writer: Box<dyn Write + Send>,
    /// Shared master handle; the kernel pty is resized through this lock.
    pub master: Arc<parking_lot::Mutex<Box<dyn portable_pty::MasterPty + Send>>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    pid: Option<u32>,
    terminated: bool,
}

#[cfg(unix)]
fn signal_group(group: i32, signal: i32) -> anyhow::Result<()> {
    if unsafe { libc::kill(group, signal) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error.into());
        }
    }
    Ok(())
}

/// Poll `try_wait` instead of blocking in `wait`, and skip portable-pty's
/// leader-only SIGHUP grace period (four 50 ms sleeps before its SIGKILL).
fn reap_within(child: &mut dyn portable_pty::Child, timeout: Duration) -> anyhow::Result<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait()?.is_some() {
            return Ok(true);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(false);
        }
        std::thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}

impl PtySession {
    /// Spawn `$SHELL` (falling back to `/bin/sh`) in a new pty.
    pub fn spawn_shell(cols: u16, rows: u16) -> anyhow::Result<Self> {
        Self::spawn_shell_in(cols, rows, None)
    }

    /// Spawn `$SHELL` (falling back to `/bin/sh`) in a new pty whose working
    /// directory is `cwd`; `None` keeps portable-pty's default (the home dir).
    pub fn spawn_shell_in(
        cols: u16,
        rows: u16,
        cwd: Option<&std::path::Path>,
    ) -> anyhow::Result<Self> {
        let shell = default_shell();
        let mut cmd = CommandBuilder::new(shell);
        cmd.arg("-l");
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        if let Some(cwd) = cwd {
            cmd.cwd(cwd);
        }
        Self::spawn_command(cmd, cols, rows)
    }

    /// Spawn `cmd` in a new pty sized to `cols` x `rows`.
    pub fn spawn_command(cmd: CommandBuilder, cols: u16, rows: u16) -> anyhow::Result<Self> {
        let pair = portable_pty::native_pty_system().openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let child = pair.slave.spawn_command(cmd)?;
        let pid = child.process_id();
        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        Ok(Self {
            reader,
            writer,
            master: Arc::new(parking_lot::Mutex::new(pair.master)),
            child,
            pid,
            terminated: false,
        })
    }

    /// Resize the underlying pty; the attached terminal grid follows via SIGWINCH.
    pub fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.master.lock().resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    /// Type a command into the shell. The pty buffers it until the shell reads.
    pub fn send_line(&mut self, command: &str) -> std::io::Result<()> {
        self.writer.write_all(command.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()
    }

    /// The process id of the pty's child, when the platform provides one.
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Terminate the child's whole process group and reap the child.
    ///
    /// On Unix the group is signalled before the leader is reaped. Descendants
    /// that form their own job-control group are not members; those rely on the
    /// kernel SIGHUP delivered when the pty master closes.
    pub fn terminate(&mut self) -> anyhow::Result<()> {
        if self.terminated {
            return Ok(());
        }

        #[cfg(unix)]
        if let Some(pid) = self.pid {
            // portable-pty calls setsid(), so -pid reaches the leader and its
            // descendants even when the leader has already exited.
            let group = -(pid as i32);
            signal_group(group, libc::SIGTERM)?;
            if !reap_within(self.child.as_mut(), Duration::from_millis(80))? {
                signal_group(group, libc::SIGKILL)?;
                if !reap_within(self.child.as_mut(), Duration::from_millis(80))? {
                    anyhow::bail!("terminal {pid} did not exit after process-group SIGKILL");
                }
            }
            self.terminated = true;
            return Ok(());
        }

        // Fallback for platforms without process groups (or without a pid).
        if let Err(err) = self.child.kill() {
            if self.child.try_wait()?.is_none() {
                return Err(err.into());
            }
        }
        self.child.wait()?;
        self.terminated = true;
        Ok(())
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn default_shell_points_to_an_existing_shell() {
        assert!(Path::new(&default_shell()).exists());
    }

    fn read_pid_with_timeout(path: &Path, timeout: Duration) -> anyhow::Result<u32> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(contents) = fs::read_to_string(path) {
                if let Ok(pid) = contents.trim().parse::<u32>() {
                    return Ok(pid);
                }
            }
            if Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for pid file {}", path.display());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(unix)]
    fn process_alive(pid: u32) -> bool {
        unsafe {
            libc::kill(pid as i32, 0) == 0
                || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
        }
    }

    #[cfg(unix)]
    fn wait_until_gone(pid: u32, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while process_alive(pid) {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        true
    }

    #[test]
    fn spawn_resize_and_terminate() {
        let mut pty = PtySession::spawn_shell(80, 24).unwrap();
        assert!(pty.pid().is_some());

        pty.resize(100, 30).unwrap();
        let size = pty.master.lock().get_size().unwrap();
        assert_eq!((size.cols, size.rows), (100, 30));

        pty.terminate().unwrap();
        assert!(pty.child.try_wait().unwrap().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn terminate_kills_descendants() {
        let marker = tempfile::NamedTempFile::new().unwrap();
        let mut script = CommandBuilder::new("/bin/sh");
        script.arg("-c");
        script.arg(format!(
            "sleep 30 & echo $! > {}; wait",
            marker.path().display()
        ));
        let mut pty = PtySession::spawn_command(script, 80, 24).unwrap();

        let bg = read_pid_with_timeout(marker.path(), Duration::from_secs(2)).unwrap();
        assert!(
            process_alive(bg),
            "background child should be alive before terminate"
        );
        pty.terminate().unwrap();
        assert!(
            wait_until_gone(bg, Duration::from_secs(2)),
            "background child {bg} survived process-group termination"
        );
    }

    #[cfg(unix)]
    #[test]
    fn terminate_kills_descendants_after_leader_exit() {
        let marker = tempfile::NamedTempFile::new().unwrap();
        let mut script = CommandBuilder::new("/bin/sh");
        script.arg("-c");
        script.arg(format!(
            "(trap '' HUP; exec sleep 30) & echo $! > {}; sleep 0.2",
            marker.path().display()
        ));
        let mut pty = PtySession::spawn_command(script, 80, 24).unwrap();

        let bg = read_pid_with_timeout(marker.path(), Duration::from_secs(2)).unwrap();
        assert!(
            process_alive(bg),
            "background child should be alive before the leader exits"
        );

        // The leader lingers briefly, then exits on its own, leaving the
        // background sleep in the same process group.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if pty.child.try_wait().unwrap().is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "leader did not exit on its own");
            std::thread::sleep(Duration::from_millis(10));
        }

        pty.terminate().unwrap();
        assert!(
            wait_until_gone(bg, Duration::from_secs(2)),
            "background child {bg} survived terminate after the leader exited"
        );
    }

    /// Strip the shell's line-editing escape sequences and carriage returns so
    /// each line of a pty transcript can be compared literally.
    fn clean_output(bytes: &[u8]) -> String {
        let text = String::from_utf8_lossy(bytes);
        let mut clean = String::new();
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '\u{1b}' => {
                    if chars.peek() == Some(&'[') {
                        chars.next();
                        for final_byte in chars.by_ref() {
                            if ('@'..='~').contains(&final_byte) {
                                break;
                            }
                        }
                    }
                }
                '\r' => {}
                _ => clean.push(ch),
            }
        }
        clean
    }

    fn shell_cwd(pty: &mut PtySession) -> String {
        let mut reader = std::mem::replace(&mut pty.reader, Box::new(std::io::empty()));
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        writeln!(pty.writer, "printf '__CWD__%s__END__\\n' \"$PWD\"").unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut raw = Vec::new();
        loop {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(chunk) => raw.extend_from_slice(&chunk),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) if Instant::now() >= deadline => {
                    panic!("timed out waiting for pwd output")
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
            let output = clean_output(&raw);
            if let Some(start) = output.rfind("__CWD__") {
                let rest = &output[start + "__CWD__".len()..];
                if let Some(end) = rest.find("__END__") {
                    let value = rest[..end].trim();
                    if value.starts_with('/') {
                        return value.to_owned();
                    }
                }
            }
        }
        panic!("shell did not report its working directory");
    }

    #[test]
    fn spawn_shell_in_uses_given_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let mut pty = PtySession::spawn_shell_in(80, 24, Some(dir.path())).unwrap();
        let expected = fs::canonicalize(dir.path()).unwrap();
        assert_eq!(shell_cwd(&mut pty), expected.to_str().unwrap());
    }

    #[test]
    fn spawn_shell_in_none_matches_spawn_shell() {
        let mut baseline = PtySession::spawn_shell(80, 24).unwrap();
        let mut pty = PtySession::spawn_shell_in(80, 24, None).unwrap();
        assert_eq!(shell_cwd(&mut pty), shell_cwd(&mut baseline));
    }
}

#[cfg(test)]
mod batch_tests {
    use super::*;

    #[test]
    fn output_is_coalesced_until_flush() {
        let (tx, rx) = mpsc::channel();
        let mut reader = BatchedReader {
            events: rx,
            pending: Vec::new(),
            ready: false,
        };
        tx.send(BatchEvent::Data(b"ab".to_vec())).unwrap();
        tx.send(BatchEvent::Data(b"cd".to_vec())).unwrap();
        tx.send(BatchEvent::Flush).unwrap();

        let mut output = [0u8; 8];
        let size = reader.read(&mut output).unwrap();
        assert_eq!(&output[..size], b"abcd");

        tx.send(BatchEvent::Done).unwrap();
        assert_eq!(reader.read(&mut output).unwrap(), 0);
    }
}
