//! Turns the bar's stylesheet into the drawer's color tokens, so the drawer
//! looks like it grew out of the bar. Only tokens the bar actually defines
//! are overridden; the rest keep the built-in theme.

use rh_core::config::Config;
use rh_core::waybar::BarTheme;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba(pub f64, pub f64, pub f64, pub f64);

impl Rgba {
    pub fn css(self) -> String {
        format!(
            "rgba({}, {}, {}, {:.3})",
            (self.0 * 255.0).round() as u8,
            (self.1 * 255.0).round() as u8,
            (self.2 * 255.0).round() as u8,
            self.3
        )
    }

    pub fn mix(self, o: Rgba, t: f64) -> Rgba {
        Rgba(
            self.0 + (o.0 - self.0) * t,
            self.1 + (o.1 - self.1) * t,
            self.2 + (o.2 - self.2) * t,
            self.3 + (o.3 - self.3) * t,
        )
    }

    pub fn with_alpha(self, a: f64) -> Rgba {
        Rgba(self.0, self.1, self.2, a)
    }
}

/// Reads `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb(...)` and `rgba(...)`.
pub fn parse_color(s: &str) -> Option<Rgba> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        let h = |i: usize, n: usize| {
            u8::from_str_radix(&hex[i..i + n], 16)
                .ok()
                .map(|v| if n == 1 { v * 17 } else { v } as f64 / 255.0)
        };
        return match hex.len() {
            3 => Some(Rgba(h(0, 1)?, h(1, 1)?, h(2, 1)?, 1.0)),
            6 => Some(Rgba(h(0, 2)?, h(2, 2)?, h(4, 2)?, 1.0)),
            8 => Some(Rgba(h(0, 2)?, h(2, 2)?, h(4, 2)?, h(6, 2)?)),
            _ => None,
        };
    }
    let inner = s
        .strip_prefix("rgba(")
        .or_else(|| s.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let parts: Vec<f64> = inner
        .split(',')
        .filter_map(|p| p.trim().parse().ok())
        .collect();
    match parts.as_slice() {
        [r, g, b] => Some(Rgba(r / 255.0, g / 255.0, b / 255.0, 1.0)),
        [r, g, b, a] => Some(Rgba(r / 255.0, g / 255.0, b / 255.0, *a)),
        _ => None,
    }
}

/// The CSS that layers the bar's look over the built-in theme.
pub fn css(bar: Option<&BarTheme>, cfg: &Config) -> String {
    let mut out = String::new();
    let mut font = cfg.theme.font.clone();
    if let Some(t) = bar.filter(|_| cfg.theme.from == "waybar") {
        // Only the bar's hue counts: a floating bar is often transparent
        // (rgba(.., 0)), and mixing that alpha into the tiles would make
        // the drawer see-through.
        let bg = t
            .background
            .as_deref()
            .and_then(parse_color)
            .map(|c| c.with_alpha(1.0));
        let fg = t
            .foreground
            .as_deref()
            .and_then(parse_color)
            .map(|c| c.with_alpha(1.0));
        if let Some(bg) = bg {
            out.push_str(&format!(
                "@define-color rh_panel {};\n",
                bg.with_alpha(0.97).css()
            ));
            out.push_str(&format!(
                "@define-color rh_well {};\n",
                bg.mix(Rgba(0.0, 0.0, 0.0, 1.0), 0.35).css()
            ));
        }
        if let Some(fg) = fg {
            out.push_str(&format!("@define-color rh_fg {};\n", fg.css()));
            if let Some(bg) = bg {
                out.push_str(&format!(
                    "@define-color rh_dim {};\n",
                    fg.mix(bg, 0.45).css()
                ));
                out.push_str(&format!(
                    "@define-color rh_tile {};\n",
                    bg.mix(fg, 0.05).css()
                ));
                out.push_str(&format!(
                    "@define-color rh_tile_border {};\n",
                    bg.mix(fg, 0.12).css()
                ));
            }
        }
        if let Some(b) = t.border.as_deref().and_then(parse_color) {
            out.push_str(&format!("@define-color rh_border {};\n", b.css()));
        }
        if let Some(h) = t.hover.as_deref().and_then(parse_color) {
            out.push_str(&format!("@define-color rh_tile_hover {};\n", h.css()));
        }
        for (name, token) in [("accent", "rh_accent"), ("frost", "rh_accent")] {
            if let Some(c) = t.colors.get(name).and_then(|c| parse_color(c)) {
                out.push_str(&format!("@define-color {token} {};\n", c.css()));
                break;
            }
        }
        if font.is_none() {
            font = t.font.clone();
        }
    }
    if let Some(a) = cfg.theme.accent.as_deref().and_then(parse_color) {
        out.push_str(&format!("@define-color rh_accent {};\n", a.css()));
    }
    if let Some(f) = font {
        let f = f.replace('"', "");
        out.push_str(&format!(
            "window.roundhouse {{ font-family: \"{f}\", monospace; }}\n"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(parse_color("#fff"), Some(Rgba(1.0, 1.0, 1.0, 1.0)));
        assert_eq!(
            parse_color("#282A2E").unwrap().css(),
            "rgba(40, 42, 46, 1.000)"
        );
        assert_eq!(parse_color("rgba(10, 20, 30, 0.5)").unwrap().3, 0.5);
        assert!(parse_color("teal").is_none());
    }

    #[test]
    fn follows_the_bar() {
        let t = rh_core::waybar::parse_style("* { font-family: \"PragmataPro Liga\", monospace; }\nwindow#waybar { background: #282A2E; color: #C5C8C6; }");
        let c = css(Some(&t), &Config::default());
        assert!(c.contains("@define-color rh_panel rgba(40, 42, 46, 0.970);"));
        assert!(c.contains("font-family: \"PragmataPro Liga\""));
        let mut cfg = Config::default();
        cfg.theme.from = "none".into();
        assert!(css(Some(&t), &cfg).is_empty());
    }

    #[test]
    fn transparent_bar_keeps_an_opaque_drawer() {
        let t = rh_core::waybar::parse_style(
            "window#waybar { background: rgba(40, 42, 46, 0); color: #C5C8C6; }",
        );
        let c = css(Some(&t), &Config::default());
        assert!(c.contains("@define-color rh_panel rgba(40, 42, 46, 0.970);"));
        assert!(
            !c.contains(", 0.000)"),
            "no tile may inherit the bar's zero alpha: {c}"
        );
    }
}
