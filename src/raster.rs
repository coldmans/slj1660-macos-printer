use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterOptions {
    pub dpi: u32,
    pub max_pages: u32,
    pub color_mode: RasterColorMode,
}

impl RasterOptions {
    pub fn black_and_white_300dpi() -> Self {
        Self {
            dpi: 300,
            max_pages: 1,
            color_mode: RasterColorMode::BlackAndWhite,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterColorMode {
    BlackAndWhite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterImage {
    pub width: u32,
    pub height: u32,
    pub dpi: u32,
    pub color_mode: RasterColorMode,
    /// Raw PBM P4 bitmap payload, 1 bit per pixel, MSB first in each byte.
    pub data: Vec<u8>,
}

pub trait Rasterizer {
    fn rasterize(&self, pdf_path: &Path, options: &RasterOptions) -> Result<RasterImage>;
}

#[derive(Debug, Clone)]
pub struct GhostscriptRasterizer {
    executable: PathBuf,
}

impl GhostscriptRasterizer {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    pub fn from_environment() -> Self {
        let executable = std::env::var_os("SLJ1660_GS")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("gs"));
        Self::new(executable)
    }
}

impl Rasterizer for GhostscriptRasterizer {
    fn rasterize(&self, pdf_path: &Path, options: &RasterOptions) -> Result<RasterImage> {
        if options.color_mode != RasterColorMode::BlackAndWhite {
            bail!("only black-and-white rasterization is implemented");
        }
        if options.max_pages != 1 {
            bail!("only single-page rasterization is implemented");
        }
        if !pdf_path.exists() {
            bail!("PDF does not exist: {}", pdf_path.display());
        }

        let output = tempfile::Builder::new()
            .prefix("slj1660-page-")
            .suffix(".pbm")
            .tempfile()
            .context("failed to create temporary PBM output")?;

        let output_path = output.path().to_path_buf();
        let status = Command::new(&self.executable)
            .arg("-q")
            .arg("-dSAFER")
            .arg("-dBATCH")
            .arg("-dNOPAUSE")
            .arg("-dFirstPage=1")
            .arg("-dLastPage=1")
            .arg("-sDEVICE=pbmraw")
            .arg(format!("-r{}", options.dpi))
            .arg(format!("-sOutputFile={}", output_path.display()))
            .arg(pdf_path)
            .status();

        match status {
            Ok(status) if status.success() => {}
            Ok(status) => {
                bail!(
                    "Ghostscript exited with status {status}. Verify the PDF is readable and try `gs` directly."
                );
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                bail!(
                    "Ghostscript executable not found. Install it with `brew install ghostscript`, \
                     or set SLJ1660_GS to a compatible `gs` executable."
                );
            }
            Err(error) => return Err(error).context("failed to run Ghostscript"),
        }

        let pbm_bytes = fs::read(&output_path).with_context(|| {
            format!(
                "failed to read Ghostscript output {}",
                output_path.display()
            )
        })?;
        parse_pbm_raw(&pbm_bytes, options.dpi, options.color_mode)
            .context("Ghostscript did not produce a valid raw PBM image")
    }
}

pub fn parse_pbm_raw(
    bytes: &[u8],
    dpi: u32,
    color_mode: RasterColorMode,
) -> Result<RasterImage, PbmParseError> {
    let mut parser = PbmParser::new(bytes);
    let magic = parser.next_token()?.ok_or(PbmParseError::MissingMagic)?;
    if magic != b"P4" {
        return Err(PbmParseError::UnsupportedMagic(
            String::from_utf8_lossy(magic).to_string(),
        ));
    }

    let width = parse_dimension(parser.next_token()?.ok_or(PbmParseError::MissingWidth)?)?;
    let height = parse_dimension(parser.next_token()?.ok_or(PbmParseError::MissingHeight)?)?;
    parser.consume_raster_separator()?;

    let row_bytes = width.div_ceil(8) as usize;
    let expected = row_bytes
        .checked_mul(height as usize)
        .ok_or(PbmParseError::ImageTooLarge)?;
    let payload = parser.remaining();

    if payload.len() < expected {
        return Err(PbmParseError::TruncatedPayload {
            expected,
            actual: payload.len(),
        });
    }

    Ok(RasterImage {
        width,
        height,
        dpi,
        color_mode,
        data: payload[..expected].to_vec(),
    })
}

fn parse_dimension(token: &[u8]) -> Result<u32, PbmParseError> {
    let text = std::str::from_utf8(token).map_err(|_| PbmParseError::InvalidDimension)?;
    let value = text
        .parse::<u32>()
        .map_err(|_| PbmParseError::InvalidDimension)?;
    if value == 0 {
        return Err(PbmParseError::InvalidDimension);
    }
    Ok(value)
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PbmParseError {
    #[error("missing PBM magic")]
    MissingMagic,
    #[error("unsupported PBM magic {0}; expected P4")]
    UnsupportedMagic(String),
    #[error("missing PBM width")]
    MissingWidth,
    #[error("missing PBM height")]
    MissingHeight,
    #[error("invalid PBM dimension")]
    InvalidDimension,
    #[error("PBM image is too large")]
    ImageTooLarge,
    #[error("PBM payload is truncated: expected {expected} bytes, got {actual}")]
    TruncatedPayload { expected: usize, actual: usize },
    #[error("invalid PBM header")]
    InvalidHeader,
}

struct PbmParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> PbmParser<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn next_token(&mut self) -> Result<Option<&'a [u8]>, PbmParseError> {
        self.skip_whitespace_and_comments();
        if self.pos >= self.bytes.len() {
            return Ok(None);
        }

        let start = self.pos;
        while self.pos < self.bytes.len() && !self.bytes[self.pos].is_ascii_whitespace() {
            if self.bytes[self.pos] == b'#' {
                return Err(PbmParseError::InvalidHeader);
            }
            self.pos += 1;
        }
        Ok(Some(&self.bytes[start..self.pos]))
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }

            if self.pos < self.bytes.len() && self.bytes[self.pos] == b'#' {
                while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }
                continue;
            }

            break;
        }
    }

    fn remaining(&self) -> &'a [u8] {
        self.bytes.get(self.pos..).unwrap_or_default()
    }

    fn consume_raster_separator(&mut self) -> Result<(), PbmParseError> {
        if self.pos >= self.bytes.len() || !self.bytes[self.pos].is_ascii_whitespace() {
            return Err(PbmParseError::InvalidHeader);
        }
        self.pos += 1;
        Ok(())
    }
}

pub fn ghostscript_setup_hint(error: &anyhow::Error) -> Option<&'static str> {
    let text = format!("{error:#}");
    text.contains("Ghostscript executable not found")
        .then_some("Install Ghostscript with `brew install ghostscript`.")
}

pub fn ensure_single_page_pdf_hint(path: &Path) -> Result<()> {
    if path.extension().and_then(|ext| ext.to_str()) != Some("pdf") {
        return Err(anyhow!("expected a PDF file: {}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_raw_pbm_with_comment() {
        let bytes = b"P4\n# created by test\n3 2\n\xa0\x40";
        let image = parse_pbm_raw(bytes, 300, RasterColorMode::BlackAndWhite).unwrap();
        assert_eq!(image.width, 3);
        assert_eq!(image.height, 2);
        assert_eq!(image.data, vec![0xa0, 0x40]);
    }

    #[test]
    fn rejects_truncated_pbm() {
        let error =
            parse_pbm_raw(b"P4\n9 2\n\x00", 300, RasterColorMode::BlackAndWhite).unwrap_err();
        assert_eq!(
            error,
            PbmParseError::TruncatedPayload {
                expected: 4,
                actual: 1
            }
        );
    }

    #[test]
    fn validates_pdf_extension_hint() {
        assert!(ensure_single_page_pdf_hint(Path::new("test.pdf")).is_ok());
        assert!(ensure_single_page_pdf_hint(Path::new("test.txt")).is_err());
    }
}
