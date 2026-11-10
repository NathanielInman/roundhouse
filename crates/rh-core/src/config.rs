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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct BarConfig {
    /// The glyph the bar module shows.
    pub icon: String,
    /// The number beside the icon: `cpu.temp`, `cpu`, `memory`, `gpu.temp`,
    /// `gpu` or `none`.
    pub pin: String,
    /// The layer-shell namespace of the bar surface, used to find its edge.
    pub namespace: String,
}

impl Default for BarConfig {
    fn default() -> Self {
        BarConfig {
            icon: "\u{f04c5}".into(),
            pin: "cpu.temp".into(),
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
            tabs: vec!["machine".into(), "storage".into(), "switchyard".into()],
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    /// Show the public IP. Off by default because it asks a third party
    /// (ifconfig.co, over plain HTTP) once a minute while the drawer is open.
    pub public_ip: bool,
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
