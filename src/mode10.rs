use anyhow::{anyhow, bail, Context, Result};

const WHITE_PIXEL: u32 = 0x00ff_fffe;
const E_RLE: u8 = 0x80;
const EE_NEW: u8 = 0x00;
const EE_W: u8 = 0x20;
const EE_NE: u8 = 0x40;
const EE_CACHED: u8 = 0x60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedMode10Row {
    pub pixels: Vec<u32>,
    pub blank: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mode10State {
    width_pixels: usize,
    seed: Vec<u32>,
}

impl Mode10State {
    pub fn new(width_pixels: usize) -> Result<Self> {
        if width_pixels == 0 {
            bail!("Mode10 width must be greater than zero");
        }

        Ok(Self {
            width_pixels,
            seed: vec![WHITE_PIXEL; width_pixels],
        })
    }

    pub fn decode_row(&mut self, payload: &[u8]) -> Result<DecodedMode10Row> {
        let pixels = if payload.is_empty() {
            self.seed.clone()
        } else {
            decode_row_payload(payload, &self.seed, self.width_pixels)?
        };
        let blank = is_blank_row(&pixels);
        self.seed.clone_from(&pixels);

        Ok(DecodedMode10Row { pixels, blank })
    }
}

fn decode_row_payload(payload: &[u8], seed: &[u32], width_pixels: usize) -> Result<Vec<u32>> {
    let mut row = seed.to_vec();
    let mut reader = PayloadReader::new(payload);
    let mut pixel_index = 0usize;
    let mut cached_color = WHITE_PIXEL;

    while pixel_index < width_pixels && !reader.is_empty() {
        let command = reader.read_u8().context("missing Mode10 command byte")?;
        let seed_copy_count = seed_copy_count(command, &mut reader)?;
        pixel_index = pixel_index
            .checked_add(seed_copy_count)
            .ok_or_else(|| anyhow!("Mode10 seed-copy count overflow"))?;
        if pixel_index > width_pixels {
            bail!("Mode10 seed-copy moved past row width");
        }
        if pixel_index == width_pixels {
            continue;
        }

        let pixel_source = command & 0x60;
        if command & E_RLE == E_RLE {
            let replacement_count = rle_replacement_count(command, &mut reader)?;
            let pixel = source_or_new_pixel(
                pixel_source,
                &row,
                seed,
                pixel_index,
                cached_color,
                &mut reader,
            )?;
            if pixel_source == EE_NEW {
                cached_color = pixel;
            }
            let end = checked_row_end(pixel_index, replacement_count, width_pixels)?;
            row[pixel_index..end].fill(pixel);
            pixel_index = end;
        } else {
            decode_literal_run(
                command,
                pixel_source,
                &mut row,
                seed,
                &mut pixel_index,
                &mut cached_color,
                &mut reader,
            )?;
        }
    }

    Ok(row)
}

fn seed_copy_count(command: u8, reader: &mut PayloadReader<'_>) -> Result<usize> {
    let base = usize::from((command >> 3) & 0x03);
    if base < 3 {
        return Ok(base);
    }
    Ok(3 + reader.read_vli().context("invalid Mode10 seed-copy VLI")?)
}

fn rle_replacement_count(command: u8, reader: &mut PayloadReader<'_>) -> Result<usize> {
    let normalized = usize::from(command & 0x07);
    if normalized < 7 {
        return Ok(normalized + 2);
    }
    Ok(9 + reader.read_vli().context("invalid Mode10 RLE-count VLI")?)
}

fn checked_row_end(start: usize, count: usize, width_pixels: usize) -> Result<usize> {
    let end = start
        .checked_add(count)
        .ok_or_else(|| anyhow!("Mode10 replacement count overflow"))?;
    if end > width_pixels {
        bail!("Mode10 replacement count moved past row width");
    }
    Ok(end)
}

fn decode_literal_run(
    command: u8,
    pixel_source: u8,
    row: &mut [u32],
    seed: &[u32],
    pixel_index: &mut usize,
    cached_color: &mut u32,
    reader: &mut PayloadReader<'_>,
) -> Result<()> {
    let low_count = usize::from(command & 0x07);
    let mut total = low_count + 1;
    let has_extension = low_count == 7;
    let mut position_in_run = 1usize;
    let mut should_update_cache = pixel_source == EE_NEW;

    while position_in_run <= total {
        if *pixel_index >= row.len() {
            bail!("Mode10 literal run moved past row width");
        }

        let pixel = if position_in_run == 1 && pixel_source != EE_NEW {
            source_pixel(pixel_source, row, seed, *pixel_index, *cached_color)?
        } else {
            read_pixel(reader, seed[*pixel_index])?
        };
        if should_update_cache {
            *cached_color = pixel;
            should_update_cache = false;
        }
        row[*pixel_index] = pixel;
        *pixel_index += 1;

        if has_extension && position_in_run >= 8 && (position_in_run - 8).is_multiple_of(255) {
            let extra = usize::from(
                reader
                    .read_u8()
                    .context("missing Mode10 literal-count extension byte")?,
            );
            total = position_in_run
                .checked_add(extra)
                .ok_or_else(|| anyhow!("Mode10 literal-count extension overflow"))?;
        }

        position_in_run += 1;
    }

    Ok(())
}

fn source_or_new_pixel(
    pixel_source: u8,
    row: &[u32],
    seed: &[u32],
    pixel_index: usize,
    cached_color: u32,
    reader: &mut PayloadReader<'_>,
) -> Result<u32> {
    if pixel_source == EE_NEW {
        read_pixel(reader, seed[pixel_index])
    } else {
        source_pixel(pixel_source, row, seed, pixel_index, cached_color)
    }
}

fn source_pixel(
    pixel_source: u8,
    row: &[u32],
    seed: &[u32],
    pixel_index: usize,
    cached_color: u32,
) -> Result<u32> {
    match pixel_source {
        EE_W => pixel_index
            .checked_sub(1)
            .and_then(|index| row.get(index).copied())
            .ok_or_else(|| anyhow!("Mode10 west pixel source is out of bounds")),
        EE_NE => seed
            .get(pixel_index + 1)
            .copied()
            .ok_or_else(|| anyhow!("Mode10 north-east pixel source is out of bounds")),
        EE_CACHED => Ok(cached_color),
        _ => bail!("unknown Mode10 pixel source 0x{pixel_source:02x}"),
    }
}

fn read_pixel(reader: &mut PayloadReader<'_>, upper_pixel: u32) -> Result<u32> {
    let first = reader.read_u8().context("missing Mode10 pixel byte")?;
    if first & 0x80 == 0x80 {
        let second = reader
            .read_u8()
            .context("missing Mode10 short-delta byte")?;
        let packed = u16::from(first) << 8 | u16::from(second);
        return Ok(decode_short_delta(packed, upper_pixel));
    }

    let second = reader
        .read_u8()
        .context("missing Mode10 literal green byte")?;
    let third = reader
        .read_u8()
        .context("missing Mode10 literal blue byte")?;
    Ok(
        ((u32::from(first) << 17) | (u32::from(second) << 9) | (u32::from(third) << 1))
            & WHITE_PIXEL,
    )
}

fn decode_short_delta(packed: u16, upper_pixel: u32) -> u32 {
    let dr = sign_extend_5((packed >> 10) & 0x1f);
    let dg = sign_extend_5((packed >> 5) & 0x1f);
    let db = sign_extend_5(packed & 0x1f) * 2;

    let r = clamp_channel(red(upper_pixel) + dr);
    let g = clamp_channel(green(upper_pixel) + dg);
    let b = clamp_channel(blue(upper_pixel) + db);
    ((r as u32) << 16 | (g as u32) << 8 | b as u32) & WHITE_PIXEL
}

fn sign_extend_5(value: u16) -> i32 {
    let value = i32::from(value);
    if value & 0x10 == 0x10 {
        value - 32
    } else {
        value
    }
}

fn clamp_channel(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

fn red(pixel: u32) -> i32 {
    ((pixel >> 16) & 0xff) as i32
}

fn green(pixel: u32) -> i32 {
    ((pixel >> 8) & 0xff) as i32
}

fn blue(pixel: u32) -> i32 {
    (pixel & 0xff) as i32
}

pub fn is_blank_row(row: &[u32]) -> bool {
    row.iter().all(|pixel| {
        let r = red(*pixel);
        let g = green(*pixel);
        let b = blue(*pixel);
        r >= 250 && g >= 250 && b >= 250
    })
}

struct PayloadReader<'a> {
    payload: &'a [u8],
    offset: usize,
}

impl<'a> PayloadReader<'a> {
    fn new(payload: &'a [u8]) -> Self {
        Self { payload, offset: 0 }
    }

    fn is_empty(&self) -> bool {
        self.offset >= self.payload.len()
    }

    fn read_u8(&mut self) -> Option<u8> {
        let byte = self.payload.get(self.offset).copied()?;
        self.offset += 1;
        Some(byte)
    }

    fn read_vli(&mut self) -> Option<usize> {
        let mut total = 0usize;
        loop {
            let value = usize::from(self.read_u8()?);
            total = total.checked_add(value)?;
            if value < 255 {
                return Some(total);
            }

            if self.payload.get(self.offset) == Some(&0) {
                self.offset += 1;
                if self.is_empty() {
                    return Some(total);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_literal_black_rle_and_zero_repeat() {
        let mut state = Mode10State::new(2).unwrap();
        let first = state.decode_row(&[0x80, 0x00, 0x00, 0x00]).unwrap();
        assert!(!first.blank);
        assert_eq!(first.pixels, vec![0, 0]);

        let repeated = state.decode_row(&[]).unwrap();
        assert!(!repeated.blank);
        assert_eq!(repeated.pixels, vec![0, 0]);
    }

    #[test]
    fn decodes_zero_repeat_from_white_seed_as_blank() {
        let mut state = Mode10State::new(2).unwrap();
        let row = state.decode_row(&[]).unwrap();
        assert!(row.blank);
    }

    #[test]
    fn decodes_uncompressed_white_literal_as_blank() {
        let mut state = Mode10State::new(1).unwrap();
        let row = state.decode_row(&[0x00, 0x7f, 0xff, 0xff]).unwrap();
        assert!(row.blank);
    }
}
