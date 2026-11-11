# Roundhouse design

Roundhouse is a system drawer for a Wayland desktop. The bar shows one lamp;
clicking it opens a panel that springs out of that icon and shows what the
machine is doing. This document is the reference for how it works and why.
The README is the tour; this is the spec.

## Goals

1. **One lamp.** The bar module is a single dot (and a pinned number only
   if you ask for one). Its color says whether anything needs you; the
   tooltip and the drawer say why.
2. **Answers, not numbers.** Each tab answers one question ("why is it
   slow?", "where did my space go?"). Raw counters are there to back the
   answer up.
3. **Zero setup.** Disks, shares, sensors and GPUs are discovered. The theme
   comes from the bar.
4. **Cheap when closed.** The daemon never starts subprocesses and samples
   only the lamp's inputs while the drawer is hidden. It talks to the
   internet only when asked to (`network.public_ip`,
   `network.internet_check`).
5. **Siblings, not dependencies.** Switchyard and Roundhouse each unlock
   features in the other when both run, and neither needs the other.

## Non-goals

- **Not a bar.** Roundhouse is one module in the bar you already have. It
  never draws a bar of its own and never owns more than one icon.
- **Not a process manager.** It shows memory by app and will offer a few
  actions; btop stays the tool for digging.
- **Not a package manager.** The planned Arch tab tells you what an update
  needs and hands you the command. It never runs pacman.

## Architecture

```mermaid
flowchart LR
  K["/proc, /sys, NVML"] --> D["rh daemon"]
  D <-- "local socket, JSON lines" --> B["rh-board (GTK4 layer-shell)"]
  D -- socket --> W["rh bar<br/>waybar module"]
  D <-- socket --> C["rh CLI"]
  D -. "sibling, optional" .-> Y["Switchyard daemon"]
  B -. "handoff, optional" .-> Y
  C -. "anchor from" .-> H["Hyprland IPC"]
```

| Crate | Role |
|---|---|
| `bar-anchor` | Hyprland anchoring and panel placement. No GTK. Started as Switchyard's code and keeps its wire format, so the two can share it. |
| `rh-core` | Collectors, the snapshot model, lamp rules, config, the socket protocol, the waybar reader and the Switchyard client. No UI. |
| `rh` | The `rh` binary: daemon, CLI and waybar module. |
| `rh-board` | The Linux drawer: GTK4 on `gtk4-layer-shell`, overlay layer, keyboard exclusive while open. |

## Paths

| What | Where | Override |
|---|---|---|
| Config | `~/.config/roundhouse/config.toml` | `ROUNDHOUSE_CONFIG_DIR` |
| User stylesheet | `~/.config/roundhouse/style.css` | `ROUNDHOUSE_CONFIG_DIR` |
| Logs | `~/.local/state/roundhouse/` | `ROUNDHOUSE_STATE_DIR` |
| Socket | `$XDG_RUNTIME_DIR/roundhouse/rh.sock` | `ROUNDHOUSE_RUNTIME_DIR` |

The runtime directory is mode 0700 and the socket 0600.

## Sampling

The daemon samples on a timer: every second while the drawer reports it is
visible, every five seconds otherwise. A visibility change wakes the sampler
at once, so the drawer never opens on old data for long.

| Data | Source | When |
|---|---|---|
| CPU usage | `/proc/stat` deltas | every sample |
| CPU model, clock | `/proc/cpuinfo` | first sample, then while open |
| Temperatures | `/sys/class/hwmon`: `k10temp`/`zenpower` Tctl, `coretemp` package, `nvme`/`drivetemp` Composite mapped to block devices, `amdgpu` edge mapped to PCI devices; each with its limit (see the lamp) | every sample |
| Memory, swap | `/proc/meminfo` | every sample |
| Fans | `/sys/class/hwmon/*/fan*_input` (spinning ones; label or "chip fanN"), NVML fan percent | every sample |
| Pressure | `/proc/pressure/{cpu,memory,io}` | every sample |
| NVIDIA | NVML, loaded at runtime once | every sample |
| AMD | `/sys/class/drm/card*/device` (`gpu_busy_percent`, `mem_info_vram_*`); names from `pci.ids` | every sample |
| Network | `/sys/class/net/*/statistics`, `speed` and `carrier_changes` for interfaces with a `device` link (no loopback, bridges or veths), counted per interface so a replugged NIC never spikes the total; `/proc/net/wireless` for Wi-Fi signal | every sample |
| Way out | `/proc/net/route` and `ipv6_route` for the default route; an ICMP echo to the gateway on an unprivileged ping socket; opt-in (`network.internet_check`), an echo to `probe_host` and a lookup of `probe_name` through the system resolver; each with a 2 s deadline and failed only after two misses in a row | on a helper thread, every 10 s open, 30 s closed |
| Tunnels | interfaces with no `device` link that are WireGuard (`DEVTYPE=wireguard`), tun (`tun_flags`) or PPP (type 512), with addresses from `getifaddrs`; listed apart from throughput, since their traffic also crosses a real link | every sample |
| Tailscale | `GET /localapi/v0/status` on tailscaled's socket, when it exists, with a 2 s deadline: state, its addresses, peers online | with the way-out probes |
| Public IP | a STUN binding request (RFC 5389) to each `network.stun` server over IPv4 from one socket, and to the first over IPv6, on a helper thread; the reply must carry our random transaction ID. Two servers seeing the same port means the NAT keeps one mapping per socket. Opt-in with `network.public_ip` | while open, at most once a minute |
| Storage | `/proc/self/mountinfo` and `statvfs` | while open, else every 30 s |
| Memory by app | `/proc/*/cgroup` and cgroup v2 files | while open only |
| Ports | `/proc/net/{tcp,tcp6}` listeners and `/proc/net/{udp,udp6}` bound service sockets (below 32768, no peer); socket inodes matched to `/proc/*/fd` for the process, `/proc/<pid>/cwd` for its directory; exposure from the bound address (`0.0.0.0`/`::` every interface, loopback, or one address); IPv4 and IPv6 fold together | while open, every 5 s |
| Audio streams | `/proc/asound/cards`, `card*/pcm*/sub*/status` and `hw_params` (running streams, rate, format, owner); `seq/clients` (MIDI devices) | every sample |
| PipeWire | its native socket (`$XDG_RUNTIME_DIR/pipewire-0`), spoken directly: bind the profiler, read quantum, DSP load and xrun counts | on a helper thread while open or during a recording session |
| Switchyard | its socket | every 2 s open, 15 s closed |

Sparklines keep the last 60 samples, and `machine.times` says when each was
taken. The drawer draws them against time over the last minute, so the
five-second samples from while it was closed sit five seconds apart, not one
slot. Ticks keep a fixed rate: the next one is due an interval after the last
was due, not after the work finished.

### Storage rules

- Real filesystems only (ext4, btrfs, xfs, f2fs, vfat, exfat, ntfs, zfs,
  bcachefs and friends) plus network ones (cifs, nfs, sshfs, rclone, 9p).
  Anything under `/proc`, `/sys`, `/dev`, `/run/user`, container storage and
  snaps is skipped.
- Mounts group by source device, so btrfs subvolumes and bind mounts are one
  filesystem. The main mount is `/`, else a mount that shows the
  filesystem's own root (`/scratch`, not its `/tmp` bind), else the shortest
  path.
- Filesystems group into their physical disk through device-mapper slaves and
  partition parents. The disk's model comes from `device/model`.
- Network filesystems are sized on helper threads, all under one 300 ms
  deadline, and each answer records how long it took (over 300 ms is
  "slow"). A share that keeps its thread busy past two seconds is marked
  "not answering" and keeps its last known size. It never counts toward the
  lamp. While it is stale, its server (`addr=` from the mount options, else
  the host in the source) is pinged every 10 s, so the drawer can say "not
  answering, but rime is up" (the share is the problem) or "rime is
  unreachable" (the server or the network is).
- An `autofs` mount point with nothing mounted on it is listed as "on
  demand" and is never touched.

### Memory by app

A process's cgroup decides how it is counted:

- `app-*.scope` or `app-*.service` (apps started by the desktop or uwsm): the
  cgroup's `memory.current` minus `inactive_file`. The name comes from the
  unit: `app-com.google.Chrome-910.scope` is "Chrome".
- `run-*.scope` (from `systemd-run`, which `lowprio` uses): the same, plus
  `memory.max` as the cap.
- Other `*.service` cgroups: the same, named after the service.
- Anything else (the compositor's session scope): RSS per process name.

Names merge case-insensitively, so Chrome's scope and stray `chrome`
processes in the session scope are one line.

## The lamp

`rh_core::lamp::evaluate` turns a snapshot into a level and a list of
reasons, most urgent first. Each reason names the tab that explains it, and
the drawer shows reasons as links to that tab.

| Rule | Heads-up | Act |
|---|---|---|
| Filesystem use | `disk_warn` (85%) | `disk_act` (95%) |
| Drive temperature | `drive_temp_warn` (limit - 10°C) | `drive_temp_act` (limit) |
| CPU temperature | `cpu_temp_warn` (limit - 10°C) | `cpu_temp_act` (limit) |
| GPU temperature | `gpu_temp_warn` (limit - 10°C) | `gpu_temp_act` (limit) |
| Memory pressure | PSI `some` avg60 at `mem_pressure_warn` (10) | PSI `full` avg10 at `mem_pressure_act` (10) |
| Free memory | | under `mem_free_act` (5%) |
| Audio dropouts | any PipeWire xrun in the last minute, during a recording session only | |
| Link speed | a link up below the best speed it negotiated since the daemon started (a bad cable) | |
| Link errors | 10 receive or transmit errors in a minute (drops do not count; every LAN has them) | |
| Link flaps | 2 down-and-up cycles in a minute | |
| Way out | the first hop that fails: no default route while a link is up; the router not answering (only one that has answered before: some block ping); the internet not answering; DNS failing | |
| Tailscale | tailscaled says `NeedsLogin` | |
| Loud fans | a fan at `fan_loud_rpm` (1600) or a GPU fan at `fan_loud_pct` (60%), during a recording session only | |

Read-only filesystems, stale shares and automount points never count.

A part that reports thermal throttling raises act at once: Intel's
`package_throttle_count` rising between samples, or NVML's thermal slowdown
reasons on an NVIDIA card.

### Envelope

`evaluate` is pure and judges one snapshot. The daemon runs its result
through `LampState`, which gives the light an envelope so it does not
chatter:

| Stage | Rule |
|---|---|
| Hysteresis | A level holds until its value falls a margin under the line that raised it: 3°C for temperatures, one point of disk use, two points of pressure, one point of free memory. |
| Attack | A temperature must stay past its heads-up line for `attack` seconds (10) before the lamp rises, and at its limit for `sustain` seconds (30) before it says act. CPUs boost until they reach their limit, so touching it is normal; sitting there is throttling. Disk use, PSI and throttle reports are already averaged or explicit and rise at once. |
| Release | A condition must stay clear for `release` seconds (30) before the lamp falls, one level at a time. |

Each reason carries a `key` naming its rule (`cpu.temp`, `fs:/data`,
`drive:nvme0n1`, `gpu:RTX 3080`, `mem.pressure`), stable across samples.
`rh status` without a daemon has no history and shows the bare rules.

### Temperature limits

A safe temperature depends on the part: a Ryzen 9000 boosts until 95°C by
design, a Samsung 990 PRO warns at 82°C and a Sabrent at 75°C. So each
sensor's thresholds come from the point where that part protects itself
(throttles), as the hardware reports it:

| Sensor | Limit |
|---|---|
| NVMe | `temp1_max` on the Composite sensor (WCTEMP), else `temp1_crit` |
| SATA (`drivetemp`) | `temp1_max`, else `temp1_crit` |
| Intel (`coretemp`) | the package sensor's `crit` (TjMax) |
| AMD CPU (`k10temp`, which reports none) | AMD's TjMax by model: 89°C for the Ryzen 7000 X3D parts, 90°C for the 5800X3D family and the 5700X to 5950X; otherwise the 95°C fallback |
| NVIDIA | NVML's slowdown temperature |
| AMD GPU | the edge sensor's `crit` |

Heads-up is 10°C under the limit and act is the limit. A value outside
40-150°C is not a limit (drives report 65261°C on their extra sensors). A
part that reports nothing gets the old fixed thresholds: CPU 85/95, GPU
83/90, drive 65/75. A `[lamp]` key the user sets wins over all of this.

Every client shows a temperature against its act line ("58 / 95°C"), the
drawer colors it green, amber or red by where it sits, and a lamp reason
names the limit ("CPU is at 87°C (limit 95°C)"), so the number
answers whether it is fine without a lookup.

## Anchoring

Waybar does not publish module geometry, so `rh toggle` asks the compositor
while the cursor is still on the icon. Over Hyprland's IPC socket (no
`hyprctl` processes) it reads the cursor position, the monitor under it and
its reserved edges, and the bar's layer surface (namespace `waybar` by
default, `bar.namespace` in the config) to tell which edge the bar is on. The
result is an `Anchor`, sent with the toggle request.

The drawer covers the monitor with a transparent layer surface and places
the panel next to the bar's edge, centered on the click and clamped to the
screen (`bar_anchor::place`). The open spring scales from the click point.
Tiles follow with a stagger. Clicking outside the panel closes it. Without
Hyprland, or with `--center`, the panel opens centered.

## Theme

With `theme.from = "waybar"` the drawer reads waybar's `style.css`: the first
font family on `*` or `window#waybar`, the background and text color of
`window#waybar`, the first border color on a module rule, the background of
the first `:hover` rule, and every `@define-color` (an `accent` or `frost`
color becomes the drawer's accent). From those it derives the panel, dim
text, tile and border tokens. Layers, lowest first: built-in
`style.css`, the bar-derived tokens, the user's `~/.config/roundhouse/style.css`.
The drawer watches all three files and the config and restyles on change.

## Siblings

Switchyard and Roundhouse are separate products that know about each other.
The rules:

1. **Neither requires the other.** Every sibling feature is hidden when the
   sibling is absent. No error, no log line, no delay longer than the
   300 ms connect deadline.
2. **Discovery is the socket.** A sibling is present when its socket exists
   and answers. Switchyard's is `$SWITCHYARD_RUNTIME_DIR/yard.sock`, else
   `$XDG_RUNTIME_DIR/switchyard/yard.sock`. Roundhouse's is
   `$ROUNDHOUSE_RUNTIME_DIR/rh.sock`, else
   `$XDG_RUNTIME_DIR/roundhouse/rh.sock`.
3. **No shared crates on the wire.** Each side reads the other's JSON loosely
   (unknown fields ignored, missing fields defaulted), so they can be
   upgraded in any order. `bar-anchor` may be shared as a library, but the
   `Anchor` JSON is the contract.
4. **Handoff keeps the anchor.** When one opens the other, it sends the same
   `Anchor` it opened with, so the second panel grows out of the same icon.
5. **One panel at a time.** Whichever panel opens sends `{"cmd":"hide"}` to
   the other, so clicking the other icon in the bar (or the other's link)
   never leaves both open.
6. **Links, not pages.** Each panel's tab row ends with a link to the other
   (`● Switchyard ↗` here, `● Roundhouse ↗` on the board) while the other
   runs. The dot carries the other's state; a click or the link's number key
   closes this panel and opens the other with the same anchor. Tab and
   Shift+Tab skip the link, so cycling never leaves the panel.

What Roundhouse uses from Switchyard today:

| Request | Used for |
|---|---|
| `{"cmd":"snapshot"}` | The link's dot: `aggregate.phase`, a CSS class on `.yard-dot` (blue while queued, waiting, building or running, green on ok, red on failed). The one-line status on the Machine tab and in the bar tooltip, while a heavy build runs or waits: `mainline.current`, `mainline.queue`, `aggregate.building`, `tasks[].id`, `tasks[].label`, and the queue head's `tasks[].phase` and `message` ("held: Roundhouse says ..."). |
| `{"cmd":"show","anchor":{...}}` | The Switchyard link and the status line. `rh toggle --tab switchyard` asks for it directly while Switchyard runs. |

What Switchyard uses from Roundhouse:

| Request | Used for |
|---|---|
| `{"cmd":"snapshot"}` on `rh.sock` | `lamp.level`, `lamp.reasons[].level`/`text` and `lamp.hold`. While `hold` is set, Switchyard holds the next heavy build with it as the message and starts the build once it clears (`mainline.respect_roundhouse` in Switchyard's config). A Switchyard that predates `hold` holds on `act`. |
| `{"cmd":"hide"}` on `rh.sock` | Sent when the board opens, so the drawer closes. |
| `{"cmd":"show","anchor":{...}}` on `rh.sock` | The board's Roundhouse link, whose dot is `lamp.level`. |

The `lamp` shape (`level`, `hold`, and `reasons` with `level`, `text`,
`tab` and `key`) is
part of the sibling contract: change it only by adding fields.

## Recording sessions

`rh session on` starts a recording session and `rh session off` ends it (the
drawer's session line ends it too). While one is on, `lamp.hold` is
"a recording session is on", so Switchyard starts no heavy build, and the
lamp keeps its color: a session is not an alarm. The session is kept in
`$ROUNDHOUSE_STATE_DIR/session.json`, so it survives a daemon restart.

`lamp.hold` is how Roundhouse tells a sibling that heavy work should wait.
It is set by a session, or by the first `act` reason that more work would
make worse (a filesystem, memory). Heat is never a hold: a build heats the
CPU, and a part at its limit protects itself. Neither is the network. The field is always sent
(`null` when clear), so a sibling can tell a calm answer from an older
Roundhouse.

### Audio

`collect/audio.rs` reads `/proc/asound`: each running PCM substream with its
card, direction, rate, format and owning process, and the hardware MIDI
clients. When an app holds a device itself (a DAW on raw ALSA), one period is
its latency; behind PipeWire, ALSA's buffer says nothing about what an app
hears, so the drawer shows PipeWire's quantum instead.

`collect/pipewire.rs` speaks PipeWire's native protocol over its socket with
no library: `Core.Hello`, `Client.UpdateProperties`, `Core.GetRegistry`, then
`Registry.Bind` on the `PipeWire:Interface:Profiler` global, answering
`Core.Ping` with `Pong`. Each profile event carries the driver's clock
(quantum and rate), its DSP load and its running xrun count; xruns in the last
minute are the deltas. Binding the profiler turns profiling on in the graph,
so the watcher connects only while the drawer is open or a session is on and
hangs up otherwise. Without PipeWire the graph line is hidden.

With `session.auto`, a recording app (a capture stream on a card listed in
`session.cards`, or any card when the list is empty) starts a session, which
ends 30 s after the recording stops. `rh session off` ends it until the next
recording.

## Protocol

Newline-delimited JSON on the socket, one request per line, one reply per
request. Shapes follow Switchyard's.

```json
{"cmd":"ping"}
{"cmd":"snapshot"}
{"cmd":"toggle","anchor":{"monitor":"DP-2","edge":"bottom","x":2310,"y":1428,"width":2560,"height":1440,"reserved":[0,0,0,24],"center":false},"tab":"storage"}
{"cmd":"show","anchor":null}
{"cmd":"hide"}
{"cmd":"board_visible","visible":true}
{"cmd":"subscribe","kind":"board"}
{"cmd":"subscribe","kind":"bar"}
{"cmd":"session","on":true}
```

Replies are `{"ok":true}` with optional `snapshot`, `version` or `error`.
`session` without `on` toggles, and its reply carries the snapshot after the
change.

A `board` subscription receives `{"event":"snapshot","snapshot":{...}}` on
every sample, plus `{"event":"show",...}`, `{"event":"toggle",...}` and
`{"event":"hide"}`. A `bar` subscription receives waybar JSON lines, already
formatted, only when they change. When a toggle arrives and no drawer is
subscribed, the daemon starts `rh-board` (next to `rh`, else on `PATH`) and
holds the show until it subscribes.

## Keyboard

| Key | Action |
|---|---|
| <kbd>1</kbd>-<kbd>9</kbd> | Tab by position; the Switchyard link's number opens the board |
| <kbd>Tab</kbd>, <kbd>→</kbd>, <kbd>l</kbd> | Next tab (skips the link) |
| <kbd>Shift</kbd>+<kbd>Tab</kbd>, <kbd>←</kbd>, <kbd>h</kbd> | Previous tab (skips the link) |
| <kbd>Esc</kbd> | Close |
