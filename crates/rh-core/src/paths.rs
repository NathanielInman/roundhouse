//! Where Roundhouse keeps things. Every location can be moved with an
//! environment variable, which tests and development builds use to stay away
//! from an installed copy.

use std::path::PathBuf;

pub fn home() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Expands a leading `~`.
pub fn expand(s: &str) -> PathBuf {
    let s = s.trim();
    if s == "~" {
        home()
    } else if let Some(rest) = s.strip_prefix("~/") {
        home().join(rest)
    } else {
        PathBuf::from(s)
    }
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn xdg_config() -> PathBuf {
    env_dir("XDG_CONFIG_HOME").unwrap_or_else(|| home().join(".config"))
}

pub fn config_dir() -> PathBuf {
    if let Some(d) = env_dir("ROUNDHOUSE_CONFIG_DIR") {
        return d;
    }
    if cfg!(windows) {
        return env_dir("APPDATA")
            .unwrap_or_else(|| home().join("AppData/Roaming"))
            .join("roundhouse");
    }
    xdg_config().join("roundhouse")
}

pub fn state_dir() -> PathBuf {
    if let Some(d) = env_dir("ROUNDHOUSE_STATE_DIR") {
        return d;
    }
    if cfg!(windows) {
        return env_dir("LOCALAPPDATA")
            .unwrap_or_else(|| home().join("AppData/Local"))
            .join("roundhouse");
    }
    env_dir("XDG_STATE_HOME")
        .unwrap_or_else(|| home().join(".local/state"))
        .join("roundhouse")
}

/// Per-user directory for the socket, shared by the daemon, bar module and
/// drawer.
pub fn runtime_dir() -> PathBuf {
    if let Some(d) = env_dir("ROUNDHOUSE_RUNTIME_DIR") {
        return d;
    }
    // `$XDG_RUNTIME_DIR/roundhouse`, or a per-user directory under `$TMPDIR`.
    #[cfg(unix)]
    {
        if let Some(d) = env_dir("XDG_RUNTIME_DIR") {
            return d.join("roundhouse");
        }
        let base = env_dir("TMPDIR").unwrap_or_else(|| PathBuf::from("/tmp"));
        base.join(format!("roundhouse-{}", unsafe { libc::getuid() }))
    }
    #[cfg(not(unix))]
    {
        env_dir("LOCALAPPDATA")
            .unwrap_or_else(|| home().join("AppData/Local"))
            .join("roundhouse")
            .join("run")
    }
}

pub fn socket_path() -> PathBuf {
    runtime_dir().join("rh.sock")
}

pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

/// The user's stylesheet for the drawer, loaded over the theme.
pub fn style_file() -> PathBuf {
    config_dir().join("style.css")
}

/// The recording session, kept so it survives a daemon restart.
pub fn session_file() -> PathBuf {
    state_dir().join("session.json")
}

pub fn board_log() -> PathBuf {
    state_dir().join("board.log")
}

/// Waybar's config directory.
pub fn waybar_dir() -> PathBuf {
    xdg_config().join("waybar")
}
