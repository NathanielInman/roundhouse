//! The daemon's local socket: a Unix socket only the user can open.
//!
//! macOS works the same way. Windows support (a token-guarded localhost port,
//! as Switchyard does) lands with the tray app.

#[cfg(unix)]
use anyhow::Context;
use anyhow::{anyhow, Result};
use std::path::Path;
use std::time::Duration;

#[cfg(unix)]
pub type Stream = std::os::unix::net::UnixStream;

#[cfg(unix)]
pub fn connect_to(path: &Path, timeout: Option<Duration>) -> Result<Stream> {
    let s = Stream::connect(path).with_context(|| format!("cannot reach {}", path.display()))?;
    s.set_read_timeout(timeout)?;
    s.set_write_timeout(timeout)?;
    Ok(s)
}

#[cfg(not(unix))]
pub fn connect_to(path: &Path, _timeout: Option<Duration>) -> Result<std::net::TcpStream> {
    Err(anyhow!(
        "{}: the Roundhouse daemon does not run on this platform yet",
        path.display()
    ))
}

/// Connects to the Roundhouse daemon.
#[cfg(unix)]
pub fn connect() -> Result<Stream> {
    let path = crate::paths::socket_path();
    connect_to(&path, None).map_err(|_| {
        anyhow!(
            "cannot reach the Roundhouse daemon at {} (start it with `rh daemon`)",
            path.display()
        )
    })
}

#[cfg(not(unix))]
pub fn connect() -> Result<std::net::TcpStream> {
    connect_to(&crate::paths::socket_path(), None)
}

pub fn daemon_alive() -> bool {
    connect().is_ok()
}

#[cfg(unix)]
pub struct Listener {
    inner: std::os::unix::net::UnixListener,
}

#[cfg(unix)]
impl Listener {
    pub fn bind() -> Result<Listener> {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::paths::runtime_dir();
        std::fs::create_dir_all(&dir)?;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        let path = crate::paths::socket_path();
        if daemon_alive() {
            return Err(anyhow!(
                "a Roundhouse daemon is already running ({})",
                path.display()
            ));
        }
        let _ = std::fs::remove_file(&path);
        let inner = std::os::unix::net::UnixListener::bind(&path)
            .with_context(|| format!("binding {}", path.display()))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Listener { inner })
    }

    pub fn accept(&self) -> std::io::Result<Stream> {
        self.inner.accept().map(|(s, _)| s)
    }

    pub fn cleanup(&self) {
        let _ = std::fs::remove_file(crate::paths::socket_path());
    }
}
