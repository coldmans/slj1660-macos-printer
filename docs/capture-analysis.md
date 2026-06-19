# Capture Analysis

Imported bundle:

- workspace copy: `<workspace>/slj1660-captures.zip`
- extracted copy: `<workspace>/captures-import/raw-extract/slj1660-captures`
- reusable raw fixtures: `fixtures/captured-confirmed/*.raw`
- bundle SHA-256:
  `daf1007ca2e13ad840f4093b14a7cff7181a7c02495484f1dfcbe4a9430baa2e`

All hashes in the supplied `SHA256SUMS.txt` verify after stripping Windows CRLF
line endings from the checksum file.

Second imported bundle:

- workspace copy:
  `<workspace>/slj1660-captures-confirm.zip`
- extracted copy:
  `<workspace>/captures-import/confirm-extract/slj1660-captures`
- reusable raw fixtures: `fixtures/captured-confirmed/*.raw`
- LEDM acknowledgement fixtures: `fixtures/confirm/*.http`
- bundle SHA-256:
  `b6022dd3065394c6135199a9e87b9451bedb2612529693ee6a0322b9a93ab9f0`

The second bundle includes Windows-observed confirmation traffic for the
low-ink gate. The `single-cartridge-ok.http` fixture is not a direct capture; it
was synthesized from the captured LEDM `ProductStatusDyn.xml` PUT pattern and
then validated by a successful physical print.

## Extraction Facts

The Windows capture identified the SL-J1660 print stream at:

- USB device address: `1`
- endpoint: `0x08`
- transfer type: bulk OUT
- filter:
  `usb.transfer_type==0x03 && usb.device_address==1 && usb.endpoint_address==0x08 && usb.capdata`

Endpoint `0x0a` was observed carrying HTTP/REST status polling and was excluded.

## Raw Files

| case | raw size | OUT packets | notes |
|---|---:|---:|---|
| `blank-page.raw` | 12,164 | 1 | setup only, no raster rows |
| `text-only.raw` | 17,176 | 2 | raster rows present |
| `horizontal-lines.raw` | 15,159 | 2 | repeated row/skip pattern |
| `black-rectangle.raw` | 38,357 | 8 | many zero-length raster row commands |
| `checkerboard.raw` | 42,032 | 9 | many zero-length raster row commands plus larger data chunks |

Every raw file starts with an 11,000-byte zero preamble. The print job begins at
offset `11000` with:

```text
ESC E
ESC %-12345X
@PJL SET STRINGCODESET=UTF8
...
@PJL ENTER LANGUAGE=PCL3GUI
@PJL SET USERNAME=""
ESC E
```

The non-blank jobs then enter a PCL raster section with:

```text
ESC &l0S
ESC &l1H
ESC *o5W <5 bytes: 0d 03 00 03 ec>
ESC *o0M
ESC &l26A
ESC &l0M
ESC *o5W <5 bytes: 0b 01 00 00 00>
ESC &u600D
ESC *t600R
ESC *g12W <12 bytes: 06 07 00 01 02 58 02 58 0a 01 20 01>
ESC *r4891S
ESC &l-2H
ESC *r1A
```

Observed interpretation candidates:

- `ESC &l26A` is consistent with A4 paper selection.
- `ESC &u600D` and `ESC *t600R` indicate 600 dpi operation in this captured
  Windows-driver path, even though the MVP target can still start from 300 dpi.
- `ESC *r4891S` is likely the effective raster width or source width.
- `ESC *b...W`, `ESC *b...Y`, and `ESC *b0V` carry/skip raster rows. Do not
  treat the `W` payload bytes as plain uncompressed bitmap rows yet.

## Command Statistics

| case | `*b...W` count | zero-length `*b0W` | W payload bytes | `*b...Y` count | `*b...Y` sum |
|---|---:|---:|---:|---:|---:|
| text-only | 133 | 1 | 4,169 | 2 | 380 |
| horizontal-lines | 326 | 276 | 1,075 | 26 | 5,563 |
| black-rectangle | 5,216 | 5,214 | 42 | 2 | 161 |
| checkerboard | 5,762 | 5,745 | 985 | 2 | 127 |

This strongly suggests that zero-length raster row commands are meaningful in
the printer language, likely interacting with seed-row/repeat/compression state.
The next encoder milestone must decode this behavior from captures before
generating printer-ready streams.

## Mode10 / Row-Compression Analysis

The `analyze-raw` command decodes enough of the row-compression stream to
separate row payloads, vertical skips, zero-length row repeats, and likely blank
rows:

```sh
cargo run -- analyze-raw fixtures/captured-confirmed/text-only.raw
```

Observed confirmed-capture results:

| case | guess | W rows | nonzero W | zero W | Y-skipped rows | W payload bytes | max W payload | payload vs RGB |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| text-only | Mode10-style delta/RLE | 133 | 132 | 1 | 380 | 4,169 | 65 | 0.05% |
| horizontal-lines | Mode10-style delta/RLE | 326 | 50 | 276 | 5,563 | 1,075 | 22 | 0.00% |
| black-rectangle | Mode10-style delta/RLE | 5,216 | 2 | 5,214 | 161 | 42 | 21 | 0.00% |
| checkerboard | Mode10-style delta/RLE | 5,762 | 17 | 5,745 | 127 | 985 | 65 | 0.00% |

This points away from page-level JPEG, FLATE/zlib, or JBIG blocks in the print
stream. The captured Windows output is using PCL3GUI row commands with
Mode10-style delta/RLE compression. The Windows driver DLLs contain strings for
JPEG/FLATE/RLE/JBIG-capable paths, but these fixture streams do not expose those
as top-level page blocks.

Local timetable experiments show the quality/speed tradeoff:

| local raw | W rows | nonzero W | zero W | Y-skipped rows | W payload bytes | max W payload | payload vs RGB |
|---|---:|---:|---:|---:|---:|---:|---:|
| `/tmp/slj1660-inu-timetable-a4-gray245.raw` | 6,206 | 3,792 | 2,414 | 681 | 1,340,920 | 3,827 | 1.32% |
| `/tmp/slj1660-inu-timetable-a4-nodither-t225.raw` | 4,299 | 2,221 | 2,078 | 2,586 | 53,945 | 155 | 0.05% |
| `/tmp/slj1660-inu-timetable-a4-q8.raw` | 6,200 | 3,191 | 3,009 | 686 | 253,083 | 847 | 0.25% |
| `/tmp/slj1660-inu-timetable-a4-q10.raw` | 6,203 | 3,051 | 3,152 | 683 | 306,526 | 1,021 | 0.30% |
| `/tmp/slj1660-inu-timetable-a4-q12.raw` | 6,205 | 3,142 | 3,063 | 682 | 353,783 | 1,205 | 0.35% |
| `/tmp/slj1660-inu-timetable-a4-q16.raw` | 6,216 | 3,446 | 2,770 | 673 | 477,473 | 1,531 | 0.47% |
| `/tmp/slj1660-inu-timetable-a4-q18.raw` | 6,218 | 3,507 | 2,711 | 672 | 465,799 | 1,468 | 0.46% |
| `/tmp/slj1660-inu-timetable-a4-document.raw` | 6,203 | 3,222 | 2,981 | 683 | 415,331 | 1,381 | 0.41% |

The grayscale output is already compressed from roughly 101 MB of logical RGB
rows down to about 1.34 MB of row payloads, but it still sends thousands of
nonzero row payloads. The thresholded B/W output is much smaller because its
rows contain long flat runs, but visual quality is lower.

The 8- to 12-level quantized candidates are a better middle path for UI-heavy
documents: they keep pale borders and course blocks visible while cutting W
payloads to roughly 19-26% of the continuous-grayscale sample. The q10 preview
looked like the best local balance before physical comparison, but the q12
print was better on paper because the timetable's pale colored regions were
less washed out. q12 is therefore the current default despite the larger raw
stream.
The q16/q18 short-delta hypothesis did not win on this sample: more gray levels
reduced exact row/seed equality enough that W payloads grew despite the smaller
per-pixel deltas. The edge-aware `document` tone mode is retained as an
experiment for mixed documents, but it did not replace q10 as the timetable
default before the q12 physical comparison.

## Windows Round 2 Capture

The second Windows capture bundle was imported under:

```text
../captures-import/slj1660-windows-capture-round2/
```

It contains normal-output, cancel, attention-state, physical-resume, and
post-clear captures plus extracted endpoint-0x08 print raw streams.

Key findings:

- Windows normal timetable output is deterministic before and after recovery:
  `01-windows-normal-timetable.raw` is 593,427 bytes and
  `05-windows-normal-after-clear.raw` is 593,567 bytes.
- Windows does not beat the local q12 stream on raw byte size for this sample.
  The Windows W payload is about 546-549 KB, while local q12 is about 354 KB.
- The perceived Windows speed advantage is therefore unlikely to come from a
  smaller final PCL3GUI stream for this timetable. It is more likely in driver
  pacing, host buffering, render pipeline overlap, or printer interaction.
- Windows did not use printer-class `GET_PORT_STATUS` or `SOFT_RESET` in these
  captures. It polled status through LEDM REST on endpoint `0x0a` / `0x8b`.
- The software equivalent of the physical resume button is:
  `PUT /DevMgmt/ProductStatusDyn.xml` with `trayEmptyOrOpen` and
  `pressOK`. The reusable request is now stored as
  `fixtures/confirm/tray-empty-or-open-resume.http`.
- Windows cancel uses `PUT /Jobs/JobList/{id}` with `JobState=Canceled`; the
  job id is dynamic, so this should not be replayed as a static fixture without
  first discovering the active job URL.

Round-2 normal-output comparison:

| raw | size bytes | W rows | nonzero W | zero W | Y skipped | W payload bytes | max W payload | payload vs RGB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Windows `01-windows-normal-timetable.raw` | 593,427 | 5,526 | 2,987 | 2,539 | 556 | 546,555 | 2,814 | 0.61% |
| Windows `05-windows-normal-after-clear.raw` | 593,567 | 5,535 | 2,992 | 2,543 | 556 | 548,855 | 2,814 | 0.61% |
| macOS q12 `/tmp/slj1660-inu-timetable-a4-default-q12.raw` | 401,091 | 6,205 | 3,142 | 3,063 | 682 | 353,768 | 1,205 | 0.35% |

For the timetable PNG specifically, blank-band skipping is not the main
remaining win in the high-quality grayscale path: the generated gray raw already
uses `*b...Y` for 681 rows and has no decoded blank `W` rows in the successful
sample. The bigger optimization target is quantizing/halftoning continuous gray
before Mode10 compression, so text and pale boxes stay readable while payloads
look more like the small thresholded stream.

## Safe Replay Candidate

`fixtures/captured-confirmed/blank-page.raw` is the lowest-ink replay candidate because it
contains only setup and page finalization, but it can still feed paper. Use
explicit user confirmation before sending any captured raw stream to hardware.

In the first observed replay session, `blank-page.raw` did not visibly eject a
page. Treat it as a protocol/setup probe, not as a reliable physical print
fixture.

## macOS Replay Verification

On 2026-06-17, raw replay was verified from this macOS host against the directly
connected SL-J1660:

```text
cargo run -- send-raw fixtures/captured-confirmed/blank-page.raw --serial <your-serial>
Sent 12164 bytes in 1 USB bulk transfer chunk(s) to endpoint 0x08.
Claimed interface 1 on 04e8:3954.

cargo run -- send-raw fixtures/captured-confirmed/text-only.raw --serial <your-serial>
Sent 17176 bytes in 2 USB bulk transfer chunk(s) to endpoint 0x08.
Claimed interface 1 on 04e8:3954.
```

This verifies the user-space USB replay path, interface selection, and endpoint
selection. It does not by itself verify the visual page output; that must be
confirmed at the physical printer.

## macOS REST Status Probe

The printer also responded over the REST/status USB channel from macOS:

```text
cargo run --example rest_status
wrote 63 byte REST request to endpoint 0x0a
HTTP/1.1 200 OK
Server: HP HTTP Server; Samsung  SL-J1660 Series - W7V17A; Serial Number: <your-serial>; ...
```

Observed status in `ProductStatusDyn.xml`:

- `cartridgeVeryLow`
- `singleCartridgeMode`
- black cartridge alert with user action `acknowledgeConsumableState`
- color cartridge / single-cartridge-mode alert with user action `pressOK`

This confirms the macOS USB writes are reaching a live device. If raw replay
does not physically print, first clear or acknowledge the printer's cartridge
state at the device before assuming the PCL3GUI stream is invalid.

## Printer-Class Status Probe

The USB printer-class control interface also responds:

```text
cargo run --example printer_class
GET_DEVICE_ID index=0x0100:
MFG:Samsung ;MDL:SL-J1660 Series;CMD:PCL3GUI,PJL,Automatic,DW-PCL,DESKJET,DYN;...
GET_PORT_STATUS index=0x0001: 0x10 (error, select, paper-present)
GET_PORT_STATUS index=0x0100: 0x10 (error, select, paper-present)
```

`SOFT_RESET` succeeds, and after reset the Windows-captured `text-only.raw`
can be sent using the original Windows USB transfer boundary:

```text
cargo run --example printer_class -- --soft-reset
cargo run -- send-raw fixtures/captured-confirmed/text-only.raw \
  --serial <your-serial> \
  --chunk-size 16227 \
  --timeout-ms 30000
Sent 17176 bytes in 2 USB bulk transfer chunk(s) to endpoint 0x08.
```

A minimal PJL/PCL text job was also accepted by endpoint `0x08`, but the
printer-class status remained `0x10`, meaning the printer still reported an
error while selected and with paper present. This makes the current physical
non-printing behavior much more likely to be a cartridge/error-state gate than
an inability to reach the print endpoint.

Samsung's public support material says SL-J1660 can print with only the black
cartridge when black-and-white printing is selected in the driver. The captured
GDI jobs may still have been rendered through the driver's default color path,
so future Windows captures should explicitly set grayscale / black-only output
before capture.

## Confirmed Physical Print

On 2026-06-17, a physical text page printed from macOS using user-space libusb.
The successful sequence was:

```text
cargo run -- send-raw fixtures/confirm/lowink-continue.http \
  --serial <your-serial> \
  --interface 3 \
  --endpoint 0x0a \
  --chunk-size 836 \
  --timeout-ms 30000

cargo run -- send-raw fixtures/confirm/cartridge-refilled-ok.http \
  --serial <your-serial> \
  --interface 3 \
  --endpoint 0x0a \
  --chunk-size 869 \
  --timeout-ms 30000

cargo run -- send-raw fixtures/captured-confirmed/text-only.raw \
  --serial <your-serial> \
  --chunk-size 16227 \
  --timeout-ms 30000

cargo run -- send-raw fixtures/confirm/single-cartridge-ok.http \
  --serial <your-serial> \
  --interface 3 \
  --endpoint 0x0a \
  --chunk-size 1024 \
  --timeout-ms 30000
```

The equivalent helper script is:

```sh
scripts/replay-confirmed-text.sh --yes
```

This proves captured PCL3GUI raw replay over macOS USB can physically print on
this SL-J1660. It does not prove that the project can yet encode arbitrary PDF
pages into printer-ready PCL3GUI.

## Generated Text Verification

After the captured-stream replay succeeded, fresh text generation was tested.
The first attempts intentionally probed the raster assumptions:

- 1-bit packed raster rows printed as corrupted horizontal bar patterns.
- 4891-byte single-channel unencoded rows printed as corrupted horizontal bar
  patterns.
- standard PCL text did not provide a useful Korean text path.
- PCL method 9 compressed rows were accepted over USB but did not reproduce the
  expected output.

The successful generated page used a Python port of HPLIP's BSD-style
`Mode10.cpp` compressor with 4891-pixel-wide RGB rows. It printed the Korean
text `와쏘베쏘` from macOS on 2026-06-17.

Reproduction:

```sh
scripts/print-text-mode10.py "와쏘베쏘" \
  --preview /tmp/slj1660-preview.png \
  --out /tmp/slj1660-mode10-text.raw \
  --send
```

## USB Transfer Pacing

The q12 timetable RAW (`/tmp/slj1660-inu-timetable-a4-default-q12.raw`,
401,091 bytes) was sent repeatedly to the real SL-J1660 on 2026-06-19. All
rows below physically transferred successfully over interface `1`, endpoint
`0x08`.

| chunk size | timeout ms | delay ms | chunks | wall time |
| ---: | ---: | ---: | ---: | ---: |
| 4,096 | 120,000 | 10 | 98 | about 5.1s |
| 8,192 | 30,000 | 5 | 49 | 3.7392s |
| 16,384 | 30,000 | 2 | 25 | 3.5623s |
| 16,384 | 30,000 | 0 | 25 | 3.5266s |
| 32,768 | 30,000 | 0 | 13 | 3.5400s |
| 65,536 | 30,000 | 0 | 7 | 3.5297s |

The practical knee is `16384 / 30000 / 0`: larger chunks reduce write calls but
do not materially improve wall time for this 401 KB job. The intentionally tiny
`65536 / 20 / 0` timeout test failed after 33,280 bytes, so very low per-write
timeouts remain unsafe.
