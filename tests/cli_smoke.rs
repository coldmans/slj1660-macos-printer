use std::fs;
use std::io::Write;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn help_command_runs() {
    Command::cargo_bin("slj1660")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("list-usb"))
        .stdout(predicate::str::contains("send-raw"))
        .stdout(predicate::str::contains("analyze-raw"))
        .stdout(predicate::str::contains("print-pdf"))
        .stdout(predicate::str::contains("serve-ipp"));
}

#[test]
fn inspect_raw_reports_size_hex_and_markers() {
    let temp = TempDir::new().unwrap();
    let raw = temp.path().join("sample.raw");
    fs::write(&raw, b"\x1b%-12345X@PJL\n\x1bE").unwrap();

    Command::cargo_bin("slj1660")
        .unwrap()
        .args(["inspect-raw", raw.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("size_bytes: 16"))
        .stdout(predicate::str::contains("1b 25 2d 31 32 33"))
        .stdout(predicate::str::contains("PJL command"));
}

#[test]
fn print_pdf_dry_run_uses_fake_ghostscript() {
    let temp = TempDir::new().unwrap();
    let fake_gs = write_fake_ghostscript(temp.path());
    let pdf = temp.path().join("sample.pdf");
    fs::write(&pdf, b"%PDF-1.4\n% fake test fixture\n").unwrap();

    Command::cargo_bin("slj1660")
        .unwrap()
        .env("SLJ1660_GS", &fake_gs)
        .args(["print-pdf", pdf.to_str().unwrap(), "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run OK"))
        .stdout(predicate::str::contains("No USB transfer was attempted"));
}

#[test]
fn print_pdf_output_raw_writes_placeholder() {
    let temp = TempDir::new().unwrap();
    let fake_gs = write_fake_ghostscript(temp.path());
    let pdf = temp.path().join("sample.pdf");
    let out = temp.path().join("out.raw");
    fs::write(&pdf, b"%PDF-1.4\n% fake test fixture\n").unwrap();

    Command::cargo_bin("slj1660")
        .unwrap()
        .env("SLJ1660_GS", &fake_gs)
        .args([
            "print-pdf",
            pdf.to_str().unwrap(),
            "--output-raw",
            out.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "warning: printer-stream encoder is incomplete",
        ))
        .stdout(predicate::str::contains(
            "Do not send this placeholder stream",
        ));

    let bytes = fs::read(out).unwrap();
    assert!(bytes.starts_with(b"SLJ1660_PLACEHOLDER_V1\n"));
}

#[cfg(unix)]
fn write_fake_ghostscript(dir: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("fake-gs.sh");
    let mut file = fs::File::create(&path).unwrap();
    file.write_all(
        br#"#!/bin/sh
set -eu
out=""
for arg in "$@"; do
  case "$arg" in
    -sOutputFile=*) out="${arg#-sOutputFile=}" ;;
  esac
done
if [ -z "$out" ]; then
  echo "missing output" >&2
  exit 2
fi
printf 'P4\n2 2\n\200@' > "$out"
"#,
    )
    .unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    path
}

#[cfg(not(unix))]
fn write_fake_ghostscript(_dir: &Path) -> std::path::PathBuf {
    panic!("fake Ghostscript helper is only implemented for Unix-like test hosts");
}
