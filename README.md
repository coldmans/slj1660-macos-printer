# SL-J1660 macOS Printer

Samsung SL-J1660을 최신 macOS에서 직접 출력하기 위한 **사용자 공간
print-only 프린터 앱**입니다. 커널 확장, 오래된 Apple HP Printer Drivers
패키지, 제조사 macOS 드라이버에 의존하지 않습니다.

현재 상태는 **SL-J1660 흑백 PDF/PNG 출력용 working beta**입니다. 컬러,
스캔, 제조사급 잉크 UI는 아직 없지만, macOS 로컬 프린터 큐를 통해 실제
문서 출력까지 검증했습니다.

- USB VID/PID `04e8:3954` Samsung SL-J1660 탐지
- PCL3GUI / Mode10 계열 RAW 스트림 분석과 전송
- PDF를 600dpi로 렌더링한 뒤 12단계 grayscale quantization으로 Mode10 인코딩
- macOS 로컬 IPP printer-app daemon 제공
- `SL_J1660_Local` CUPS 큐를 통해 일반 PDF 출력
- low-ink, single-cartridge, feed-attention 상태에 대한 LEDM 확인/재개 요청
- 필요할 때 켤 수 있는 LEDM resume watchdog

아직 scanner, ink/status UI, 컬러 관리, 완전한 제조사급 드라이버 기능은 없습니다.
목표는 “맥에서 SL-J1660으로 흑백 문서를 편하게 뽑는 실사용 경로”입니다.

## 빠른 설치

비개발자/다른 Mac에서는 GitHub Releases에서 최신
`slj1660-macos-printer-*-macos-arm64.tar.gz`를 받은 뒤 압축을 풀고
`Install SL-J1660.command`를 더블클릭하는 방식을 권장합니다.

소스에서 바로 설치하려면:

```sh
git clone https://github.com/coldmans/slj1660-macos-printer.git
cd slj1660-macos-printer
open "Install SL-J1660.command"
```

또는 Finder에서 `Install SL-J1660.command`를 더블클릭하세요. Release 번들은
미리 빌드된 `bin/slj1660`을 포함하므로 Rust/Cargo가 없어도 설치할 수 있습니다.

설치기는 다음 작업을 한 번에 처리합니다.

- 프로젝트를 `~/Library/Application Support/slj1660-macos-printer`로 복사
- Homebrew 의존성 `libusb`, `poppler`, `ghostscript` 확인/설치
- 전용 Python venv와 `Pillow` 준비
- `slj1660` release 바이너리 빌드 또는 번들 바이너리 사용
- LaunchAgent 등록
- macOS CUPS 프린터 큐 `SL_J1660_Local` 추가

문제가 있으면:

```sh
scripts/doctor-macos-printer.sh
```

## macOS 프린터 큐 설치

개발 중 직접 설치하려면 아래 스크립트를 실행해도 됩니다.

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
open "Uninstall SL-J1660.command"
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

LEDM/status 요청은 interface `3`, endpoint `0x0a`를 사용합니다. 수동 복구가
필요하면 다음을 실행할 수 있습니다.

```sh
scripts/resume-feed-attention.sh
```

출력 중 급지 resume 요청을 자동으로 보내는 watchdog은 안전을 위해 기본 OFF입니다.
필요할 때만 `SLJ1660_AUTO_RESUME=1 scripts/install-local-ipp-printer.sh`로 켜세요.

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
- Mode10 compressor/decoder 일부: HP HPLIP `Mode10.cpp`에서 파생된 BSD-style 코드

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

GitHub Release용 더블클릭 설치 번들 생성:

```sh
scripts/build-macos-bundle.sh
```

문서:

- [docs/architecture.md](docs/architecture.md)
- [docs/macos-setup.md](docs/macos-setup.md)
- [docs/capture-guide.md](docs/capture-guide.md)
- [docs/protocol-notes.md](docs/protocol-notes.md)
- [docs/capture-analysis.md](docs/capture-analysis.md)

---

# English Summary

This is a working-beta user-space, print-only macOS printer app for monochrome
Samsung SL-J1660 document printing. It discovers the USB printer, renders PDF
pages, encodes them as PCL3GUI / Mode10-style raster rows, and sends the
resulting stream through libusb. It also exposes a local IPP endpoint so macOS
can print PDFs through a normal CUPS queue.

It is not a full vendor driver. There is no scanning support, no full ink/status
UI, no color-management pipeline, and no kernel extension. The practical goal is
simple: make the SL-J1660 usable for document printing on modern macOS.

Quick install:

For non-developer installs, download the latest
`slj1660-macos-printer-*-macos-arm64.tar.gz` from GitHub Releases, extract it,
and double-click `Install SL-J1660.command`.

From source:

```sh
git clone https://github.com/coldmans/slj1660-macos-printer.git
cd slj1660-macos-printer
open "Install SL-J1660.command"
```

Then print a PDF:

```sh
scripts/print-via-ipp.sh sample.pdf
```

If more than one matching printer is connected, set:

```sh
export SLJ1660_SERIAL="<your-serial>"
```
