# macOS Setup

## One-Click-ish Install

For another Mac, the easiest path is the release bundle:

1. Download the latest `slj1660-macos-printer-*-macos-arm64.tar.gz` from GitHub
   Releases.
2. Extract it.
3. Double-click `Install SL-J1660.command`.

The release bundle includes a prebuilt `bin/slj1660`, so it does not require
Rust/Cargo on the target Mac.

For a source checkout, use:

```sh
git clone https://github.com/coldmans/slj1660-macos-printer.git
cd slj1660-macos-printer
open "Install SL-J1660.command"
```

You can also double-click `Install SL-J1660.command` in Finder.

The installer copies the project into:

```text
~/Library/Application Support/slj1660-macos-printer
```

Then it installs/checks Homebrew runtime tools, creates a private Python venv
with Pillow, builds or uses the `slj1660` binary, registers the LaunchAgent,
and adds the CUPS queue:

```text
SL_J1660_Local -> ipp://127.0.0.1:8631/printers/slj1660
```

If something fails, run:

```sh
scripts/doctor-macos-printer.sh
```

Remove the LaunchAgent and queue with:

```sh
open "Uninstall SL-J1660.command"
```

## Requirements

- modern macOS on Apple Silicon
- Homebrew
- libusb
- Poppler for PDF rendering in the local IPP printer-app path
- Ghostscript for the older placeholder PDF rasterization path
- Python 3 with Pillow
- Rust and Cargo only when installing from source instead of a release bundle

Install runtime tools:

```sh
brew install libusb ghostscript poppler
python3 -m pip install Pillow
```

Build:

```sh
cd <workspace>/slj1660-mac-driver
cargo build
```

List USB devices:

```sh
cargo run -- list-usb
```

## USB Access Caveats

This project uses user-space libusb access. It does not install kernel
extensions and does not unload system drivers.

On macOS, `send-raw` may fail while claiming the USB interface if another
process or driver owns the device. If that happens:

1. Close printer utilities and any app that may be printing.
2. Remove or pause unrelated CUPS queues for this USB printer.
3. Unplug and reconnect the printer.
4. Retry `cargo run -- list-usb`, then `cargo run -- send-raw <capture.raw>`.

Do not install Apple HP Printer Drivers 5.1.1 for this MVP. Do not disable SIP
or add codeless kexts as part of this project.

## Ghostscript

`print-pdf --dry-run` shells out to `gs`:

```sh
cargo run -- print-pdf sample.pdf --dry-run
```

If Ghostscript lives somewhere unusual, set:

```sh
SLJ1660_GS=/path/to/gs cargo run -- print-pdf sample.pdf --dry-run
```

## Local IPP Printer App

The driver-like path is a local user-space IPP server. The recommended installer
is:

```sh
open "Install SL-J1660.command"
```

For development, start it manually:

```sh
cd <workspace>/slj1660-mac-driver
scripts/run-ipp-server.sh
```

Health check:

```sh
curl http://127.0.0.1:8631/health
```

Dry-run mode renders accepted jobs to raw files under `/tmp/slj1660-ipp-spool`
without sending USB bytes:

```sh
scripts/run-ipp-server.sh --dry-run
```

To register it as a macOS queue:

```sh
scripts/install-local-ipp-printer.sh
```

This lower-level script installs a LaunchAgent for:

```text
slj1660 serve-ipp --bind 127.0.0.1:8631
```

and then registers a CUPS queue:

```text
SL_J1660_Local -> ipp://127.0.0.1:8631/printers/slj1660
```

Print a PDF through that queue:

```sh
scripts/print-via-ipp.sh sample.pdf
```

If the queue is missing, `print-via-ipp.sh` installs it first. For a dry-run
install that renders jobs without sending USB bytes:

```sh
SLJ1660_IPP_DRY_RUN=1 scripts/print-via-ipp.sh sample.pdf
```

Remove both pieces with:

```sh
scripts/install-local-ipp-printer.sh --remove
```

The higher-level `Install SL-J1660.command` wraps this script and also handles
dependency setup, private Python venv creation, copying the project to
Application Support, and opening macOS printer settings after install.

Logs are written to:

```text
~/Library/Logs/com.local.slj1660.printerapp.out.log
~/Library/Logs/com.local.slj1660.printerapp.err.log
```

If the printer appears in macOS Settings but jobs fail with "unable to add
document to job", check the LaunchAgent runtime path. The daemon needs Homebrew
tools such as `pdftoppm` and `pdfinfo`, so the installer writes:

```text
PATH=/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin
```

The daemon also needs a Python executable with Pillow installed. The installer
detects one and writes it as `SLJ1660_PYTHON`. Override it manually when needed:

```sh
SLJ1660_PYTHON=/path/to/python3 scripts/install-local-ipp-printer.sh
```

The installer uses the fastest locally validated USB pacing for generated PDF
jobs:

```text
SLJ1660_PRINT_CHUNK_SIZE=16384
SLJ1660_TIMEOUT_MS=30000
SLJ1660_CHUNK_DELAY_MS=0
```

The older conservative setting was `4096 / 120000 / 10`. It is still useful as
a fallback for unusually large or fragile jobs, but the timetable q12 RAW
validated `16384 / 30000 / 0` without mid-page bulk write timeouts.

During normal local IPP printing, the daemon has an automatic resume watchdog:
if a raw page transfer stays active long enough to look like the printer is
waiting for its blinking resume button, it sends the Windows-captured LEDM
resume request itself.

If a job fails or is interrupted outside the active IPP flow and the printer is
left in a feed-attention state, the Windows round-2 captures showed that the
physical resume button has a software equivalent over LEDM:

```sh
scripts/resume-feed-attention.sh
```

This sends `fixtures/confirm/tray-empty-or-open-resume.http` to USB interface
`3`, endpoint `0x0a`. It can resume a buffered job and feed paper, so use it
only when that is intended.

Re-run `scripts/install-local-ipp-printer.sh` after changing dependencies or
moving Homebrew.

This is a working beta for the tested black-and-white SL-J1660 path. The local
IPP endpoint accepts PDF jobs and routes them through the Mode10 path. If macOS
sends a non-PDF document format, the server rejects that job instead of
pretending it can print it. The server supports direct `Print-Job` and split
`Create-Job` / `Send-Document` local flows.

## Release Bundle

Build a double-click install bundle for GitHub Releases:

```sh
scripts/build-macos-bundle.sh
```

The archive is written under `dist/`. It includes a prebuilt `bin/slj1660`, the
installer commands, scripts, license notices, and the LEDM confirmation fixtures.
On another Apple Silicon Mac, extract it and double-click
`Install SL-J1660.command`.
