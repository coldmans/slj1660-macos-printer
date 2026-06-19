use anyhow::Result;

use crate::raster::RasterImage;

pub trait PrinterStreamEncoder {
    fn encode(&self, raster: &RasterImage) -> Result<EncodedStream>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedStream {
    pub bytes: Vec<u8>,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct PlaceholderEncoder;

impl PrinterStreamEncoder for PlaceholderEncoder {
    fn encode(&self, raster: &RasterImage) -> Result<EncodedStream> {
        let warning = "printer-stream encoder is incomplete; output is a local debug placeholder, not a Samsung/HP/PCL3GUI printer-ready stream";
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"SLJ1660_PLACEHOLDER_V1\n");
        bytes.extend_from_slice(b"WARNING: NOT PRINTER READY\n");
        bytes.extend_from_slice(format!("dpi: {}\n", raster.dpi).as_bytes());
        bytes.extend_from_slice(format!("width: {}\n", raster.width).as_bytes());
        bytes.extend_from_slice(format!("height: {}\n", raster.height).as_bytes());
        bytes.extend_from_slice(b"format: pbm-p4-1bpp-msb\n");
        bytes.extend_from_slice(b"\n");
        bytes.extend_from_slice(&raster.data);

        Ok(EncodedStream {
            bytes,
            warning: Some(warning.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::raster::RasterColorMode;

    use super::*;

    #[test]
    fn placeholder_encoder_warns_and_marks_output() {
        let raster = RasterImage {
            width: 2,
            height: 2,
            dpi: 300,
            color_mode: RasterColorMode::BlackAndWhite,
            data: vec![0x80, 0x40],
        };

        let encoded = PlaceholderEncoder::default().encode(&raster).unwrap();
        assert!(encoded.warning.unwrap().contains("encoder is incomplete"));
        assert!(encoded.bytes.starts_with(b"SLJ1660_PLACEHOLDER_V1\n"));
        assert!(encoded.bytes.ends_with(&[0x80, 0x40]));
    }
}
