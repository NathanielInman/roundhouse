//! `~/.config/roundhouse/config.toml`. Every key is optional; an empty or
//! missing file gives a working drawer.

use crate::model::TempLimit;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub bar: BarConfig,
    pub theme: ThemeConfig,
    pub drawer: DrawerConfig,
    pub storage: StorageConfig,
    pub lamp: LampConfig,
    pub network: NetworkConfig,
    pub siblings: SiblingsConfig,
    pub session: SessionConfig,
    pub remote: RemoteConfig,
}

/// Lamps across machines.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct RemoteConfig {
    /// Serve this machine's lamp on one address (`100.101.102.103` or
    /// `host:port`, port 7461 by default), such as its tailnet IP. Never
    /// every interface. Read once, at daemon start.
    pub listen: Option<String>,
    /// Readers must send this. Nothing is served without one.
    pub token: String,
    /// Other machines to watch.
    pub hosts: Vec<RemoteHostConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct RemoteHostConfig {
    pub name: String,
    /// `host` or `host:port`.
    pub addr: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct BarConfig {
    /// The glyph the bar module shows.
    pub icon: String,
    /// Glyphs for the other states, so the lamp reads without color (for
    /// color-blind eyes or a one-color bar). Unset, each uses `icon`.
    pub icon_heads_up: Option<String>,
    pub icon_act: Option<String>,
    pub icon_offline: Option<String>,
    /// Shown while a recording session is on.
    pub icon_session: Option<String>,
    /// The number beside the icon: `cpu.temp`, `cpu`, `memory`, `gpu.temp`,
    /// `gpu` or `none`.
    pub pin: String,
    /// The layer-shell namespace of the bar surface, used to find its edge.
    pub namespace: String,
}

impl Default for BarConfig {
    fn default() -> Self {
        BarConfig {
            icon: "\u{f0765}".into(),
            icon_heads_up: None,
            icon_act: None,
            icon_offline: None,
            icon_session: None,
            pin: "none".into(),
            namespace: "waybar".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeConfig {
    /// `waybar` reads the bar's style.css for font and colors; `none` uses
    /// the built-in theme.
    pub from: String,
    /// Overrides the accent color (sparklines, selected tab).
    pub accent: Option<String>,
    /// Overrides the font family.
    pub font: Option<String>,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        ThemeConfig {
            from: "waybar".into(),
            accent: None,
            font: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct DrawerConfig {
    /// Panel width in logical pixels.
    pub width: f64,
    /// Largest height as a fraction of the monitor.
    pub max_height: f64,
    /// Gap between the bar and the panel.
    pub gap: f64,
    /// Tabs in order. Unknown names are ignored; a sibling's tab only shows
    /// while that sibling is running.
    pub tabs: Vec<String>,
}

impl Default for DrawerConfig {
    fn default() -> Self {
        DrawerConfig {
            width: 460.0,
            max_height: 0.8,
            gap: 8.0,
            tabs: vec![
                "machine".into(),
                "storage".into(),
                "network".into(),
                "ports".into(),
                "switchyard".into(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct StorageConfig {
    /// Mount points to leave out. A filesystem disappears only when all of
    /// its mount points are hidden.
    pub hide: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct LampConfig {
    /// Filesystem use, in percent.
    pub disk_warn: f32,
    pub disk_act: f32,
    /// CPU package temperature, in °C. Unset, each one follows the part's
    /// own limit (see `TempLimit`).
    pub cpu_temp_warn: Option<f32>,
    pub cpu_temp_act: Option<f32>,
    /// GPU temperature, in °C.
    pub gpu_temp_warn: Option<f32>,
    pub gpu_temp_act: Option<f32>,
    /// NVMe and disk temperature, in °C.
    pub drive_temp_warn: Option<f32>,
    pub drive_temp_act: Option<f32>,
    /// Memory pressure (PSI `some` avg60), in percent of time stalled.
    pub mem_pressure_warn: f32,
    /// Memory pressure (PSI `full` avg10).
    pub mem_pressure_act: f32,
    /// Available memory under this percentage of total is a problem.
    pub mem_free_act: f32,
    /// During a recording session, a fan at or above this speed (RPM) is a
    /// heads-up: it is in the take.
    pub fan_loud_rpm: u32,
    /// The same for a GPU fan, in percent.
    pub fan_loud_pct: f32,
    /// Seconds a temperature must stay past its heads-up line before the
    /// lamp rises, so a spike does not change the light.
    pub attack: f32,
    /// Seconds a part must sit at its limit before the lamp says act. Modern
    /// CPUs boost until they reach their limit, so touching it is normal.
    pub sustain: f32,
    /// Seconds a condition must stay clear before the lamp falls.
    pub release: f32,
}

impl Default for LampConfig {
    fn default() -> Self {
        LampConfig {
            disk_warn: 85.0,
            disk_act: 95.0,
            cpu_temp_warn: None,
            cpu_temp_act: None,
            gpu_temp_warn: None,
            gpu_temp_act: None,
            drive_temp_warn: None,
            drive_temp_act: None,
            mem_pressure_warn: 10.0,
            mem_pressure_act: 10.0,
            mem_free_act: 5.0,
            fan_loud_rpm: 1600,
            fan_loud_pct: 60.0,
            attack: 10.0,
            sustain: 30.0,
            release: 30.0,
        }
    }
}

/// Degrees under a part's own limit where the lamp turns yellow.
pub const TEMP_HEADROOM: f32 = 10.0;

/// Builds the thresholds for one sensor: the keys the user set, else
/// `TEMP_HEADROOM` under the hardware's limit and the limit itself, else
/// `fallback` when the hardware reports nothing.
fn temp_limit(
    warn: Option<f32>,
    act: Option<f32>,
    hw: Option<f32>,
    fallback: TempLimit,
) -> TempLimit {
    let base = hw.map_or(fallback, |l| TempLimit {
        warn: l - TEMP_HEADROOM,
        act: l,
    });
    let act = act.unwrap_or(base.act);
    TempLimit {
        warn: warn.unwrap_or(base.warn).min(act),
        act,
    }
}

impl LampConfig {
    /// `hw` is the CPU's TjMax when known.
    pub fn cpu_temp(&self, hw: Option<f32>) -> TempLimit {
        let fallback = TempLimit {
            warn: 85.0,
            act: 95.0,
        };
        temp_limit(self.cpu_temp_warn, self.cpu_temp_act, hw, fallback)
    }

    /// `hw` is the card's slowdown temperature when known.
    pub fn gpu_temp(&self, hw: Option<f32>) -> TempLimit {
        let fallback = TempLimit {
            warn: 83.0,
            act: 90.0,
        };
        temp_limit(self.gpu_temp_warn, self.gpu_temp_act, hw, fallback)
    }

    /// `hw` is the drive's warning temperature when known.
    pub fn drive_temp(&self, hw: Option<f32>) -> TempLimit {
        let fallback = TempLimit {
            warn: 65.0,
            act: 75.0,
        };
        temp_limit(self.drive_temp_warn, self.drive_temp_act, hw, fallback)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    /// Show the public IP (and IPv6, and how the NAT maps ports). Off by
    /// default because it asks third parties
    /// (the `stun` servers) once a minute while the drawer is open.
    pub public_ip: bool,
    /// STUN servers to ask (`host:port`). Two let the drawer say how the NAT
    /// maps ports.
    pub stun: Vec<String>,
    /// `bytes` (kB/s, the default) or `bits` (Mb/s, like link speeds).
    pub units: String,
    /// Also check past the router: an ICMP echo to `probe_host` and a DNS
    /// lookup of `probe_name`, every 10 s while the drawer is open and every
    /// 30 s otherwise. Off by default: it talks to the internet.
    pub internet_check: bool,
    pub probe_host: String,
    pub probe_name: String,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        NetworkConfig {
            public_ip: false,
            stun: vec![
                "stun.cloudflare.com:3478".into(),
                "stun.l.google.com:19302".into(),
            ],
            units: "bytes".into(),
            internet_check: false,
            probe_host: "1.1.1.1".into(),
            probe_name: "example.com".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SessionConfig {
    /// Start a recording session on its own while an app records from a
    /// sound card, and end it 30 s after the recording stops.
    pub auto: bool,
    /// Only these cards start one (names as in `rh status`, such as
    /// "Yeti X"). Empty means any card, a webcam's microphone included.
    pub cards: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct SiblingsConfig {
    /// Look for a running Switchyard and show its main line.
    pub switchyard: bool,
}

impl Default for SiblingsConfig {
    fn default() -> Self {
        SiblingsConfig { switchyard: true }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// Loads the config file. A broken file falls back to the defaults and
    /// returns the error, so the drawer still opens and can say what is wrong.
    pub fn load() -> (Config, Option<String>) {
        let path = crate::paths::config_file();
        match std::fs::read_to_string(&path) {
            Ok(text) => match Config::parse(&text) {
                Ok(c) => (c, None),
                Err(e) => (Config::default(), Some(format!("{}: {e}", path.display()))),
            },
            Err(_) => (Config::default(), None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_default() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
    }

    #[test]
    fn partial_tables() {
        let c =
            Config::parse("[bar]\npin = \"gpu.temp\"\n[storage]\nhide = [\"/swap\"]\n").unwrap();
        assert_eq!(c.bar.pin, "gpu.temp");
        assert_eq!(c.bar.namespace, "waybar");
        assert_eq!(c.storage.hide, vec!["/swap".to_string()]);
        assert_eq!(c.lamp.disk_warn, 85.0);
    }

    #[test]
    fn temperatures_follow_the_hardware() {
        let t = LampConfig::default();
        // A Samsung 990 PRO reports an 81.85°C warning temperature.
        assert_eq!(
            t.drive_temp(Some(81.85)),
            TempLimit {
                warn: 71.85,
                act: 81.85
            }
        );
        // Nothing reported: the old fixed thresholds.
        assert_eq!(
            t.gpu_temp(None),
            TempLimit {
                warn: 83.0,
                act: 90.0
            }
        );
        // A key the user set wins, and warn never passes act.
        let t = Config::parse("[lamp]\ncpu_temp_act = 80\n").unwrap().lamp;
        assert_eq!(
            t.cpu_temp(Some(95.0)),
            TempLimit {
                warn: 80.0,
                act: 80.0
            }
        );
    }

    #[test]
    fn typos_are_errors() {
        assert!(Config::parse("[bar]\npinn = \"cpu\"\n").is_err());
    }
}
