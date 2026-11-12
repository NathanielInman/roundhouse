#!/usr/bin/env python3
"""Writes docs/assets/hero.svg: the README's animated hero.

A 14 second loop under the floating Platform waybar, on the same desktop as
the siblings' heroes. The cursor clicks Roundhouse's green lamp and the
drawer springs out of it: Switchyard's build on one line, the tiles arrive
and their sparklines draw, and the network tile names the busiest apps. A
scroll brings up the fans, the audio card and memory by app. The Network tab
shows the way out hop by hop with the public IP hidden, the link, and every
app talking out; npm finishes and stays a moment as a dimmed shadow. The
Storage tab fills its meters, and a click outside closes the drawer.

The drawer is drawn by drawer() and its parts in its own coordinates, at the
app's own size and with no bar or desktop globals, so other heroes can port
it (dot-files' docs/bar/scenes.py does).

Pure CSS animation inside the SVG, so GitHub plays it in an <img>. With
reduced motion the art is a still of the open drawer.

    python3 docs/assets/hero.py
"""

import random
from pathlib import Path

W, H = 1200, 660
T = 14  # loop seconds

# Nord, from the Platform waybar's style.css.
BG0, BG1 = "#14171c", "#0d0f12"
PILL, PILL_LINE = "rgba(40,42,46,.84)", "rgba(76,86,106,.55)"
HOVER, WS_ACTIVE = "rgba(129,161,193,.18)", "rgba(129,161,193,.26)"
BAR_FG, BRIGHT, BAR_DIM = "#c5c8c6", "#d8dee9", "#616e88"
FROST, GREEN, YELLOW, RED, PURPLE = "#88c0d0", "#a3be8c", "#ebcb8b", "#bf616a", "#b48ead"

# The drawer's theme, as theme.from = "waybar" derives it from the bar: the
# panel is the bar's hue, tiles and dim text are mixes of it and the bar's
# foreground, the border is the pills' border, the accent is frost.
PANEL, PANEL_LINE = "#282a2e", "rgba(76,86,106,.55)"
TILE, TILE_LINE, WELL = "#303236", "#3b3d40", "#1a1b1e"
FG, DIM, IDLE = "#c5c8c6", "#7e8182", "#555b64"
ACCENT, CALM, WARN, ACT = FROST, GREEN, YELLOW, RED
FONT = "JetBrains Mono, DejaVu Sans Mono, Menlo, Consolas, monospace"


# ---------------------------------------------------------------- drawing kit

def esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def t(x, y, s, size=12, fill=FG, weight=400, anchor="start", spacing=0, attrs=""):
    """A <text> element, as a string."""
    ls = f' letter-spacing="{spacing}"' if spacing else ""
    a = f" {attrs}" if attrs else ""
    return (f'<text x="{x:.1f}" y="{y:.1f}" font-size="{size}" fill="{fill}" font-weight="{weight}" '
            f'text-anchor="{anchor}"{ls}{a}>{esc(s)}</text>')


def adv(s, size, spacing=0):
    """Width of monospace text: every face in FONT advances 0.6 em."""
    return len(s) * (size * 0.6 + spacing)


def fit(s, size, width, spacing=0):
    """Ellipsizes at the end to fit, as GTK does."""
    if adv(s, size, spacing) <= width:
        return s
    n = max(1, int(width / (size * 0.6 + spacing)) - 1)
    return s[:n].rstrip() + "…"


# Nerd Font glyphs the bar and drawer print, drawn as outlines so they
# survive an <img>. Taken from JetBrains Mono Nerd Font: 1 em = 1000 units,
# origin on the baseline, y down; each is (ink box, path).
GLYPHS = {
    "lamp": ((0, -776, 832, 56), "M416 -776Q334 -776 257 -745Q180 -714 121 -655Q62 -596 31 -519Q0 -442 0 -360Q0 -278 31 -201Q62 -124 121 -65Q180 -6 257 25Q334 56 416 56Q498 56 575 25Q652 -6 711 -65Q770 -124 801 -201Q832 -278 832 -360Q832 -442 801 -519Q770 -596 711 -655Q652 -714 575 -745Q498 -776 416 -776Z"),  # U+F0765
    "yard": ((0, -798, 875, 78), "M770 -380H707V-548Q707 -580 682 -605Q658 -630 625 -630H457V-692Q457 -736 426 -767Q396 -798 354 -798Q311 -798 280 -766Q250 -734 250 -692V-630H82Q49 -630 24 -605Q0 -580 0 -548V-390H62Q109 -390 142 -356Q174 -322 174 -276Q174 -230 142 -196Q109 -162 62 -162H0V-4Q0 28 24 53Q49 78 82 78H240V16Q240 -32 274 -64Q307 -96 354 -96Q400 -96 434 -64Q467 -32 467 16V78H625Q658 78 682 53Q707 28 707 -4V-172H770Q813 -172 844 -203Q875 -234 875 -276Q875 -318 844 -349Q813 -380 770 -380Z"),  # U+F0431
    "broker": ((0, -776, 832, 56), "M416 -26Q326 -26 249 -71Q172 -116 127 -193Q82 -270 82 -360Q82 -450 127 -527Q172 -604 249 -649Q326 -694 416 -694Q506 -694 583 -649Q660 -604 705 -527Q750 -450 750 -360Q750 -270 705 -193Q660 -116 583 -71Q506 -26 416 -26ZM416 -776Q334 -776 257 -745Q180 -714 121 -655Q62 -596 31 -519Q0 -442 0 -360Q0 -278 31 -201Q62 -124 121 -65Q180 -6 257 25Q334 56 416 56Q498 56 575 25Q652 -6 711 -65Q770 -124 801 -201Q832 -278 832 -360Q832 -442 801 -519Q770 -596 711 -655Q652 -714 575 -745Q498 -776 416 -776Z"),  # U+F0766
    "vpn": ((0, -819, 918, 99), "M125 -819Q92 -819 68 -794Q43 -770 43 -735V-569Q43 -534 68 -510Q92 -485 125 -485H0V-401H500V-485H375Q410 -485 434 -510Q459 -534 459 -569V-735Q459 -770 434 -794Q410 -819 375 -819ZM125 -735H375V-569H125ZM543 -319Q508 -319 484 -294Q459 -270 459 -235V-69Q459 -34 484 -10Q508 15 543 15H418V99H918V15H793Q826 15 850 -10Q875 -34 875 -69V-235Q875 -270 850 -294Q826 -319 793 -319ZM121 -299 61 -241 150 -151 61 -63 121 -5 209 -92 297 -5 357 -63 268 -151 357 -241 297 -299 209 -210ZM543 -235H793V-69H543Z"),  # U+F0319
    "bell": ((1, -822, 811, 102), "M406 -822Q430 -820 447 -804Q464 -788 464 -764V-730Q564 -710 629 -630Q694 -550 694 -446V-412Q694 -282 782 -182L795 -168Q820 -140 805 -106Q790 -72 752 -72H60Q22 -72 7 -106Q-8 -140 17 -168L30 -182Q117 -280 117 -412V-446Q116 -550 182 -630Q248 -710 348 -730V-764Q348 -788 365 -804Q382 -820 406 -822ZM488 68Q454 102 406 102Q358 102 324 68Q290 34 290 -14H406H521Q521 34 488 68Z"),  # U+F0F3
    "vol": ((0, -772, 1154, 52), "M962 -763Q1052 -690 1103 -585Q1154 -480 1154 -360Q1154 -240 1103 -135Q1052 -30 962 43Q948 54 930 52Q912 50 901 37Q890 23 892 5Q894 -13 907 -24Q982 -84 1024 -172Q1066 -260 1067 -360Q1066 -460 1024 -548Q982 -636 907 -696Q894 -707 892 -725Q890 -743 901 -757Q912 -770 930 -772Q948 -774 962 -763ZM853 -628Q913 -580 947 -510Q981 -440 981 -360Q981 -280 947 -210Q913 -140 853 -91Q839 -80 821 -82Q803 -84 792 -97Q780 -111 782 -129Q784 -147 798 -158Q894 -236 894 -360Q894 -483 798 -561Q784 -572 782 -590Q780 -608 792 -622Q803 -636 821 -638Q839 -640 853 -629ZM744 -494Q808 -441 808 -360Q808 -278 744 -225Q730 -214 712 -216Q694 -218 683 -232Q672 -246 674 -264Q676 -282 689 -293Q721 -320 721 -360Q721 -400 689 -427Q676 -438 674 -456Q672 -474 683 -488Q694 -502 712 -504Q730 -506 744 -494ZM543 -758Q577 -743 577 -706V-14Q577 24 543 39Q508 54 481 30L238 -187H115Q68 -186 34 -220Q0 -254 0 -302V-417Q0 -465 34 -499Q68 -533 115 -533H238L481 -749Q508 -773 543 -758Z"),  # U+F028
    "mouse": ((0, -817, 668, 97), "M293 -817Q211 -807 144 -760Q78 -714 39 -642Q0 -571 0 -487H293ZM0 -237Q0 -147 45 -70Q90 7 167 52Q244 97 334 97Q424 97 501 52Q578 7 623 -70Q668 -147 668 -237V-403H0ZM375 -817V-487H668Q668 -571 629 -642Q590 -714 524 -760Q457 -807 375 -817Z"),  # U+F037D
    "mic": ((9, -756, 591, 36), "M300 -756Q353 -756 389 -720Q425 -684 425 -630V-380Q425 -330 389 -293Q353 -256 300 -256Q247 -256 211 -293Q175 -330 175 -380V-630Q175 -684 211 -720Q247 -756 300 -756ZM591 -380Q591 -308 559 -246Q527 -184 469 -144Q411 -104 341 -92V36H259V-92Q189 -104 131 -144Q73 -184 41 -246Q9 -308 9 -380H91Q91 -340 108 -302Q124 -264 154 -234Q183 -204 221 -189Q259 -174 300 -174Q357 -174 404 -201Q452 -228 480 -276Q509 -324 509 -380Z"),  # U+F036C
    "chevron": ((69, -764, 531, 44), "M86 -400V-401L432 -747Q449 -764 473 -764Q497 -764 514 -747Q531 -730 531 -706Q531 -682 514 -665L208 -360L514 -54Q531 -38 531 -14Q531 10 514 27Q497 44 473 44Q449 44 432 27L86 -319Q69 -336 69 -360Q69 -384 86 -400Z"),  # U+F053
    "chrome": ((0, -821, 923, 102), "M0 -360Q0 -482 62 -592L260 -247Q290 -193 344 -161Q398 -129 462 -129Q498 -129 535 -141L398 98Q228 74 114 -56Q0 -186 0 -360ZM658 -241Q692 -295 692 -360Q692 -465 614 -533H890Q923 -454 923 -360Q922 -168 788 -34Q654 100 462 102ZM861 -590H462Q378 -590 314 -538Q250 -486 235 -405L98 -644Q162 -726 258 -774Q354 -822 462 -821Q588 -820 694 -758Q800 -696 861 -590ZM303 -360Q304 -426 350 -472Q396 -518 462 -518Q528 -518 574 -472Q620 -426 620 -360Q620 -294 574 -248Q528 -202 462 -201Q396 -202 350 -248Q304 -294 303 -360Z"),  # U+F268
    "slack": ((0, -764, 808, 44), "M170 -253Q170 -218 145 -193Q120 -168 85 -168Q50 -168 25 -193Q0 -218 0 -253Q0 -288 25 -313Q50 -338 85 -338H170ZM212 -253Q212 -288 237 -313Q262 -338 297 -338Q332 -338 357 -313Q382 -288 382 -253V-41Q382 -6 357 19Q332 44 297 44Q262 44 237 19Q212 -6 212 -41ZM297 -594Q262 -594 237 -619Q212 -644 212 -679Q212 -714 237 -739Q262 -764 297 -764Q332 -764 357 -739Q382 -714 382 -679V-594ZM297 -551Q332 -551 357 -526Q382 -501 382 -466Q382 -431 357 -406Q332 -381 297 -381H85Q50 -381 25 -406Q0 -431 0 -466Q0 -501 25 -526Q50 -551 85 -551ZM638 -466Q638 -501 663 -526Q688 -551 723 -551Q758 -551 783 -526Q808 -501 808 -466Q808 -431 783 -406Q758 -381 723 -381H638ZM595 -466Q595 -431 570 -406Q545 -381 510 -381Q474 -381 450 -406Q426 -431 426 -466V-679Q426 -714 450 -739Q474 -764 510 -764Q545 -764 570 -739Q595 -714 595 -679ZM510 -126Q545 -126 570 -101Q595 -76 595 -41Q595 -6 570 19Q545 44 510 44Q474 44 450 19Q426 -6 426 -41V-126ZM510 -168Q474 -168 450 -193Q426 -218 426 -253Q426 -288 450 -313Q474 -338 510 -338H723Q758 -338 783 -313Q808 -288 808 -253Q808 -218 783 -193Q758 -168 723 -168Z"),  # U+F198
    "t3": ((0, -822, 1154, 102), "M577 -822Q601 -820 618 -804Q635 -788 635 -764V-648H851Q905 -648 943 -610Q981 -572 981 -518V-28Q981 26 943 64Q905 102 851 102H303Q249 102 211 64Q173 26 173 -28V-518Q173 -572 211 -610Q249 -648 303 -648H519V-764Q519 -788 536 -804Q553 -820 577 -822ZM375 -130Q362 -128 354 -120Q346 -112 346 -100Q346 -88 354 -80Q362 -72 375 -72H433Q446 -72 454 -80Q462 -88 462 -100Q462 -112 454 -120Q446 -128 433 -130ZM548 -130Q536 -128 528 -120Q520 -112 519 -100Q520 -88 528 -80Q536 -72 548 -72H606Q618 -72 626 -80Q634 -88 635 -100Q634 -112 626 -120Q618 -128 606 -130ZM721 -130Q708 -128 700 -120Q692 -112 692 -100Q692 -88 700 -80Q708 -72 721 -72H779Q792 -72 800 -80Q808 -88 808 -100Q808 -112 800 -120Q792 -128 779 -130ZM455 -309Q476 -330 476 -360Q476 -390 455 -411Q434 -432 404 -432Q374 -432 353 -411Q332 -390 332 -360Q332 -330 353 -309Q374 -288 404 -288Q434 -288 455 -309ZM699 -309Q720 -288 750 -288Q780 -288 801 -309Q822 -330 822 -360Q822 -390 801 -411Q780 -432 750 -432Q720 -432 699 -411Q678 -390 678 -360Q678 -330 699 -309ZM87 -418H115V-72H87Q52 -70 26 -96Q0 -122 0 -158V-332Q0 -368 26 -393Q52 -418 87 -418ZM1067 -418Q1102 -418 1128 -393Q1154 -368 1154 -332V-158Q1154 -122 1128 -96Q1102 -70 1067 -72H1038V-418Z"),  # U+EE0D
    "vim": ((0, -768, 873, 48), "M13 -768 334 -762 354 -738V-682L337 -664H312Q307 -664 304 -662Q302 -658 302 -654L299 -410V-408Q301 -406 301 -404Q303 -400 304 -400Q308 -398 310 -398Q312 -398 318 -402L569 -648Q569 -648 570 -648Q572 -650 572 -654Q573 -656 573 -658Q573 -660 570 -662Q568 -664 563 -664H541L526 -684V-742L543 -764L858 -762L873 -746V-680L156 48H76L53 24L51 -656Q51 -660 48 -662Q46 -666 42 -666L16 -668L0 -682V-752Z"),  # U+E62B
    "term": ((0, -764, 1038, 44), "M17 -665Q0 -682 0 -706Q0 -730 17 -747Q34 -764 58 -764Q82 -764 99 -747L445 -400Q462 -384 462 -360Q462 -336 445 -319L99 27Q82 44 58 44Q34 44 17 27Q0 10 0 -14Q0 -38 17 -54L322 -360ZM462 -71H981Q1006 -70 1022 -54Q1038 -38 1038 -14Q1038 10 1022 27Q1006 44 981 44H462Q438 44 421 27Q404 10 404 -14Q404 -38 421 -54Q438 -70 462 -71Z"),  # U+F120
    "discord": ((0, -840, 832, 120), "M832 120 613 -90 641 -6Q642 -6 104 -6Q61 -6 30 -36Q0 -66 0 -110V-734Q0 -778 31 -809Q62 -840 104 -840H728Q771 -840 802 -808Q832 -776 832 -734ZM416 -598Q367 -598 316 -586Q281 -576 250 -562L227 -548Q252 -574 301 -594L344 -610L336 -616Q293 -616 244 -592Q219 -580 203 -566Q170 -500 154 -428Q140 -376 137 -328L135 -288Q172 -242 230 -224Q259 -216 281 -218L311 -256Q288 -262 266 -276Q250 -288 236 -300L225 -312L250 -298Q281 -282 316 -274Q365 -260 416 -260Q467 -260 516 -274Q551 -284 582 -298L607 -312L596 -300Q582 -286 566 -276Q545 -262 521 -256L551 -218L576 -220Q605 -224 633 -236Q670 -254 697 -288L695 -328Q691 -376 678 -428Q662 -500 629 -566L609 -580Q586 -594 561 -604Q528 -618 496 -616L488 -610L531 -594Q580 -574 605 -548L582 -562Q551 -576 516 -586Q465 -598 416 -598ZM330 -440Q350 -440 364 -424Q379 -408 379 -386Q379 -364 364 -349Q350 -334 330 -334Q311 -334 296 -349Q281 -364 281 -386Q281 -408 295 -424Q309 -440 330 -440ZM504 -440Q524 -440 538 -424Q553 -408 553 -386Q553 -364 538 -349Q523 -334 504 -334Q484 -334 470 -349Q455 -364 455 -386Q455 -408 469 -424Q483 -440 504 -440Z"),  # U+F066F
    "bluetooth": ((0, -821, 692, 102), "M470 -513 392 -435V-590ZM392 -116V-272L469 -194ZM692 -354Q692 -244 676 -164Q660 -84 634 -33Q606 18 563 48Q520 78 472 90Q424 102 358 102Q293 102 242 90Q190 78 144 48Q98 17 67 -33Q36 -83 18 -164Q0 -245 0 -354Q0 -489 26 -583Q52 -678 102 -728Q152 -778 212 -800Q273 -822 354 -821Q436 -822 494 -800Q552 -778 598 -727Q644 -676 668 -582Q692 -488 692 -354ZM406 -354 549 -513 324 -755V-436L191 -570L142 -521L309 -353L142 -186L191 -137L324 -270L328 36L558 -194Z"),  # U+F293
}


def glyph(name, cx, cy, size, fill, attrs=""):
    """A glyph at `size` px, its ink centered on (cx, cy)."""
    (x0, y0, x1, y1), d = GLYPHS[name]
    k = size / 1000
    tx, ty = cx - (x0 + x1) / 2 * k, cy - (y0 + y1) / 2 * k
    a = f" {attrs}" if attrs else ""
    return f'<path transform="translate({tx:.2f},{ty:.2f}) scale({k:.4f})" d="{d}" fill="{fill}"{a}/>'


def glyph_w(name, size):
    (x0, _, x1, _), _ = GLYPHS[name]
    return (x1 - x0) * size / 1000


# ---------------------------------------------------------------- the drawer
#
# Everything below draws in the drawer's own coordinates: (0, 0) is the
# panel's top-left corner, and the size is the app's (drawer.width = 460;
# the height is drawer.max_height = 0.8 of this 660 px desktop). Each
# function takes `add`, a callable that takes one SVG string, and the state
# it draws. `fx` maps a role to a function of an index that returns extra
# attributes, so a story can animate parts; without it the drawer is a still.
#
#   roles: "rise" (tiles and cards arriving, by order), "draw" and "fade"
#   (sparkline strokes and their fill), "meter" (storage meters, by row),
#   "scroll" (the Machine page's content), "scrollbar" (its indicator, drawn
#   only with a hook), "live" and "gone" (the app that closes on the Network
#   tab; with both hooks both versions are drawn).

DRAWER_W, DRAWER_H = 460, 528
P = 14  # panel padding
IW = DRAWER_W - 2 * P
VIEW_Y0, VIEW_Y1 = 98, DRAWER_H - 38  # the scroller, between the tabs and the foot

TABS = [("machine", "Machine"), ("storage", "Storage"), ("network", "Network"), ("ports", "Ports"), ("switchyard", "Switchyard")]
TAB_Y, TAB_H = 58, 30
TAB_W = (IW - 6 - 2 * 4) / 5  # homogeneous buttons, 3 px padding, 2 px spacing


def tab_x(i):
    return P + 3 + i * (TAB_W + 2)


# Where a story can click, in drawer coordinates.
DRAWER_SPOTS = {tid: (tab_x(i) + TAB_W / 2, TAB_Y + TAB_H / 2) for i, (tid, _) in enumerate(TABS)}
DRAWER_SPOTS["yard-line"] = (P + IW / 2, VIEW_Y0 + 15)  # calm Machine page: opens the board too
DRAWER_SPOTS["show-ip"] = (P + IW / 2, VIEW_Y0 + 24 + 5 * 23 + 4 + 14)  # Network page
DRAWER_SPOTS["machine-page"] = (P + IW / 2, 300)  # somewhere to scroll

# Switchyard's phase colors the dot on its link (.yard-dot).
YARD_DOT = {"idle": IDLE, "ok": CALM, "queued": ACCENT, "waiting": ACCENT, "building": ACCENT, "running": ACCENT, "failed": ACT}
LEVEL_COLOR = {"calm": CALM, "heads-up": WARN, "act": ACT}

# What the Machine page shows at each lamp level: calm while Switchyard
# builds storefront, or red with memory stalling and android held.
MACHINE = {
    "calm": dict(
        chip="calm", reasons=[],
        yard="Switchyard is building storefront · 1 queued",
        cpu=("CPU · RYZEN 7 9800X3D", "64 / 95°C", "calm", "38", "% · 5.1 GHz", (40, 38, 18, 0.14)),
        gpu=("GPU · RTX 3080", "41 / 95°C", "calm", "8", "% · 2.1 G / 10 G", (40, 9, 9, 0.14)),
        mem=("MEMORY", "SWAP 0 B", "", "21G", "/ 60 G", (40, 35, 1.5, 0.3)),
        psi=("PRESSURE", "SOME · 10 S", "", "0.0", "mem · cpu 1.8 · io 0.3", None),
        apps=[("Chrome", "40 procs", "7.6 G", 7.6 / 60, ACCENT), ("storefront", "lowprio scope", "3.1 G / 16 G", 0.19, CALM),
              ("T3Code", "14 procs", "1.5 G", 1.5 / 60, ACCENT), ("Slack", "9 procs", "820 M", 0.8 / 60, ACCENT),
              ("Spotify", "6 procs", "410 M", 0.4 / 60, ACCENT)],
    ),
    "act": dict(
        chip="needs you", reasons=["everything is stalling on memory (22% of the last 10 s)"],
        yard="Switchyard is holding android: Roundhouse says everything is stalling on memory",
        cpu=("CPU · RYZEN 7 9800X3D", "71 / 95°C", "calm", "64", "% · 5.2 GHz", (40, 62, 18, 0.14)),
        gpu=("GPU · RTX 3080", "41 / 95°C", "calm", "8", "% · 2.1 G / 10 G", (40, 9, 9, 0.14)),
        mem=("MEMORY", "SWAP 2.4 G", "", "58G", "/ 60 G", (40, 94, 1.5, 0.3)),
        psi=("PRESSURE", "SOME · 10 S", "", "22.4", "mem · cpu 1.8 · io 0.3", None),
        apps=[("storefront", "lowprio scope", "14.1 G / 16 G", 0.88, WARN), ("Chrome", "40 procs", "7.6 G", 7.6 / 60, ACCENT),
              ("T3Code", "14 procs", "1.5 G", 1.5 / 60, ACCENT), ("Slack", "9 procs", "820 M", 0.8 / 60, ACCENT),
              ("Spotify", "6 procs", "410 M", 0.4 / 60, ACCENT)],
    ),
}
MACHINE["heads-up"] = dict(MACHINE["calm"], chip="1 heads-up", reasons=["CPU is at 88°C (limit 95°C)"],
                           cpu=("CPU · RYZEN 7 9800X3D", "88 / 95°C", "heads-up", "92", "% · 5.2 GHz", (40, 90, 10, 0.14)))


def hook(fx, role, i=0):
    f = (fx or {}).get(role)
    return " " + f(i) if f else ""


def walk(n, base, amp, pull, seed=7):
    rnd = random.Random(seed * 1000 + int(base))
    v, pts = base, []
    for _ in range(n):
        v += (rnd.random() - 0.5) * amp + (base - v) * pull
        pts.append(max(2, min(98, v)))
    return pts


def spark(add, x, y, w, h, values, vmax, fx, i):
    """Widgets' Spark: an area, its line and an end dot, in the accent."""
    n = len(values)
    pts = [(x + k / (n - 1) * w, y + h - values[k] / vmax * (h - 2)) for k in range(n)]
    line = " ".join(f"{a:.1f},{b:.1f}" for a, b in pts)
    add(f'<path d="M{x:.1f},{y + h:.1f} L{line} L{x + w:.1f},{y + h:.1f} Z" fill="{ACCENT}" fill-opacity=".16"{hook(fx, "fade", i)}/>')
    add(f'<polyline points="{line}" fill="none" stroke="{ACCENT}" stroke-width="1.4" stroke-linejoin="round" '
        f'pathLength="100" stroke-dasharray="100"{hook(fx, "draw", i)}/>')
    ex, ey = pts[-1]
    add(f'<circle cx="{ex - 1.5:.1f}" cy="{ey:.1f}" r="2" fill="{ACCENT}"{hook(fx, "fade", i)}/>')


def box(add, x, y, w, h, fill=TILE, line=TILE_LINE, rx=10):
    add(f'<rect x="{x:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h:.1f}" rx="{rx}" fill="{fill}" stroke="{line}"/>')


def label(add, x, y, s, fill=DIM, anchor="start"):
    """.t-label: small bold caps."""
    add(t(x, y, s, 10, fill, 700, anchor, 1))


def tile(add, x, y, w, h, data, fx, i, talkers=None):
    """A metric tile: label and aside, a big number and its unit, a sparkline."""
    title, aside, cls, value, unit, series = data
    box(add, x, y, w, h)
    label(add, x + w - 10, y + 19, aside, LEVEL_COLOR.get(cls, DIM), "end")
    label(add, x + 10, y + 19, fit(title, 10, w - 30 - adv(aside, 10, 1), 1))
    add(t(x + 10, y + 45, value, 20, FG, 700))
    add(t(x + 15 + adv(value, 20), y + 45, fit(unit, 11, w - 25 - adv(value, 20)), 11, DIM))
    if series:
        n, base, amp, pull = series
        spark(add, x + 10, y + 52, w - 20, 26, walk(n, base, amp, pull), 100, fx, i)
    if talkers:
        add(t(x + 10, y + 94, fit(talkers, 10.5, w - 20), 10.5, DIM))


ROW, ROW_METER, HEAD_PAD = 23, 29, 6


def rows(add, x, y, w, items, fx=None, meter_role=None):
    """widgets::Rows: name, dim detail that ellipsizes, right-hand text, and
    an optional meter. Items are dicts with name, detail, right, cls (right
    text color), meter (0..1), color (meter fill), head, dim. Returns the y
    below the last row."""
    for k, r in enumerate(items):
        if r.get("head") and k:
            y += HEAD_PAD
        base = y + 16
        right = r.get("right", "")
        rw = adv(right, 12) + 8 if right else 0
        name = r["name"]
        nw = adv(name, 12)
        detail = fit(r.get("detail", ""), 12, w - nw - 8 - rw)
        op = ' opacity=".4"' if r.get("dim") else ""
        add(f"<g{op}>")
        add(t(x, base, name, 12, FG, 700 if r.get("head") else 400))
        if detail:
            add(t(x + nw + 8, base, detail, 12, DIM))
        if right:
            add(t(x + w, base, right, 12, LEVEL_COLOR.get(r.get("cls", ""), DIM), anchor="end"))
        if r.get("meter") is not None:
            my = y + 22
            add(f'<rect x="{x:.1f}" y="{my:.1f}" width="{w:.1f}" height="4" rx="2" fill="{TILE_LINE}"/>')
            role = hook(fx, meter_role, k) if meter_role else ""
            add(f'<rect x="{x:.1f}" y="{my:.1f}" width="{max(3, w * r["meter"]):.1f}" height="4" rx="2" fill="{r.get("color", ACCENT)}"{role}/>')
            y += ROW_METER
        else:
            y += ROW
        add("</g>")
    return y


def card(add, x, y, w, title, aside, items, fx=None, meter_role=None, below=None):
    """A .card: label row, then rows; `below(add, y)` draws more inside it
    and returns the new y. Returns the card's bottom."""
    end = y + 24
    # Measure first, so the card's box goes under its rows.
    sink = []
    end = rows(sink.append, x + 10, end, w - 20, items)
    if below:
        end = below(sink.append, end + 4)
    h = end - y + 8
    box(add, x, y, w, h)
    label(add, x + 10, y + 19, title)
    if aside:
        label(add, x + w - 10, y + 19, aside, anchor="end")
    rows(add, x + 10, y + 24, w - 20, items, fx, meter_role)
    if below:
        below(add, rows(lambda s: None, x + 10, y + 24, w - 20, items) + 4)
    return y + h


def line_button(add, x, y, w, s, h=30):
    """.yard-line: a full-width button in a tile."""
    box(add, x, y, w, h)
    add(t(x + 10, y + h / 2 + 4.5, fit(s, 12, w - 20), 12, FG))


def drawer_head(add, level="calm", yard="building"):
    """The header (lamp, host, uptime, chip) and the tab row with every tab
    inactive."""
    color = LEVEL_COLOR[level]
    add(f'<circle cx="{P + 5}" cy="29" r="5" fill="{color}"/>')
    add(t(P + 20, 33, "FROSTYARCH", 14, FG, 700, spacing=1))
    add(t(P + 20, 49, "up 2d 18h · linux 7.2.14", 11, DIM))
    chip = MACHINE[level]["chip"]
    cw = adv(chip, 11) + 18
    add(f'<rect x="{DRAWER_W - P - cw:.1f}" y="19" width="{cw:.1f}" height="20" rx="10" fill="{color}" fill-opacity=".14"/>')
    add(t(DRAWER_W - P - cw / 2, 33, chip, 11, color, 700, "middle"))
    add(f'<rect x="{P}" y="{TAB_Y}" width="{IW}" height="{TAB_H}" rx="9" fill="{TILE}"/>')
    for i, (tid, name) in enumerate(TABS):
        cx = tab_x(i) + TAB_W / 2
        if tid == "switchyard":
            # A link to the board, not a page: dot, name, arrow.
            nw = 64
            x0 = cx - (5 + 4 + nw + 4 + 7) / 2
            add(f'<circle cx="{x0 + 2.5:.1f}" cy="{TAB_Y + 14.5}" r="2.5" fill="{YARD_DOT[yard]}"/>')
            add(t(x0 + 9, TAB_Y + 19.5, name, 11.5, DIM, attrs=f'textLength="{nw}" lengthAdjust="spacingAndGlyphs"'))
            add(t(x0 + 9 + nw + 4, TAB_Y + 19.5, "↗", 11.5, DIM))
        else:
            add(t(cx, TAB_Y + 19.5, name, 12, DIM, 700, "middle"))


def drawer_foot(add, h=DRAWER_H):
    add(t(P, h - 17, "1-5 tabs · Esc close · Arch Linux", 10.5, DIM))


def drawer_frame(add, level="calm", yard="building", h=DRAWER_H):
    """The panel at height h, its header and tab row, and the foot."""
    add(f'<rect width="{DRAWER_W}" height="{h}" rx="16" fill="{PANEL}" fill-opacity=".97" stroke="{PANEL_LINE}" filter="url(#shadow)"/>')
    drawer_head(add, level, yard)
    drawer_foot(add, h)


def drawer_tab(add, tab):
    """The active tab's highlight: an accent button with dark text."""
    i = [tid for tid, _ in TABS].index(tab)
    add(f'<rect x="{tab_x(i):.1f}" y="{TAB_Y + 3}" width="{TAB_W:.1f}" height="{TAB_H - 6}" rx="7" fill="{ACCENT}"/>')
    add(t(tab_x(i) + TAB_W / 2, TAB_Y + 19.5, TABS[i][1], 12, WELL, 700, "middle"))


def page_clip(add, clip):
    add(f'<clipPath id="{clip}"><rect x="{P - 2}" y="{VIEW_Y0}" width="{IW + 4}" height="{VIEW_Y1 - VIEW_Y0}"/></clipPath>')


MACHINE_SCROLL = 254  # how far the Machine page scrolls to its end


def page_machine(add, level="calm", fx=None, clip="rh-machine"):
    """Machine: the verdict when the lamp is lit, Switchyard's line, CPU,
    GPU, memory and pressure tiles, the network tile with its busiest apps,
    fans, audio, and memory by app. Taller than the scroller: the rest is
    MACHINE_SCROLL px further down."""
    d = MACHINE[level]
    page_clip(add, clip)
    add(f'<g clip-path="url(#{clip})"><g{hook(fx, "scroll")}>')
    y, n = VIEW_Y0, 0
    if d["reasons"]:
        color = LEVEL_COLOR[level]
        h = 14 + 20 * len(d["reasons"])
        add(f'<g{hook(fx, "rise", n)}>')
        add(f'<rect x="{P}" y="{y}" width="{IW}" height="{h}" rx="10" fill="{color}" fill-opacity=".1" stroke="{color}" stroke-opacity=".35"/>')
        for k, r in enumerate(d["reasons"]):
            add(t(P + 10, y + 21 + 20 * k, fit(r, 12, IW - 20), 12, FG))
        add("</g>")
        y, n = y + h + 8, n + 1
    add(f'<g{hook(fx, "rise", n)}>')
    line_button(add, P, y, IW, d["yard"])
    add("</g>")
    y, n = y + 38, n + 1
    tw = (IW - 8) / 2
    for k, key in enumerate(["cpu", "gpu", "mem", "psi"]):
        add(f'<g{hook(fx, "rise", n + k)}>')
        tile(add, P + (k % 2) * (tw + 8), y + (k // 2) * 94, tw, 86, d[key], fx, k)
        add("</g>")
    y, n = y + 188, n + 4
    add(f'<g{hook(fx, "rise", n)}>')
    net = ("NETWORK · ENO1", "", "", "↓ 1.2 MB/s", "↑ 84 kB/s", (60, 30, 26, 0.08))
    tile(add, P, y, IW, 102, net, fx, 4, talkers="Chrome 940 kB/s · npm 310 kB/s · Slack 2 kB/s")
    add("</g>")
    y, n = y + 110, n + 1
    add(f'<g{hook(fx, "rise", n)}>')
    add(t(P + 2, y + 10, "FANS · CPU 1180 RPM · RTX 3080 38%", 10.5, DIM, spacing=0.4))
    add("</g>")
    y, n = y + 20, n + 1
    add(f'<g{hook(fx, "rise", n)}>')
    y = card(add, P, y, IW, "AUDIO", "", [
        dict(name="PipeWire", detail="Arctis Nova Pro", right="1024 @ 48 kHz · 21.3 ms · DSP 4%", head=True),
        dict(name="Arctis Nova Pro", detail="playing · Spotify", right="48 kHz · S24_3LE · 2 ch"),
        dict(name="MIDI", detail="Arturia KeyStep 37"),
    ]) + 8
    add("</g>")
    n += 1
    add(f'<g{hook(fx, "rise", n)}>')
    end = card(add, P, y, IW, "MEMORY BY APP", "NOT BY PID", [
        dict(name=a, detail=b, right=c, meter=f, color=col) for a, b, c, f, col in d["apps"]
    ])
    add("</g>")
    add("</g></g>")
    sb = (fx or {}).get("scrollbar")
    if sb:
        # GTK's overlay indicator, shown while the page scrolls.
        view = VIEW_Y1 - VIEW_Y0
        length = view * view / (view + MACHINE_SCROLL)
        add(f'<rect x="{P + IW - 4}" y="{VIEW_Y0 + 2}" width="3" height="{length - 4:.1f}" rx="1.5" fill="{FG}" fill-opacity=".45" {sb(0)}/>')
    return end


# The app whose connections close while the Network tab is open.
CLOSING = 1


def page_network(add, closed=False, fx=None, clip="rh-network"):
    """Network: the way out hop by hop with the public IP hidden behind a
    button, the link, and every app talking out. The second app (npm)
    finishes: with closed=True it is a dimmed shadow marked closed."""
    page_clip(add, clip)
    add(f'<g clip-path="url(#{clip})">')
    y = VIEW_Y0

    def show_ip(sink, y0):
        line_button(sink, P + 10, y0, IW - 20, "Show public IP", 28)
        return y0 + 28

    add(f'<g{hook(fx, "rise", 0)}>')
    way = [
        dict(name="Route", detail="via 192.168.1.1 on eno1", right="IPv4 only"),
        dict(name="Router", detail="192.168.1.1", right="0.2 ms", cls="calm"),
        dict(name="Internet", detail="1.1.1.1", right="11.8 ms", cls="calm"),
        dict(name="DNS", detail="looks up example.com", right="14.0 ms", cls="calm"),
        dict(name="Public IP", detail="hidden", right="NAT keeps ports"),
    ]
    y = card(add, P, y, IW, "WAY OUT", "", way, below=show_ip) + 8
    add("</g>")
    add(f'<g{hook(fx, "rise", 1)}>')
    y = card(add, P, y, IW, "LINKS", "", [
        dict(name="eno1", detail="2.5 Gb/s", right="↓ 1.2 MB/s ↑ 84 kB/s", meter=0.004, color=CALM),
    ]) + 8
    add("</g>")
    apps = [
        dict(name="Chrome", detail="192.0.2.14:443, 192.0.2.80:443 +16", right="↓ 940 kB/s ↑ 12 kB/s"),
        dict(name="npm", detail="198.51.100.7:443 +3", right="↓ 310 kB/s ↑ 4 kB/s"),
        dict(name="Slack", detail="203.0.113.9:443 +2", right="3 TCP"),
        dict(name="ssh", detail="192.168.1.51:22", right="1 TCP"),
    ]
    shadow = dict(apps[CLOSING], detail="closed", right="", dim=True)
    both = fx and "live" in fx and "gone" in fx
    if closed and not both:
        apps[CLOSING] = shadow
    add(f'<g{hook(fx, "rise", 2)}>')
    if both:
        # Draw the card with the live row hidden, then each version on top.
        live = apps[CLOSING]
        apps[CLOSING] = dict(name="", detail="")
        end = card(add, P, y, IW, "APPS", "TCP · BY APP", apps)
        ry = y + 24 + CLOSING * ROW
        for role, r in (("live", live), ("gone", shadow)):
            add(f'<g{hook(fx, role)}>')
            rows(add, P + 10, ry, IW - 20, [dict(r, dim=False)])
            add("</g>")
    else:
        end = card(add, P, y, IW, "APPS", "TCP · BY APP", apps)
    add("</g>")
    add("</g>")
    return end


def page_storage(add, fx=None, clip="rh-storage"):
    """Storage: each disk with its temperature, its filesystems with
    meters, a network share and an automount left asleep."""
    page_clip(add, clip)
    add(f'<g clip-path="url(#{clip})">')
    items = [
        dict(name="Sabrent", detail="nvme0n1", right="41 / 75°C", cls="calm", head=True),
        dict(name="/ +1", detail="btrfs", right="143 G / 1.9 T · 8%", meter=0.08),
        dict(name="/boot", detail="vfat", right="237 M / 2.0 G · 12%", meter=0.12),
        dict(name="Samsung SSD 990 PRO 2TB", detail="nvme1n1", right="48 / 82°C", cls="calm", head=True),
        dict(name="~/Sites +1", detail="btrfs", right="1.3 T / 1.8 T · 72%", meter=0.72),
        dict(name="Samsung SSD 990 PRO 2TB", detail="nvme2n1", right="46 / 82°C", cls="calm", head=True),
        dict(name="/scratch +1", detail="ext4", right="322 G / 1.8 T · 18%", meter=0.18),
        dict(name="nas · media", detail="network share", head=True),
        dict(name="~/Media", detail="cifs · 1 ms", right="4.1 T / 6.6 T · 62%", meter=0.62),
        dict(name="backup", detail="automount", right="not mounted, not woken", head=True),
    ]
    add(f'<g{hook(fx, "rise", 0)}>')
    end = card(add, P, VIEW_Y0, IW, "DISKS AND SHARES", "", items, fx, "meter")
    add("</g>")
    add("</g>")
    return end


PAGES = {"machine": page_machine, "network": page_network, "storage": page_storage}


def drawer_height(tab, level="calm"):
    """The panel fits its page (GTK interpolates between pages) up to
    DRAWER_H, where the Machine page scrolls instead."""
    args = (level,) if tab == "machine" else ()
    end = PAGES[tab](lambda s: None, *args)
    return min(DRAWER_H, round(end) + DRAWER_H - VIEW_Y1)


def drawer(add, tab="machine", level="calm", yard="building", closed=False, fx=None, clip="rh"):
    """The whole drawer as a still, drawer_height(tab, level) tall: the
    frame, the active tab and its page. tab: machine, storage or network.
    level: calm, heads-up or act (the lamp, the chip, the verdict and the
    Machine page's numbers). yard: Switchyard's phase, for the dot on its
    link. closed: on the Network tab, npm has finished and shows as a
    shadow. The drawer is anchored at its bottom, 8 px above the bar."""
    drawer_frame(add, level, yard, drawer_height(tab, level))
    drawer_tab(add, tab)
    if tab == "machine":
        page_machine(add, level, fx, f"{clip}-machine")
    elif tab == "network":
        page_network(add, closed, fx, f"{clip}-network")
    elif tab == "storage":
        page_storage(add, fx, f"{clip}-storage")


# ---- end of the panel ----
# dot-files' bar hero (docs/bar/apps.py) runs everything above this line
# to draw the panel in its own tour, so keep the panel's code above it and
# the story below it.

# ---------------------------------------------------------------- the story

CLICK_LAMP, OPEN = 1.0, 1.08
SCROLL = 3.5  # the wheel turns
CLICK_NET = 5.3
GONE = 6.9  # npm's connections close
CLICK_STORE = 8.7
CLICK_OUT, CLOSED = 11.0, 11.45

out, kfs = [], []


def add(s):
    out.append(s)


def pct(sec):
    return f"{max(0.0, min(100.0, sec / T * 100)):.2f}%"


def keys(pts):
    """A keyframe set from (seconds, declarations) points; returns its name."""
    name = f"k{len(kfs)}"
    kfs.append(f"@keyframes {name} {{ " + " ".join(f"{pct(s)} {{ {v}; }}" for s, v in pts) + " }")
    return name


def anim(prop, pts):
    return keys([(s, f"{prop}: {v}") for s, v in pts])


def fade(spans, ramp=0.2):
    """Opacity keyframes, visible during each (start, end) span."""
    pts = []
    for s, e in spans:
        if s <= 0:
            pts.append((0, 1))
        else:
            pts += [(0, 0)] if not pts else []
            pts += [(s, 0), (s + ramp, 1)]
        if e >= T:
            pts.append((T, 1))
        else:
            pts += [(e, 1), (e + ramp, 0)]
    if pts[-1][0] < T:
        pts.append((T, 0))
    return anim("opacity", pts)


def style(*names, extra="", cls="a"):
    return f'class="{cls}" style="animation-name:{",".join(names)};{extra}"'


# ---- desktop
add(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" font-family="{FONT}" xml:space="preserve" role="img" aria-label="__ALT__">')
STYLE_AT = len(out)
add("")
add(f"""<defs>
<linearGradient id="wall" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{BG0}"/><stop offset="1" stop-color="{BG1}"/></linearGradient>
<radialGradient id="haze1" cx=".85" cy="0" r=".75"><stop offset="0" stop-color="{FROST}" stop-opacity=".13"/><stop offset="1" stop-color="{FROST}" stop-opacity="0"/></radialGradient>
<radialGradient id="haze2" cx=".05" cy="1" r=".6"><stop offset="0" stop-color="{PURPLE}" stop-opacity=".09"/><stop offset="1" stop-color="{PURPLE}" stop-opacity="0"/></radialGradient>
<linearGradient id="cover" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#5e81ac"/><stop offset="1" stop-color="{PURPLE}"/></linearGradient>
<filter id="shadow" x="-20%" y="-20%" width="140%" height="150%"><feDropShadow dx="0" dy="18" stdDeviation="22" flood-color="#000" flood-opacity=".65"/></filter>
<clipPath id="screen"><rect width="{W}" height="{H}" rx="18"/></clipPath>
</defs>""")
add('<g clip-path="url(#screen)">')
add(f'<rect width="{W}" height="{H}" fill="url(#wall)"/><rect width="{W}" height="{H}" fill="url(#haze1)"/><rect width="{W}" height="{H}" fill="url(#haze2)"/>')

# T3 Code on workspace 3, with an editor behind it, so the drawer has
# something to sit over.
rnd = random.Random(7)
for (x, y, w, h, n) in [(36, 36, 560, 360, 11), (210, 300, 470, 300, 8)]:
    add(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="12" fill="#1a1d22" stroke="#2a2e35"/>')
    add(f'<rect x="{x}" y="{y}" width="{w}" height="30" rx="12" fill="#1f2328"/>')
    for k, c in enumerate([RED, YELLOW, GREEN]):
        add(f'<circle cx="{x + 18 + k * 16}" cy="{y + 15}" r="5" fill="{c}" fill-opacity=".55"/>')
    for i in range(n):
        lw = rnd.randint(int(w * 0.25), int(w * 0.8))
        indent = rnd.choice([0, 0, 24, 48])
        col = rnd.choice(["#3b4252", "#434c5e", "#4c566a", "#5e81ac55", "#88c0d044"])
        add(f'<rect x="{x + 22 + indent}" y="{y + 50 + i * 24}" width="{lw - indent}" height="9" rx="4" fill="{col}"/>')

# ---- the bar: pills floating 6 px off the bottom (config.jsonc margins)
BAR_Y, BAR_H = 620, 34
MID = BAR_Y + BAR_H / 2
BASE = MID + 4.5  # text baseline
BS, CW = 13, 7.8  # bar font size and its advance
GS = 13  # glyph size


def pill(x, w):
    add(f'<rect x="{x:.1f}" y="{BAR_Y + 0.5}" width="{w:.1f}" height="{BAR_H - 1}" rx="10" fill="{PILL}" stroke="{PILL_LINE}"/>')


# Left: workspaces (a dim number, then a glyph per window) and the title.
WS = [("1", ["chrome"]), ("2", ["slack"]), ("3", ["t3"]), ("4", ["vim", "term"])]
ACTIVE = "3"
GLYPH_ADV = 15  # the title's glyph


def gadv(g):
    """A window's glyph and the space before it."""
    return CW * 0.5 + glyph_w(g, GS) + 3


def ws_width(wins):
    return 8 + CW + sum(gadv(g) for g in wins) + 8


ws_w = 4 + sum(ws_width(w) + 2 for _, w in WS) + 4
pill(6, ws_w)
x = 6 + 4 + 1
for num, wins in WS:
    bw = ws_width(wins)
    on = num == ACTIVE
    if on:
        add(f'<rect x="{x:.1f}" y="{BAR_Y + 4}" width="{bw:.1f}" height="{BAR_H - 8}" rx="7" fill="{WS_ACTIVE}"/>')
    add(t(x + 8, BASE, num, BS, BRIGHT if on else BAR_FG, attrs='fill-opacity=".55"'))
    gx = x + 8 + CW
    for g in wins:
        add(glyph(g, gx + gadv(g) - (glyph_w(g, GS) + 3) / 2, MID, GS, BRIGHT if on else BAR_FG))
        gx += gadv(g)
    x += bw + 2
title = "T3 Code"
WIN_X = 6 + ws_w + 6
win_w = 12 + GLYPH_ADV + CW + len(title) * CW + 12
pill(WIN_X, win_w)
add(glyph("t3", WIN_X + 12 + 7, MID, GS, BRIGHT))
add(t(WIN_X + 12 + GLYPH_ADV + CW, BASE, title, BS, BRIGHT))

# Right, packed from the edge: tray, clock, desk, audio, lamps, media.
x = W - 6
TRAY = [("slack", "#e01e5a"), ("discord", "#7289da"), ("bluetooth", "#5e81ac")]
tray_w = 10 + len(TRAY) * 14 + (len(TRAY) - 1) * 8 + 10
x -= tray_w
pill(x, tray_w)
for k, (g, c) in enumerate(TRAY):
    add(glyph(g, x + 10 + 7 + k * 22, MID, GS, c))

clock = "Tue Nov 10  14:32"
clock_w = 12 + len(clock) * CW + 12
x -= 6 + clock_w
pill(x, clock_w)
add(t(x + 12, BASE, clock, BS, PURPLE))


def slots(x0, items):
    """Modules in a pill: 4 px pill padding, 8 px module padding."""
    xs, x = [], x0 + 4
    for w in items:
        xs.append((x, x + 16 + w))
        x += 16 + w
    return xs, x + 4 - x0


# Desk: Logbook's mic lamp (dim: nothing listens), lit key lights only while
# on (they are off), and the drawer's chevron handle.
desk, desk_w = slots(0, [11, 9])
x -= 6 + desk_w
pill(x, desk_w)
add(glyph("mic", x + (desk[0][0] + desk[0][1]) / 2, MID, GS, BAR_DIM))
add(glyph("chevron", x + (desk[1][0] + desk[1][1]) / 2, MID, GS * 0.9, BAR_DIM))

# Audio: volume in frost, the mouse's battery dim.
vol_w = glyph_w("vol", GS) + 5 + 3 * CW
mouse_w = glyph_w("mouse", GS) + 5 + 3 * CW
audio, audio_w = slots(0, [vol_w, mouse_w])
x -= 6 + audio_w
pill(x, audio_w)
for (a, _), g, s, c in [(audio[0], "vol", "42%", FROST), (audio[1], "mouse", "80%", BAR_DIM)]:
    gw = glyph_w(g, GS)
    add(glyph(g, x + a + 8 + gw / 2, MID, GS, c))
    add(t(x + a + 8 + gw + 5, BASE, s, BS, c))

# Lamps: Roundhouse (green, calm), Switchyard (amber, building), Signal Box
# (quiet: a dim ring), the VPN (dim, disconnected), the bell (dim).
LAMPS = [("lamp", GREEN), ("yard", YELLOW), ("broker", BAR_DIM), ("vpn", BAR_DIM), ("bell", BAR_DIM)]
lamps, lamps_w = slots(0, [12] * len(LAMPS))
x -= 6 + lamps_w
LAMPS_X = x
pill(x, lamps_w)
LAMP_SLOT = (x + lamps[0][0], x + lamps[0][1])
LAMP_X, LAMP_Y = (LAMP_SLOT[0] + LAMP_SLOT[1]) / 2, MID
hover = fade([(CLICK_LAMP - 0.25, 2.55)], 0.12)
add(f'<rect x="{LAMP_SLOT[0]:.1f}" y="{BAR_Y + 3}" width="{LAMP_SLOT[1] - LAMP_SLOT[0]:.1f}" height="{BAR_H - 6}" rx="7" fill="{HOVER}" {style(hover)} opacity="0"/>')
for (a, b), (g, c) in zip(lamps, LAMPS):
    add(glyph(g, x + (a + b) / 2, MID, GS * (0.95 if g == "lamp" else 1), c))

# Media: Tannoy's cover, then the song in yellow while it plays.
song = "Lofi Rails · Night Shift"
media_w = 10 + 18 + 8 + len(song) * CW + 12
x -= 6 + media_w
pill(x, media_w)
add(f'<rect x="{x + 10:.1f}" y="{MID - 9:.1f}" width="18" height="18" rx="3" fill="url(#cover)"/>')
add(f'<circle cx="{x + 23:.1f}" cy="{MID - 3:.1f}" r="3" fill="#eceff4" fill-opacity=".85"/>')
add(f'<path d="M{x + 10:.1f},{MID + 5:.1f} l5,-4 l4,3 l5,-5 l4,3 v7 h-18 z" fill="#2e3440" fill-opacity=".7"/>')
add(t(x + 36, BASE, song, BS, YELLOW))
assert x > WIN_X + win_w + 12, "the bar's left and right halves overlap"

# ---- the drawer, centered on the lamp and 8 px above the bar
DX = min(max(LAMP_X - DRAWER_W / 2, 10), W - 10 - DRAWER_W)
DY = BAR_Y - 8 - DRAWER_H


def at_screen(spot):
    sx, sy = spot
    return DX + sx, DY + sy


spring = anim("transform", [
    (0, "translateY(26px) scale(.06)"), (OPEN - 0.01, "translateY(26px) scale(.06)"),
    (OPEN + 0.10, "translateY(14px) scale(.5)"), (OPEN + 0.20, "translateY(4px) scale(.9)"),
    (OPEN + 0.30, "translateY(-1px) scale(1.035)"), (OPEN + 0.42, "translateY(0px) scale(.988)"),
    (OPEN + 0.56, "translateY(0px) scale(1.006)"), (OPEN + 0.72, "translateY(0px) scale(1)"),
    (CLICK_OUT + 0.08, "translateY(0px) scale(1)"), (CLOSED, "translateY(14px) scale(.4)"), (T, "translateY(14px) scale(.4)"),
])
show = anim("opacity", [(0, 0), (OPEN - 0.01, 0), (OPEN + 0.12, 0.85), (OPEN + 0.2, 1), (CLICK_OUT + 0.08, 1), (CLOSED, 0), (T, 0)])
add(f'<g {style(spring, show, extra=f"transform-origin:{LAMP_X:.1f}px {LAMP_Y:.1f}px")}><g transform="translate({DX:.1f},{DY:.1f})">')
LIT = CLOSED + 0.1  # back on the Machine tab for the next opening

# The panel fits each page and GTK interpolates its height; anchored at the
# bottom, its top moves. Two rounded caps and a middle that stretches
# between them, so the corners keep their shape.
DROP = {tab: DRAWER_H - drawer_height(tab) for tab in PAGES}
GROW = 0.22
size = [(0, 0), (CLICK_NET, 0), (CLICK_NET + GROW, DROP["network"]), (CLICK_STORE, DROP["network"]),
        (CLICK_STORE + GROW, DROP["storage"]), (LIT, DROP["storage"]), (LIT + 0.01, 0), (T, 0)]
slide = style(anim("transform", [(s, f"translateY({d}px)") for s, d in size]), extra="animation-timing-function:ease-out")
span = DRAWER_H - 40
squash = style(anim("transform", [(s, f"scaleY({(span - d) / span:.4f})") for s, d in size]),
               extra="animation-timing-function:ease-out;transform-box:fill-box;transform-origin:center bottom")
add('<g filter="url(#shadow)">')
add(f'<rect y="{DRAWER_H - 40}" width="{DRAWER_W}" height="40" rx="16" fill="{PANEL}" stroke="{PANEL_LINE}"/>')
add(f'<rect width="{DRAWER_W}" height="40" rx="16" fill="{PANEL}" stroke="{PANEL_LINE}" {slide}/>')
add(f'<g {squash}><rect y="20" width="{DRAWER_W}" height="{span}" fill="{PANEL}"/>')
add(f'<path d="M0,20 V{DRAWER_H - 20} M{DRAWER_W},20 V{DRAWER_H - 20}" stroke="{PANEL_LINE}"/></g>')
add("</g>")
drawer_foot(add)
# Pages slide past the panel's padding and shrink under its foot, so clip
# them to the scroller: its bottom stays put, its sides move with nothing.
add(f'<clipPath id="view"><rect width="{DRAWER_W}" height="{VIEW_Y1}"/></clipPath>')
add(f'<clipPath id="sides"><rect x="{P - 2}" width="{IW + 4}" height="{DRAWER_H}"/></clipPath>')
add(f'<g clip-path="url(#view)"><g {slide}>')
drawer_head(add, "calm", "building")

# Tabs light as they are picked (a 200 ms background transition, not a slide).
for tab, spans, on in [
    ("machine", [(0, CLICK_NET + 0.02), (LIT, T)], True),
    ("network", [(CLICK_NET + 0.02, CLICK_STORE + 0.02)], False),
    ("storage", [(CLICK_STORE + 0.02, LIT)], False),
]:
    add(f'<g {style(fade(spans, 0.2))} opacity="{1 if on else 0}">')
    drawer_tab(add, tab)
    add("</g>")


def rise(i, start):
    return style(keys([
        (0, "opacity: 0; transform: translateY(12px)"), (start + i * 0.06, "opacity: 0; transform: translateY(12px)"),
        (start + i * 0.06 + 0.4, "opacity: 1; transform: none"), (T, "opacity: 1; transform: none"),
    ]), extra="animation-timing-function:cubic-bezier(.2,.8,.2,1)")


def page(spans):
    """A page of the stack, sliding in and out as GTK's SlideLeftRight does:
    each span is (start, end, from, to), the sides it comes in from and
    leaves to (-1 left, 1 right, 0 in place)."""
    here, pts = "opacity: 1; transform: translateX(0px)", []
    for s, e, a, b in spans:
        if s <= 0:
            pts.append((0, here))
        else:
            away = f"opacity: 0; transform: translateX({36 * a}px)"
            if not pts:
                pts.append((0, away))
            pts += [(s, away if a else pts[-1][1]), (s + (0.24 if a else 0.01), here)]
        if e < T:
            pts += [(e, here), (e + 0.24, f"opacity: 0; transform: translateX({36 * b}px)")]
    if pts[-1][0] < T:
        pts.append((T, pts[-1][1]))
    return style(keys(pts), extra="animation-timing-function:cubic-bezier(.3,0,.2,1)")


START = OPEN + 0.12
fall = keys([(0, "transform: translateY(0px)"), (SCROLL, "transform: translateY(0px)"),
             (SCROLL + 0.7, f"transform: translateY(-{MACHINE_SCROLL}px)"),
             (LIT, f"transform: translateY(-{MACHINE_SCROLL}px)"), (LIT + 0.01, "transform: translateY(0px)"), (T, "transform: translateY(0px)")])
bar = keys([(0, "opacity: 0; transform: translateY(0px)"), (SCROLL - 0.05, "opacity: 0; transform: translateY(0px)"),
            (SCROLL + 0.1, "opacity: 1; transform: translateY(0px)"),
            (SCROLL + 0.7, f"opacity: 1; transform: translateY({MACHINE_SCROLL * (VIEW_Y1 - VIEW_Y0) / (VIEW_Y1 - VIEW_Y0 + MACHINE_SCROLL):.1f}px)"),
            (SCROLL + 1.5, f"opacity: 0; transform: translateY({MACHINE_SCROLL * (VIEW_Y1 - VIEW_Y0) / (VIEW_Y1 - VIEW_Y0 + MACHINE_SCROLL):.1f}px)"),
            (T, f"opacity: 0; transform: translateY(0px)")])
draw = keys([(0, "stroke-dashoffset: 100"), (OPEN + 0.25, "stroke-dashoffset: 100"), (OPEN + 1.6, "stroke-dashoffset: 0"), (T, "stroke-dashoffset: 0")])
spark_in = keys([(0, "opacity: 0"), (OPEN + 0.5, "opacity: 0"), (OPEN + 1.1, "opacity: 1"), (T, "opacity: 1")])
machine_fx = {
    "rise": lambda i: rise(i, START),
    "scroll": lambda i: style(fall, extra="animation-timing-function:cubic-bezier(.2,.7,.2,1)"),
    "scrollbar": lambda i: style(bar, extra="animation-timing-function:cubic-bezier(.2,.7,.2,1)") + ' opacity="0"',
    "draw": lambda i: style(draw, extra="animation-timing-function:ease-out"),
    "fade": lambda i: style(spark_in),
}
add('<g clip-path="url(#sides)">')
add(f"<g {page([(0, CLICK_NET, 0, -1), (LIT, T, 0, 0)])}>")
page_machine(add, "calm", machine_fx)
add("</g>")

network_fx = {
    "rise": lambda i: rise(i, CLICK_NET + 0.05),
    "live": lambda i: style(fade([(0, GONE)], 0.05)),
    "gone": lambda i: style(anim("opacity", [(0, 0), (GONE, 0), (GONE + 0.05, 1), (GONE + 0.45, 0.4), (T, 0.4)])) + ' opacity="0"',
}
add(f'<g {page([(CLICK_NET, CLICK_STORE, 1, 1)])} opacity="0">')
page_network(add, fx=network_fx)
add("</g>")

grow = keys([(0, "transform: scaleX(0)"), (CLICK_STORE + 0.3, "transform: scaleX(0)"), (CLICK_STORE + 1.3, "transform: scaleX(1)"), (T, "transform: scaleX(1)")])
storage_fx = {
    "meter": lambda i: style(grow, extra=f"transform-box:fill-box;transform-origin:left center;animation-delay:{i * 0.04:.2f}s;animation-timing-function:cubic-bezier(.2,.8,.2,1)"),
}
add(f'<g {page([(CLICK_STORE, LIT, -1, 0)])} opacity="0">')
page_storage(add, storage_fx)
add("</g>")
add("</g>")
add("</g></g>")
add("</g></g>")

# ---- cursor and clicks
REST = (250, 330)
OUT = (230, 250)
NET_TAB, STORE_TAB = at_screen(DRAWER_SPOTS["network"]), at_screen(DRAWER_SPOTS["storage"])
SCROLL_AT = at_screen(DRAWER_SPOTS["machine-page"])
APPS_AT = (DX + 236, DY + 424)
for k, (s, (cx, cy)) in enumerate([(CLICK_LAMP, (LAMP_X, LAMP_Y)), (CLICK_NET, NET_TAB), (CLICK_STORE, STORE_TAB), (CLICK_OUT, OUT)]):
    k1 = anim("opacity", [(0, 0), (s - 0.01, 0), (s, 0.9), (s + 0.45, 0), (T, 0)])
    k2 = anim("transform", [(0, "scale(.3)"), (s, "scale(.3)"), (s + 0.45, "scale(1.6)"), (T, "scale(1.6)")])
    add(f'<circle {style(k1, k2, extra="transform-box:fill-box;transform-origin:center", cls="a ripple")} opacity="0" cx="{cx:.1f}" cy="{cy:.1f}" r="18" fill="none" stroke="{FROST}" stroke-width="2.5"/>')


def tr(p):
    return f"translate({p[0] - 3:.1f}px, {p[1] - 4:.1f}px)"


cur = anim("transform", [
    (0, tr(REST)), (0.2, tr(REST)), (CLICK_LAMP - 0.15, tr((LAMP_X, LAMP_Y))), (2.6, tr((LAMP_X, LAMP_Y))),
    (3.2, tr(SCROLL_AT)), (4.6, tr(SCROLL_AT)),
    (CLICK_NET - 0.15, tr(NET_TAB)), (5.9, tr(NET_TAB)), (6.5, tr(APPS_AT)), (7.9, tr(APPS_AT)),
    (CLICK_STORE - 0.15, tr(STORE_TAB)), (9.9, tr(STORE_TAB)),
    (CLICK_OUT - 0.15, tr(OUT)), (12.2, tr(OUT)), (13.4, tr(REST)), (T, tr(REST)),
])
add(f'<g id="cursor" {style(cur, extra="animation-timing-function:cubic-bezier(.45,0,.25,1)")} transform="translate({REST[0] - 3},{REST[1] - 4})">')
add('<path d="M0,0 L0,22 L6,17 L10,26 L14,24 L10,15 L18,15 Z" fill="#eceff4" stroke="#1d2024" stroke-width="1.4" stroke-linejoin="round"/>')
# The wheel turns.
add(f'<g {style(fade([(SCROLL - 0.25, SCROLL + 0.8)], 0.15))} opacity="0"><rect x="22" y="14" width="58" height="22" rx="6" fill="{PANEL}" stroke="{PANEL_LINE}"/>')
add(t(51, 29, "scroll", 11.5, FG, anchor="middle"))
add("</g></g>")
add("</g></svg>")

ALT = (
    "A floating waybar of frosted pills: workspaces, the T3 Code window title, a song playing, five lamps with "
    "Roundhouse's green dot first and Switchyard's amber puzzle piece beside it, volume, the desk pill and the "
    "clock. A click on the green dot springs the Roundhouse drawer out of it: FROSTYARCH, calm, tabs for Machine, "
    "Storage, Network and Ports, and a Switchyard link dotted with what it is doing. The Machine tab says "
    "Switchyard is building storefront, then CPU, GPU, memory and pressure tiles with temperatures against their "
    "limits, and the network tile names the busiest apps. A scroll brings up the fans, the audio card and memory "
    "by app. The Network tab shows the way out hop by hop with the public IP hidden, the link, and every app "
    "talking out; npm finishes and stays a moment as a dimmed shadow marked closed. The Storage tab fills a meter "
    "for each disk and the network share, and a click outside closes the drawer."
)

css = f"""
text {{ white-space: pre; }}
.a {{ animation-duration: {T}s; animation-iteration-count: infinite; animation-timing-function: linear; animation-fill-mode: both; }}
@media (prefers-reduced-motion: reduce) {{
  .a {{ animation: none !important; }}
  #cursor, .ripple {{ display: none; }}
}}
"""
out[STYLE_AT] = "<style>" + css + "\n".join(kfs) + "</style>"
svg = "\n".join(out).replace("__ALT__", esc(ALT).replace('"', "&quot;")) + "\n"
Path(__file__).with_name("hero.svg").write_text(svg)
print(f"wrote hero.svg ({len(svg) // 1024} KB, {len(kfs)} keyframe sets)")
