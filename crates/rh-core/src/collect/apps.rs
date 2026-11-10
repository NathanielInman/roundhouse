//! Memory by app, not by process.
//!
//! Modern desktops start each app in its own systemd scope
//! (`app-com.google.Chrome-910.scope`), so the cgroup's own counter is the
//! honest answer: 38 Chrome processes read as one line, shared pages counted
//! once. Transient scopes (`systemd-run`, `lowprio`) show their memory cap.
//! Processes that share a session scope (launched straight from the
//! compositor or a terminal) fall back to grouping by name.

use super::{read, read_trim};
use crate::model::AppUsage;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Group {
    /// An app or service with its own cgroup.
    Cgroup {
        path: String,
        name: Option<String>,
        detail: &'static str,
    },
    /// Group by process name.
    Process,
}

/// Undoes systemd's unit-name escaping (`\x2d` is `-`).
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("\\x") {
        out.push_str(&rest[..i]);
        let hex = rest.get(i + 2..i + 4).unwrap_or("");
        match u8::from_str_radix(hex, 16) {
            Ok(b) => {
                out.push(b as char);
                rest = &rest[i + 4..];
            }
            Err(_) => {
                out.push_str("\\x");
                rest = &rest[i + 2..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn title(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// The app a unit name stands for: `app-com.google.Chrome-910.scope` is
/// "Chrome", `app-Hyprland-kitty@a1.service` is "Kitty".
pub fn app_name(unit: &str) -> String {
    let u = unescape(unit);
    let mut s = u.strip_prefix("app-").unwrap_or(&u);
    for suffix in [".scope", ".service"] {
        s = s.strip_suffix(suffix).unwrap_or(s);
    }
    if let Some(i) = s.find('@') {
        s = &s[..i];
    }
    // Trailing "-<pid>" or "-<random>" on scopes.
    if let Some(i) = s.rfind('-') {
        let tail = &s[i + 1..];
        if !tail.is_empty()
            && tail.chars().all(|c| c.is_ascii_hexdigit())
            && tail.chars().any(|c| c.is_ascii_digit())
        {
            s = &s[..i];
        }
    }
    // Launcher prefixes from uwsm and friends.
    for p in [
        "Hyprland-",
        "hyprland-",
        "niri-",
        "sway-",
        "gnome-",
        "kde-",
        "flatpak-",
        "uwsm-",
    ] {
        if let Some(rest) = s.strip_prefix(p) {
            s = rest;
            break;
        }
    }
    // Reverse-DNS ids keep their last part.
    let parts: Vec<&str> = s.split('.').collect();
    let name = if parts.len() >= 3 {
        parts[parts.len() - 1]
    } else {
        s
    };
    title(name)
}

/// Decides how a process at this cgroup path is counted.
pub fn classify(cgroup: &str) -> Group {
    let leaf = cgroup.rsplit('/').next().unwrap_or("");
    if leaf.starts_with("app-") && (leaf.ends_with(".scope") || leaf.ends_with(".service")) {
        return Group::Cgroup {
            path: cgroup.into(),
            name: Some(app_name(leaf)),
            detail: "app",
        };
    }
    if leaf.starts_with("run-") && (leaf.ends_with(".scope") || leaf.ends_with(".service")) {
        return Group::Cgroup {
            path: cgroup.into(),
            name: None,
            detail: "scope",
        };
    }
    if leaf.ends_with(".service") && !leaf.starts_with("user@") {
        let n = leaf.trim_end_matches(".service");
        let n = n.split('@').next().unwrap_or(n);
        return Group::Cgroup {
            path: cgroup.into(),
            name: Some(unescape(n)),
            detail: "service",
        };
    }
    Group::Process
}

/// Working set of a cgroup: `memory.current` minus inactive page cache, the
/// number container runtimes use for the same question.
fn cgroup_mem(path: &str) -> Option<(u64, Option<u64>)> {
    let dir = Path::new("/sys/fs/cgroup").join(path.trim_start_matches('/'));
    let current: u64 = read_trim(dir.join("memory.current"))?.parse().ok()?;
    let inactive = read(dir.join("memory.stat"))
        .and_then(|s| {
            s.lines().find_map(|l| {
                l.strip_prefix("inactive_file ")
                    .and_then(|v| v.trim().parse::<u64>().ok())
            })
        })
        .unwrap_or(0);
    let cap = read_trim(dir.join("memory.max")).and_then(|v| v.parse::<u64>().ok());
    Some((current.saturating_sub(inactive), cap))
}

fn page_size() -> u64 {
    #[cfg(unix)]
    {
        let p = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if p > 0 {
            return p as u64;
        }
    }
    4096
}

/// The `limit` largest apps by memory, ignoring anything under 16 MB.
pub fn sample(limit: usize) -> Vec<AppUsage> {
    let Ok(rd) = std::fs::read_dir("/proc") else {
        return vec![];
    };
    let page = page_size();
    // cgroup path -> (name, detail, procs)
    let mut cgroups: HashMap<String, (Option<String>, &'static str, usize)> = HashMap::new();
    // process name -> (rss, procs)
    let mut by_name: HashMap<String, (u64, usize)> = HashMap::new();
    let mut pids: Vec<u32> = rd
        .flatten()
        .filter_map(|e| e.file_name().to_string_lossy().parse().ok())
        .collect();
    pids.sort_unstable();
    for pid in pids {
        let base = Path::new("/proc").join(pid.to_string());
        let Some(cg) = read(base.join("cgroup")) else {
            continue;
        };
        let Some(path) = cg.lines().find_map(|l| l.strip_prefix("0::")) else {
            continue;
        };
        match classify(path.trim()) {
            Group::Cgroup { path, name, detail } => {
                let e = cgroups.entry(path).or_insert((None, detail, 0));
                e.2 += 1;
                if e.0.is_none() {
                    e.0 = name.or_else(|| read_trim(base.join("comm")));
                }
            }
            Group::Process => {
                let rss = read(base.join("statm"))
                    .and_then(|s| {
                        s.split_whitespace()
                            .nth(1)
                            .and_then(|v| v.parse::<u64>().ok())
                    })
                    .unwrap_or(0)
                    * page;
                if rss == 0 {
                    continue; // kernel threads
                }
                let Some(comm) = read_trim(base.join("comm")) else {
                    continue;
                };
                let e = by_name.entry(comm).or_insert((0, 0));
                e.0 += rss;
                e.1 += 1;
            }
        }
    }

    // Keyed by lowercase name, so Chrome's scope and stray `chrome`
    // processes in the session scope add up to one line.
    let mut merged: HashMap<String, AppUsage> = HashMap::new();
    for (path, (name, detail, procs)) in cgroups {
        let Some((mem, cap)) = cgroup_mem(&path) else {
            continue;
        };
        let name = title(&name.unwrap_or_else(|| "unknown".into()));
        let e = merged
            .entry(name.to_lowercase())
            .or_insert_with(|| AppUsage {
                name,
                detail: detail.into(),
                ..Default::default()
            });
        e.mem += mem;
        e.procs += procs;
        if detail == "scope" {
            e.cap = cap;
        }
    }
    for (comm, (rss, procs)) in by_name {
        let name = title(&comm);
        let e = merged
            .entry(name.to_lowercase())
            .or_insert_with(|| AppUsage {
                name,
                detail: "process".into(),
                ..Default::default()
            });
        e.mem += rss;
        e.procs += procs;
    }

    let mut out: Vec<AppUsage> = merged.into_values().filter(|a| a.mem >= 16 << 20).collect();
    for a in &mut out {
        if a.procs > 1 {
            a.detail = format!("{} procs", a.procs);
        }
        if a.cap.is_some() {
            a.detail = "lowprio scope".into();
        }
    }
    out.sort_by(|a, b| b.mem.cmp(&a.mem).then(a.name.cmp(&b.name)));
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_names() {
        assert_eq!(app_name("app-com.google.Chrome-910.scope"), "Chrome");
        assert_eq!(app_name("app-slack-913.scope"), "Slack");
        assert_eq!(app_name("app-Hyprland-kitty@a1b2.service"), "Kitty");
        assert_eq!(app_name("app-dev.zed.Zed-4411.scope"), "Zed");
        assert_eq!(app_name("app-t3\\x2dcode-77.scope"), "T3-code");
    }

    #[test]
    fn classifies_cgroups() {
        let base = "/user.slice/user-1000.slice/user@1000.service";
        match classify(&format!("{base}/app.slice/app-slack-913.scope")) {
            Group::Cgroup { name, detail, .. } => {
                assert_eq!((name.as_deref(), detail), (Some("Slack"), "app"))
            }
            g => panic!("{g:?}"),
        }
        match classify(&format!("{base}/app.slice/run-u42.scope")) {
            Group::Cgroup { name, detail, .. } => assert_eq!((name, detail), (None, "scope")),
            g => panic!("{g:?}"),
        }
        match classify("/system.slice/docker.service") {
            Group::Cgroup { name, .. } => assert_eq!(name.as_deref(), Some("docker")),
            g => panic!("{g:?}"),
        }
        assert_eq!(
            classify("/user.slice/user-1000.slice/session-1.scope"),
            Group::Process
        );
        assert_eq!(classify(&format!("{base}/init.scope")), Group::Process);
    }
}
