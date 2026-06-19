# SL-J1660 macOS User-Space Printer

Samsung SL-J1660을 최신 macOS에서 직접 출력하기 위한 **사용자 공간
print-only 프린터 앱/드라이버 실험체**입니다. 커널 확장, 오래된 Apple HP
Printer Drivers 패키지, 제조사 macOS 드라이버에 의존하지 않습니다.

현재 상태는 MVP지만 실제 출력까지 검증했습니다.

- USB VID/PID `04e8:3954` Samsung SL-J1660 탐지
- PCL3GUI / Mode10 계열 RAW 스트림 분석과 전송
- PDF를 600dpi로 렌더링한 뒤 12단계 grayscale quantization으로 Mode10 인코딩
- macOS 로컬 IPP printer-app daemon 제공
- `SL_J1660_Local` CUPS 큐를 통해 일반 PDF 출력
- low-ink, single-cartridge, feed-attention 상태에 대한 LEDM 확인/재개 요청
- 출력 중 멈춤처럼 보이는 상태에서 자동 LEDM resume watchdog

아직 scanner, ink/status UI, 컬러 관리, 완전한 제조사급 드라이버 기능은 없습니다.
목표는 “맥에서 SL-J1660으로 문서를 뽑을 수 있는 최소 실사용 경로”입니다.

## 빠른 시작

```sh
brew install libusb ghostscript poppler
python3 -m pip install Pillow
cargo build
cargo run -- list-usb
```

프린터가 하나만 연결되어 있으면 serial 지정 없이 VID/PID로 자동 매칭합니다.
여러 대가 연결되어 있으면 환경변수나 옵션으로 serial을 지정하세요.

```sh
export SLJ1660_SERIAL="<your-serial>"
```

## macOS 프린터 큐 설치

```sh
scripts/install-local-ipp-printer.sh
```

설치 후 macOS 프린터 목록에 로컬 큐가 생깁니다.

```text
SL_J1660_Local -> ipp://127.0.0.1:8631/printers/slj1660
```

PDF를 직접 보낼 수도 있습니다.

```sh
scripts/print-via-ipp.sh sample.pdf
```

제거:

```sh
scripts/install-local-ipp-printer.sh --remove
```

## 출력 파이프라인

```text
macOS 앱 / Preview / lp
  -> CUPS queue
  -> local IPP daemon: slj1660 serve-ipp
  -> PDF page render via Poppler pdftoppm
  -> grayscale q12 preprocessing
  -> PCL3GUI / Mode10 row compression
  -> libusb bulk OUT endpoint 0x08
```

LEDM/status 요청은 interface `3`, endpoint `0x0a`를 사용합니다. 출력 중 RAW
전송이 오래 지속되면 daemon이 Windows 캡처에서 확인한 resume 요청을 자동으로
보냅니다. 수동 복구가 필요하면 다음을 실행할 수 있습니다.

```sh
scripts/resume-feed-attention.sh
```

## 실험용 명령

USB 장치 확인:

```sh
cargo run -- list-usb
```

RAW 스트림 분석:

```sh
cargo run -- analyze-raw fixtures/captured-confirmed/text-only.raw
```

검증된 fixture replay:

```sh
scripts/replay-confirmed-text.sh --yes
```

새 텍스트 RAW 생성/출력:

```sh
scripts/print-text-mode10.py "와쏘베쏘" \
  --preview /tmp/slj1660-preview.png \
  --out /tmp/slj1660-text.raw \
  --send
```

PDF 첫 페이지 RAW 생성:

```sh
scripts/print-pdf-mode10.py sample.pdf \
  --preview /tmp/slj1660-pdf-preview.png \
  --out /tmp/slj1660-pdf.raw
```

`--send`를 붙이면 실제 용지와 잉크를 사용합니다.

## 배포/라이선스 주의

이 프로젝트에는 두 종류의 코드가 섞여 있습니다.

- Rust CLI/IPP/USB glue: MIT
- Python Mode10 compressor 일부: HP HPLIP `Mode10.cpp`에서 포팅한 BSD-style 코드

자세한 내용은 [LICENSE](LICENSE)와
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)를 보세요.

캡처 fixture는 연구/상호운용성 검증용입니다. 제조사 드라이버나 HPLIP GPL 코드를
그대로 포함하지 않으며, 원본 Windows USBPcap/pcapng/드라이버 추출물은 배포하지
않습니다.

## 개발

```sh
cargo fmt --check
cargo test
cargo build --release
python3 -m py_compile scripts/print-text-mode10.py scripts/print-pdf-mode10.py
```

문서:

- [docs/architecture.md](docs/architecture.md)
- [docs/macos-setup.md](docs/macos-setup.md)
- [docs/capture-guide.md](docs/capture-guide.md)
- [docs/protocol-notes.md](docs/protocol-notes.md)
- [docs/capture-analysis.md](docs/capture-analysis.md)

---

# English Summary

This is an experimental user-space, print-only macOS printer app for the
Samsung SL-J1660. It discovers the USB printer, renders PDF pages, encodes them
as PCL3GUI / Mode10-style raster rows, and sends the resulting stream through
libusb. It also exposes a local IPP endpoint so macOS can print PDFs through a
normal CUPS queue.

It is not a full vendor driver. There is no scanning support, no full ink/status
UI, no color-management pipeline, and no kernel extension. The practical goal is
simple: make the SL-J1660 usable for document printing on modern macOS.

Quick install:

```sh
brew install libusb ghostscript poppler
python3 -m pip install Pillow
scripts/install-local-ipp-printer.sh
```

Then print a PDF:

```sh
scripts/print-via-ipp.sh sample.pdf
```

If more than one matching printer is connected, set:

```sh
export SLJ1660_SERIAL="<your-serial>"
```
