#!/usr/bin/env python3
"""Generate or send an experimental SL-J1660 PCL3GUI Mode10 text page.

The Mode10 compressor below is a small Python port of HP HPLIP's
prnt/hpcups/Mode10.cpp algorithm. The original file carries a BSD-style HP
license; keep that attribution if this script is copied elsewhere.
Source reference:
https://github.com/Distrotech/hplip/blob/master/prnt/hpcups/Mode10.cpp

Original Mode10.cpp notice:
Copyright (c) 1996 - 2001, Hewlett-Packard Co. All rights reserved.
Redistribution and use in source and binary forms, with or without
modification, are permitted provided that source redistributions retain the
copyright notice, conditions, and disclaimer; binary redistributions reproduce
them in documentation or other materials; and neither the Hewlett-Packard name
nor contributor names are used for endorsement without prior written
permission. The original software is provided "as is", without express or
implied warranties, and the author disclaims liability for damages arising from
use.
"""

from __future__ import annotations

import argparse
import os
import subprocess
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

KWHITE = 0x00FFFFFE
E_LITERAL = 0x00
E_RLE = 0x80
EE_NEW = 0x00
EE_W = 0x20
EE_NE = 0x40
EE_CACHED = 0x60
WIDTH_PIXELS = 4891
BYTES_PER_PIXEL = 3
ROW_BYTES = WIDTH_PIXELS * BYTES_PER_PIXEL


def get_pixel(row: bytearray | bytes, pixel: int) -> int:
    index = pixel * BYTES_PER_PIXEL
    return ((row[index] << 16) | (row[index + 1] << 8) | row[index + 2]) & KWHITE


def put_pixel(row: bytearray, pixel: int, value: int) -> None:
    value &= KWHITE
    index = pixel * BYTES_PER_PIXEL
    row[index] = (value >> 16) & 0xFF
    row[index + 1] = (value >> 8) & 0xFF
    row[index + 2] = value & 0xFF


def red(pixel: int) -> int:
    return (pixel >> 16) & 0xFF


def green(pixel: int) -> int:
    return (pixel >> 8) & 0xFF


def blue(pixel: int) -> int:
    return pixel & 0xFF


def short_delta(pixel: int, upper_pixel: int) -> int:
    dr = red(pixel) - red(upper_pixel)
    dg = green(pixel) - green(upper_pixel)
    db = blue(pixel) - blue(upper_pixel)
    if -16 <= dr <= 15 and -16 <= dg <= 15 and -32 <= db <= 30:
        return ((dr << 10) & 0x007C00) | ((dg << 5) & 0x0003E0) | ((db >> 1) & 0x001F) | 0x8000
    return 0


def output_vli(number: int, out: bytearray) -> None:
    while True:
        value = min(number, 255)
        out.append(value)
        if number == 255:
            out.append(0)
        number -= value
        if number == 0:
            return


def emit_pixel(pixel: int, upper_pixel: int, out: bytearray) -> None:
    compressed = short_delta(pixel, upper_pixel)
    if compressed:
        out.append((compressed >> 8) & 0xFF)
        out.append(compressed & 0xFF)
        return

    uncompressed = pixel >> 1
    out.append((uncompressed >> 16) & 0xFF)
    out.append((uncompressed >> 8) & 0xFF)
    out.append(uncompressed & 0xFF)


def mode10_row(row_bytes: bytes, seed_row: bytearray) -> bytes:
    cur = bytearray(row_bytes)
    out = bytearray()
    last_pixel = WIDTH_PIXELS - 1
    real_last_pixel = get_pixel(cur, last_pixel)

    new_last_pixel = real_last_pixel
    while (
        get_pixel(cur, last_pixel - 1) == new_last_pixel
        or get_pixel(seed_row, last_pixel) == new_last_pixel
    ):
        new_last_pixel += 0x100
        put_pixel(cur, last_pixel, new_last_pixel)

    cur_pixel = 0
    cached_color = KWHITE

    while cur_pixel <= last_pixel:
        seed_copy_start = cur_pixel
        while get_pixel(seed_row, cur_pixel) == get_pixel(cur, cur_pixel):
            cur_pixel += 1
        seed_copy_count = cur_pixel - seed_copy_start

        cmd = 0
        replacement_count = 0
        pixel_source = EE_NEW

        if cur_pixel == last_pixel:
            put_pixel(cur, last_pixel, real_last_pixel)
            if get_pixel(seed_row, cur_pixel) == real_last_pixel:
                break
            cmd = E_LITERAL
            pixel_source = EE_NEW
            replacement_count = 1
            cur_pixel += 1
        else:
            start = cur_pixel
            rle_run = get_pixel(cur, cur_pixel)
            cur_pixel += 1
            while rle_run == get_pixel(cur, cur_pixel):
                cur_pixel += 1
            cur_pixel -= 1
            replacement_count = cur_pixel - start

            if replacement_count > 0:
                cur_pixel += 1
                replacement_count += 1
                run_start = cur_pixel - replacement_count
                if cached_color == rle_run:
                    pixel_source = EE_CACHED
                elif get_pixel(seed_row, run_start + 1) == rle_run:
                    pixel_source = EE_NE
                elif run_start > 0 and get_pixel(cur, run_start - 1) == rle_run:
                    pixel_source = EE_W
                else:
                    pixel_source = EE_NEW
                    cached_color = rle_run
                cmd = E_RLE

            if cur_pixel == last_pixel and real_last_pixel == rle_run:
                put_pixel(cur, last_pixel, real_last_pixel)
                replacement_count += 1
                cur_pixel += 1

            if replacement_count == 0:
                temp_pixel = get_pixel(cur, cur_pixel)
                cmd = E_LITERAL
                if cached_color == temp_pixel:
                    pixel_source = EE_CACHED
                elif get_pixel(seed_row, cur_pixel + 1) == temp_pixel:
                    pixel_source = EE_NE
                elif cur_pixel > 0 and get_pixel(cur, cur_pixel - 1) == temp_pixel:
                    pixel_source = EE_W
                else:
                    pixel_source = EE_NEW
                    cached_color = temp_pixel

                literal_start = cur_pixel
                next_pixel = get_pixel(cur, cur_pixel + 1)
                while True:
                    cur_pixel += 1
                    if cur_pixel == last_pixel:
                        put_pixel(cur, last_pixel, real_last_pixel)
                        cur_pixel += 1
                        break
                    cache_pixel = next_pixel
                    next_pixel = get_pixel(cur, cur_pixel + 1)
                    if cache_pixel == next_pixel or cache_pixel == get_pixel(seed_row, cur_pixel):
                        break
                replacement_count = cur_pixel - literal_start

        if cmd == E_LITERAL:
            normalized = replacement_count - 1
            cmd_byte = cmd | pixel_source | (min(3, seed_copy_count) << 3) | min(7, normalized)
            out.append(cmd_byte)
            if seed_copy_count >= 3:
                output_vli(seed_copy_count - 3, out)

            total = replacement_count
            remaining = replacement_count
            upward = 1
            if pixel_source != EE_NEW:
                remaining -= 1
                upward = 2
            while upward <= total:
                index = cur_pixel - remaining
                emit_pixel(get_pixel(cur, index), get_pixel(seed_row, index), out)
                if (upward - 8) % 255 == 0:
                    out.append(min(255, total - upward))
                remaining -= 1
                upward += 1
        else:
            normalized = replacement_count - 2
            cmd_byte = cmd | pixel_source | (min(3, seed_copy_count) << 3) | min(7, normalized)
            out.append(cmd_byte)
            if seed_copy_count >= 3:
                output_vli(seed_copy_count - 3, out)
            if pixel_source == EE_NEW:
                index = cur_pixel - replacement_count
                emit_pixel(get_pixel(cur, index), get_pixel(seed_row, index), out)
            if replacement_count - 2 >= 7:
                output_vli(replacement_count - 9, out)

    return bytes(out)


def render_rgb_rows(args: argparse.Namespace) -> tuple[int, list[bytes]]:
    font = load_font(args)
    gray = Image.new("L", (WIDTH_PIXELS, args.canvas_height), 255)
    draw = ImageDraw.Draw(gray)
    draw.text((args.x, args.text_y), args.text, font=font, fill=0)

    rows: list[tuple[bool, bytes]] = []
    for y in range(gray.height):
        data = bytearray(ROW_BYTES)
        any_black = False
        for x in range(WIDTH_PIXELS):
            black = gray.getpixel((x, y)) < args.threshold
            index = x * BYTES_PER_PIXEL
            if black:
                any_black = True
                data[index] = 0x00
                data[index + 1] = 0x00
                data[index + 2] = 0x00
            else:
                data[index] = 0xFF
                data[index + 1] = 0xFF
                data[index + 2] = 0xFF
        rows.append((any_black, bytes(data)))

    nonblank = [index for index, (any_black, _) in enumerate(rows) if any_black]
    if not nonblank:
        raise SystemExit("rendered text is blank")

    first = min(nonblank)
    last = max(nonblank)
    return first, [row for _, row in rows[first : last + 1]]


def write_preview(path: Path, rows: list[bytes], crop_left: int, crop_right: int) -> None:
    preview = Image.new("L", (WIDTH_PIXELS, len(rows)), 255)
    pixels = preview.load()
    for y, row in enumerate(rows):
        for x in range(WIDTH_PIXELS):
            if row[x * BYTES_PER_PIXEL] == 0:
                pixels[x, y] = 0
    preview.crop((crop_left, 0, crop_right, len(rows))).save(path)


def load_font(args: argparse.Namespace) -> ImageFont.FreeTypeFont | ImageFont.ImageFont:
    candidates = []
    if args.font:
        candidates.append(Path(args.font))
    if os.environ.get("SLJ1660_FONT"):
        candidates.append(Path(os.environ["SLJ1660_FONT"]))
    candidates.extend(
        [
            Path("/System/Library/Fonts/AppleSDGothicNeo.ttc"),
            Path("/System/Library/Fonts/Supplemental/AppleGothic.ttf"),
            Path("/System/Library/Fonts/Supplemental/Arial Unicode.ttf"),
            Path("/System/Library/Fonts/Supplemental/Arial.ttf"),
        ]
    )

    for candidate in candidates:
        if candidate.exists():
            return ImageFont.truetype(str(candidate), args.font_size)

    return ImageFont.load_default()


def build_raw(args: argparse.Namespace) -> bytes:
    first, rows = render_rgb_rows(args)
    if args.preview:
        write_preview(Path(args.preview), rows, args.preview_left, args.preview_right)

    seed = bytearray([0xFF] * ROW_BYTES)
    compressed_rows = []
    for row in rows:
        payload = mode10_row(row, seed)
        if payload:
            compressed_rows.append(f"\x1b*b{len(payload)}W".encode("ascii") + payload)
        else:
            compressed_rows.append(b"\x1b*b0W")
        seed[:] = row

    template = Path(args.template).read_bytes()
    raster_start = template.find(b"\x1b*r1A")
    raster_end = template.find(b"\x1b*rC")
    if raster_start < 0 or raster_end < 0:
        raise SystemExit(f"template is missing raster markers: {args.template}")

    prefix = template[: raster_start + len(b"\x1b*r1A")]
    suffix = template[raster_end:]
    raster = bytearray()
    raster += b"\x1b*p498Y"
    raster += f"\x1b*b{args.top_skip + first}Y".encode("ascii")
    raster += b"".join(compressed_rows)
    raster += f"\x1b*b{args.bottom_skip}Y".encode("ascii")
    raster += b"\x1b*b0V\x1b*b0W"
    return prefix + raster + suffix


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("text")
    parser.add_argument("--out", default="/tmp/slj1660-mode10-text.raw")
    parser.add_argument(
        "--template",
        default="fixtures/captured-confirmed/text-only.raw",
        help="captured PCL3GUI raw stream to reuse for PJL/header/footer",
    )
    parser.add_argument(
        "--font",
        help="font path; defaults to SLJ1660_FONT or common macOS fonts",
    )
    parser.add_argument("--font-size", type=int, default=220)
    parser.add_argument("--x", type=int, default=520)
    parser.add_argument("--text-y", type=int, default=40)
    parser.add_argument("--canvas-height", type=int, default=300)
    parser.add_argument("--threshold", type=int, default=160)
    parser.add_argument("--top-skip", type=int, default=120)
    parser.add_argument("--bottom-skip", type=int, default=220)
    parser.add_argument("--preview")
    parser.add_argument("--preview-left", type=int, default=400)
    parser.add_argument("--preview-right", type=int, default=1900)
    parser.add_argument("--send", action="store_true")
    parser.add_argument("--serial")
    parser.add_argument("--chunk-size", type=int, default=16227)
    parser.add_argument("--timeout-ms", type=int, default=30000)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    out = Path(args.out)
    raw = build_raw(args)
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
        ]
        if args.serial:
            cmd.extend(["--serial", args.serial])
        subprocess.run(cmd, check=True)


if __name__ == "__main__":
    main()
