#!/usr/bin/env python3
"""Generate and validate the ECorp README arcade artwork.

The raster hero is drawn on a 480x160 logical canvas, then enlarged 3x with
nearest-neighbor sampling. Every color is selected from one fixed 16-color
palette, so the animated GIF stays crisp, deterministic, and compact.

Outputs:
  - ecorp-arcade-hero.gif
  - ecorp-arcade-hero-static.png
  - ecorp-mission-loop.svg

Run:
    python docs/assets/readme/generate_ecorp_arcade_assets.py
"""

from __future__ import annotations

import hashlib
import json
import struct
import xml.etree.ElementTree as ET
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageSequence


ROOT = Path(__file__).resolve().parent
GIF_PATH = ROOT / "ecorp-arcade-hero.gif"
PNG_PATH = ROOT / "ecorp-arcade-hero-static.png"
SVG_PATH = ROOT / "ecorp-mission-loop.svg"

BASE_WIDTH = 480
BASE_HEIGHT = 160
SCALE = 3
FINAL_SIZE = (BASE_WIDTH * SCALE, BASE_HEIGHT * SCALE)
FRAME_COUNT = 32
FRAME_DURATION_MS = 90


# A restrained palette shared by the current ECorp sprite world and this
# original README artwork. There are no gradients or semi-transparent pixels.
COLORS = {
    "bg": "#080712",
    "scanline": "#0D0A1A",
    "shadow": "#05040A",
    "ink": "#12101E",
    "panel": "#211638",
    "violet": "#39245E",
    "slate": "#66647B",
    "cream": "#F7F1DF",
    "cyan": "#35D6D0",
    "mint": "#75C09F",
    "gold": "#FFD447",
    "amber": "#F4A340",
    "magenta": "#F04B8B",
    "red": "#D84D5B",
    "blue": "#6A8CFF",
    "green": "#58D68D",
}

PALETTE_HEX = list(COLORS.values())
PALETTE_RGB = [tuple(bytes.fromhex(color[1:])) for color in PALETTE_HEX]
RGB_TO_INDEX = {rgb: index for index, rgb in enumerate(PALETTE_RGB)}


# Purpose-built 5x7 uppercase font. Only letters used by the hero signage are
# included. Drawing the glyphs as rectangles prevents antialiasing.
PIXEL_FONT = {
    " ": ("00000",) * 7,
    "A": (
        "01110",
        "10001",
        "10001",
        "11111",
        "10001",
        "10001",
        "10001",
    ),
    "C": (
        "01111",
        "10000",
        "10000",
        "10000",
        "10000",
        "10000",
        "01111",
    ),
    "E": (
        "11111",
        "10000",
        "10000",
        "11110",
        "10000",
        "10000",
        "11111",
    ),
    "F": (
        "11111",
        "10000",
        "10000",
        "11110",
        "10000",
        "10000",
        "10000",
    ),
    "G": (
        "01111",
        "10000",
        "10000",
        "10111",
        "10001",
        "10001",
        "01111",
    ),
    "H": (
        "10001",
        "10001",
        "10001",
        "11111",
        "10001",
        "10001",
        "10001",
    ),
    "I": (
        "11111",
        "00100",
        "00100",
        "00100",
        "00100",
        "00100",
        "11111",
    ),
    "K": (
        "10001",
        "10010",
        "10100",
        "11000",
        "10100",
        "10010",
        "10001",
    ),
    "M": (
        "10001",
        "11011",
        "10101",
        "10101",
        "10001",
        "10001",
        "10001",
    ),
    "N": (
        "10001",
        "11001",
        "11001",
        "10101",
        "10011",
        "10011",
        "10001",
    ),
    "O": (
        "01110",
        "10001",
        "10001",
        "10001",
        "10001",
        "10001",
        "01110",
    ),
    "P": (
        "11110",
        "10001",
        "10001",
        "11110",
        "10000",
        "10000",
        "10000",
    ),
    "R": (
        "11110",
        "10001",
        "10001",
        "11110",
        "10100",
        "10010",
        "10001",
    ),
    "S": (
        "01111",
        "10000",
        "10000",
        "01110",
        "00001",
        "00001",
        "11110",
    ),
    "T": (
        "11111",
        "00100",
        "00100",
        "00100",
        "00100",
        "00100",
        "00100",
    ),
    "U": (
        "10001",
        "10001",
        "10001",
        "10001",
        "10001",
        "10001",
        "01110",
    ),
    "V": (
        "10001",
        "10001",
        "10001",
        "10001",
        "10001",
        "01010",
        "00100",
    ),
    "W": (
        "10001",
        "10001",
        "10001",
        "10101",
        "10101",
        "11011",
        "10001",
    ),
    "Y": (
        "10001",
        "10001",
        "01010",
        "00100",
        "00100",
        "00100",
        "00100",
    ),
}


def rgb(name: str) -> str:
    return COLORS[name]


def text_size(text: str, scale: int = 1, spacing: int | None = None) -> tuple[int, int]:
    spacing = scale if spacing is None else spacing
    if not text:
        return (0, 7 * scale)
    return (len(text) * (5 * scale + spacing) - spacing, 7 * scale)


def draw_pixel_text(
    draw: ImageDraw.ImageDraw,
    xy: tuple[int, int],
    text: str,
    *,
    fill: str,
    scale: int = 1,
    anchor: str = "lt",
    shadow: bool = True,
) -> None:
    text = text.upper()
    for char in text:
        if char not in PIXEL_FONT:
            raise ValueError(f"Unsupported hero glyph: {char!r}")

    width, height = text_size(text, scale)
    x, y = xy
    horizontal = anchor[0] if anchor else "l"
    vertical = anchor[1] if len(anchor) > 1 else "t"
    if horizontal == "m":
        x -= width // 2
    elif horizontal == "r":
        x -= width
    if vertical == "m":
        y -= height // 2
    elif vertical == "b":
        y -= height

    def paint(offset_x: int, offset_y: int, color: str) -> None:
        cursor = x + offset_x
        for char in text:
            glyph = PIXEL_FONT[char]
            for row, bits in enumerate(glyph):
                for column, bit in enumerate(bits):
                    if bit == "1":
                        left = cursor + column * scale
                        top = y + offset_y + row * scale
                        draw.rectangle(
                            (left, top, left + scale - 1, top + scale - 1),
                            fill=color,
                        )
            cursor += 6 * scale

    if shadow:
        paint(1, 1, rgb("shadow"))
    paint(0, 0, fill)


def draw_label(
    draw: ImageDraw.ImageDraw,
    center_x: int,
    lines: tuple[str, ...],
    accent: str,
) -> None:
    line_height = 16
    start_y = 4 if len(lines) == 2 else 12
    for index, line in enumerate(lines):
        draw_pixel_text(
            draw,
            (center_x, start_y + index * line_height),
            line,
            fill=rgb("cream") if index == 0 else accent,
            scale=2,
            anchor="mt",
        )
    underline_width = max(text_size(line, 2)[0] for line in lines)
    left = center_x - underline_width // 2
    draw.rectangle((left, 35, left + underline_width - 1, 36), fill=accent)


def draw_background(draw: ImageDraw.ImageDraw, frame: int) -> None:
    draw.rectangle((0, 0, BASE_WIDTH - 1, BASE_HEIGHT - 1), fill=rgb("bg"))
    for y in range(2 + frame % 4, BASE_HEIGHT, 4):
        draw.line((1, y, BASE_WIDTH - 2, y), fill=rgb("scanline"))

    # Fixed stars and circuit flecks keep the scene textured without noise.
    flecks = (
        (6, 48, "violet"),
        (24, 151, "panel"),
        (82, 46, "cyan"),
        (159, 151, "magenta"),
        (244, 45, "violet"),
        (333, 149, "gold"),
        (382, 47, "panel"),
        (473, 147, "cyan"),
    )
    for x, y, color in flecks:
        if (frame + x) % 8 < 5:
            draw.rectangle((x, y, x + 1, y + 1), fill=rgb(color))

    draw.rectangle((1, 1, BASE_WIDTH - 2, BASE_HEIGHT - 2), outline=rgb("violet"))
    draw.line((1, 39, BASE_WIDTH - 2, 39), fill=rgb("panel"))
    draw.line((1, 40, BASE_WIDTH - 2, 40), fill=rgb("ink"))


def draw_flow_rails(draw: ImageDraw.ImageDraw, frame: int) -> None:
    # Outbound mission rail.
    draw.rectangle((76, 130, 469, 136), fill=rgb("shadow"))
    draw.line((77, 132, 468, 132), fill=rgb("violet"))
    draw.line((77, 133, 468, 133), fill=rgb("panel"))
    for x in range(91, 461, 31):
        draw.polygon(((x, 130), (x + 4, 132), (x, 134)), fill=rgb("gold"))

    # Return rail carries state and evidence back to the operators.
    draw.line((77, 45, 468, 45), fill=rgb("panel"))
    draw.line((77, 46, 468, 46), fill=rgb("violet"))
    draw.line((77, 45, 77, 132), fill=rgb("panel"))
    draw.line((468, 45, 468, 132), fill=rgb("panel"))
    for x in range(96, 457, 37):
        draw.polygon(((x + 4, 43), (x, 45), (x + 4, 47)), fill=rgb("mint"))

    # A moving low-intensity rail pulse makes the world feel operational even
    # when the main mission packet is elsewhere.
    pulse_x = 80 + (frame * 17) % 380
    draw.rectangle((pulse_x, 135, pulse_x + 7, 136), fill=rgb("cyan"))


def draw_human(
    draw: ImageDraw.ImageDraw,
    x: int,
    y: int,
    *,
    facing_right: bool,
    jacket: str,
    hair: str,
    frame: int,
) -> None:
    typing = (frame // 2) % 2
    # Shadow, legs, jacket, head, and hair are intentionally chunky.
    draw.rectangle((x + 1, y + 24, x + 11, y + 26), fill=rgb("shadow"))
    draw.rectangle((x + 3, y + 17, x + 6, y + 24), fill=rgb("slate"))
    draw.rectangle((x + 8, y + 17, x + 11, y + 24), fill=rgb("slate"))
    draw.rectangle((x + 2, y + 9, x + 12, y + 18), fill=rgb("ink"))
    draw.rectangle((x + 3, y + 10, x + 11, y + 17), fill=jacket)
    draw.rectangle((x + 5, y + 2, x + 11, y + 9), fill=rgb("cream"))
    draw.rectangle((x + 4, y + 1, x + 12, y + 4), fill=hair)
    if facing_right:
        draw.rectangle((x + 10, y + 5, x + 11, y + 6), fill=rgb("ink"))
        draw.rectangle((x + 11, y + 12 + typing, x + 16, y + 14 + typing), fill=rgb("cream"))
    else:
        draw.rectangle((x + 5, y + 5, x + 6, y + 6), fill=rgb("ink"))
        draw.rectangle((x - 3, y + 12 + typing, x + 3, y + 14 + typing), fill=rgb("cream"))
    draw.rectangle((x + 6, y + 12, x + 7, y + 14), fill=rgb("cream"))


def draw_human_station(draw: ImageDraw.ImageDraw, frame: int) -> None:
    draw_human(
        draw,
        10,
        71,
        facing_right=True,
        jacket=rgb("cyan"),
        hair=rgb("magenta"),
        frame=frame,
    )
    draw_human(
        draw,
        62,
        71,
        facing_right=False,
        jacket=rgb("magenta"),
        hair=rgb("cyan"),
        frame=frame + 2,
    )

    # Shared mission console.
    draw.rectangle((29, 73, 58, 100), fill=rgb("shadow"))
    draw.rectangle((31, 72, 56, 97), fill=rgb("panel"))
    draw.rectangle((34, 75, 53, 87), fill=rgb("ink"))
    draw.rectangle((36, 77, 51, 78), fill=rgb("cyan"))
    draw.rectangle((36, 81, 44 + frame % 6, 82), fill=rgb("gold"))
    draw.rectangle((37, 91, 50, 94), fill=rgb("violet"))
    draw.rectangle((42, 97, 46, 112), fill=rgb("slate"))
    draw.rectangle((34, 111, 55, 115), fill=rgb("ink"))

    # The two colored inputs visibly converge before entering ECorp.
    input_phase = frame % 8
    left_x = 22 + min(input_phase, 6) * 3
    right_x = 62 - min(input_phase, 6) * 3
    draw.line((22, 67, 43, 67), fill=rgb("panel"))
    draw.line((43, 67, 62, 67), fill=rgb("panel"))
    draw.rectangle((left_x, 65, left_x + 2, 68), fill=rgb("cyan"))
    draw.rectangle((right_x - 2, 65, right_x, 68), fill=rgb("magenta"))
    draw.rectangle((42, 64, 46, 69), fill=rgb("gold"))
    draw.rectangle((43, 65, 45, 68), fill=rgb("cream"))

    draw.line((46, 112, 46, 132), fill=rgb("cyan"))
    draw.rectangle((43, 116, 49, 119), fill=rgb("gold"))


def draw_server(draw: ImageDraw.ImageDraw, frame: int, active: bool) -> None:
    x1, y1, x2, y2 = 91, 55, 158, 128
    draw.rectangle((x1 + 3, y1 + 3, x2 + 3, y2 + 3), fill=rgb("shadow"))
    draw.rectangle((x1, y1, x2, y2), fill=rgb("ink"))
    draw.rectangle((x1 + 3, y1 + 3, x2 - 3, y2 - 3), fill=rgb("panel"))
    draw.rectangle((x1 + 6, y1 + 6, x2 - 6, y1 + 19), fill=rgb("violet"))

    # Original E/ slash mark.
    draw.rectangle((99, 64, 111, 66), fill=rgb("cream"))
    draw.rectangle((99, 68, 107, 70), fill=rgb("cream"))
    draw.rectangle((99, 72, 111, 74), fill=rgb("cream"))
    draw.polygon(((117, 62), (121, 62), (113, 76), (109, 76)), fill=rgb("magenta"))

    # Durable journal bays.
    for row in range(3):
        top = 80 + row * 9
        draw.rectangle((98, top, 121, top + 5), fill=rgb("ink"))
        color = ("cyan", "gold", "mint")[(row + frame // 3) % 3]
        width = 8 + ((frame + row * 5) % 12)
        draw.rectangle((100, top + 2, 100 + width, top + 3), fill=rgb(color))

    # Authoritative core.
    pulse = 1 if active or frame % 8 < 3 else 0
    draw.rectangle((128 - pulse, 81 - pulse, 151 + pulse, 105 + pulse), fill=rgb("shadow"))
    draw.rectangle((131, 84, 148, 102), fill=rgb("gold"))
    draw.rectangle((134, 87, 145, 99), fill=rgb("ink"))
    draw.rectangle((137, 90, 142, 96), fill=rgb("cyan"))
    draw.rectangle((139, 88, 143, 90), fill=rgb("cream"))

    # Database drum and outbound port.
    draw.rectangle((111, 110, 140, 122), fill=rgb("shadow"))
    draw.rectangle((114, 108, 137, 111), fill=rgb("slate"))
    draw.rectangle((114, 112, 137, 119), fill=rgb("violet"))
    draw.rectangle((116, 115, 135, 117), fill=rgb("mint"))
    draw.line((125, 122, 125, 132), fill=rgb("cyan"))
    draw.rectangle((122, 125, 128, 129), fill=rgb("gold"))

    # Outbound runner connection.
    draw.line((158, 94, 171, 94), fill=rgb("cyan"))
    draw.rectangle((162, 91, 167, 96), fill=rgb("gold") if active else rgb("violet"))


def draw_runner(draw: ImageDraw.ImageDraw, frame: int, active: bool) -> None:
    # A compact rail machine. The flywheel and wheels rotate continuously,
    # reinforcing that runner lifetime is independent from a browser window.
    draw.rectangle((172, 85, 236, 122), fill=rgb("shadow"))
    draw.rectangle((175, 83, 231, 118), fill=rgb("ink"))
    draw.rectangle((178, 86, 218, 115), fill=rgb("violet"))
    draw.rectangle((181, 89, 202, 104), fill=rgb("panel"))
    draw.rectangle((183, 91, 200, 93), fill=rgb("cyan"))
    draw.rectangle((183, 97, 195, 99), fill=rgb("gold"))
    draw.rectangle((181, 107, 202, 112), fill=rgb("slate"))

    draw.rectangle((219, 92, 233, 114), fill=rgb("gold"))
    draw.rectangle((222, 95, 230, 111), fill=rgb("amber"))
    draw.rectangle((224, 97, 228, 109), fill=rgb("ink"))
    draw.rectangle((214, 82, 227, 86), fill=rgb("cyan") if active else rgb("mint"))
    draw.rectangle((218, 78, 222, 82), fill=rgb("cream"))

    cx, cy = 210, 99
    draw.rectangle((cx - 7, cy - 7, cx + 7, cy + 7), fill=rgb("shadow"))
    draw.rectangle((cx - 5, cy - 5, cx + 5, cy + 5), fill=rgb("panel"))
    spin = frame % 4
    if spin % 2 == 0:
        draw.rectangle((cx - 1, cy - 5, cx + 1, cy + 5), fill=rgb("cyan"))
        draw.rectangle((cx - 5, cy - 1, cx + 5, cy + 1), fill=rgb("cyan"))
    else:
        draw.line((cx - 4, cy - 4, cx + 4, cy + 4), fill=rgb("cyan"), width=2)
        draw.line((cx - 4, cy + 4, cx + 4, cy - 4), fill=rgb("cyan"), width=2)
    draw.rectangle((cx - 1, cy - 1, cx + 1, cy + 1), fill=rgb("cream"))

    for wheel_x in (184, 219):
        draw.rectangle((wheel_x - 7, 116, wheel_x + 7, 126), fill=rgb("shadow"))
        draw.rectangle((wheel_x - 5, 117, wheel_x + 5, 124), fill=rgb("slate"))
        if frame % 2 == 0:
            draw.line((wheel_x, 117, wheel_x, 124), fill=rgb("cream"))
        else:
            draw.line((wheel_x - 4, 121, wheel_x + 4, 121), fill=rgb("cream"))

    draw.line((203, 126, 203, 132), fill=rgb("cyan"))


def draw_worktree(draw: ImageDraw.ImageDraw, frame: int, active: bool) -> None:
    left, top, right, bottom = 244, 53, 330, 127
    # Dashed isolation boundary.
    for x in range(left, right + 1, 8):
        draw.rectangle((x, top, min(x + 4, right), top + 1), fill=rgb("magenta"))
        draw.rectangle((x, bottom - 1, min(x + 4, right), bottom), fill=rgb("magenta"))
    for y in range(top, bottom + 1, 8):
        draw.rectangle((left, y, left + 1, min(y + 4, bottom)), fill=rgb("magenta"))
        draw.rectangle((right - 1, y, right, min(y + 4, bottom)), fill=rgb("magenta"))

    draw.rectangle((249, 58, 325, 123), fill=rgb("shadow"))
    draw.rectangle((252, 61, 322, 120), fill=rgb("panel"))
    draw.rectangle((255, 64, 319, 115), fill=rgb("ink"))
    draw.rectangle((258, 67, 316, 70), fill=rgb("cyan"))

    # A branch-shaped worktree grows only inside its fenced pod.
    branch_color = rgb("gold") if active else rgb("mint")
    draw.rectangle((286, 82, 290, 112), fill=rgb("cream"))
    draw.line((288, 93, 274, 84), fill=branch_color, width=3)
    draw.line((288, 101, 304, 90), fill=branch_color, width=3)
    draw.line((288, 108, 272, 108), fill=branch_color, width=3)
    for node_x, node_y in ((273, 82), (304, 88), (271, 106), (288, 79)):
        size = 3 if active and (frame + node_x) % 4 < 2 else 2
        draw.rectangle(
            (node_x - size, node_y - size, node_x + size, node_y + size),
            fill=rgb("cyan") if node_x % 2 else rgb("magenta"),
        )

    # Workspace desk and isolated checkout base.
    draw.rectangle((257, 92, 276, 106), fill=rgb("violet"))
    draw.rectangle((260, 95, 273, 100), fill=rgb("blue"))
    draw.rectangle((260, 102, 271, 104), fill=rgb("gold"))
    draw.rectangle((265, 106, 268, 113), fill=rgb("slate"))
    draw.line((257, 115, 313, 115), fill=rgb("slate"))
    draw.line((288, 115, 288, 132), fill=rgb("cyan"))

    # Lock glyph.
    draw.rectangle((309, 76, 316, 84), fill=rgb("gold"))
    draw.rectangle((310, 72, 315, 78), outline=rgb("gold"))
    draw.rectangle((312, 79, 313, 82), fill=rgb("ink"))


def gate_lift(frame: int) -> int:
    if 15 <= frame <= 17:
        return (frame - 14) * 7
    if 18 <= frame <= 20:
        return 21
    if 21 <= frame <= 23:
        return max(0, (24 - frame) * 7)
    return 0


def draw_approval_gate(draw: ImageDraw.ImageDraw, frame: int, active: bool) -> None:
    left, right, top, bottom = 338, 377, 57, 128
    draw.rectangle((left, top + 12, left + 4, bottom), fill=rgb("amber"))
    draw.rectangle((right - 4, top + 12, right, bottom), fill=rgb("amber"))
    draw.rectangle((left, top + 8, right, top + 14), fill=rgb("gold"))
    draw.rectangle((left + 5, top + 15, right - 5, bottom - 4), fill=rgb("ink"))
    draw.rectangle((left + 7, top + 18, right - 7, bottom - 7), fill=rgb("panel"))

    lift = gate_lift(frame)
    barrier_y = 101 - lift
    draw.rectangle((left + 4, barrier_y, right - 4, barrier_y + 5), fill=rgb("shadow"))
    draw.rectangle((left + 6, barrier_y, right - 6, barrier_y + 2), fill=rgb("gold"))
    for x in range(left + 8, right - 7, 8):
        draw.rectangle((x, barrier_y + 3, x + 3, barrier_y + 5), fill=rgb("red"))

    if lift >= 14 or active:
        # Human-authorized check.
        draw.line((350, 86, 356, 92), fill=rgb("green"), width=3)
        draw.line((356, 92, 368, 78), fill=rgb("green"), width=3)
        draw.rectangle((352, 64, 363, 68), fill=rgb("cream"))
        draw.rectangle((356, 59, 360, 65), fill=rgb("cream"))
    else:
        draw.rectangle((355, 76, 360, 86), fill=rgb("gold"))
        draw.rectangle((355, 90, 360, 94), fill=rgb("gold"))

    draw.line((357, 124, 357, 132), fill=rgb("gold"))


def draw_evidence_scanner(draw: ImageDraw.ImageDraw, frame: int, active: bool) -> None:
    left, right, top, bottom = 384, 432, 57, 128
    draw.rectangle((left, top + 9, left + 4, bottom), fill=rgb("cyan"))
    draw.rectangle((right - 4, top + 9, right, bottom), fill=rgb("cyan"))
    draw.rectangle((left, top + 6, right, top + 12), fill=rgb("mint"))
    draw.rectangle((left + 5, top + 13, right - 5, bottom - 4), fill=rgb("ink"))

    # Evidence envelope with visible file, command, and check rows.
    draw.rectangle((394, 72, 422, 115), fill=rgb("cream"))
    draw.polygon(((414, 72), (422, 80), (414, 80)), fill=rgb("slate"))
    draw.rectangle((399, 83, 416, 86), fill=rgb("violet"))
    draw.rectangle((399, 91, 417, 94), fill=rgb("blue"))
    draw.rectangle((399, 99, 410, 102), fill=rgb("amber"))
    draw.rectangle((399, 107, 404, 111), fill=rgb("green"))
    draw.line((405, 109, 408, 112), fill=rgb("green"), width=2)
    draw.line((408, 112, 414, 105), fill=rgb("green"), width=2)

    if 19 <= frame <= 26 or active:
        scan_progress = min(1.0, max(0.0, (frame - 19) / 7.0))
        scan_y = 75 + int(scan_progress * 36)
    else:
        scan_y = 75 + (frame * 4) % 36
    draw.rectangle((390, scan_y, 426, scan_y + 2), fill=rgb("cyan"))
    draw.rectangle((387, scan_y - 1, 390, scan_y + 3), fill=rgb("cream"))
    draw.line((408, 118, 408, 132), fill=rgb("cyan"))


def draw_pr_portal(draw: ImageDraw.ImageDraw, frame: int, active: bool) -> None:
    left, right, top, bottom = 439, 477, 59, 128
    glow = 2 if active or frame >= 27 else 0
    draw.rectangle(
        (left - glow, top - glow, right + glow, bottom + glow),
        fill=rgb("magenta"),
    )
    draw.rectangle((left + 4, top + 5, right - 4, bottom - 3), fill=rgb("ink"))
    draw.rectangle((left + 8, top + 10, right - 8, bottom - 7), fill=rgb("panel"))

    # Pull-request branch glyph.
    draw.line((449, 82, 449, 104), fill=rgb("cream"), width=3)
    draw.line((449, 91, 463, 91), fill=rgb("cream"), width=3)
    draw.line((463, 91, 463, 103), fill=rgb("cream"), width=3)
    for cx, cy, color in (
        (449, 78, "cyan"),
        (449, 108, "gold"),
        (463, 87, "magenta"),
        (463, 107, "green"),
    ):
        draw.rectangle((cx - 3, cy - 3, cx + 3, cy + 3), fill=rgb(color))
        draw.rectangle((cx - 1, cy - 1, cx + 1, cy + 1), fill=rgb("cream"))
    draw.polygon(((468, 99), (475, 104), (468, 109)), fill=rgb("gold"))
    draw.line((461, 104, 471, 104), fill=rgb("gold"), width=2)

    if active or frame >= 27:
        for dx, dy, color in (
            (-4, -8, "cyan"),
            (17, -5, "gold"),
            (33, 6, "mint"),
            (27, 39, "magenta"),
        ):
            draw.rectangle((left + dx, top + dy, left + dx + 2, top + dy + 2), fill=rgb(color))

    draw.line((458, 124, 458, 132), fill=rgb("magenta"))


def packet_position(frame: int) -> tuple[int, int, str]:
    if frame <= 23:
        x = 77 + round((468 - 77) * frame / 23)
        return (x, 132, "gold")
    if frame <= 25:
        y = 132 - round((132 - 45) * (frame - 23) / 2)
        return (468, y, "mint")
    if frame <= 30:
        x = 468 - round((468 - 77) * (frame - 25) / 5)
        return (x, 45, "mint")
    return (77, 88, "cyan")


def draw_packet(draw: ImageDraw.ImageDraw, frame: int) -> tuple[int, int]:
    x, y, color = packet_position(frame)
    if frame <= 23:
        draw.rectangle((x - 8, y - 1, x - 4, y + 1), fill=rgb("cyan"))
    elif 26 <= frame <= 30:
        draw.rectangle((x + 4, y - 1, x + 8, y + 1), fill=rgb("violet"))
    draw.rectangle((x - 3, y - 3, x + 3, y + 3), fill=rgb(color))
    draw.rectangle((x - 1, y - 1, x + 1, y + 1), fill=rgb("cream"))
    return (x, y)


def render_base_frame(frame: int) -> Image.Image:
    image = Image.new("RGB", (BASE_WIDTH, BASE_HEIGHT), rgb("bg"))
    draw = ImageDraw.Draw(image)

    draw_background(draw, frame)
    draw_label(draw, 43, ("HUMANS",), rgb("cyan"))
    draw_label(draw, 124, ("ECORP", "SERVER"), rgb("magenta"))
    draw_label(draw, 204, ("RUNNER",), rgb("gold"))
    draw_label(draw, 287, ("WORK", "TREE"), rgb("mint"))
    draw_label(draw, 355, ("GATE",), rgb("amber"))
    draw_label(draw, 414, ("PROOF",), rgb("cyan"))
    draw_label(draw, 460, ("PR",), rgb("magenta"))

    draw_flow_rails(draw, frame)
    packet_x, packet_y, _ = packet_position(frame)
    outbound = packet_y >= 120

    draw_human_station(draw, frame)
    draw_server(draw, frame, outbound and 96 <= packet_x <= 160)
    draw_runner(draw, frame, outbound and 168 <= packet_x <= 239)
    draw_worktree(draw, frame, outbound and 241 <= packet_x <= 332)
    draw_approval_gate(draw, frame, outbound and 335 <= packet_x <= 380)
    draw_evidence_scanner(draw, frame, outbound and 382 <= packet_x <= 434)
    draw_pr_portal(draw, frame, outbound and packet_x >= 435)
    draw_packet(draw, frame)

    # Foreground cabinet edge.
    draw.rectangle((0, 151, BASE_WIDTH - 1, 159), fill=rgb("shadow"))
    draw.line((1, 151, BASE_WIDTH - 2, 151), fill=rgb("violet"))
    for x in range(8, BASE_WIDTH - 8, 24):
        draw.rectangle((x, 154, x + 9, 155), fill=rgb("panel"))
    return image


def to_indexed(image: Image.Image) -> Image.Image:
    """Map a palette-only RGB image to one stable indexed palette."""
    indexed = Image.new("P", image.size)
    flat_palette: list[int] = []
    for red, green, blue in PALETTE_RGB:
        flat_palette.extend((red, green, blue))
    indexed.putpalette(flat_palette)

    source_bytes = image.tobytes()
    palette_indexes = bytearray(len(source_bytes) // 3)
    try:
        for offset in range(0, len(source_bytes), 3):
            palette_indexes[offset // 3] = RGB_TO_INDEX[
                (
                    source_bytes[offset],
                    source_bytes[offset + 1],
                    source_bytes[offset + 2],
                )
            ]
    except KeyError as error:
        raise RuntimeError(f"Unexpected non-palette color: {error.args[0]}") from error
    indexed.frombytes(bytes(palette_indexes))
    return indexed


def scale_indexed(image: Image.Image) -> Image.Image:
    return image.resize(FINAL_SIZE, Image.Resampling.NEAREST)


def build_svg() -> str:
    """Return a self-contained, accessible mission-loop companion diagram."""
    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="1000" height="800" viewBox="0 0 1000 800" shape-rendering="crispEdges" role="img" aria-labelledby="title desc">
  <title id="title">ECorp governed mission loop</title>
  <desc id="desc">Two human operators direct an ECorp control server. A durable runner executes in an isolated git worktree. Human approval and evidence verification gate publication of a pull request, while events and evidence return to the operators.</desc>
  <style>
    .heading {{ font: 700 32px ui-monospace, SFMono-Regular, Consolas, "Liberation Mono", monospace; letter-spacing: 1px; }}
    .label {{ font: 700 26px ui-monospace, SFMono-Regular, Consolas, "Liberation Mono", monospace; letter-spacing: .5px; }}
    .small {{ font: 700 17px ui-monospace, SFMono-Regular, Consolas, "Liberation Mono", monospace; letter-spacing: .5px; }}
    .micro {{ font: 700 15px ui-monospace, SFMono-Regular, Consolas, "Liberation Mono", monospace; letter-spacing: .5px; }}
  </style>

  <rect width="1000" height="800" fill="{rgb('bg')}"/>
  <rect x="8" y="8" width="984" height="784" fill="none" stroke="{rgb('violet')}" stroke-width="8"/>
  <rect x="22" y="22" width="956" height="756" fill="none" stroke="{rgb('panel')}" stroke-width="2"/>
  <text x="50" y="62" class="heading" fill="{rgb('cream')}">GOVERNED MISSION LOOP</text>
  <rect x="50" y="76" width="390" height="6" fill="{rgb('gold')}"/>
  <text x="950" y="61" text-anchor="end" class="small" fill="{rgb('mint')}">HUMAN AUTHORITY OUTBOUND</text>

  <!-- Outbound path -->
  <polyline points="290,180 350,180" fill="none" stroke="{rgb('gold')}" stroke-width="8"/>
  <polygon points="350,168 374,180 350,192" fill="{rgb('gold')}"/>
  <polyline points="625,180 685,180" fill="none" stroke="{rgb('gold')}" stroke-width="8"/>
  <polygon points="685,168 709,180 685,192" fill="{rgb('gold')}"/>
  <polyline points="835,265 835,319" fill="none" stroke="{rgb('gold')}" stroke-width="8"/>
  <polygon points="823,319 835,343 847,319" fill="{rgb('gold')}"/>
  <polyline points="710,410 650,410" fill="none" stroke="{rgb('gold')}" stroke-width="8"/>
  <polygon points="650,398 626,410 650,422" fill="{rgb('gold')}"/>
  <polyline points="375,410 315,410" fill="none" stroke="{rgb('gold')}" stroke-width="8"/>
  <polygon points="315,398 291,410 315,422" fill="{rgb('gold')}"/>
  <polyline points="165,495 165,549" fill="none" stroke="{rgb('gold')}" stroke-width="8"/>
  <polygon points="153,549 165,573 177,549" fill="{rgb('gold')}"/>

  <!-- Return path -->
  <polyline points="290,640 950,640 950,755 20,755 20,180 40,180" fill="none" stroke="{rgb('mint')}" stroke-width="6"/>
  <polygon points="34,168 58,180 34,192" fill="{rgb('mint')}"/>
  <text x="340" y="742" class="micro" fill="{rgb('mint')}">DURABLE STATE + AUDIT EVENTS + ACCEPTED EVIDENCE RETURN</text>

  <!-- Two human operators -->
  <g aria-label="Two human operators">
    <rect x="40" y="95" width="250" height="170" fill="{rgb('ink')}" stroke="{rgb('cyan')}" stroke-width="6"/>
    <text x="165" y="130" text-anchor="middle" class="label" fill="{rgb('cream')}">TWO OPERATORS</text>
    <rect x="67" y="170" width="28" height="28" fill="{rgb('cream')}"/>
    <rect x="64" y="164" width="34" height="10" fill="{rgb('magenta')}"/>
    <rect x="70" y="198" width="22" height="38" fill="{rgb('cyan')}"/>
    <rect x="236" y="170" width="28" height="28" fill="{rgb('cream')}"/>
    <rect x="233" y="164" width="34" height="10" fill="{rgb('cyan')}"/>
    <rect x="239" y="198" width="22" height="38" fill="{rgb('magenta')}"/>
    <rect x="116" y="166" width="98" height="62" fill="{rgb('panel')}" stroke="{rgb('gold')}" stroke-width="4"/>
    <rect x="133" y="181" width="64" height="9" fill="{rgb('cyan')}"/>
    <rect x="145" y="201" width="40" height="9" fill="{rgb('gold')}"/>
    <text x="165" y="252" text-anchor="middle" class="small" fill="{rgb('mint')}">STEER + AUTHORIZE</text>
  </g>

  <!-- ECorp control server -->
  <g aria-label="ECorp control server">
    <rect x="375" y="95" width="250" height="170" fill="{rgb('ink')}" stroke="{rgb('magenta')}" stroke-width="6"/>
    <text x="500" y="130" text-anchor="middle" class="label" fill="{rgb('cream')}">ECORP CONTROL</text>
    <rect x="414" y="158" width="172" height="76" fill="{rgb('panel')}"/>
    <rect x="432" y="174" width="54" height="9" fill="{rgb('cyan')}"/>
    <rect x="432" y="193" width="83" height="9" fill="{rgb('gold')}"/>
    <rect x="432" y="212" width="68" height="9" fill="{rgb('mint')}"/>
    <rect x="536" y="172" width="30" height="48" fill="{rgb('gold')}"/>
    <rect x="544" y="182" width="14" height="28" fill="{rgb('ink')}"/>
    <text x="500" y="252" text-anchor="middle" class="small" fill="{rgb('magenta')}">AUTHORITATIVE STATE</text>
  </g>

  <!-- Durable runner -->
  <g aria-label="Durable runner">
    <rect x="710" y="95" width="250" height="170" fill="{rgb('ink')}" stroke="{rgb('gold')}" stroke-width="6"/>
    <text x="835" y="122" text-anchor="middle" class="label" fill="{rgb('cream')}">DURABLE</text>
    <text x="835" y="151" text-anchor="middle" class="label" fill="{rgb('cream')}">RUNNER</text>
    <rect x="748" y="176" width="150" height="48" fill="{rgb('violet')}"/>
    <rect x="890" y="186" width="34" height="38" fill="{rgb('amber')}"/>
    <rect x="772" y="188" width="58" height="9" fill="{rgb('cyan')}"/>
    <rect x="772" y="207" width="42" height="9" fill="{rgb('gold')}"/>
    <rect x="765" y="225" width="34" height="16" fill="{rgb('slate')}"/>
    <rect x="869" y="225" width="34" height="16" fill="{rgb('slate')}"/>
    <text x="835" y="252" text-anchor="middle" class="small" fill="{rgb('gold')}">SURVIVES THE UI</text>
  </g>

  <!-- Isolated worktree -->
  <g aria-label="Isolated git worktree">
    <rect x="710" y="325" width="250" height="170" fill="{rgb('ink')}" stroke="{rgb('mint')}" stroke-width="6" stroke-dasharray="14 8"/>
    <text x="835" y="358" text-anchor="middle" class="label" fill="{rgb('cream')}">ISOLATED</text>
    <text x="835" y="387" text-anchor="middle" class="label" fill="{rgb('cream')}">WORKTREE</text>
    <line x1="835" y1="403" x2="835" y2="463" stroke="{rgb('cream')}" stroke-width="8"/>
    <line x1="835" y1="420" x2="792" y2="401" stroke="{rgb('cyan')}" stroke-width="7"/>
    <line x1="835" y1="436" x2="881" y2="407" stroke="{rgb('magenta')}" stroke-width="7"/>
    <rect x="784" y="392" width="20" height="20" fill="{rgb('cyan')}"/>
    <rect x="874" y="398" width="20" height="20" fill="{rgb('magenta')}"/>
    <rect x="825" y="454" width="20" height="20" fill="{rgb('gold')}"/>
    <rect x="918" y="398" width="20" height="30" fill="{rgb('gold')}"/>
    <rect x="922" y="387" width="12" height="18" fill="none" stroke="{rgb('gold')}" stroke-width="5"/>
    <text x="835" y="483" text-anchor="middle" class="small" fill="{rgb('mint')}">TASK BRANCH ONLY</text>
  </g>

  <!-- Approval gate -->
  <g aria-label="Human approval gate">
    <rect x="375" y="325" width="250" height="170" fill="{rgb('ink')}" stroke="{rgb('amber')}" stroke-width="6"/>
    <text x="500" y="360" text-anchor="middle" class="label" fill="{rgb('cream')}">APPROVAL GATE</text>
    <rect x="420" y="394" width="12" height="68" fill="{rgb('gold')}"/>
    <rect x="568" y="394" width="12" height="68" fill="{rgb('gold')}"/>
    <rect x="420" y="394" width="160" height="12" fill="{rgb('gold')}"/>
    <rect x="432" y="429" width="136" height="10" fill="{rgb('red')}"/>
    <polyline points="456,427 481,451 545,396" fill="none" stroke="{rgb('green')}" stroke-width="10"/>
    <text x="500" y="483" text-anchor="middle" class="small" fill="{rgb('amber')}">HUMAN DECISION</text>
  </g>

  <!-- Evidence verifier -->
  <g aria-label="Evidence verification">
    <rect x="40" y="325" width="250" height="170" fill="{rgb('ink')}" stroke="{rgb('cyan')}" stroke-width="6"/>
    <text x="165" y="360" text-anchor="middle" class="label" fill="{rgb('cream')}">EVIDENCE CHECK</text>
    <rect x="112" y="386" width="106" height="78" fill="{rgb('cream')}"/>
    <rect x="130" y="401" width="56" height="8" fill="{rgb('violet')}"/>
    <rect x="130" y="420" width="70" height="8" fill="{rgb('blue')}"/>
    <polyline points="132,444 146,458 178,430" fill="none" stroke="{rgb('green')}" stroke-width="9"/>
    <rect x="96" y="438" width="138" height="7" fill="{rgb('cyan')}"/>
    <text x="165" y="483" text-anchor="middle" class="small" fill="{rgb('cyan')}">VERIFIER PASSES</text>
  </g>

  <!-- Pull request publication -->
  <g aria-label="Pull request publication">
    <rect x="40" y="575" width="250" height="150" fill="{rgb('ink')}" stroke="{rgb('magenta')}" stroke-width="6"/>
    <text x="165" y="610" text-anchor="middle" class="label" fill="{rgb('cream')}">PULL REQUEST</text>
    <line x1="120" y1="637" x2="120" y2="686" stroke="{rgb('cream')}" stroke-width="8"/>
    <line x1="120" y1="655" x2="193" y2="655" stroke="{rgb('cream')}" stroke-width="8"/>
    <line x1="193" y1="655" x2="193" y2="686" stroke="{rgb('cream')}" stroke-width="8"/>
    <rect x="109" y="626" width="22" height="22" fill="{rgb('cyan')}"/>
    <rect x="109" y="680" width="22" height="22" fill="{rgb('gold')}"/>
    <rect x="182" y="644" width="22" height="22" fill="{rgb('magenta')}"/>
    <rect x="182" y="680" width="22" height="22" fill="{rgb('green')}"/>
    <polygon points="218,644 244,657 218,670" fill="{rgb('gold')}"/>
    <text x="165" y="714" text-anchor="middle" class="small" fill="{rgb('magenta')}">PUBLISH FOR REVIEW</text>
  </g>
</svg>
"""


def save_assets() -> None:
    base_frames = [render_base_frame(frame) for frame in range(FRAME_COUNT)]
    indexed_frames = [scale_indexed(to_indexed(frame)) for frame in base_frames]

    indexed_frames[0].save(PNG_PATH, format="PNG", optimize=True, compress_level=9)
    indexed_frames[0].save(
        GIF_PATH,
        format="GIF",
        save_all=True,
        append_images=indexed_frames[1:],
        duration=[FRAME_DURATION_MS] * FRAME_COUNT,
        loop=0,
        disposal=1,
        optimize=True,
        comment=b"Original ECorp governed mission-flow sprite art",
    )
    SVG_PATH.write_text(build_svg(), encoding="utf-8", newline="\n")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def image_report(path: Path) -> dict[str, object]:
    with Image.open(path) as image:
        initial_mode = image.mode
        frame_durations: list[int] = []
        used_colors: set[tuple[int, int, int]] = set()
        for frame in ImageSequence.Iterator(image):
            frame_durations.append(int(frame.info.get("duration", image.info.get("duration", 0))))
            converted = frame.convert("RGB")
            colors = converted.getcolors(maxcolors=converted.width * converted.height)
            if colors is None:
                raise AssertionError(f"Could not enumerate colors in {path.name}")
            used_colors.update(color for _, color in colors)
        report: dict[str, object] = {
            "format": image.format,
            "signature_hex": path.read_bytes()[:8].hex(),
            "dimensions": list(image.size),
            "mode": initial_mode,
            "frames": getattr(image, "n_frames", 1),
            "durations_ms": frame_durations,
            "loop": image.info.get("loop"),
            "transparency": image.info.get("transparency"),
            "used_palette_colors": len(used_colors),
            "size_bytes": path.stat().st_size,
            "sha256": sha256(path),
        }
        file_bytes = path.read_bytes()
        if image.format == "GIF":
            packed = file_bytes[10]
            report["global_palette_entries"] = 2 ** ((packed & 0b111) + 1) if packed & 0x80 else 0
        elif image.format == "PNG":
            report["bit_depth"] = file_bytes[24]
            report["color_type"] = file_bytes[25]
        return report


def validate_assets() -> dict[str, object]:
    gif_report = image_report(GIF_PATH)
    png_report = image_report(PNG_PATH)

    if GIF_PATH.read_bytes()[:6] not in (b"GIF87a", b"GIF89a"):
        raise AssertionError("GIF signature is invalid")
    if PNG_PATH.read_bytes()[:8] != b"\x89PNG\r\n\x1a\n":
        raise AssertionError("PNG signature is invalid")
    if gif_report["dimensions"] != list(FINAL_SIZE):
        raise AssertionError(f"Unexpected GIF dimensions: {gif_report['dimensions']}")
    if png_report["dimensions"] != list(FINAL_SIZE):
        raise AssertionError(f"Unexpected PNG dimensions: {png_report['dimensions']}")
    if gif_report["frames"] != FRAME_COUNT:
        raise AssertionError(f"Unexpected GIF frame count: {gif_report['frames']}")
    if gif_report["durations_ms"] != [FRAME_DURATION_MS] * FRAME_COUNT:
        raise AssertionError(f"Unexpected GIF durations: {gif_report['durations_ms']}")
    if gif_report["transparency"] is not None:
        raise AssertionError("GIF should be opaque")
    if png_report["transparency"] is not None:
        raise AssertionError("PNG should be opaque")
    if gif_report["global_palette_entries"] != len(PALETTE_RGB):
        raise AssertionError(
            f"Unexpected GIF palette size: {gif_report['global_palette_entries']}"
        )
    if png_report["bit_depth"] != 4 or png_report["color_type"] != 3:
        raise AssertionError(
            f"PNG should be 4-bit indexed color, got bit depth {png_report['bit_depth']} "
            f"and color type {png_report['color_type']}"
        )
    if int(gif_report["used_palette_colors"]) > len(PALETTE_RGB):
        raise AssertionError("GIF exceeded the fixed palette")
    if int(png_report["used_palette_colors"]) > len(PALETTE_RGB):
        raise AssertionError("PNG exceeded the fixed palette")
    if GIF_PATH.stat().st_size >= 1_500_000:
        raise AssertionError("Hero GIF exceeds the 1.5 MB target")

    with Image.open(GIF_PATH) as gif, Image.open(PNG_PATH) as png:
        first_frame = gif.seek(0) or gif.convert("RGB")
        if ImageChops.difference(first_frame, png.convert("RGB")).getbbox() is not None:
            raise AssertionError("Static PNG is not pixel-identical to GIF frame 0")

    svg_bytes = SVG_PATH.read_bytes()
    if not svg_bytes.startswith(b"<svg"):
        raise AssertionError("SVG signature is invalid")
    svg_root = ET.fromstring(svg_bytes)
    if svg_root.tag != "{http://www.w3.org/2000/svg}svg":
        raise AssertionError("SVG root is invalid")
    if svg_root.attrib.get("width") != "1000" or svg_root.attrib.get("height") != "800":
        raise AssertionError("SVG dimensions are invalid")
    if b"<title " not in svg_bytes or b"<desc " not in svg_bytes:
        raise AssertionError("SVG requires title and description")
    if b"http://" in svg_bytes.replace(b"http://www.w3.org/2000/svg", b""):
        raise AssertionError("SVG contains an external HTTP reference")
    if b"https://" in svg_bytes:
        raise AssertionError("SVG contains an external HTTPS reference")

    asset_bytes = GIF_PATH.stat().st_size + PNG_PATH.stat().st_size + SVG_PATH.stat().st_size
    if asset_bytes >= 2_000_000:
        raise AssertionError("Generated asset set exceeds the 2 MB target")

    return {
        "hero_gif": gif_report,
        "hero_static_png": png_report,
        "mission_loop_svg": {
            "format": "SVG",
            "signature_hex": svg_bytes[:8].hex(),
            "dimensions": [1000, 800],
            "opaque_background": True,
            "external_references": False,
            "size_bytes": SVG_PATH.stat().st_size,
            "sha256": sha256(SVG_PATH),
        },
        "asset_set_size_bytes": asset_bytes,
        "asset_set_under_2_mb": asset_bytes < 2_000_000,
        "hero_under_1_5_mb": GIF_PATH.stat().st_size < 1_500_000,
        "first_frame_matches_static_png": True,
    }


def main() -> None:
    ROOT.mkdir(parents=True, exist_ok=True)
    save_assets()
    report = validate_assets()
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
