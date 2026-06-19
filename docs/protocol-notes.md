# Protocol Notes

## Current Hypothesis

Samsung SL-J1660 appears related to the low-cost HP DeskJet inkjet family, and
may use an HP DeskJet 1000 J110 / PCL3GUI-style raster pipeline. This is a
working hypothesis only. The project must verify it through captured streams.

## Public Research

- HP lists `HP DeskJet 1000 j110 Series` as supported by HPLIP starting with
  version `3.10.9`, with USB connectivity:
  <https://developers.hp.com/hp-linux-imaging-and-printing/supported_devices/index>
- HP states that PCL3GUI remains proprietary, uses compressed raster data, and
  is not compatible with standard PCL3:
  <https://developers.hp.com/hp-printer-command-languages-pcl>
- OpenPrinting's HPLIP Printer Application wraps HPLIP resources including
  `hpcups` and the `hp` backend:
  <https://github.com/OpenPrinting/hplip-printer-app>
- Debian describes `printer-driver-hpcups` as a CUPS-Raster-based driver for
  many HP inkjet printers, with PPDs generated from CUPS DDK `.drv` metadata:
  <https://packages.debian.org/bookworm/printer-driver-hpcups>
- HPLIP licensing is mixed. HP describes the general source as GPL with
  exceptions for the backend under MIT and HPIJS under BSD:
  <https://developers.hp.com/hp-linux-imaging-and-printing/license>

## Licensing Boundary

This repository started as a clean-room scaffold. For local personal use, the
experimental text path now includes a Python port of HP HPLIP's
`prnt/hpcups/Mode10.cpp` compressor. That HPLIP file carries a BSD-style HP
license header, so keep the attribution and license notice if copying or
redistributing the script. Do not describe the Mode10 encoder as clean-room.

## Unknowns

- exact transport framing for SL-J1660
- whether raw replay should use simple bulk stream writes or an IEEE-1284.4-like
  packet protocol for this device
- page/job start and end commands
- raster resolution declarations
- row ordering and padding
- compression method
- checksums or length fields
- whether Windows and HPLIP streams differ materially

## Required Fixtures

Protocol work should not proceed until these captures exist:

- blank single page
- text-only single page
- black rectangle
- horizontal lines
- checkerboard

Each fixture should be paired with a short note describing OS, driver version,
application used, paper size, grayscale/color settings, and whether replay on
macOS succeeded.

## Imported Windows Captures

Five Windows-driver captures were imported on 2026-06-17 and summarized in
`docs/capture-analysis.md`. The print stream was isolated to USB bulk OUT
endpoint `0x08`; endpoint `0x0a` carried REST status polling and was excluded.
The captures show a PJL wrapper entering `PCL3GUI`, followed by PCL raster
commands including `ESC*t600R`, `ESC*r4891S`, and many `ESC*b...W/Y/V`
sequences. These are enough to guide a parser, but not enough to claim a
complete encoder yet.

The macOS `send-raw` path has successfully replayed `blank-page.raw` and
`text-only.raw` to interface `1`, endpoint `0x08`.

A macOS REST probe to endpoint `0x0a` / `0x8b` returned `HTTP/1.1 200 OK`, which
confirms user-space USB writes can reach and receive from the live device. The
reported printer state included `cartridgeVeryLow` and `singleCartridgeMode`;
clear or acknowledge those states before treating non-printing replay as a
protocol failure.

USB printer-class `GET_PORT_STATUS` returned `0x10` during the first cartridge
gate session, decoded here as selected, paper present, and error. `SOFT_RESET`
succeeded and allowed replay with the Windows transfer boundary, but the device
still reported error afterward. The blocking condition was not the print
endpoint; it was a cartridge/status gate.

In a later generated-PDF print session, `GET_PORT_STATUS` returned `0x30`.
The USB printer-class bit name looks like `paper-empty`, but the physical
observation was more nuanced: the printer's restart/resume button was blinking,
pressing it ejected or cleared the pending page, and the next send succeeded
without changing the generated raw stream. Treat `0x30` on this device as
`paper-empty-or-feed-attention`, not proof that the tray is literally empty.

Windows round-2 captures then clarified the higher-level recovery path:
Windows did not use printer-class `GET_PORT_STATUS` or `SOFT_RESET` for the
observed recovery. It used LEDM REST on endpoint `0x0a` / `0x8b`. The relevant
software-resume request is preserved as
`fixtures/confirm/tray-empty-or-open-resume.http` and can be sent with
`scripts/resume-feed-attention.sh`.

The local IPP daemon uses the same fixture as an automatic in-job watchdog:
when a generated raw page transfer remains active past the normal fast-transfer
window, it sends the LEDM resume request on interface `3`, endpoint `0x0a` while
the printer data endpoint is still owned by the active job. This keeps the
manual helper available for recovery, but the normal print path no longer needs
a separate resume command from the user.

## Confirmed Replay Path

Captured raw replay physically printed on 2026-06-17 after acknowledging the
printer's cartridge state over the USB REST/status endpoint:

- endpoint `0x0a`, interface `3`: LEDM HTTP requests such as
  `PUT /DevMgmt/ConsumableConfigDyn.xml` and
  `PUT /DevMgmt/ProductStatusDyn.xml`
- endpoint `0x08`, interface `1`: PCL3GUI/PJL print stream
- working raw fixture: `fixtures/captured-confirmed/text-only.raw`
- working print transfer boundary: first chunk `16227` bytes, then remaining
  bytes

The `lowink-continue.http` and `cartridge-refilled-ok.http` requests came from
Windows captures. The `single-cartridge-ok.http` request was synthesized from
the same captured LEDM XML shape and validated by the successful print. Keep
that distinction in future notes.

The `analyze-raw` command now identifies the captured `ESC*b...W/Y/V` stream as
Mode10-style PCL3GUI delta/RLE row compression rather than page-level
JPEG/FLATE/JBIG blocks. The remaining encoder work is therefore not "find a
different transport"; it is to produce better row content before Mode10:
halftone or quantize gray pages so they compress closer to Windows output while
keeping 600 dpi text quality.

## Generated Mode10 Text Success

On 2026-06-17, a freshly generated Korean text page printed from macOS. The
working path was:

- render text to a 600 dpi, 4891-pixel-wide, 3-byte RGB row buffer
- use white pixels as `ff ff ff` and black pixels as `00 00 00`
- initialize the Mode10 seed row to `ff`
- compress rows with an HPLIP `Mode10.cpp`-derived encoder
- reuse the Windows-captured PJL/PCL3GUI wrapper through `ESC*r1A`
- emit `ESC*p498Y`, `ESC*b<top>Y`, Mode10 `ESC*b...W` row payloads,
  `ESC*b...Y`, `ESC*b0V`, `ESC*b0W`, and the captured footer

Failed experiments before this success:

- 1-bit packed rows: physically printed corrupted bar patterns.
- 4891-byte single-channel unencoded rows: physically printed corrupted bar
  patterns.
- standard PCL text with UTF-8 Korean: accepted over USB but did not produce a
  useful page.
- PCL method 9 compressed replacement delta rows: accepted over USB but did not
  match the working output path.

The current local reproduction command is:

```sh
scripts/print-text-mode10.py "와쏘베쏘" --out /tmp/slj1660-text.raw --send
```

## Experimental PDF Path

PDF printing should use the same Mode10 encoder after rasterization. The local
helper renders the first PDF page through Poppler `pdftoppm`, scales it to the
captured 4891-pixel print width, converts it to 12-level grayscale by default,
then encodes nonblank rows with the same Mode10 path:

```sh
scripts/print-pdf-mode10.py sample.pdf \
  --preview /tmp/slj1660-pdf-preview.png \
  --out /tmp/slj1660-pdf.raw
```

Append `--send` to print. This has the right protocol shape, but should still
be treated as experimental until several PDFs with different page layouts are
physically verified. Multi-page documents are currently sent one page at a time
with `--page <n>`.

The PDF helper still exposes `--tone-mode binary`, `--tone-mode gray`,
`--tone-mode quantize`, `--tone-mode document`, and `--tone-mode ordered` for
experiments. On the INU timetable PNG/PDF sample, 10-level quantization
produced a roughly 354 KB raw stream and 12-level quantization produced a
roughly 401 KB raw stream versus roughly 1.39 MB for continuous grayscale.
The physical q12 print kept the timetable's pale colored regions better than
q10 while remaining small enough for practical USB transfer, so q12 is now the
default.

When the generator emits `ESC*b#Y` for blank rows, it now resets the local
Mode10 seed row to white before encoding the next nonblank row. This matches
the expected PCL seed-row behavior better than keeping the previous nonblank
row as the host-side seed.
