//! The daemon's local socket: a Unix socket only the user can open.
//!
//! macOS works the same way. Windows support (a token-guarded localhost port)
//! lands with the tray app.

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

    /// Removes the socket when the daemon is told to stop (SIGTERM from
    /// systemd, SIGINT from a terminal, SIGHUP when its session ends), so no
    /// stale `rh.sock` is left for clients to try.
    pub fn cleanup_on_signal(&self) {
        let path = crate::paths::socket_path();
        let Ok(c) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else {
            return;
        };
        if SOCKET.set(c).is_err() {
            return;
        }
        for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            // SAFETY: on_signal only calls async-signal-safe functions.
            unsafe {
                libc::signal(sig, on_signal as *const () as libc::sighandler_t);
            }
        }
    }
}

#[cfg(unix)]
static SOCKET: std::sync::OnceLock<std::ffi::CString> = std::sync::OnceLock::new();

#[cfg(unix)]
extern "C" fn on_signal(_sig: libc::c_int) {
    if let Some(p) = SOCKET.get() {
        // SAFETY: unlink and _exit are async-signal-safe; p is a valid C
        // string that lives for the whole program.
        unsafe {
            libc::unlink(p.as_ptr());
        }
    }
    // A requested stop is a clean exit, so systemd does not mark the unit
    // failed. SAFETY: as above.
    unsafe { libc::_exit(0) }
}
