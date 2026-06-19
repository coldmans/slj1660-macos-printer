# Capturing Known-Good Raw Streams

The next protocol milestone depends on real raw streams from a working driver.
Do not invent the printer language from names alone.

## Windows Capture Workflow

1. Install the official Windows Samsung SL-J1660 driver on a Windows machine.
2. Connect the printer over USB and confirm a normal Windows test page prints.
3. Print a simple one-page black-and-white test pattern.
4. If the driver shows a low-ink or cartridge prompt, press the same confirmation
   path that makes the page print, and capture that traffic too.
5. Capture the printer payload using a tool such as USBPcap with Wireshark, or
   recover the spool output if the driver leaves a usable raw spool file.
6. Extract the payload bytes sent from host to printer.
7. Separate the print stream from status/control traffic:
   - endpoint `0x08` carried the observed PCL3GUI/PJL print stream
   - endpoint `0x0a` carried observed LEDM HTTP status/confirmation requests
8. Save each captured payload under `fixtures/`, for example:
   - `fixtures/blank-page.raw`
   - `fixtures/text-only.raw`
   - `fixtures/black-rectangle.raw`
   - `fixtures/horizontal-lines.raw`
   - `fixtures/checkerboard.raw`
9. Inspect each capture:

   ```sh
   cargo run -- inspect-raw fixtures/text-only.raw
   ```

10. Replay only a known-good capture:

   ```sh
   cargo run -- send-raw fixtures/text-only.raw
   ```

## Capture Set

Use multiple small patterns so the protocol can be inferred safely:

- blank page
- text-only page
- solid black rectangle
- simple horizontal lines
- simple checkerboard

Compare captures to identify job headers, page boundaries, resolution fields,
raster row layout, compression, checksums, and end-of-job commands.

## Safety Notes

Keep captures small and single-page. Do not repeatedly replay unknown or partial
captures. If a replay causes the printer to blink indefinitely or feed paper
incorrectly, power-cycle the printer and label that fixture as unsafe.

The first confirmed macOS print required acknowledging cartridge state before
and after the raw print stream. Use `scripts/replay-confirmed-text.sh --dry-run`
to see the exact command sequence before sending it to hardware.
