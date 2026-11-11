//! Command-line entry points.

use anyhow::{anyhow, Result};
use rh_core::config::Config;
use rh_core::model::{DiskKind, Level, Snapshot};
use rh_core::protocol::{Request, SubKind};
use rh_core::{human, paths};
use std::io::Write;
use std::time::Duration;

const HELP: &str = "\
rh: Roundhouse, a system drawer that opens from your bar

Usage:
  rh toggle [--tab NAME] [--center]   open or close the drawer from the clicked icon
  rh show [--tab NAME] [--center]     open the drawer
  rh hide                             close the drawer
  rh bar                              the waybar module (prints JSON lines)
  rh session [on|off|toggle]          a recording session: Switchyard holds heavy
                                      builds until it ends (no argument: show it)
  rh status [--json]                  print what the drawer shows
  rh doctor                           check the setup
  rh daemon                           run the daemon in the foreground
  rh --version

Tabs: machine, storage; switchyard opens Switchyard's board (while it runs).
Config: ~/.config/roundhouse/config.toml
";

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn value(args: &[String], name: &str) -> Option<String> {
    let eq = format!("{name}=");
    args.iter().enumerate().find_map(|(i, a)| {
        if a == name {
            args.get(i + 1).cloned()
        } else {
            a.strip_prefix(&eq).map(String::from)
        }
    })
}

pub fn run(args: &[String]) -> Result<i32> {
    let rest = args.get(1..).unwrap_or(&[]);
    match args.first().map(String::as_str) {
        None | Some("help" | "-h" | "--help") => {
            print!("{HELP}");
            Ok(0)
        }
        Some("-V" | "--version" | "version") => {
            println!("rh {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        Some("daemon") => daemon(),
        Some("bar") => bar(),
        Some(cmd @ ("toggle" | "show")) => open(cmd == "toggle", rest),
        Some("hide") => {
            if rh_core::transport::daemon_alive() {
                send(&Request::Hide)?;
            }
            Ok(0)
        }
        Some("status") => status(flag(rest, "--json")),
        Some("session") => session(rest.first().map(String::as_str)),
        Some("doctor") => doctor(),
        Some(other) => Err(anyhow!("unknown command `{other}` (try `rh --help`)")),
    }
}

#[cfg(unix)]
fn daemon() -> Result<i32> {
    crate::daemon::run()
}

#[cfg(not(unix))]
fn daemon() -> Result<i32> {
    Err(anyhow!("the daemon does not run on this platform yet; macOS and Windows support comes with the tray app"))
}

fn send(req: &Request) -> Result<()> {
    let r = rh_core::protocol::request(req)?;
    match r.error {
        Some(e) => Err(anyhow!(e)),
        None => Ok(()),
    }
}

/// Starts the daemon in the background when it is not running, and waits
/// for its socket.
fn ensure_daemon() -> Result<()> {
    if rh_core::transport::daemon_alive() {
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    let log = paths::state_dir().join("daemon.log");
    std::fs::create_dir_all(paths::state_dir())?;
    let out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(out);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own session, so closing the terminal or restarting the bar
        // does not take the daemon with it.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    cmd.spawn()?;
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(50));
        if rh_core::transport::daemon_alive() {
            return Ok(());
        }
    }
    Err(anyhow!("the daemon did not start; see {}", log.display()))
}

fn open(toggle: bool, args: &[String]) -> Result<i32> {
    let (cfg, _) = Config::load();
    // Ask the compositor first: the cursor is still on the icon.
    let anchor = bar_anchor::hypr::anchor(&cfg.bar.namespace, flag(args, "--center"));
    let tab = value(args, "--tab");
    ensure_daemon()?;
    send(&if toggle {
        Request::Toggle { anchor, tab }
    } else {
        Request::Show { anchor, tab }
    })?;
    Ok(0)
}

fn bar() -> Result<i32> {
    let (cfg, _) = Config::load();
    let offline = serde_json::to_string(&crate::bar::offline(&cfg))?;
    let mut out = std::io::stdout().lock();
    let mut last = String::new();
    let mut tried_start = false;
    loop {
        match rh_core::protocol::subscribe_raw(SubKind::Bar) {
            Ok(lines) => {
                for line in lines {
                    let Ok(line) = line else { break };
                    if line != last {
                        writeln!(out, "{line}")?;
                        out.flush()?;
                        last = line;
                    }
                }
            }
            Err(_) if !tried_start => {
                tried_start = true;
                let _ = ensure_daemon();
                continue;
            }
            Err(_) => {}
        }
        if last != offline {
            writeln!(out, "{offline}")?;
            out.flush()?;
            last = offline.clone();
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// A snapshot from the daemon, or a fresh local one when it is not running.
fn snapshot() -> (Snapshot, bool) {
    if let Ok(r) = rh_core::protocol::request(&Request::Snapshot) {
        if let Some(s) = r.snapshot {
            if s.now > 0.0 {
                return (*s, true);
            }
        }
    }
    let (cfg, err) = Config::load();
    let mut c = rh_core::collect::Collector::new();
    c.sample(true, &cfg);
    std::thread::sleep(Duration::from_millis(400));
    let mut s = c.sample(true, &cfg);
    if cfg.siblings.switchyard {
        s.siblings.switchyard = rh_core::sibling::switchyard();
    }
    s.lamp = rh_core::lamp::evaluate(&s, &cfg.lamp);
    s.config_error = err;
    (s, false)
}

fn session_line(s: &Snapshot) -> String {
    match &s.session {
        Some(x) => {
            let how = if x.auto {
                "started by a recording app, "
            } else {
                ""
            };
            format!(
                "on for {} ({how}heavy builds wait)",
                rh_core::duration(s.now - x.since)
            )
        }
        None => "off".into(),
    }
}

fn session(arg: Option<&str>) -> Result<i32> {
    let on = match arg {
        None => {
            let (s, live) = snapshot();
            if !live {
                println!("off (the daemon is not running)");
            } else {
                println!("{}", session_line(&s));
            }
            return Ok(0);
        }
        Some("on" | "start") => Some(true),
        Some("off" | "stop" | "end") => Some(false),
        Some("toggle") => None,
        Some(other) => {
            return Err(anyhow!(
                "unknown session argument `{other}` (on, off or toggle)"
            ))
        }
    };
    ensure_daemon()?;
    let r = rh_core::protocol::request(&Request::Session { on })?;
    if let Some(e) = r.error {
        return Err(anyhow!(e));
    }
    if let Some(s) = r.snapshot {
        println!("recording session {}", session_line(&s));
    }
    Ok(0)
}

fn status(json: bool) -> Result<i32> {
    let (s, _) = snapshot();
    if json {
        println!("{}", serde_json::to_string_pretty(&s)?);
        return Ok(0);
    }
    let m = &s.machine;
    println!(
        "{} · {} · linux {} · up {}",
        s.host.name,
        s.host.os,
        s.host.kernel,
        rh_core::duration(s.host.uptime)
    );
    let lamp = match s.lamp.level {
        Level::Calm => "calm".to_string(),
        l => format!(
            "{}: {}",
            l.as_str(),
            s.lamp
                .reasons
                .iter()
                .map(|r| r.text.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        ),
    };
    println!("{:<12}{lamp}", "lamp");
    if s.session.is_some() {
        println!("{:<12}{}", "session", session_line(&s));
    }
    let t = Config::load().0.lamp;
    let mut cpu = format!("{}  {:.0}%", m.cpu.model, m.cpu.usage);
    if let Some(temp) = m.cpu.temp {
        cpu.push_str(&format!("  {}", t.cpu_temp(m.cpu.temp_limit).show(temp)));
    }
    println!("{:<12}{cpu}", "cpu");
    println!(
        "{:<12}{} / {} ({:.0}%)  swap {} / {}",
        "memory",
        human(m.memory.used()),
        human(m.memory.total),
        m.memory.used_pct(),
        human(m.memory.swap_used),
        human(m.memory.swap_total)
    );
    let psi = |p: Option<rh_core::model::Psi>| {
        p.map(|p| format!("{:.2}", p.some60))
            .unwrap_or_else(|| "-".into())
    };
    println!(
        "{:<12}cpu {}  memory {}  io {}  (some, avg60)",
        "pressure",
        psi(m.pressure.cpu),
        psi(m.pressure.memory),
        psi(m.pressure.io)
    );
    for g in &m.gpus {
        let mut line = g.name.clone();
        if let Some(u) = g.usage {
            line.push_str(&format!("  {u:.0}%"));
        }
        if let Some(temp) = g.temp {
            line.push_str(&format!("  {}", t.gpu_temp(g.temp_limit).show(temp)));
        }
        if let (Some(u), Some(t)) = (g.vram_used, g.vram_total) {
            line.push_str(&format!("  {} / {}", human(u), human(t)));
        }
        println!("{:<12}{line}", "gpu");
    }
    let n = &m.network;
    let mut net = format!(
        "down {}  up {}  ({})",
        rh_core::collect::net::rate(n.down),
        rh_core::collect::net::rate(n.up),
        n.interfaces.join(", ")
    );
    if let Some(ip) = &n.public_ip {
        net.push_str(&format!("  public {ip}"));
    }
    println!("{:<12}{net}", "network");
    println!("storage");
    for d in &s.disks {
        let mut head = match (&d.model, d.kind) {
            (_, DiskKind::OnDemand) => format!("{} (automount, not mounted)", d.name),
            (Some(model), _) => format!("{model} ({})", d.name),
            (None, _) => d.name.clone(),
        };
        if let Some(temp) = d.temp {
            head.push_str(&format!("  {}", t.drive_temp(d.temp_limit).show(temp)));
        }
        println!("  {head}");
        if d.kind == DiskKind::OnDemand {
            continue;
        }
        for f in &d.filesystems {
            let extra = if f.mounts.len() > 1 {
                format!(" +{}", f.mounts.len() - 1)
            } else {
                String::new()
            };
            let stale = if f.stale { "  (not answering)" } else { "" };
            println!(
                "    {:<24}{:>8} / {:<8}{:>4.0}%{stale}",
                format!("{}{extra}", paths_tilde(&f.mounts[0])),
                human(f.used),
                human(f.total),
                f.used_pct()
            );
        }
    }
    if !s.apps.is_empty() {
        println!("apps");
        for a in s.apps.iter().take(6) {
            let cap = a
                .cap
                .map(|c| format!(" of {}", human(c)))
                .unwrap_or_default();
            println!("    {:<24}{:>8}{cap}  {}", a.name, human(a.mem), a.detail);
        }
    }
    match &s.siblings.switchyard {
        Some(y) => {
            let mut line = format!("{} · {} tasks", y.phase, y.tasks);
            if let Some(status) = y.status_line() {
                line.push_str(&format!(" · {status}"));
            }
            println!("{:<12}{line}", "switchyard");
        }
        None => println!("{:<12}not running", "switchyard"),
    }
    if let Some(e) = &s.config_error {
        println!("\nconfig: {e}");
    }
    Ok(0)
}

fn paths_tilde(p: &str) -> String {
    let home = paths::home();
    let h = home.to_string_lossy();
    match p.strip_prefix(h.as_ref()) {
        Some("") => "~".into(),
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => p.to_string(),
    }
}

fn check(ok: Option<bool>, what: &str, detail: &str) {
    let tag = match ok {
        Some(true) => "ok  ",
        Some(false) => "fail",
        None => "info",
    };
    if detail.is_empty() {
        println!("{tag} {what}");
    } else {
        println!("{tag} {what}: {detail}");
    }
}

fn doctor() -> Result<i32> {
    let mut failed = false;
    let (cfg, err) = Config::load();
    match err {
        Some(e) => {
            failed = true;
            check(Some(false), "config", &e);
        }
        None if paths::config_file().exists() => check(
            Some(true),
            "config",
            &paths::config_file().display().to_string(),
        ),
        None => check(None, "config", "no config file, using defaults"),
    }
    let alive = rh_core::transport::daemon_alive();
    check(
        alive.then_some(true),
        "daemon",
        if alive {
            "running"
        } else {
            "not running (`rh bar` or `rh toggle` starts it)"
        },
    );
    match bar_anchor::hypr::socket() {
        Some(_) => check(
            Some(true),
            "Hyprland IPC",
            "the drawer opens from the clicked icon",
        ),
        None => check(None, "Hyprland IPC", "not found; the drawer opens centered"),
    }
    match rh_core::waybar::config_path() {
        Some(p) => {
            let l = rh_core::waybar::load_layout().unwrap_or_default();
            check(
                Some(true),
                "waybar config",
                &format!(
                    "{} (position {})",
                    p.display(),
                    l.position.as_deref().unwrap_or("top")
                ),
            );
        }
        None => check(None, "waybar config", "not found"),
    }
    if cfg.theme.from == "waybar" {
        match rh_core::waybar::load_theme() {
            Some(t) => check(
                Some(true),
                "waybar theme",
                &format!(
                    "font {}, background {}, text {}",
                    t.font.as_deref().unwrap_or("-"),
                    t.background.as_deref().unwrap_or("-"),
                    t.foreground.as_deref().unwrap_or("-")
                ),
            ),
            None => check(
                None,
                "waybar theme",
                "no style.css; using the built-in theme",
            ),
        }
    }
    let board = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("rh-board")))
        .filter(|p| p.exists());
    match board {
        Some(p) => check(Some(true), "drawer", &p.display().to_string()),
        None => check(
            None,
            "drawer",
            "rh-board not next to rh; it must be on PATH",
        ),
    }
    let (s, _) = snapshot();
    check(
        s.machine.cpu.temp.is_some().then_some(true),
        "CPU temperature",
        &s.machine
            .cpu
            .temp
            .map(|t| format!("{t:.0}°C"))
            .unwrap_or_else(|| "no sensor found".into()),
    );
    if s.machine.gpus.is_empty() {
        check(
            None,
            "GPUs",
            "none found (NVIDIA needs its driver's NVML library)",
        );
    } else {
        check(
            Some(true),
            "GPUs",
            &s.machine
                .gpus
                .iter()
                .map(|g| g.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    check(
        Some(!s.disks.is_empty()),
        "storage",
        &format!("{} disks and shares", s.disks.len()),
    );
    match rh_core::sibling::switchyard() {
        Some(y) => check(
            Some(true),
            "Switchyard",
            &format!("running, {} tasks; the drawer links to its board", y.tasks),
        ),
        None => check(
            None,
            "Switchyard",
            "not running (optional; the drawer links to its board when it is)",
        ),
    }
    Ok(if failed { 1 } else { 0 })
}
