#!/usr/bin/env python3
"""Render the README formatter demo from genuine `forformat` output.

This helper intentionally has one optional dependency, Pillow.  It keeps the
checked-in GIF reproducible without tying documentation builds to an image
toolchain::

    python -m venv /tmp/forformat-readme-venv
    /tmp/forformat-readme-venv/bin/pip install Pillow
    /tmp/forformat-readme-venv/bin/python tools/render_readme_demo.py

Set FORFORMAT_BIN to select a formatter binary explicitly.
"""

from __future__ import annotations

import argparse
import glob
import os
from pathlib import Path
import re
import subprocess
from typing import Optional

from PIL import Image, ImageDraw, ImageFont


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_OUTPUT = ROOT / "docs" / "assets" / "forformat-demo.gif"

BEFORE = """module particle_dynamics
implicit none
type::particle_t
real::position(3)
end type particle_t
contains
subroutine drift(particles,velocities,accelerations,time_step)
type(particle_t),intent(inout)::particles(:)
real,intent(in)::velocities(:,:),accelerations(:,:),time_step
integer::i
do I=1,size(particles)
 particles(i)%POSITION=particles(I)%position+velocities(:,I)*time_step+0.5*accelerations(:,I)*time_step**2
enddo
ENDSUBROUTINE DRIFT
end module particle_dynamics
"""

WIDTH, HEIGHT = 1000, 500
CODE_X, CODE_Y = 72, 56
LINE_HEIGHT = 22

COLORS = {
    "editor": "#1e1e1e",
    "gutter": "#191919",
    "divider": "#2b2b2b",
    "text": "#d4d4d4",
    "muted": "#858585",
    "keyword": "#c586c0",
    "intrinsic": "#4ec9b0",
    "number": "#b5cea8",
    "string": "#ce9178",
    "comment": "#6a9955",
    "success": "#4ec9b0",
    "warning": "#d7ba7d",
}

KEYWORDS = {
    "module",
    "implicit",
    "none",
    "type",
    "real",
    "end",
    "contains",
    "subroutine",
    "intent",
    "inout",
    "in",
    "integer",
    "do",
}
INTRINSICS = {"size"}
TOKEN_RE = re.compile(r"('[^']*'|\"[^\"]*\"|![^\n]*|[A-Za-z_]\w*|\d+(?:\.\d*)?|\s+|.)")


def find_formatter() -> Path:
    candidates = [
        os.environ.get("FORFORMAT_BIN"),
        str(Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")) / "release" / "forformat"),
        str(ROOT / "target" / "release" / "forformat"),
    ]
    for candidate in candidates:
        if candidate and Path(candidate).is_file():
            return Path(candidate)
    subprocess.run(["cargo", "build", "--locked", "--release"], cwd=ROOT, check=True)
    return ROOT / "target" / "release" / "forformat"


def genuine_after() -> str:
    result = subprocess.run(
        [str(find_formatter()), "--stdin"],
        cwd=ROOT,
        input=BEFORE,
        text=True,
        capture_output=True,
        check=True,
    )
    if "particles(i)%position = &\n" not in result.stdout:
        raise RuntimeError("README example no longer demonstrates default line wrapping")
    return result.stdout


def font_path(monospace: bool) -> str:
    names = ["DejaVuSansMono.ttf", "LiberationMono-Regular.ttf"] if monospace else ["DejaVuSans.ttf"]
    patterns = [f"/usr/share/fonts/**/{name}" for name in names]
    patterns += [
        "/vscode/vscode-server/**/KaTeX_Typewriter-Regular*.ttf"
        if monospace
        else "/vscode/vscode-server/**/KaTeX_SansSerif-Regular*.ttf"
    ]
    for pattern in patterns:
        matches = glob.glob(pattern, recursive=True)
        if matches:
            return matches[0]
    raise RuntimeError("No suitable TrueType font found")


def rounded(
    draw: ImageDraw.ImageDraw,
    box: tuple[int, int, int, int],
    radius: int,
    fill: str,
    outline: Optional[str] = None,
) -> None:
    draw.rounded_rectangle(box, radius=radius, fill=fill, outline=outline)


def draw_code(draw: ImageDraw.ImageDraw, code: str, code_font: ImageFont.FreeTypeFont) -> None:
    for line_number, line in enumerate(code.rstrip("\n").split("\n"), 1):
        y = CODE_Y + (line_number - 1) * LINE_HEIGHT
        number = str(line_number)
        draw.text((51 - draw.textlength(number, font=code_font), y), number, font=code_font, fill=COLORS["muted"])
        x = CODE_X
        for token in TOKEN_RE.findall(line):
            lower = token.lower()
            if lower in KEYWORDS:
                color = COLORS["keyword"]
            elif lower in INTRINSICS:
                color = COLORS["intrinsic"]
            elif token.startswith(("'", '"')):
                color = COLORS["string"]
            elif token.startswith("!"):
                color = COLORS["comment"]
            elif token[:1].isdigit():
                color = COLORS["number"]
            else:
                color = COLORS["text"]
            draw.text((x, y), token, font=code_font, fill=color)
            x += draw.textlength(token, font=code_font)


def base_frame(code: str, state: str, code_font: ImageFont.FreeTypeFont, ui_font: ImageFont.FreeTypeFont) -> Image.Image:
    image = Image.new("RGB", (WIDTH, HEIGHT), COLORS["editor"])
    draw = ImageDraw.Draw(image)

    # Keep the crop focused on the editor rather than recreating the whole window.
    draw.rectangle((0, 0, 63, HEIGHT), fill=COLORS["gutter"])
    draw.line((63, 0, 63, HEIGHT), fill=COLORS["divider"])

    draw_code(draw, code, code_font)

    is_before = state == "Before"
    chip_fill = "#392f22" if is_before else "#173934"
    chip_color = COLORS["warning"] if is_before else COLORS["success"]
    chip_label = "Before formatting" if is_before else "After formatting"
    x1, y1, x2, y2 = WIDTH - 190, 14, WIDTH - 18, 47
    rounded(draw, (x1, y1, x2, y2), 16, chip_fill)
    draw.ellipse((x1 + 14, y1 + 11, x1 + 25, y1 + 22), fill=chip_color)
    if not is_before:
        draw.line((x1 + 16, y1 + 17, x1 + 19, y1 + 20), fill=chip_fill, width=2)
        draw.line((x1 + 19, y1 + 20, x1 + 24, y1 + 14), fill=chip_fill, width=2)
    draw.text((x1 + 34, y1 + 8), chip_label, font=ui_font, fill=chip_color)
    return image


def wipe(before: Image.Image, after: Image.Image, progress: float) -> Image.Image:
    result = before.copy()
    boundary = int(WIDTH * progress)
    result.paste(after.crop((0, 0, boundary, HEIGHT)), (0, 0))
    draw = ImageDraw.Draw(result, "RGBA")
    draw.rectangle((boundary - 14, 0, boundary + 14, HEIGHT), fill=(55, 148, 255, 24))
    draw.line((boundary, 0, boundary, HEIGHT), fill=(80, 170, 255, 210), width=2)
    return result


def render(output: Path) -> None:
    after_text = genuine_after()
    code_font = ImageFont.truetype(font_path(True), 16)
    ui_font = ImageFont.truetype(font_path(False), 13)
    before = base_frame(BEFORE, "Before", code_font, ui_font)
    after = base_frame(after_text, "After", code_font, ui_font)

    frames = [before]
    durations = [1700]
    for step in range(1, 13):
        frames.append(wipe(before, after, step / 12))
        durations.append(65)
    frames.append(after)
    durations.append(2800)

    output.parent.mkdir(parents=True, exist_ok=True)
    frames[0].save(
        output,
        save_all=True,
        append_images=frames[1:],
        duration=durations,
        loop=0,
        optimize=True,
        disposal=2,
    )
    try:
        display_path = output.relative_to(ROOT)
    except ValueError:
        display_path = output
    print(f"Rendered {display_path} from {find_formatter()}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()
    render(args.output.resolve())


if __name__ == "__main__":
    main()
