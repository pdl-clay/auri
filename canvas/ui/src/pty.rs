//! PTY lifecycle for canvas terminals: spawn a command into a pseudo-terminal,
//! expose its streams and tear down the whole process group.

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize};

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
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let mut cmd = CommandBuilder::new(shell);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
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
    use std::path::Path;

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
            "(trap '' HUP; sleep 30) & echo $! > {}",
            marker.path().display()
        ));
        let mut pty = PtySession::spawn_command(script, 80, 24).unwrap();

        let bg = read_pid_with_timeout(marker.path(), Duration::from_secs(2)).unwrap();
        assert!(
            process_alive(bg),
            "background child should be alive before the leader exits"
        );

        // No `wait` in the script: the leader exits on its own, leaving the
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
}
