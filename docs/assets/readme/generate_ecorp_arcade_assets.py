from __future__ import annotations

import hashlib
import json
from pathlib import Path

import math

from PIL import Image, ImageChops, ImageDraw, ImageEnhance, ImageFont

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]
SOURCE = REPO / "docs" / "assets" / "ecorp-control-room.png"
TITLE_FONT = REPO / "apps" / "web" / "src" / "assets" / "fonts" / "PressStart2P-Regular.ttf"
DISPLAY_FONT = REPO / "apps" / "web" / "src" / "assets" / "fonts" / "VT323-Regular.ttf"
GIF_PATH = ROOT / "ecorp-arcade-hero.gif"
PNG_PATH = ROOT / "ecorp-arcade-hero-static.png"

WIDTH, HEIGHT = 1440, 920
FRAME_COUNT = 25
FRAME_MS = 90
COLORS = 64

INK = "#070710"
NAVY = "#100b20"
PANEL = "#16102a"
PURPLE = "#35204f"
CYAN = "#38e5dc"
MAGENTA = "#ff3f98"
AMBER = "#ffd44a"
MINT = "#7ee2b8"
WHITE = "#fff8df"
MUTED = "#9b93b3"
RED = "#ff5c6c"


def font(path: Path, size: int) -> ImageFont.FreeTypeFont:
    return ImageFont.truetype(str(path), size)


def text_width(draw: ImageDraw.ImageDraw, value: str, f: ImageFont.FreeTypeFont) -> int:
    box = draw.textbbox((0, 0), value, font=f)
    return box[2] - box[0]


def pixel_line(draw: ImageDraw.ImageDraw, xy: tuple[int, int, int, int], fill: str, width: int = 4) -> None:
    x1, y1, x2, y2 = xy
    draw.rectangle((x1, y1, x2, y1 + width - 1), fill=fill)
    draw.rectangle((x2 - width + 1, y1, x2, y2), fill=fill)


def draw_mark(draw: ImageDraw.ImageDraw, x: int, y: int) -> None:
    draw.rectangle((x, y, x + 88, y + 88), fill=CYAN)
    draw.rectangle((x + 8, y + 8, x + 80, y + 80), fill="#0a1820")
    draw.rectangle((x + 54, y + 8, x + 67, y + 80), fill=MAGENTA)
    draw.polygon([(x + 48, y + 8), (x + 62, y + 8), (x + 34, y + 80), (x + 20, y + 80)], fill=INK)
    draw.text((x + 16, y + 17), "E", font=font(DISPLAY_FONT, 48), fill=WHITE)


def build_base() -> Image.Image:
    canvas = Image.new("RGB", (WIDTH, HEIGHT), INK)
    draw = ImageDraw.Draw(canvas)

    # Deep cabinet backdrop with disciplined pixel texture.
    draw.rectangle((12, 12, WIDTH - 13, HEIGHT - 13), fill=NAVY, outline=PURPLE, width=6)
    draw.rectangle((24, 24, WIDTH - 25, HEIGHT - 25), outline="#5a3276", width=2)
    for y in range(30, HEIGHT - 30, 12):
        draw.line((30, y, WIDTH - 31, y), fill="#130e23", width=1)
    for x in range(42, WIDTH - 42, 96):
        draw.rectangle((x, HEIGHT - 34, x + 52, HEIGHT - 29), fill=PURPLE)

    # Header marquee.
    draw.rectangle((36, 36, WIDTH - 37, 188), fill="#0b0914", outline="#4d2b66", width=4)
    draw.rectangle((36, 36, 44, 188), fill=MAGENTA)
    draw.rectangle((44, 180, WIDTH - 37, 188), fill=AMBER)
    draw_mark(draw, 66, 68)

    title = font(TITLE_FONT, 72)
    subtitle = font(DISPLAY_FONT, 38)
    label = font(DISPLAY_FONT, 26)
    micro = font(DISPLAY_FONT, 22)
    draw.text((182, 55), "ECORP", font=title, fill=WHITE, stroke_width=2, stroke_fill="#4a315a")
    draw.text((187, 133), "GOVERNED AGENT COMMAND", font=subtitle, fill=CYAN)

    # Co-op status panel.
    sx1, sy1, sx2, sy2 = 1000, 60, 1352, 163
    draw.rectangle((sx1, sy1, sx2, sy2), fill=PANEL, outline="#5e3b75", width=3)
    draw.text((1022, 66), "2P CO-OP", font=label, fill=AMBER)
    draw.text((1022, 101), "HUMANS IN COMMAND", font=micro, fill=WHITE)
    draw.text((1022, 130), "RUNNER ONLINE // WORKTREE ISOLATED", font=font(DISPLAY_FONT, 17), fill=MINT)
    for i, color in enumerate((CYAN, MAGENTA, AMBER, MINT)):
        x = 1240 + i * 24
        draw.rectangle((x, 78, x + 12, 90), fill=color)

    # Main screen frame.
    fx1, fy1, fx2, fy2 = 58, 210, 1382, 862
    draw.rectangle((fx1, fy1, fx2, fy2), fill="#020207", outline="#56376f", width=8)
    draw.rectangle((fx1 + 12, fy1 + 12, fx2 - 12, fy2 - 12), outline=CYAN, width=2)
    draw.rectangle((fx1 + 18, fy1 + 18, fx2 - 18, fy2 - 18), outline="#201434", width=3)

    source = Image.open(SOURCE).convert("RGB")
    # Crop into the playable world, not the browser-like lower dock.
    crop = source.crop((0, 66, 1440, 786))
    crop = ImageEnhance.Contrast(crop).enhance(1.05)
    crop = ImageEnhance.Color(crop).enhance(1.08)
    crop = crop.resize((1280, 640), Image.Resampling.LANCZOS)
    canvas.paste(crop, (80, 216))

    # Cabinet glass and pixel corner brackets.
    overlay = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    od = ImageDraw.Draw(overlay)
    od.rectangle((80, 216, 1360, 856), fill=(5, 3, 15, 18))
    for y in range(220, 856, 5):
        od.line((82, y, 1358, y), fill=(0, 0, 0, 20), width=1)
    # Vignette blocks, intentionally stepped rather than smooth-vector glossy.
    for i, alpha in enumerate((70, 46, 28, 14)):
        inset = i * 8
        od.rectangle((80 + inset, 216 + inset, 1359 - inset, 855 - inset), outline=(3, 2, 9, alpha), width=8)
    canvas = Image.alpha_composite(canvas.convert("RGBA"), overlay).convert("RGB")
    draw = ImageDraw.Draw(canvas)

    # Strong corner hardware around the real product capture.
    for x, y, sx, sy, color in (
        (68, 202, 1, 1, CYAN), (1372, 202, -1, 1, MAGENTA),
        (68, 872, 1, -1, MAGENTA), (1372, 872, -1, -1, CYAN),
    ):
        horizontal = (x, y, x + sx * 62, y + sy * 8)
        vertical = (x, y, x + sx * 8, y + sy * 62)
        draw.rectangle(
            (
                min(horizontal[0], horizontal[2]),
                min(horizontal[1], horizontal[3]),
                max(horizontal[0], horizontal[2]),
                max(horizontal[1], horizontal[3]),
            ),
            fill=color,
        )
        draw.rectangle(
            (
                min(vertical[0], vertical[2]),
                min(vertical[1], vertical[3]),
                max(vertical[0], vertical[2]),
                max(vertical[1], vertical[3]),
            ),
            fill=color,
        )

    # Bottom mission rail is animated per frame.
    draw.rectangle((72, 870, WIDTH - 73, 910), fill="#0b0914", outline=PURPLE, width=2)

    return canvas


def build_frame(base: Image.Image, index: int) -> Image.Image:
    frame = base.convert("RGBA")
    phase = (2 * math.pi * index) / FRAME_COUNT

    # A continuous camera push and drift makes the live floor feel inhabited.
    source = Image.open(SOURCE).convert("RGB").crop((0, 66, 1440, 786))
    source = ImageEnhance.Brightness(source).enhance(1.08)
    source = ImageEnhance.Contrast(source).enhance(1.08)
    source = ImageEnhance.Color(source).enhance(1.18)
    zoom = 1.018 + 0.012 * math.sin(phase)
    sw, sh = int(1280 * zoom), int(640 * zoom)
    source = source.resize((sw, sh), Image.Resampling.LANCZOS)
    drift_x = int(8 * math.sin(phase))
    drift_y = int(5 * math.cos(phase))
    left = max(0, (sw - 1280) // 2 + drift_x)
    top = max(0, (sh - 640) // 2 + drift_y)
    left = min(left, sw - 1280)
    top = min(top, sh - 640)
    source = source.crop((left, top, left + 1280, top + 640))
    frame.paste(source, (80, 216))

    overlay = Image.new("RGBA", frame.size, (0, 0, 0, 0))
    draw = ImageDraw.Draw(overlay)

    # CRT glass, scanlines, and a bright operational sweep.
    draw.rectangle((80, 216, 1360, 856), fill=(5, 3, 15, 12))
    for scan_y in range(220, 856, 5):
        draw.line((82, scan_y, 1358, scan_y), fill=(0, 0, 0, 17), width=1)
    y = 224 + int((620 * index) / FRAME_COUNT)
    draw.rectangle((86, y, 1354, y + 2), fill=(56, 229, 220, 154))
    draw.rectangle((86, y + 3, 1354, y + 6), fill=(255, 63, 152, 54))

    # Operational status lights pulse in sequence.
    active = index % 4
    for i, color in enumerate((CYAN, MAGENTA, AMBER, MINT)):
        x = 1240 + i * 24
        if i == active:
            draw.rectangle((x - 5, 73, x + 17, 95), fill=(*ImageColor(color), 58))
            draw.rectangle((x, 78, x + 12, 90), fill=(*ImageColor(color), 255))

    # A five-stage mission rail lights up from intent to published review.
    stages = (
        ("PLAN", CYAN),
        ("RUN", AMBER),
        ("APPROVE", MAGENTA),
        ("VERIFY", MINT),
        ("PUBLISH", CYAN),
    )
    active_stage = min(4, index // 5)
    stage_font = font(DISPLAY_FONT, 23)
    box_w, gap = 190, 12
    total_w = box_w * len(stages) + gap * (len(stages) - 1)
    start_x = (WIDTH - total_w) // 2
    for i, (name, color) in enumerate(stages):
        x1 = start_x + i * (box_w + gap)
        x2 = x1 + box_w
        rgb = ImageColor(color)
        if i < active_stage:
            fill = (*rgb, 88)
            outline = (*rgb, 220)
            text_fill = WHITE
        elif i == active_stage:
            fill = (*rgb, 210)
            outline = (*ImageColor(WHITE), 255)
            text_fill = INK
            glow = 5 + int(3 * (1 + math.sin(phase)))
            draw.rectangle((x1 - glow, 869 - glow, x2 + glow, 911 + glow), fill=(*rgb, 30))
        else:
            fill = (24, 16, 42, 235)
            outline = (82, 52, 105, 255)
            text_fill = MUTED
        draw.rectangle((x1, 874, x2, 906), fill=fill, outline=outline, width=2)
        tw = text_width(draw, name, stage_font)
        draw.text((x1 + (box_w - tw) // 2, 876), name, font=stage_font, fill=text_fill)

    # One bright packet moves continuously across the mission path.
    rail_x = start_x + int((total_w - 12) * index / (FRAME_COUNT - 1))
    packet_color = stages[active_stage][1]
    draw.rectangle((rail_x, 864, rail_x + 12, 872), fill=(*ImageColor(packet_color), 255))

    # Perimeter energy moves around the cabinet rather than flashing the frame.
    travel = int((index / FRAME_COUNT) * 2 * (1280 + 640))
    perimeter = 2 * (1280 + 640)
    travel %= perimeter
    if travel < 1280:
        px, py = 80 + travel, 208
    elif travel < 1280 + 640:
        px, py = 1364, 208 + (travel - 1280)
    elif travel < 2 * 1280 + 640:
        px, py = 1360 - (travel - 1280 - 640), 860
    else:
        px, py = 72, 856 - (travel - 2 * 1280 - 640)
    draw.rectangle((px, py, px + 18, py + 8), fill=(*ImageColor(AMBER), 255))

    # Title underline breathes while the camera moves.
    pulse = 34 + int(26 * (0.5 + 0.5 * math.sin(phase)))
    draw.rectangle((178, 50, 590, 55), fill=(56, 229, 220, pulse))

    return Image.alpha_composite(frame, overlay).convert("RGB")


def ImageColor(value: str) -> tuple[int, int, int]:
    value = value.lstrip("#")
    return tuple(int(value[i:i+2], 16) for i in (0, 2, 4))


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def generate() -> dict[str, object]:
    base = build_base()
    frames = [build_frame(base, i) for i in range(FRAME_COUNT)]

    # One shared palette keeps the animated product capture compact and stable.
    palette = frames[0].quantize(colors=COLORS, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE)
    indexed = [f.quantize(palette=palette, dither=Image.Dither.NONE) for f in frames]
    # Static fallback uses the exact indexed pixels stored in GIF frame zero.
    indexed[0].save(PNG_PATH, format="PNG", optimize=True)
    indexed[0].save(
        GIF_PATH,
        save_all=True,
        append_images=indexed[1:],
        duration=[FRAME_MS] * FRAME_COUNT,
        loop=0,
        optimize=True,
        disposal=1,
    )

    with Image.open(GIF_PATH) as gif, Image.open(PNG_PATH) as png:
        gif.seek(0)
        matches = ImageChops.difference(gif.convert("RGB"), png.convert("RGB")).getbbox() is None
        frame_total = getattr(gif, "n_frames", 1)
        dimensions = gif.size
        duration = sum((gif.seek(i) or gif.info.get("duration", 0)) for i in range(frame_total))

    if dimensions != (WIDTH, HEIGHT):
        raise AssertionError(f"unexpected hero dimensions: {dimensions}")
    if frame_total != FRAME_COUNT:
        raise AssertionError(f"unexpected frame count: {frame_total}")
    if not matches:
        raise AssertionError("static fallback must match GIF frame zero")
    if GIF_PATH.stat().st_size > 2_500_000:
        raise AssertionError("hero GIF exceeds 2.5 MB")

    return {
        "source": str(SOURCE.relative_to(REPO)),
        "gif": {
            "path": str(GIF_PATH.relative_to(REPO)),
            "dimensions": dimensions,
            "frames": frame_total,
            "duration_ms": duration,
            "bytes": GIF_PATH.stat().st_size,
            "sha256": sha(GIF_PATH),
        },
        "static_png": {
            "path": str(PNG_PATH.relative_to(REPO)),
            "dimensions": Image.open(PNG_PATH).size,
            "bytes": PNG_PATH.stat().st_size,
            "sha256": sha(PNG_PATH),
            "matches_first_frame": matches,
        },
    }


def main() -> None:
    ROOT.mkdir(parents=True, exist_ok=True)
    report = generate()
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
