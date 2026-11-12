//! Reads waybar's own files so the drawer looks like part of the bar: the
//! font and colors from `style.css`, and the position from `config.jsonc`.
//!
//! These are small, forgiving parsers. Waybar accepts more CSS than they
//! understand; anything they cannot read falls back to Roundhouse's theme.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// What the drawer borrows from the bar's stylesheet.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BarTheme {
    pub font: Option<String>,
    pub background: Option<String>,
    pub foreground: Option<String>,
    /// The first border color on a module rule.
    pub border: Option<String>,
    /// The background of the first `:hover` rule.
    pub hover: Option<String>,
    /// Every `@define-color`.
    pub colors: BTreeMap<String, String>,
}

/// What the drawer borrows from the bar's config.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BarLayout {
    pub position: Option<String>,
    pub height: Option<u32>,
    pub layer: Option<String>,
    /// Where the module that runs `rh bar` sits along the bar: 0 in
    /// `modules-left`, 0.5 in `modules-center`, 1 in `modules-right`.
    pub along: Option<f64>,
}

impl BarLayout {
    /// What `bar_anchor` needs to open the drawer from this bar under a
    /// compositor that cannot say where the cursor is (sway, niri).
    pub fn hint(&self) -> bar_anchor::BarHint {
        let edge = match self.position.as_deref() {
            Some("bottom") => bar_anchor::Edge::Bottom,
            Some("left") => bar_anchor::Edge::Left,
            Some("right") => bar_anchor::Edge::Right,
            // Waybar's default.
            _ => bar_anchor::Edge::Top,
        };
        bar_anchor::BarHint {
            edge: Some(edge),
            size: self.height.map(f64::from),
            along: self.along,
        }
    }
}

pub fn style_path() -> PathBuf {
    crate::paths::waybar_dir().join("style.css")
}

/// `config.jsonc`, then `config`, as waybar looks for them.
pub fn config_path() -> Option<PathBuf> {
    let dir = crate::paths::waybar_dir();
    ["config.jsonc", "config"]
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.exists())
}

pub fn load_theme() -> Option<BarTheme> {
    std::fs::read_to_string(style_path())
        .ok()
        .map(|t| parse_style(&t))
}

pub fn load_layout() -> Option<BarLayout> {
    std::fs::read_to_string(config_path()?)
        .ok()
        .map(|t| parse_config(&t))
}

fn strip_css_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        match rest[i + 2..].find("*/") {
            Some(j) => rest = &rest[i + 2 + j + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// The first color token in a CSS value: a hex color, `rgb()`/`rgba()`, or
/// an `@name` reference resolved against `colors`.
fn first_color(value: &str, colors: &BTreeMap<String, String>) -> Option<String> {
    for word in value.split_whitespace() {
        let w = word.trim_end_matches([';', ',']);
        if w.starts_with('#') && w.len() >= 4 && w[1..].chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(w.to_string());
        }
        if let Some(name) = w.strip_prefix('@') {
            if let Some(c) = colors.get(name) {
                return Some(c.clone());
            }
        }
    }
    let lower = value.to_ascii_lowercase();
    for f in ["rgba(", "rgb("] {
        if let Some(i) = lower.find(f) {
            if let Some(j) = value[i..].find(')') {
                return Some(value[i..i + j + 1].to_string());
            }
        }
    }
    None
}

pub fn parse_style(text: &str) -> BarTheme {
    let css = strip_css_comments(text);
    let mut theme = BarTheme::default();

    for line in css.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("@define-color") {
            let mut it = rest.trim().splitn(2, char::is_whitespace);
            if let (Some(name), Some(value)) = (it.next(), it.next()) {
                theme.colors.insert(
                    name.to_string(),
                    value.trim().trim_end_matches(';').trim().to_string(),
                );
            }
        }
    }

    for block in css.split('}') {
        let Some((sel, body)) = block.split_once('{') else {
            continue;
        };
        // Statements before the selector (`@define-color x y;`) are not part of it.
        let sel = sel.rsplit(';').next().unwrap_or(sel);
        let selectors: Vec<&str> = sel
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.starts_with('@'))
            .collect();
        if selectors.is_empty() {
            continue;
        }
        let decls: Vec<(String, String)> = body
            .split(';')
            .filter_map(|d| d.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        let get = |name: &str| {
            decls
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        };

        if selectors.contains(&"*") || selectors.contains(&"window#waybar") {
            if let Some(f) = get("font-family") {
                theme.font.get_or_insert(f);
            }
        }
        if selectors.contains(&"window#waybar") {
            if let Some(c) = get("background")
                .or_else(|| get("background-color"))
                .and_then(|v| first_color(&v, &theme.colors))
            {
                theme.background = Some(c);
            }
            if let Some(c) = get("color").and_then(|v| first_color(&v, &theme.colors)) {
                theme.foreground = Some(c);
            }
        }
        if theme.border.is_none()
            && selectors
                .iter()
                .any(|s| s.starts_with('#') && !s.starts_with("window"))
        {
            theme.border = decls
                .iter()
                .filter(|(k, _)| {
                    k.starts_with("border") && !k.contains("radius") && !k.contains("width")
                })
                .find_map(|(_, v)| first_color(v, &theme.colors));
        }
        if theme.hover.is_none() && selectors.iter().any(|s| s.ends_with(":hover")) {
            theme.hover = get("background")
                .or_else(|| get("background-color"))
                .and_then(|v| first_color(&v, &theme.colors));
        }
    }

    // A font list keeps its first family: "PragmataPro Liga", monospace.
    theme.font = theme.font.and_then(|f| {
        let first = f
            .split(',')
            .next()?
            .trim()
            .trim_matches(['"', '\''])
            .to_string();
        (!first.is_empty()).then_some(first)
    });
    theme
}

/// Removes `//` and `/* */` comments and trailing commas, leaving strings
/// alone, so serde_json can read waybar's JSONC.
pub fn strip_jsonc(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut in_str = false;
    while i < c.len() {
        let ch = c[i];
        let next = c.get(i + 1).copied();
        if in_str {
            out.push(ch);
            if ch == '\\' {
                if let Some(n) = next {
                    out.push(n);
                    i += 1;
                }
            } else if ch == '"' {
                in_str = false;
            }
            i += 1;
        } else if ch == '"' {
            in_str = true;
            out.push(ch);
            i += 1;
        } else if ch == '/' && next == Some('/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch == '/' && next == Some('*') {
            i += 2;
            while i < c.len() && !(c[i] == '*' && c.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
        } else if ch == ','
            && c[i + 1..]
                .iter()
                .find(|x| !x.is_whitespace())
                .is_some_and(|x| *x == '}' || *x == ']')
        {
            i += 1;
        } else {
            out.push(ch);
            i += 1;
        }
    }
    out
}

pub fn parse_config(text: &str) -> BarLayout {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&strip_jsonc(text)) else {
        return BarLayout::default();
    };
    // A config may hold one bar or a list of bars: the one with our module
    // wins, else the first.
    let bars = match v {
        serde_json::Value::Array(a) => a,
        other => vec![other],
    };
    let found = bars
        .iter()
        .find_map(|b| module_along(b).map(|along| (b, along)));
    let (bar, along) = match found {
        Some((b, along)) => (b.clone(), Some(along)),
        None => (bars.first().cloned().unwrap_or_default(), None),
    };
    BarLayout {
        position: bar
            .get("position")
            .and_then(|p| p.as_str())
            .map(String::from),
        height: bar.get("height").and_then(|h| h.as_u64()).map(|h| h as u32),
        layer: bar.get("layer").and_then(|l| l.as_str()).map(String::from),
        along,
    }
}

/// Where in `bar` the module that runs `rh bar` sits, by the list that
/// names it (directly or inside a `group/`).
fn module_along(bar: &serde_json::Value) -> Option<f64> {
    [
        ("modules-left", 0.0),
        ("modules-center", 0.5),
        ("modules-right", 1.0),
    ]
    .into_iter()
    .find(|(list, _)| names_our_module(bar, bar.get(*list), 0))
    .map(|(_, along)| along)
}

fn names_our_module(bar: &serde_json::Value, list: Option<&serde_json::Value>, depth: u8) -> bool {
    let names = list.and_then(|l| l.as_array()).into_iter().flatten();
    names.filter_map(|n| n.as_str()).any(|name| {
        let Some(module) = bar.get(name) else {
            return false;
        };
        runs_our_bar(module)
            || (depth < 4
                && name.starts_with("group/")
                && names_our_module(bar, module.get("modules"), depth + 1))
    })
}

/// The module's `exec` is `rh bar`, by any path.
fn runs_our_bar(module: &serde_json::Value) -> bool {
    let exec = module
        .get("exec")
        .and_then(|e| e.as_str())
        .unwrap_or_default();
    let mut words = exec.split_whitespace();
    let program = words.next().unwrap_or_default();
    let program = program.rsplit('/').next().unwrap_or_default();
    program == "rh" && words.next() == Some("bar")
}

#[cfg(test)]
mod tests {
    use super::*;

    const STYLE: &str = r#"
/* Nord-themed Waybar */
@define-color frost #88c0d0;
* {
  font-family: "PragmataPro Liga", monospace;
  font-size: 18px;
}
window#waybar {
  background: #282A2E;
  color: #C5C8C6;
}
window#waybar.hidden { opacity: 0.2; }
#workspaces,
#cpu {
  padding: 0 6px;
  border-right: 1px solid #4c566a;
}
#cpu:hover { background: #373B41; }
#clock { color: @frost; }
"#;

    #[test]
    fn reads_a_nord_bar() {
        let t = parse_style(STYLE);
        assert_eq!(t.font.as_deref(), Some("PragmataPro Liga"));
        assert_eq!(t.background.as_deref(), Some("#282A2E"));
        assert_eq!(t.foreground.as_deref(), Some("#C5C8C6"));
        assert_eq!(t.border.as_deref(), Some("#4c566a"));
        assert_eq!(t.hover.as_deref(), Some("#373B41"));
        assert_eq!(t.colors.get("frost").map(String::as_str), Some("#88c0d0"));
    }

    #[test]
    fn resolves_named_and_rgba_colors() {
        let t = parse_style("@define-color bg rgba(10, 20, 30, 0.9);\nwindow#waybar { background-color: @bg; color: rgb(200,200,200); }");
        assert_eq!(t.background.as_deref(), Some("rgba(10, 20, 30, 0.9)"));
        assert_eq!(t.foreground.as_deref(), Some("rgb(200,200,200)"));
    }

    #[test]
    fn jsonc_config() {
        let cfg = r#"{
  // the bar
  "layer": "top",
  "position": "bottom", /* where */
  "height": 24,
  "format": "{icon} // not a comment °C 󰓅",
  "modules-right": ["cpu", "memory",],
}"#;
        let l = parse_config(cfg);
        assert_eq!(l.position.as_deref(), Some("bottom"));
        assert_eq!(l.height, Some(24));
        assert_eq!(l.layer.as_deref(), Some("top"));
        assert!(strip_jsonc(cfg).contains("// not a comment °C 󰓅"));
    }

    #[test]
    fn list_of_bars() {
        let l = parse_config(r#"[{"position":"left"},{"position":"top"}]"#);
        assert_eq!(l.position.as_deref(), Some("left"));
        assert_eq!(l.along, None);
    }

    #[test]
    fn finds_our_module_and_its_bar() {
        let cfg = r#"[
  {"position": "left", "modules-left": ["clock"], "clock": {}},
  {
    "position": "bottom",
    "height": 26,
    "modules-left": ["custom/other"],
    "modules-right": ["cpu", "group/media"],
    "group/media": {"modules": ["image#art", "custom/ours"]},
    "custom/other": {"exec": "other bar"},
    "custom/ours": {"exec": "~/.local/bin/rh bar", "return-type": "json"}
  }
]"#;
        let l = parse_config(cfg);
        assert_eq!(l.position.as_deref(), Some("bottom"));
        assert_eq!(l.along, Some(1.0));
        let h = l.hint();
        assert_eq!(h.edge, Some(bar_anchor::Edge::Bottom));
        assert_eq!(h.size, Some(26.0));
        assert_eq!(h.along, Some(1.0));
        let center = parse_config(
            r#"{"modules-center": ["custom/x"], "custom/x": {"exec": "rh bar --verbose"}}"#,
        );
        assert_eq!(center.along, Some(0.5));
        assert_eq!(center.hint().edge, Some(bar_anchor::Edge::Top));
        let lookalike =
            parse_config(r#"{"modules-left": ["custom/x"], "custom/x": {"exec": "notrh bar"}}"#);
        assert_eq!(lookalike.along, None);
    }
}
