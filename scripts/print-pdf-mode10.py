#!/usr/bin/env python3
"""Render a PDF page and generate/send experimental SL-J1660 Mode10 raw output.

This script reuses the local Mode10 compressor in print-text-mode10.py. It uses
Poppler's `pdftoppm` when available, then converts the rendered page to
quantized grayscale RGB rows that the SL-J1660 accepts through the same PCL3GUI
wrapper.
"""

from __future__ import annotations

import argparse
import importlib.util
import math
import shutil
import subprocess
import tempfile
from pathlib import Path

from PIL import Image, ImageFilter, ImageOps


def load_mode10_module():
    path = Path(__file__).with_name("print-text-mode10.py")
    spec = importlib.util.spec_from_file_location("slj1660_print_text_mode10", path)
    if spec is None or spec.loader is None:
        raise SystemExit(f"failed to load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


MODE10 = load_mode10_module()


def render_pdf_page(pdf: Path, dpi: int, page: int) -> Image.Image:
    pdftoppm = shutil.which("pdftoppm")
    if pdftoppm is None:
        raise SystemExit(
            "pdftoppm is required for PDF rendering. Install poppler, for example: brew install poppler"
        )

    with tempfile.TemporaryDirectory(prefix="slj1660-pdf-") as tmp:
        prefix = Path(tmp) / "page"
        subprocess.run(
            [
                pdftoppm,
                "-r",
                str(dpi),
                "-png",
                "-singlefile",
                "-f",
                str(page),
                "-l",
                str(page),
                str(pdf),
                str(prefix),
            ],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
        )
        rendered = prefix.with_suffix(".png")
        if not rendered.exists():
            raise SystemExit(f"pdftoppm did not produce {rendered}")
        return Image.open(rendered).convert("L").copy()


def fit_to_print_width(image: Image.Image) -> Image.Image:
    if image.width == MODE10.WIDTH_PIXELS:
        return image
    height = max(1, round(image.height * MODE10.WIDTH_PIXELS / image.width))
    return image.resize((MODE10.WIDTH_PIXELS, height), Image.Resampling.LANCZOS)


def prepare_tone_image(image: Image.Image, args: argparse.Namespace) -> Image.Image:
    if args.tone_mode == "document":
        return document_tone(
            apply_gamma(image, args.gamma),
            args.quantize_levels,
            args.edge_levels,
            args.edge_threshold,
            args.snap_black,
            args.snap_white,
        )
    if args.tone_mode == "gray":
        return apply_gamma(image, args.gamma)
    if args.tone_mode == "quantize":
        return quantize_gray(apply_gamma(image, args.gamma), args.quantize_levels)
    if args.tone_mode == "ordered":
        return ordered_halftone(apply_gamma(image, args.gamma), args.ordered_levels)
    if args.tone_mode != "binary":
        raise SystemExit(f"unsupported tone mode: {args.tone_mode}")

    if args.no_dither:
        return image.point(lambda p: 0 if p < args.threshold else 255, "L")

    adjusted = ImageOps.autocontrast(image)
    return adjusted.convert("1", dither=Image.Dither.FLOYDSTEINBERG).convert("L")


def apply_gamma(image: Image.Image, gamma: float) -> Image.Image:
    if gamma <= 0:
        raise SystemExit("--gamma must be greater than zero")
    if math.isclose(gamma, 1.0):
        return image

    inv = 1.0 / gamma
    table = [round(((value / 255) ** inv) * 255) for value in range(256)]
    return image.point(table, "L")


def quantize_gray(image: Image.Image, levels: int) -> Image.Image:
    if levels < 2 or levels > 256:
        raise SystemExit("--quantize-levels must be between 2 and 256")
    if levels == 256:
        return image

    table = []
    for value in range(256):
        level = round(value * (levels - 1) / 255)
        table.append(round(level * 255 / (levels - 1)))
    return image.point(table, "L")


def quantize_table(levels: int) -> list[int]:
    if levels < 2 or levels > 256:
        raise SystemExit("quantize levels must be between 2 and 256")
    return [
        round(round(value * (levels - 1) / 255) * 255 / (levels - 1))
        for value in range(256)
    ]


def document_tone(
    image: Image.Image,
    base_levels: int,
    edge_levels: int,
    edge_threshold: int,
    snap_black: int,
    snap_white: int,
) -> Image.Image:
    if not 0 <= edge_threshold <= 255:
        raise SystemExit("--edge-threshold must be between 0 and 255")
    if not 0 <= snap_black <= 255:
        raise SystemExit("--snap-black must be between 0 and 255")
    if not 0 <= snap_white <= 255:
        raise SystemExit("--snap-white must be between 0 and 255")
    if snap_black >= snap_white:
        raise SystemExit("--snap-black must be smaller than --snap-white")

    base = quantize_table(base_levels)
    edge = quantize_table(edge_levels)
    edge_map = image.filter(ImageFilter.FIND_EDGES).tobytes()
    src = image.tobytes()
    out = bytearray(len(src))

    for i, value in enumerate(src):
        if value <= snap_black:
            out[i] = 0
        elif value >= snap_white and edge_map[i] < edge_threshold:
            out[i] = 255
        elif edge_map[i] >= edge_threshold:
            out[i] = edge[value]
        else:
            out[i] = base[value]

    return Image.frombytes("L", image.size, bytes(out))


def ordered_halftone(image: Image.Image, levels: int) -> Image.Image:
    if levels < 2 or levels > 16:
        raise SystemExit("--ordered-levels must be between 2 and 16")

    bayer = (
        (0, 48, 12, 60, 3, 51, 15, 63),
        (32, 16, 44, 28, 35, 19, 47, 31),
        (8, 56, 4, 52, 11, 59, 7, 55),
        (40, 24, 36, 20, 43, 27, 39, 23),
        (2, 50, 14, 62, 1, 49, 13, 61),
        (34, 18, 46, 30, 33, 17, 45, 29),
        (10, 58, 6, 54, 9, 57, 5, 53),
        (42, 26, 38, 22, 41, 25, 37, 21),
    )
    src = image.tobytes()
    out = bytearray(len(src))
    width, height = image.size
    for y in range(height):
        row_start = y * width
        matrix_row = bayer[y & 7]
        for x in range(width):
            value = src[row_start + x]
            threshold = (matrix_row[x & 7] + 0.5) / 64 - 0.5
            scaled = value * (levels - 1) / 255 + threshold
            level = min(levels - 1, max(0, round(scaled)))
            out[row_start + x] = round(level * 255 / (levels - 1))
    return Image.frombytes("L", image.size, bytes(out))


def rows_from_image(image: Image.Image, blank_threshold: int) -> list[tuple[bool, bytes]]:
    if image.width != MODE10.WIDTH_PIXELS:
        raise SystemExit(f"expected width {MODE10.WIDTH_PIXELS}, got {image.width}")
    if not 0 <= blank_threshold <= 255:
        raise SystemExit("--blank-threshold must be between 0 and 255")

    triplets = [bytes((value, value, value)) for value in range(256)]
    src = image.tobytes()
    rows: list[tuple[bool, bytes]] = []
    for y in range(image.height):
        row = src[y * image.width : (y + 1) * image.width]
        has_ink = any(value < blank_threshold for value in row)
        if has_ink:
            rows.append((True, b"".join(triplets[value] for value in row)))
        else:
            rows.append((False, bytes([0xFF]) * MODE10.ROW_BYTES))
    return rows


def write_preview(path: Path, image: Image.Image, crop_width: int) -> None:
    image.crop((0, 0, min(image.width, crop_width), image.height)).save(path)


def build_pdf_raw(args: argparse.Namespace) -> bytes:
    image = fit_to_print_width(render_pdf_page(Path(args.pdf), args.dpi, args.page))
    if args.max_height and image.height > args.max_height:
        image = image.crop((0, 0, image.width, args.max_height))

    image = prepare_tone_image(image, args)

    if args.preview:
        write_preview(Path(args.preview), image, args.preview_width)

    row_infos = rows_from_image(image, args.blank_threshold)
    nonblank = [index for index, (has_ink, _) in enumerate(row_infos) if has_ink]
    if not nonblank:
        raise SystemExit("rendered PDF page is blank after tone conversion")

    first = min(nonblank)
    last = max(nonblank)
    rows = [row for _, row in row_infos[first : last + 1]]

    seed = bytearray([0xFF] * MODE10.ROW_BYTES)
    raster = bytearray()
    raster += f"\x1b*p{args.cursor_y}Y".encode("ascii")
    raster += f"\x1b*b{args.top_skip + first}Y".encode("ascii")

    blank_run = 0
    for row in rows:
        if all(value == 0xFF for value in row):
            blank_run += 1
            continue

        if blank_run:
            raster += f"\x1b*b{blank_run}Y".encode("ascii")
            seed[:] = b"\xFF" * MODE10.ROW_BYTES
            blank_run = 0

        payload = MODE10.mode10_row(row, seed)
        if payload:
            raster += f"\x1b*b{len(payload)}W".encode("ascii")
            raster += payload
        else:
            raster += b"\x1b*b0W"
        seed[:] = row

    if blank_run:
        raster += f"\x1b*b{blank_run}Y".encode("ascii")
    raster += f"\x1b*b{args.bottom_skip}Y".encode("ascii")
    raster += b"\x1b*b0V\x1b*b0W"

    template = Path(args.template).read_bytes()
    raster_start = template.find(b"\x1b*r1A")
    raster_end = template.find(b"\x1b*rC")
    if raster_start < 0 or raster_end < 0:
        raise SystemExit(f"template is missing raster markers: {args.template}")

    prefix = template[: raster_start + len(b"\x1b*r1A")]
    suffix = template[raster_end:]
    return prefix + raster + suffix


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pdf")
    parser.add_argument("--out", default="/tmp/slj1660-pdf-mode10.raw")
    parser.add_argument("--preview")
    parser.add_argument("--preview-width", type=int, default=1800)
    parser.add_argument("--template", default="fixtures/captured-confirmed/text-only.raw")
    parser.add_argument("--page", type=int, default=1)
    parser.add_argument("--dpi", type=int, default=600)
    parser.add_argument(
        "--tone-mode",
        choices=["binary", "gray", "quantize", "document", "ordered"],
        default="quantize",
        help="tone conversion before Mode10 compression",
    )
    parser.add_argument("--threshold", type=int, default=170)
    parser.add_argument("--no-dither", action="store_true")
    parser.add_argument(
        "--quantize-levels",
        type=int,
        default=12,
        help="number of gray levels for --tone-mode quantize",
    )
    parser.add_argument(
        "--ordered-levels",
        type=int,
        default=2,
        help="number of gray levels for --tone-mode ordered",
    )
    parser.add_argument(
        "--edge-levels",
        type=int,
        default=12,
        help="number of gray levels for edge pixels in --tone-mode document",
    )
    parser.add_argument(
        "--edge-threshold",
        type=int,
        default=18,
        help="edge-map threshold for --tone-mode document",
    )
    parser.add_argument(
        "--snap-black",
        type=int,
        default=8,
        help="input gray values at or below this are snapped to black in --tone-mode document",
    )
    parser.add_argument(
        "--snap-white",
        type=int,
        default=250,
        help="non-edge input gray values at or above this are snapped to white in --tone-mode document",
    )
    parser.add_argument(
        "--gamma",
        type=float,
        default=1.0,
        help="gamma adjustment before gray/quantize/ordered tone conversion",
    )
    parser.add_argument(
        "--blank-threshold",
        type=int,
        default=250,
        help="rows whose pixels are all at least this value are skipped as blank",
    )
    parser.add_argument("--cursor-y", type=int, default=0)
    parser.add_argument("--top-skip", type=int, default=0)
    parser.add_argument("--bottom-skip", type=int, default=220)
    parser.add_argument(
        "--max-height",
        type=int,
        default=7200,
        help="crop very tall rendered pages for the current experimental single-page path",
    )
    parser.add_argument("--send", action="store_true")
    parser.add_argument("--serial")
    parser.add_argument("--chunk-size", type=int, default=16227)
    parser.add_argument("--timeout-ms", type=int, default=30000)
    parser.add_argument("--chunk-delay-ms", type=int, default=0)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    out = Path(args.out)
    raw = build_pdf_raw(args)
    out.write_bytes(raw)
    print(f"wrote {out} ({len(raw)} bytes)")
    if args.preview:
        print(f"wrote preview {args.preview}")

    if args.send:
        cmd = [
            "cargo",
            "run",
            "--",
            "send-raw",
            str(out),
            "--chunk-size",
            str(args.chunk_size),
            "--timeout-ms",
            str(args.timeout_ms),
            "--chunk-delay-ms",
            str(args.chunk_delay_ms),
        ]
        if args.serial:
            cmd.extend(["--serial", args.serial])
        subprocess.run(cmd, check=True)


if __name__ == "__main__":
    main()
