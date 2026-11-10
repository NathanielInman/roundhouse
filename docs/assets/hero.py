#!/usr/bin/env python3
"""Writes docs/assets/hero.svg: the README's animated hero.

A 12 second loop: the cursor clicks the lamp in a waybar, the drawer springs
out of that icon, tiles arrive in order and their sparklines draw, the cursor
switches to the Storage tab, the meters fill, and a click outside closes it.

Pure CSS animation inside the SVG, so GitHub plays it in an <img>. With
reduced motion the art is a still of the open drawer.

    python3 docs/assets/hero.py
"""

import math
import random
from pathlib import Path

W, H = 1200, 660
T = 12  # loop seconds

# Nord, as the default waybar theme the drawer borrows from.
BG0, BG1 = "#14171c", "#0d0f12"
BAR, BAR_FG, BAR_DIM, BAR_LINE = "#282a2e", "#c5c8c6", "#7c838e", "#3b4048"
PANEL, PANEL_LINE = "#25282c", "#41464e"
TILE, TILE_LINE = "#2c2f34", "#363a40"
FG, DIM = "#c5c8c6", "#7c838e"
ACCENT, CALM, WARN, ACT, DARK = "#88c0d0", "#a3be8c", "#ebcb8b", "#bf616a", "#1d2024"
FONT = "JetBrains Mono, DejaVu Sans Mono, Menlo, Consolas, monospace"

# The lamp sits in the bar; the drawer opens above it, clamped to the screen.
BAR_Y, BAR_H = 616, 44
LAMP_X, LAMP_Y = 1006, BAR_Y + BAR_H / 2
DW, DH = 520, 528
DX, DY = W - DW - 18, BAR_Y - 12 - DH

random.seed(7)

out = []


def add(s):
    out.append(s)


def esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def text(x, y, s, size=13, fill=FG, weight=400, anchor="start", spacing=0, cls=""):
    c = f' class="{cls}"' if cls else ""
    ls = f' letter-spacing="{spacing}"' if spacing else ""
    add(f'<text x="{x}" y="{y}" font-size="{size}" fill="{fill}" font-weight="{weight}" text-anchor="{anchor}"{ls}{c}>{esc(s)}</text>')


def walk(n, base, amp, lo=2, hi=98, pull=0.12):
    v, pts = base, []
    for _ in range(n):
        v += (random.random() - 0.5) * amp + (base - v) * pull
        v = max(lo, min(hi, v))
        pts.append(v)
    return pts


def spark(x, y, w, h, values, vmax, cls, delay):
    n = len(values)
    pts = [(x + i / (n - 1) * w, y + h - values[i] / vmax * (h - 2)) for i in range(n)]
    line = " ".join(f"{px:.1f},{py:.1f}" for px, py in pts)
    area = f"M{x},{y + h} L" + " L".join(f"{px:.1f},{py:.1f}" for px, py in pts) + f" L{x + w},{y + h} Z"
    length = sum(math.dist(pts[i], pts[i + 1]) for i in range(n - 1)) + 10
    add(f'<path d="{area}" fill="{ACCENT}" fill-opacity=".14" class="a fade" style="animation-delay:{delay}s"/>')
    add(
        f'<polyline points="{line}" fill="none" stroke="{ACCENT}" stroke-width="1.6" stroke-linejoin="round" '
        f'class="a draw" style="stroke-dasharray:{length:.0f};stroke-dashoffset:0;--len:{length:.0f};animation-delay:{delay}s"/>'
    )
    ex, ey = pts[-1]
    add(f'<circle cx="{ex - 1:.1f}" cy="{ey:.1f}" r="2.6" fill="{ACCENT}" class="a fade" style="animation-delay:{delay + 0.5}s"/>')


def tile(x, y, w, h, label, aside, value, unit, delay, values=None, vmax=100, note=None):
    add(f'<g class="a rise" style="animation-delay:{delay}s">')
    add(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="10" fill="{TILE}" stroke="{TILE_LINE}"/>')
    text(x + 12, y + 20, label, 10.5, DIM, 700, spacing=1)
    text(x + w - 12, y + 20, aside, 10.5, DIM, 700, "end", 1)
    text(x + 12, y + 50, value, 25, FG, 700)
    text(x + 14 + len(value) * 15.5, y + 50, unit, 11.5, DIM)
    if values:
        spark(x + 12, y + 58, w - 24, h - 66, values, vmax, "s", delay + 0.15)
    if note:
        text(x + 12, y + 74, note, 11.5, CALM)
    add("</g>")


def meter_row(x, y, w, name, detail, right, frac, color, cls, delay):
    text(x, y, name, 12.5, FG)
    text(x + 8 + len(name) * 7.6, y, detail, 12.5, DIM)
    text(x + w, y, right, 12.5, DIM, anchor="end")
    add(f'<rect x="{x}" y="{y + 7}" width="{w}" height="4" rx="2" fill="{TILE_LINE}"/>')
    add(f'<rect x="{x}" y="{y + 7}" width="{max(3, w * frac):.1f}" height="4" rx="2" fill="{color}" class="a {cls}" style="animation-delay:{delay}s"/>')


def lamp_icon(cx, cy, r, center):
    add(f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="none" stroke="{BAR_FG}" stroke-width="1.6"/>')
    for k in range(8):
        a = k * math.pi / 4
        x1, y1 = cx + math.cos(a) * (r - 4), cy + math.sin(a) * (r - 4)
        x2, y2 = cx + math.cos(a) * (r - 1), cy + math.sin(a) * (r - 1)
        add(f'<line x1="{x1:.1f}" y1="{y1:.1f}" x2="{x2:.1f}" y2="{y2:.1f}" stroke="{BAR_FG}" stroke-width="1.6" stroke-linecap="round"/>')
    add(f'<circle cx="{cx}" cy="{cy}" r="{r * 0.36:.1f}" fill="{center}"/>')


def pct(p):
    return f"{p:.2f}%"


# Times in seconds within the loop, turned into keyframe percentages.
def at(sec):
    return sec / T * 100


CLICK_LAMP, OPEN, CLICK_TAB, CLICK_OUT, CLOSED = 1.0, 1.08, 5.6, 9.3, 9.75

css = f"""
.a {{ animation-duration: {T}s; animation-iteration-count: infinite; animation-timing-function: linear; animation-fill-mode: both; }}
#drawer {{ transform-origin: {LAMP_X}px {LAMP_Y}px; animation-name: spring; }}
@keyframes spring {{
  0%, {pct(at(OPEN) - 0.01)} {{ transform: translateY(26px) scale(.06); opacity: 0; }}
  {pct(at(OPEN + 0.10))} {{ transform: translateY(14px) scale(.5); opacity: .85; }}
  {pct(at(OPEN + 0.20))} {{ transform: translateY(4px) scale(.9); opacity: 1; }}
  {pct(at(OPEN + 0.30))} {{ transform: translateY(-1px) scale(1.035); }}
  {pct(at(OPEN + 0.42))} {{ transform: translateY(0) scale(.988); }}
  {pct(at(OPEN + 0.56))} {{ transform: scale(1.006); }}
  {pct(at(OPEN + 0.72))}, {pct(at(CLICK_OUT + 0.08))} {{ transform: scale(1); opacity: 1; }}
  {pct(at(CLOSED))}, 100% {{ transform: translateY(14px) scale(.4); opacity: 0; }}
}}
.rise {{ animation-name: rise; animation-timing-function: cubic-bezier(.2,.8,.2,1); }}
@keyframes rise {{
  0%, {pct(at(OPEN + 0.12))} {{ opacity: 0; transform: translateY(12px); }}
  {pct(at(OPEN + 0.52))}, 100% {{ opacity: 1; transform: none; }}
}}
.fade {{ animation-name: fade; }}
@keyframes fade {{ 0%, {pct(at(OPEN + 0.5))} {{ opacity: 0; }} {pct(at(OPEN + 1.1))}, 100% {{ opacity: 1; }} }}
.draw {{ animation-name: draw; animation-timing-function: ease-out; }}
@keyframes draw {{ 0%, {pct(at(OPEN + 0.25))} {{ stroke-dashoffset: var(--len); }} {pct(at(OPEN + 1.6))}, 100% {{ stroke-dashoffset: 0; }} }}
#mview {{ animation-name: mout; }}
@keyframes mout {{
  0%, {pct(at(CLICK_TAB + 0.05))} {{ opacity: 1; transform: none; }}
  {pct(at(CLICK_TAB + 0.3))}, 100% {{ opacity: 0; transform: translateX(-36px); }}
}}
#sview {{ opacity: 0; animation-name: sin; }}
@keyframes sin {{
  0%, {pct(at(CLICK_TAB + 0.12))} {{ opacity: 0; transform: translateX(36px); }}
  {pct(at(CLICK_TAB + 0.42))}, 100% {{ opacity: 1; transform: none; }}
}}
.fill {{ transform-box: fill-box; transform-origin: left center; animation-name: fill; animation-timing-function: cubic-bezier(.2,.8,.2,1); }}
@keyframes fill {{ 0%, {pct(at(CLICK_TAB + 0.35))} {{ transform: scaleX(0); }} {pct(at(CLICK_TAB + 1.3))}, 100% {{ transform: scaleX(1); }} }}
#pill {{ animation-name: pill; animation-timing-function: cubic-bezier(.3,1.3,.5,1); }}
@keyframes pill {{ 0%, {pct(at(CLICK_TAB + 0.05))} {{ transform: none; }} {pct(at(CLICK_TAB + 0.35))}, 100% {{ transform: translateX(161px); }} }}
#tab1 {{ animation-name: t1; }}
@keyframes t1 {{ 0%, {pct(at(CLICK_TAB + 0.1))} {{ fill: {DARK}; }} {pct(at(CLICK_TAB + 0.2))}, 100% {{ fill: {DIM}; }} }}
#tab2 {{ fill: {DIM}; animation-name: t2; }}
@keyframes t2 {{ 0%, {pct(at(CLICK_TAB + 0.15))} {{ fill: {DIM}; }} {pct(at(CLICK_TAB + 0.3))}, 100% {{ fill: {DARK}; }} }}
#lampbg {{ animation-name: lampbg; }}
@keyframes lampbg {{ 0%, {pct(at(CLICK_LAMP + 0.05))} {{ opacity: 0; }} {pct(at(CLICK_LAMP + 0.2))}, {pct(at(CLICK_OUT + 0.1))} {{ opacity: 1; }} {pct(at(CLOSED))}, 100% {{ opacity: 0; }} }}
#cursor {{ animation-name: cursor; animation-timing-function: cubic-bezier(.45,0,.25,1); }}
@keyframes cursor {{
  0% {{ transform: translate(760px, 380px); }}
  {pct(at(CLICK_LAMP - 0.15))}, {pct(at(4.6))} {{ transform: translate({LAMP_X - 3}px, {LAMP_Y - 4}px); }}
  {pct(at(CLICK_TAB - 0.15))}, {pct(at(8.3))} {{ transform: translate({DX + 260}px, {DY + 82}px); }}
  {pct(at(CLICK_OUT - 0.15))}, {pct(at(10.4))} {{ transform: translate(470px, 300px); }}
  100% {{ transform: translate(760px, 380px); }}
}}
.ripple {{ transform-box: fill-box; transform-origin: center; animation-name: var(--k); }}
"""
for i, sec in enumerate([CLICK_LAMP, CLICK_TAB, CLICK_OUT]):
    css += (
        f"@keyframes r{i} {{ 0%, {pct(at(sec) - 0.01)} {{ opacity: 0; transform: scale(.3); }} "
        f"{pct(at(sec))} {{ opacity: .9; transform: scale(.3); }} "
        f"{pct(at(sec + 0.45))}, 100% {{ opacity: 0; transform: scale(1.6); }} }}\n"
    )
css += """
.pulse { transform-box: fill-box; transform-origin: center; animation: pulse 2.4s ease-out infinite; }
@keyframes pulse { 0% { opacity: .55; transform: scale(.6); } 80%, 100% { opacity: 0; transform: scale(1.9); } }
@media (prefers-reduced-motion: reduce) {
  .a, .pulse, .ripple { animation: none !important; }
  #cursor, .ripple { display: none; }
}
"""

add(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" font-family="{FONT}" role="img" aria-label="A waybar with one lamp icon. A click on it opens the Roundhouse drawer out of the icon: CPU, GPU, memory, pressure and network tiles with sparklines, memory by app, then the Storage tab with disks and a network share.">')
add(f"<style>{css}</style>")
add(f"""<defs>
<linearGradient id="wall" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{BG0}"/><stop offset="1" stop-color="{BG1}"/></linearGradient>
<radialGradient id="haze1" cx=".85" cy="0" r=".75"><stop offset="0" stop-color="{ACCENT}" stop-opacity=".13"/><stop offset="1" stop-color="{ACCENT}" stop-opacity="0"/></radialGradient>
<radialGradient id="haze2" cx=".05" cy="1" r=".6"><stop offset="0" stop-color="#b48ead" stop-opacity=".09"/><stop offset="1" stop-color="#b48ead" stop-opacity="0"/></radialGradient>
<filter id="shadow" x="-20%" y="-20%" width="140%" height="150%"><feDropShadow dx="0" dy="18" stdDeviation="22" flood-color="#000" flood-opacity=".65"/></filter>
<clipPath id="screen"><rect width="{W}" height="{H}" rx="18"/></clipPath>
</defs>""")
add('<g clip-path="url(#screen)">')
add(f'<rect width="{W}" height="{H}" fill="url(#wall)"/><rect width="{W}" height="{H}" fill="url(#haze1)"/><rect width="{W}" height="{H}" fill="url(#haze2)"/>')

# A couple of editor windows behind, so the drawer has something to sit over.
for (x, y, w, h, lines) in [(36, 36, 560, 360, 11), (210, 300, 470, 280, 7)]:
    add(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="12" fill="#1a1d22" stroke="#2a2e35"/>')
    add(f'<rect x="{x}" y="{y}" width="{w}" height="30" rx="12" fill="#1f2328"/>')
    for k, c in enumerate(["#bf616a", "#ebcb8b", "#a3be8c"]):
        add(f'<circle cx="{x + 18 + k * 16}" cy="{y + 15}" r="5" fill="{c}" fill-opacity=".55"/>')
    for i in range(lines):
        lw = random.randint(int(w * 0.25), int(w * 0.8))
        indent = random.choice([0, 0, 24, 48])
        col = random.choice(["#3b4252", "#434c5e", "#4c566a", "#5e81ac55", "#88c0d044"])
        add(f'<rect x="{x + 22 + indent}" y="{y + 50 + i * 24}" width="{lw - indent}" height="9" rx="4" fill="{col}"/>')

# ---- the bar
add(f'<rect x="0" y="{BAR_Y}" width="{W}" height="{BAR_H}" fill="{BAR}"/>')
add(f'<line x1="0" y1="{BAR_Y}" x2="{W}" y2="{BAR_Y}" stroke="#000" stroke-opacity=".4"/>')
mid = BAR_Y + BAR_H / 2 + 5
for i in range(6):
    x = 18 + i * 30
    if i == 3:
        add(f'<rect x="{x - 4}" y="{BAR_Y + 9}" width="26" height="26" rx="6" fill="{ACCENT}"/>')
    text(x + 9, mid, str(i + 1), 14, DARK if i == 3 else BAR_DIM, 700 if i == 3 else 400, "middle")
text(208, mid, "nvim  crates/rh-board/src/drawer.rs", 14, BAR_FG)
mods = [(612, "♪ lo-fi radio"), (760, "vol 42%"), (858, "mouse 81%")]
for x, s in mods:
    text(x, mid, s, 14, BAR_DIM)
    add(f'<line x1="{x - 14}" y1="{BAR_Y + 10}" x2="{x - 14}" y2="{BAR_Y + BAR_H - 10}" stroke="{BAR_LINE}"/>')
add(f'<line x1="{LAMP_X - 30}" y1="{BAR_Y + 10}" x2="{LAMP_X - 30}" y2="{BAR_Y + BAR_H - 10}" stroke="{BAR_LINE}"/>')
add(f'<rect id="lampbg" class="a" x="{LAMP_X - 22}" y="{BAR_Y + 7}" width="74" height="30" rx="7" fill="{ACCENT}" fill-opacity=".22"/>')
add(f'<circle class="pulse" cx="{LAMP_X}" cy="{LAMP_Y}" r="13" fill="none" stroke="{CALM}" stroke-width="2"/>')
lamp_icon(LAMP_X, LAMP_Y, 11, CALM)
text(LAMP_X + 18, mid, "54°", 14, BAR_FG)
add(f'<line x1="{LAMP_X + 62}" y1="{BAR_Y + 10}" x2="{LAMP_X + 62}" y2="{BAR_Y + BAR_H - 10}" stroke="{BAR_LINE}"/>')
text(W - 22, mid, "14:32", 14, BAR_FG, anchor="end")

# ---- the drawer
add(f'<g id="drawer" class="a"><g transform="translate({DX},{DY})">')
add(f'<rect width="{DW}" height="{DH}" rx="16" fill="{PANEL}" stroke="{PANEL_LINE}" filter="url(#shadow)"/>')
add(f'<circle cx="24" cy="30" r="5" fill="{CALM}"/>')
text(40, 34, "FROSTYARCH", 15, FG, 700, spacing=1.5)
text(40, 53, "up 15h 11m · linux 7.2.14", 12, DIM)
add(f'<rect x="{DW - 82}" y="20" width="64" height="24" rx="12" fill="{CALM}" fill-opacity=".14"/>')
text(DW - 50, 36, "calm", 12, CALM, 700, "middle")

add(f'<rect x="16" y="68" width="{DW - 32}" height="34" rx="9" fill="{TILE}"/>')
add(f'<rect id="pill" class="a" x="19" y="71" width="160" height="28" rx="7" fill="{ACCENT}"/>')
text(99, 90, "Machine", 13, DARK, 700, "middle", cls="a\" id=\"tab1")
text(260, 90, "Storage", 13, DIM, 700, "middle", cls="a\" id=\"tab2")
text(421, 90, "Switchyard", 13, DIM, 700, "middle")

# Machine view
add('<g id="mview" class="a">')
tw = (DW - 40) / 2
tile(16, 114, tw, 92, "CPU · RYZEN 7 9800X3D", "54°C", "12", "% · 4.7 GHz", 0.0, walk(40, 18, 14), 60)
tile(24 + tw, 114, tw, 92, "GPU · RTX 3080", "41°C", "8", "% · 2.1 G / 10 G", 0.06, walk(40, 10, 10), 60)
tile(16, 214, tw, 92, "MEMORY", "SWAP 0 B", "13G", "/ 60 G", 0.12, walk(40, 22, 1.5, pull=0.3), 60)
tile(24 + tw, 214, tw, 92, "PRESSURE", "SOME · 10 S", "0.0", "mem · cpu 1.8 · io 0.3", 0.18, note="nothing is waiting")
tile(16, 314, DW - 32, 86, "NETWORK · ENO1", "203.0.113.42", "↓ 1.2 MB/s", "↑ 84 kB/s", 0.24, walk(60, 30, 26, pull=0.08), 100)
add(f'<g class="a rise" style="animation-delay:.3s">')
add(f'<rect x="16" y="408" width="{DW - 32}" height="104" rx="10" fill="{TILE}" stroke="{TILE_LINE}"/>')
text(28, 428, "MEMORY BY APP", 10.5, DIM, 700, spacing=1)
text(DW - 28, 428, "NOT BY PID", 10.5, DIM, 700, "end", 1)
for i, (n, d, r, f, c) in enumerate([
    ("Chrome", "40 procs", "7.6 G", 0.13, ACCENT),
    ("Blender", "lowprio scope", "2.2 G / 24 G", 0.09, CALM),
    ("T3Code", "14 procs", "1.5 G", 0.03, ACCENT),
]):
    y = 452 + i * 24
    text(28, y, n, 12.5, FG)
    text(36 + len(n) * 7.6, y, d, 12.5, DIM)
    text(DW - 28, y, r, 12.5, DIM, anchor="end")
    add(f'<rect x="28" y="{y + 6}" width="{DW - 56}" height="3" rx="1.5" fill="{TILE_LINE}"/>')
    add(f'<rect x="28" y="{y + 6}" width="{max(4, (DW - 56) * f):.1f}" height="3" rx="1.5" fill="{c}"/>')
add("</g></g>")

# Storage view
add('<g id="sview" class="a">')
add(f'<rect x="16" y="114" width="{DW - 32}" height="398" rx="10" fill="{TILE}" stroke="{TILE_LINE}"/>')
text(28, 134, "DISKS AND SHARES", 10.5, DIM, 700, spacing=1)
y = 164
rows = [
    ("head", "Sabrent", "nvme0n1", "41°C"),
    ("fs", "/ +5", "btrfs", "784 G / 1.9 T · 41%", 0.41, ACCENT),
    ("fs", "/boot", "vfat", "237 M / 2.0 G · 12%", 0.12, ACCENT),
    ("head", "Samsung SSD 990 PRO 2TB", "nvme1n1", "48°C"),
    ("fs", "~/Sites +1", "btrfs", "1.3 T / 1.8 T · 72%", 0.72, ACCENT),
    ("head", "Samsung SSD 990 PRO 2TB", "nvme2n1", "46°C"),
    ("fs", "/scratch +1", "ext4", "322 G / 1.8 T · 18%", 0.18, ACCENT),
    ("head", "nas · media", "network share", ""),
    ("fs", "~/Media", "cifs", "4.1 T / 6.6 T · 62%", 0.62, ACCENT),
    ("head", "backup", "automount", "not mounted, not woken"),
]
for k, r in enumerate(rows):
    if r[0] == "head":
        y += 8 if k else 0
        text(28, y, r[1], 12.5, FG, 700)
        text(36 + len(r[1]) * 7.8, y, r[2], 12.5, DIM)
        text(DW - 28, y, r[3], 12.5, DIM, anchor="end")
        y += 26
    else:
        meter_row(28, y, DW - 56, r[1], r[2], r[3], r[4], r[5], "fill", 0.05 * k)
        y += 32
add("</g>")
add("</g></g>")

# ---- cursor and clicks
for i, (cx, cy) in enumerate([(LAMP_X, LAMP_Y), (DX + 260, DY + 85), (470, 300)]):
    add(f'<circle class="a ripple" style="--k:r{i}" cx="{cx}" cy="{cy}" r="18" fill="none" stroke="{ACCENT}" stroke-width="2.5"/>')
add('<g id="cursor" class="a">')
add('<path d="M0,0 L0,22 L6,17 L10,26 L14,24 L10,15 L18,15 Z" fill="#eceff4" stroke="#1d2024" stroke-width="1.4" stroke-linejoin="round"/>')
add("</g>")
add("</g></svg>")

Path(__file__).with_name("hero.svg").write_text("\n".join(out) + "\n")
print(f"wrote hero.svg ({sum(len(s) + 1 for s in out) // 1024} KB)")
