# Architecture

The MVP now has two front doors over the same user-space print path:

```text
PDF/text/captured raw
  -> CLI, scripts, or local IPP printer app
  -> PDF/text raster path or raw stream reader
  -> PCL3GUI/Mode10 stream generation boundary
  -> USB print and LEDM/status transports
  -> Samsung SL-J1660
```

The original Rust CLI remains the safest inspection and replay tool. The
experimental `serve-ipp` command is a local printer-application wrapper: macOS
or CUPS sends a PDF job to `ipp://127.0.0.1:8631/printers/slj1660`, the daemon
spools it, invokes the current Mode10 PDF generator, and then sends generated
raw pages over USB.

## Modules

- `cli`: command-line parsing and command orchestration.
- `ipp`: minimal local IPP server, printer-attribute responses, PDF job spooling,
  and bridge into the Mode10 print pipeline.
- `raw`: raw stream inspection, hex preview, and lightweight marker detection.
- `raster`: Ghostscript-backed first-page PDF rasterization to PBM P4 at 300 dpi
  black-and-white.
- `encoder`: `PrinterStreamEncoder` trait and placeholder encoder. The current
  encoder is deliberately not printer-ready.
- `usb`: USB discovery, endpoint enumeration, transport abstraction, and libusb
  bulk OUT writes.
- `scripts/print-text-mode10.py`: experimental text renderer and Mode10
  compressor port.
- `scripts/print-pdf-mode10.py`: experimental PDF page renderer using Poppler
  `pdftoppm`, grayscale quantization or optional binarization/dithering,
  Mode10 compression, and captured PCL3GUI job wrapper reuse.

## Data Flow

`send-raw` reads bytes from disk and sends them directly through
`LibusbTransport`. It does not parse, mutate, or validate printer protocol
content beyond rejecting empty files.

`print-pdf --dry-run` rasterizes only the first page and reports the raster
dimensions. It does not touch USB.

`print-pdf --output-raw` rasterizes the first page and writes a placeholder
debug stream. That stream is useful for testing file flow and fixture handling,
but it is not a Samsung/HP/PCL3GUI printer stream.

`serve-ipp` accepts a small IPP operation set:

- `Get-Printer-Attributes`
- `Validate-Job`
- `Print-Job`
- `Create-Job`
- `Send-Document`
- `Cancel-Job`
- `Get-Jobs`
- `Get-Job-Attributes`

For `Print-Job` and `Send-Document`, the daemon currently expects PDF document
bytes after the IPP attribute section. It writes each accepted job under the
spool directory, uses `pdfinfo` to detect page count when available, invokes
`scripts/print-pdf-mode10.py` once per page, and sends each generated raw page
through the Rust USB transport. In `--dry-run` mode it stops after raw
generation.

The daemon keeps a small in-memory job store so `Create-Job`, `Send-Document`,
`Cancel-Job`, and `Get-Job-Attributes` can share the same job id during a local
session. It is not a durable spool database.

The IPP server is intentionally minimal. It is a local bridge for this one
printer, not a general-purpose IPP implementation, and Bonjour/mDNS discovery is
not implemented yet. The install script registers the queue explicitly with:

```text
ipp://127.0.0.1:8631/printers/slj1660
```

## User-Space Boundary

The transport uses libusb through Rust `rusb`. It claims a USB interface and
writes to a bulk OUT endpoint discovered on the target device. If multiple bulk
OUT endpoints exist, it prefers a standard USB printer-class interface
(`class 07`) and otherwise falls back to the first bulk OUT endpoint. The CLI
also exposes `--interface` and `--endpoint` overrides for hardware experiments.
It does not install a kernel extension, unload system drivers, modify CUPS
queues, or depend on deprecated Apple driver packages.

On macOS, claiming an interface can fail if another driver or process owns the
device. The MVP reports this as an operational blocker rather than attempting
privileged driver-detach workarounds.

The local printer-app daemon uses the same user-space boundary. It does not
install a kext, DriverKit extension, or deprecated Apple HP package. The optional
LaunchAgent keeps the user-space daemon running; the optional CUPS queue only
points macOS at the local IPP endpoint.

## Driverization Path

The intended driver-like architecture is:

```text
slj1660-core
  renderer        PDF/text -> raster rows
  mode10          raster rows -> compressed PCL3GUI payloads
  job             PJL/PCL3GUI headers, pages, margins, finishing
  transport       USB print endpoint + LEDM/status endpoint
  spooler         queue, retry, status, logs

frontends
  slj1660 CLI
  slj1660 serve-ipp local printer app
  optional Bonjour advertisement
```

The current implementation still keeps the working Mode10 encoder in Python
scripts. A later cleanup should move the Mode10 and job-wrapper generation into
Rust so the CLI, IPP daemon, tests, and any future UI all share one core library.
