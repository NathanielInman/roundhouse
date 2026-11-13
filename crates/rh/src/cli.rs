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
  rh session [on|off|toggle]          a recording session: audio dropouts and loud
                                      fans light the lamp (no argument: show it)
  rh status [--json] [--show-ip]      print what the drawer shows (the public
                                      IP stays hidden without --show-ip)
  rh history [--since 24h] [--json]   what the lamp said, and when
  rh doctor [--audio]                 check the setup (--audio: is it fit to record?)
  rh daemon                           run the daemon in the foreground
  rh restart                          restart the daemon and the drawer (after an
                                      upgrade they do it on their own)
  rh --version

Tabs: machine, storage, network, ports.
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
        Some("restart") => restart(),
        Some("bar") => bar(),
        Some(cmd @ ("toggle" | "show")) => open(cmd == "toggle", rest),
        Some("hide") => {
            if rh_core::transport::daemon_alive() {
                ensure_daemon()?;
                send(&Request::Hide)?;
            }
            Ok(0)
        }
        Some("status") => status(flag(rest, "--json"), flag(rest, "--show-ip")),
        Some("session") => session(rest.first().map(String::as_str)),
        Some("history") => history(
            value(rest, "--since").as_deref().unwrap_or("24h"),
            flag(rest, "--json"),
        ),
        Some("doctor") if flag(rest, "--audio") => doctor_audio(),
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

/// Asks the daemon to quit and waits until that daemon is gone (its end of
/// the socket closes). Another may already be starting: the bar module
/// starts one as soon as its stream ends. False when the daemon refused (a
/// daemon from before `quit` does not know it).
fn stop_daemon() -> bool {
    use std::io::{BufRead, BufReader};
    let Ok(mut s) =
        rh_core::transport::connect_to(&paths::socket_path(), Some(Duration::from_secs(2)))
    else {
        return true;
    };
    let line = serde_json::to_string(&Request::Quit).unwrap_or_default();
    if writeln!(s, "{line}").is_err() {
        return true;
    }
    let mut lines = BufReader::new(s).lines();
    let ok = match lines.next() {
        Some(Ok(reply)) => {
            serde_json::from_str::<rh_core::protocol::Reply>(&reply).is_ok_and(|r| r.ok)
        }
        _ => true,
    };
    // The daemon exits right after its reply; wait for the close.
    ok && lines.all(|l| l.is_ok())
}

fn restart() -> Result<i32> {
    if rh_core::transport::daemon_alive() && !stop_daemon() {
        return Err(anyhow!(
            "the running daemon did not quit; stop it with `pkill -f 'rh daemon'`"
        ));
    }
    start_daemon()?;
    println!("rh: daemon restarted; the drawer starts on the next click");
    Ok(0)
}

/// Starts the daemon in the background when it is not running, and waits
/// for its socket. A daemon whose binary an upgrade replaced is swapped for
/// the new one first, so the command does not reach old code.
fn ensure_daemon() -> Result<()> {
    if rh_core::transport::daemon_alive() {
        let stale = rh_core::protocol::request_within(&Request::Ping, Duration::from_secs(1))
            .is_ok_and(|r| r.stale);
        if !stale || !stop_daemon() {
            return Ok(());
        }
    }
    start_daemon()
}

/// A daemon answers a ping. A bare connect is not enough: a daemon that is
/// exiting can still accept for a moment.
fn daemon_answers() -> bool {
    rh_core::protocol::request_within(&Request::Ping, Duration::from_millis(500)).is_ok()
}

/// Starts a daemon in the background and waits until one answers.
///
/// A new daemon gives up when another holds the lock. That one may be
/// starting (it answers soon), or it may be the one that just quit: its
/// socket closes a moment before the kernel releases its lock. So a daemon
/// that gave up while nobody answers is started again.
fn start_daemon() -> Result<()> {
    let exe = paths::exe().ok_or_else(|| anyhow!("cannot find the rh binary"))?;
    let log = paths::daemon_log();
    std::fs::create_dir_all(paths::state_dir())?;
    let spawn = || -> Result<std::process::Child> {
        let out = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)?;
        let mut cmd = std::process::Command::new(&exe);
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
        Ok(cmd.spawn()?)
    };
    let mut child = spawn()?;
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        if daemon_answers() {
            std::thread::spawn(move || child.wait());
            return Ok(());
        }
        if let Ok(Some(_)) = child.try_wait() {
            child = spawn()?;
        }
    }
    Err(anyhow!("the daemon did not start; see {}", log.display()))
}

fn open(toggle: bool, args: &[String]) -> Result<i32> {
    let (cfg, _) = Config::load();
    // Ask the compositor first: the cursor is still on the icon.
    let hint = rh_core::waybar::load_layout().unwrap_or_default().hint();
    let anchor = bar_anchor::anchor(&cfg.bar.namespace, hint, flag(args, "--center"));
    let tab = value(args, "--tab");
    ensure_daemon()?;
    send(&if toggle {
        Request::Toggle { anchor, tab }
    } else {
        Request::Show { anchor, tab }
    })?;
    Ok(0)
}

/// Runs the new `rh bar` in this process (same pid, same stdout to waybar)
/// after an upgrade replaced the binary.
#[cfg(unix)]
fn reexec_bar() {
    use std::os::unix::process::CommandExt;
    if let Some(exe) = paths::exe() {
        let err = std::process::Command::new(exe).arg("bar").exec();
        eprintln!("rh: cannot restart the bar module: {err}");
    }
}

#[cfg(not(unix))]
fn reexec_bar() {}

fn bar() -> Result<i32> {
    let (cfg, _) = Config::load();
    let offline = serde_json::to_string(&crate::bar::offline(&cfg))?;
    let mut out = std::io::stdout().lock();
    let mut last = String::new();
    // Starts the daemon when it is missing (first run, a crash, an
    // upgrade), at most every 10 seconds.
    let mut started: Option<std::time::Instant> = None;
    loop {
        // An upgrade: the old daemon retired and ended the stream. Become the
        // new bar, which starts the new daemon.
        if paths::exe_replaced() {
            reexec_bar();
        }
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
                if paths::exe_replaced() {
                    continue;
                }
            }
            Err(_) if started.is_none_or(|t| t.elapsed() > Duration::from_secs(10)) => {
                started = Some(std::time::Instant::now());
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
    s.lamp = rh_core::lamp::evaluate(&s, &cfg.lamp);
    s.config_error = err;
    (s, false)
}

fn history(since: &str, json: bool) -> Result<i32> {
    let span = rh_core::parse_span(since)
        .ok_or_else(|| anyhow!("--since takes a span like 30m, 24h or 7d"))?;
    let entries = rh_core::history::read(&rh_core::history::file(), rh_core::now() - span);
    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(0);
    }
    if entries.is_empty() {
        println!("nothing in the last {since}: the lamp stayed calm");
        return Ok(0);
    }
    for e in entries {
        let mut what = e
            .reasons
            .iter()
            .map(|r| r.text.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        if e.session {
            what = if what.is_empty() {
                "recording session".into()
            } else {
                format!("{what} (recording session)")
            };
        }
        println!("{}  {:<9} {what}", rh_core::clock(e.t), e.level.as_str());
    }
    Ok(0)
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

fn status(json: bool, show_ip: bool) -> Result<i32> {
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
    let mut fans: Vec<String> = m
        .fans
        .iter()
        .map(|f| format!("{} {} RPM", f.name, f.rpm))
        .collect();
    fans.extend(
        m.gpus
            .iter()
            .filter_map(|g| g.fan.map(|f| format!("{} {f:.0}%", g.name))),
    );
    if !fans.is_empty() {
        println!("{:<12}{}", "fans", fans.join("  "));
    }
    let n = &m.network;
    let units = Config::load().0.network.units;
    let rate = |b: f64| rh_core::collect::net::rate_in(b, &units);
    let mut net = format!(
        "down {}  up {}  ({})",
        rate(n.down),
        rate(n.up),
        n.interfaces.join(", ")
    );
    // Hidden unless asked for, so a shared terminal does not give it away.
    match (&n.public_ip, &n.public_ipv6) {
        (None, None) => {}
        _ if !show_ip => net.push_str("  public IP hidden (--show-ip)"),
        (v4, v6) => {
            for ip in [v4, v6].into_iter().flatten() {
                net.push_str(&format!("  public {ip}"));
            }
        }
    }
    match n.nat_keeps_port {
        Some(true) => net.push_str("  (NAT keeps ports)"),
        Some(false) => net.push_str("  (NAT changes ports per peer)"),
        None => {}
    }
    println!("{:<12}{net}", "network");
    let reach = |r: &rh_core::model::Reach| match (r.ok, r.ms) {
        (true, Some(ms)) => format!("{} {ms:.1} ms", r.target),
        (true, None) => format!("{} (waiting)", r.target),
        (false, _) if !r.ever => format!("{} (does not answer ping)", r.target),
        (false, _) => format!("{} NOT ANSWERING", r.target),
    };
    let mut way = vec![match &n.route {
        Some(r) => format!(
            "route via {} on {}{}",
            r.gateway,
            r.iface,
            if r.ipv6 { " (+IPv6)" } else { "" }
        ),
        None if !n.probed => "route not probed yet".into(),
        None => "no default route".into(),
    }];
    if let Some(g) = &n.gateway {
        way.push(format!("router {}", reach(g)));
    }
    if let Some(i) = &n.internet {
        way.push(format!("internet {}", reach(i)));
    }
    if let Some(d) = &n.dns {
        way.push(format!("dns {}", reach(d)));
    }
    println!("{:<12}{}", "", way.join("  "));
    if let Some(t) = &n.tailscale {
        println!(
            "{:<12}Tailscale {}{}  {} of {} peers online",
            "",
            t.state,
            t.ips.first().map(|ip| format!(" {ip}")).unwrap_or_default(),
            t.peers_online,
            t.peers
        );
    }
    for t in &n.tunnels {
        println!(
            "{:<12}{} {}{}  {}",
            "",
            t.name,
            t.kind,
            t.addresses
                .first()
                .map(|a| format!(" {a}"))
                .unwrap_or_default(),
            if t.up { "up" } else { "down" }
        );
    }
    for l in &n.links {
        let mut line = l.name.clone();
        if let Some(sp) = l.speed {
            line.push_str(&format!("  {}", rh_core::collect::net::speed(sp)));
        }
        if let Some(dbm) = l.signal_dbm {
            line.push_str(&format!("  {dbm:.0} dBm"));
        }
        if l.up {
            line.push_str(&format!("  down {}  up {}", rate(l.down), rate(l.up_rate)));
        } else {
            line.push_str("  down (no link)");
        }
        if l.errors_minute > 0 {
            line.push_str(&format!("  {} errors/min", l.errors_minute));
        }
        if l.flaps_minute > 0 {
            line.push_str(&format!("  {} link drops/min", l.flaps_minute));
        }
        println!("{:<12}{line}", "");
    }
    let a = &s.audio;
    if let Some(g) = &a.graph {
        println!(
            "{:<12}PipeWire {} @ {} Hz ({:.1} ms) on {}  DSP {:.0}%  {} dropouts in the last minute",
            "audio",
            g.quantum,
            g.rate,
            g.latency_ms(),
            g.driver,
            g.load,
            g.xruns_minute
        );
    }
    for st in &a.streams {
        let mut line = format!(
            "{} {}",
            st.card,
            if st.capture { "recording" } else { "playing" }
        );
        if let Some(o) = &st.owner {
            line.push_str(&format!(" via {o}"));
        }
        if let Some(r) = st.rate {
            line.push_str(&format!("  {r} Hz"));
        }
        if !st.format.is_empty() {
            line.push_str(&format!(" {}", st.format));
        }
        if let Some(l) = st.latency_ms {
            line.push_str(&format!("  {l:.1} ms"));
        }
        println!("{:<12}{line}", if a.graph.is_some() { "" } else { "audio" });
    }
    if !a.midi.is_empty() {
        println!("{:<12}{}", "midi", a.midi.join(", "));
    }
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
            let stale = if f.stale {
                format!("  ({})", f.stale_text())
            } else if let Some(ms) = f.probe_ms {
                format!("  ({ms:.0} ms{})", if f.slow() { ", slow" } else { "" })
            } else {
                String::new()
            };
            println!(
                "    {:<24}{:>8} / {:<8}{:>4.0}%{stale}",
                format!("{}{extra}", paths_tilde(&f.mounts[0])),
                human(f.used),
                human(f.total),
                f.used_pct()
            );
        }
    }
    for h in &s.remotes {
        let state = if h.online {
            match h.reasons.first() {
                Some((_, t)) => format!("{}: {t}", h.level.as_str()),
                None => "calm".into(),
            }
        } else if h.ever {
            "not answering".into()
        } else {
            "never answered".into()
        };
        println!("{:<12}{} ({})  {state}", "remote", h.name, h.addr);
    }
    if !s.ports.is_empty() {
        println!("ports");
        for p in &s.ports {
            let who = match (&p.process, &p.cwd) {
                (Some(proc_), Some(cwd)) => format!("{proc_} in {}", paths_tilde(cwd)),
                (Some(proc_), None) => proc_.clone(),
                (None, _) if p.uid == 0 => "system".into(),
                (None, _) => format!("uid {}", p.uid),
            };
            let exp = match p.exposure.as_str() {
                "all" => "EVERY INTERFACE".to_string(),
                "address" => p.addr.clone(),
                _ => "this machine".to_string(),
            };
            println!(
                "    {:<10}{:<18}{who}",
                format!(":{}{}", p.port, if p.proto == "udp" { "/udp" } else { "" }),
                exp
            );
        }
    }
    if !s.net_apps.is_empty() {
        println!("talking out");
        for a in s.net_apps.iter().filter(|a| !a.gone).take(8) {
            let traffic = if a.down + a.up > 0.0 {
                format!("↓ {} ↑ {}", rate(a.down), rate(a.up))
            } else {
                String::new()
            };
            println!(
                "    {:<24}{traffic:<24}{}  {}",
                a.name,
                a.counts(),
                a.peers_text(2)
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

fn sys(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
}

/// Is the machine set up to record without dropouts? Every line is a fact
/// and, where it matters, the fix.
fn doctor_audio() -> Result<i32> {
    use rh_core::collect::audio;
    let mut failed = false;

    // CPU frequency: a governor that ramps slowly costs small buffers.
    let cpu = "/sys/devices/system/cpu/cpu0/cpufreq";
    let governor = sys(&format!("{cpu}/scaling_governor"));
    let epp = sys(&format!("{cpu}/energy_performance_preference"));
    let driver = sys(&format!("{cpu}/scaling_driver")).unwrap_or_default();
    match (governor.as_deref(), epp.as_deref()) {
        (Some("performance"), _) | (_, Some("performance")) => check(
            Some(true),
            "CPU frequency",
            &format!("{driver}, {} (clocks stay up)", epp.as_deref().or(governor.as_deref()).unwrap_or("-")),
        ),
        (Some(g), e) => check(
            None,
            "CPU frequency",
            &format!(
                "{driver}, governor {g}{}: fine for playback; for buffers under 128 samples set the EPP (or governor) to performance while recording",
                e.map(|e| format!(", EPP {e}")).unwrap_or_default()
            ),
        ),
        (None, _) => check(None, "CPU frequency", "no cpufreq; nothing to tune"),
    }

    // PipeWire's data thread must run realtime, or anything busy delays it.
    match audio::pipewire_rt() {
        Some(Some((thread, prio))) => check(
            Some(true),
            "PipeWire realtime",
            &format!("{thread} runs realtime at priority {prio}"),
        ),
        Some(None) => {
            failed = true;
            check(
                Some(false),
                "PipeWire realtime",
                "no realtime thread; check that rtkit is running or give your user an rtprio limit (the realtime-privileges group on Arch)",
            )
        }
        None => check(None, "PipeWire realtime", "PipeWire is not running"),
    }

    // Apps that ask for realtime themselves (a DAW on JACK or ALSA).
    let limit = sys("/proc/self/limits").and_then(|l| audio::parse_rtprio_limit(&l));
    let rtkit = std::fs::read_dir("/proc").ok().is_some_and(|rd| {
        rd.flatten().any(|e| {
            sys(&format!("{}/comm", e.path().display())).as_deref() == Some("rtkit-daemon")
        })
    });
    match (limit, rtkit) {
        (Some(n), _) if n > 0 => check(Some(true), "realtime limit", &format!("apps may use priority {n}")),
        (_, true) => check(
            Some(true),
            "realtime limit",
            "0, but rtkit grants realtime to apps that ask (a DAW that wants more needs the realtime-privileges group)",
        ),
        _ => check(
            None,
            "realtime limit",
            "0 and no rtkit: apps cannot get realtime; install realtime-privileges and add yourself to its group",
        ),
    }

    // Kernel preemption and threaded interrupts.
    let version = sys("/proc/sys/kernel/version").unwrap_or_default();
    let cmdline = sys("/proc/cmdline").unwrap_or_default();
    let has = |w: &str| cmdline.split_whitespace().any(|x| x == w);
    if version.contains("PREEMPT_RT") {
        check(Some(true), "kernel", "realtime kernel (PREEMPT_RT)");
    } else if has("preempt=full") {
        check(Some(true), "kernel", "full preemption (preempt=full)");
    } else if version.contains("PREEMPT_DYNAMIC") {
        check(
            None,
            "kernel",
            "PREEMPT_DYNAMIC without preempt=full; add preempt=full to the kernel command line for lower latency",
        );
    } else {
        check(None, "kernel", "no preemption flags found");
    }
    if has("threadirqs") || version.contains("PREEMPT_RT") {
        check(Some(true), "threaded IRQs", "on");
    } else {
        check(
            None,
            "threaded IRQs",
            "off; add threadirqs to the kernel command line so the sound card's interrupt can be prioritized",
        );
    }

    // USB audio interfaces that may autosuspend drop out mid-take.
    let usb = audio::usb_power();
    if usb.is_empty() {
        check(None, "USB audio", "no USB sound cards");
    }
    for (name, control) in usb {
        if control == "on" {
            check(Some(true), &name, "USB autosuspend off");
        } else {
            check(
                None,
                &name,
                &format!("USB power/control is {control}: it may autosuspend; a udev rule setting it to on keeps it awake"),
            );
        }
    }
    Ok(if failed { 1 } else { 0 })
}

fn doctor() -> Result<i32> {
    let r = doctor_setup();
    check(
        None,
        "audio",
        "`rh doctor --audio` checks whether the machine is fit to record",
    );
    r
}

fn doctor_setup() -> Result<i32> {
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
    match bar_anchor::compositor() {
        Some("Hyprland") => check(
            Some(true),
            "compositor",
            "Hyprland: the drawer opens from the clicked icon",
        ),
        Some(wm) => check(
            Some(true),
            "compositor",
            &format!(
                "{wm}: the drawer opens on the clicked bar, where waybar's config puts the module"
            ),
        ),
        None => check(
            None,
            "compositor",
            "not Hyprland, sway or niri; the drawer opens centered",
        ),
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
    if s.machine.fans.is_empty() {
        check(
            None,
            "fans",
            "the board reports none; its sensor chip's driver may not be loaded (often nct6775 or it87: try `sudo modprobe nct6775`, then sensors-detect)",
        );
    } else {
        check(
            Some(true),
            "fans",
            &format!("{} spinning", s.machine.fans.len()),
        );
    }
    check(
        Some(!s.disks.is_empty()),
        "storage",
        &format!("{} disks and shares", s.disks.len()),
    );
    if let Some(l) = &cfg.remote.listen {
        match rh_core::remote::listen_addr(l) {
            Ok(a) if cfg.remote.token.is_empty() => {
                failed = true;
                check(
                    Some(false),
                    "remote",
                    &format!("listening on {a} but remote.token is empty, so nothing is served"),
                )
            }
            Ok(a) => check(Some(true), "remote", &format!("serving the lamp on {a}")),
            Err(e) => {
                failed = true;
                check(Some(false), "remote", &e)
            }
        }
    }
    Ok(if failed { 1 } else { 0 })
}
