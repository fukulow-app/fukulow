use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};

const TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct AppProcess {
    child: Child,
    logs: Receiver<String>,
}

impl AppProcess {
    pub(crate) fn spawn(mut command: Command) -> Result<Self> {
        command
            .env("FUKULOW_BIND_ADDR", "127.0.0.1:0")
            .env("RUST_LOG", "info");
        Self::spawn_configured(command)
    }

    pub(crate) fn spawn_configured(mut command: Command) -> Result<Self> {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let stderr = child.stderr.take().context("child stderr is missing")?;
        let (sender, logs) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else {
                    break;
                };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self { child, logs })
    }

    pub(crate) fn wait_for_log(&self, marker: &str) -> Result<String> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let line = self
                .logs
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .with_context(|| format!("child did not log {marker}"))?;
            if line.contains(marker) {
                return Ok(line);
            }
        }
    }

    pub(crate) fn address(&self) -> Result<SocketAddr> {
        let line = self.wait_for_log("Listening on ")?;
        line.split_once("Listening on ")
            .context("listening address is missing")?
            .1
            .trim()
            .parse()
            .context("invalid listening address")
    }

    pub(crate) fn wait_for_exit(&mut self) -> Result<ExitStatus> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                bail!("child did not exit within the deadline");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub(crate) fn remaining_logs(&self) -> String {
        self.logs.iter().collect::<Vec<_>>().join("\n")
    }

    #[cfg(unix)]
    pub(crate) fn signal(&mut self, signal: &str) -> Result<()> {
        anyhow::ensure!(self.child.try_wait()?.is_none(), "child already exited");
        let status = Command::new("kill")
            .arg(signal)
            .arg(self.child.id().to_string())
            .status()?;
        anyhow::ensure!(status.success(), "could not send shutdown signal");
        Ok(())
    }
}

impl Drop for AppProcess {
    fn drop(&mut self) {
        // A failed assertion must not leave a child holding a port or awaiting a body.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(crate) fn connect(address: SocketAddr) -> Result<TcpStream> {
    let stream = TcpStream::connect_timeout(&address, TIMEOUT)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    Ok(stream)
}

pub(crate) fn health(address: SocketAddr) -> Result<()> {
    let mut stream = connect(address)?;
    stream.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let (headers, body) = response.split_once("\r\n\r\n").context("missing body")?;
    assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"), "{headers}");
    assert!(
        headers
            .lines()
            .any(|line| line.eq_ignore_ascii_case("content-type: application/json"))
    );
    assert_eq!(body, r#"{"status":"ok"}"#);
    Ok(())
}
