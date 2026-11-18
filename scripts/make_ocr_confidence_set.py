#!/usr/bin/env python3
"""Writes a labelled scan set for `search-core/examples/ocr_confidence_eval.rs`.

Pages of known text (sentences from this repository's own docs) are drawn in several fonts and sizes, then degraded
the way real scans are: blur, noise, speckle, low contrast, rotation, low resolution, JPEG recompression, uneven
lighting and combinations. Per page the output folder gets `page-NNN.pgm` (8-bit greyscale, binary P5) and
`page-NNN.tsv` (one row per truth line: x0, y0, x1, y1 of the line's centre line after rotation, height in pixels,
text); `manifest.tsv` lists page, condition, font, point size and dpi.

  python3 scripts/make_ocr_confidence_set.py OUT_DIR [--pages-per-condition N] [--seed S] [--corpus DIR]

Needs Pillow and numpy and the macOS system fonts named in FONTS (pass `--font-dir` for another folder with the same
file names, or edit FONTS). The set is synthetic: it shows whether the recogniser's own probabilities track its errors
on text with known truth, not how a particular scanner behaves.
"""
import argparse
import io
import math
import random
import re
import unicodedata
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

FONTS = {
    "arial": "Arial.ttf",
    "times": "Times New Roman.ttf",
    "courier": "Courier New.ttf",
    "georgia": "Georgia.ttf",
    "verdana": "Verdana.ttf",
    "trebuchet": "Trebuchet MS.ttf",
}

# name: (dpi, blur px, gaussian noise sigma, speckle fraction, contrast (1 = full), rotation degrees, jpeg quality, shading)
CONDITIONS = {
    "clean": (200, 0, 0, 0, 1.0, 0, 0, 0),
    "dpi100": (100, 0, 0, 0, 1.0, 0, 0, 0),
    "dpi300": (300, 0, 0, 0, 1.0, 0, 0, 0),
    "blur1": (200, 1.0, 0, 0, 1.0, 0, 0, 0),
    "blur2": (200, 2.0, 0, 0, 1.0, 0, 0, 0),
    "noise": (200, 0, 25, 0, 1.0, 0, 0, 0),
    "noise-heavy": (200, 0, 55, 0, 1.0, 0, 0, 0),
    "speckle": (200, 0, 0, 0.02, 1.0, 0, 0, 0),
    "speckle-heavy": (200, 0, 0, 0.06, 1.0, 0, 0, 0),
    "low-contrast": (200, 0, 6, 0, 0.35, 0, 0, 0),
    "rotate1": (200, 0, 0, 0, 1.0, 1.0, 0, 0),
    "rotate2": (200, 0, 0, 0, 1.0, 2.0, 0, 0),
    "jpeg20": (200, 0, 0, 0, 1.0, 0, 20, 0),
    "shading": (200, 0, 5, 0, 1.0, 0, 0, 90),
    "combined": (150, 1.0, 22, 0.01, 0.7, 1.0, 40, 40),
    "combined-heavy": (150, 1.5, 35, 0.03, 0.5, 1.5, 25, 60),
}


def corpus_sentences(root: Path, rng: random.Random):
    """Printable-ASCII prose lines of 30-90 characters from the markdown files under root."""
    seen, out = set(), []
    for path in sorted(root.rglob("*.md")):
        for raw in path.read_text(errors="ignore").splitlines():
            line = unicodedata.normalize("NFKC", raw)
            line = re.sub(r"[`*_#>|]", "", line).strip()
            line = line.encode("ascii", "ignore").decode()
            line = re.sub(r"\s+", " ", line)
            if not 30 <= len(line) <= 90 or sum(c.isalpha() for c in line) < 15 or line in seen:
                continue
            seen.add(line)
            out.append(line)
    rng.shuffle(out)
    return out


def render_page(lines, font_path, pt, dpi, rng):
    """Draws the lines that fit between the margins; returns the image, truth rows and how many lines were consumed."""
    width, height = round(8.27 * dpi), round(11.69 * dpi)
    px = max(8, round(pt * dpi / 72))
    font = ImageFont.truetype(font_path, px)
    img = Image.new("L", (width, height), 255)
    draw = ImageDraw.Draw(img)
    ascent, descent = font.getmetrics()
    pitch = round((ascent + descent) * 1.35)
    margin = round(0.9 * dpi)
    room = width - 2 * margin - round(0.05 * dpi)
    truth, y = [], margin
    for used, text in enumerate(lines, 1):
        if y + pitch > height - margin:
            used -= 1
            break
        if draw.textlength(text, font=font) > room:
            continue
        x = margin + rng.randint(0, round(0.05 * dpi))
        draw.text((x, y + ascent), text, fill=0, font=font, anchor="ls")
        right = x + draw.textlength(text, font=font)
        centre = y + ascent - px * 0.3  # about the middle of the x-height
        truth.append([x, centre, right, centre, px, text])
        y += pitch
    return img, truth, used


def rotate_point(x, y, angle, cx, cy):
    """PIL's rotate(angle) turns the picture counter-clockwise on screen (y down)."""
    t = math.radians(angle)
    dx, dy = x - cx, y - cy
    return cx + dx * math.cos(t) + dy * math.sin(t), cy - dx * math.sin(t) + dy * math.cos(t)


def degrade(img, truth, cond, rng, np_rng):
    dpi, blur, noise, speckle, contrast, rot, jpeg, shading = cond
    if contrast < 1:
        arr = np.asarray(img, dtype=np.float32)
        img = Image.fromarray((255 - (255 - arr) * contrast * 0.9).astype(np.uint8))
    if blur:
        img = img.filter(ImageFilter.GaussianBlur(blur))
    if rot:
        angle = rot * rng.choice([-1, 1])
        w, h = img.size
        img = img.rotate(angle, resample=Image.BICUBIC, fillcolor=255)
        for row in truth:
            row[0], row[1] = rotate_point(row[0], row[1], angle, w / 2, h / 2)
            row[2], row[3] = rotate_point(row[2], row[3], angle, w / 2, h / 2)
    arr = np.asarray(img, dtype=np.float32)
    if shading:
        h, w = arr.shape
        ramp = np.linspace(0, 1, w)[None, :] * 0.6 + np.linspace(0, 1, h)[:, None] * 0.4
        arr = arr - shading * ramp
    if noise:
        arr = arr + np_rng.normal(0, noise, arr.shape)
    if speckle:
        hits = np_rng.random(arr.shape)
        arr[hits < speckle / 2] = 0
        arr[hits > 1 - speckle / 2] = 255
    img = Image.fromarray(np.clip(arr, 0, 255).astype(np.uint8))
    if jpeg:
        buf = io.BytesIO()
        img.save(buf, "JPEG", quality=jpeg)
        img = Image.open(io.BytesIO(buf.getvalue())).convert("L")
    return img


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("--pages-per-condition", type=int, default=5)
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--corpus", default="docs")
    ap.add_argument("--font-dir", default="/System/Library/Fonts/Supplemental")
    args = ap.parse_args()
    rng = random.Random(args.seed)
    np_rng = np.random.default_rng(args.seed)
    sentences = corpus_sentences(Path(args.corpus), rng)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    manifest, page, cursor = ["page\tcondition\tfont\tpt\tdpi"], 0, 0
    fonts = list(FONTS)
    for name, cond in CONDITIONS.items():
        for _ in range(args.pages_per_condition):
            font = fonts[page % len(fonts)]
            pt = rng.choice([9, 10, 11, 12])
            chunk = [sentences[(cursor + i) % len(sentences)] for i in range(120)]
            img, truth, used = render_page(chunk, str(Path(args.font_dir) / FONTS[font]), pt, cond[0], rng)
            cursor += used
            img = degrade(img, truth, cond, rng, np_rng)
            stem = f"page-{page:03d}"
            w, h = img.size
            (out / f"{stem}.pgm").write_bytes(f"P5\n{w} {h}\n255\n".encode() + img.tobytes())
            (out / f"{stem}.tsv").write_text(
                "".join(f"{r[0]:.1f}\t{r[1]:.1f}\t{r[2]:.1f}\t{r[3]:.1f}\t{r[4]}\t{r[5]}\n" for r in truth)
            )
            manifest.append(f"{stem}\t{name}\t{font}\t{pt}\t{cond[0]}")
            page += 1
    (out / "manifest.tsv").write_text("\n".join(manifest) + "\n")
    print(f"{page} pages in {out}")


if __name__ == "__main__":
    main()
