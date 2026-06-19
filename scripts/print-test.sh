#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

out="${1:-fixtures/generated-test.pdf}"

python3 - "$out" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
path.parent.mkdir(parents=True, exist_ok=True)

objects = [
    b"1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n",
    b"2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n",
    b"3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> endobj\n",
    b"4 0 obj << /Length 44 >> stream\nBT /F1 24 Tf 72 180 Td (SL-J1660 test) Tj ET\nendstream endobj\n",
    b"5 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj\n",
]

data = bytearray(b"%PDF-1.4\n")
offsets = [0]
for obj in objects:
    offsets.append(len(data))
    data.extend(obj)

xref_start = len(data)
data.extend(f"xref\n0 {len(objects) + 1}\n".encode())
data.extend(b"0000000000 65535 f \n")
for offset in offsets[1:]:
    data.extend(f"{offset:010d} 00000 n \n".encode())
data.extend(f"trailer << /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref_start}\n%%EOF\n".encode())

path.write_bytes(data)
print(path)
PY

cargo run -- print-pdf "$out" --dry-run
