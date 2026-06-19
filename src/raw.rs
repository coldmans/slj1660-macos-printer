use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::mode10::Mode10State;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawInspection {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub first_bytes_hex: String,
    pub first_nonzero_offset: Option<usize>,
    pub pjl_start_offset: Option<usize>,
    pub pcl3gui_offset: Option<usize>,
    pub pcl_summary: Option<PclSummary>,
    pub markers: Vec<RawMarker>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawMarker {
    pub offset: usize,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PclSummary {
    pub start_offset: usize,
    pub command_count: usize,
    pub paper_size_code: Option<i32>,
    pub resolution_dpi: Option<i32>,
    pub raster_width_s: Option<i32>,
    pub raster_start_count: usize,
    pub raster_end_count: usize,
    pub b_w_count: usize,
    pub b_w_zero_count: usize,
    pub b_w_payload_bytes: usize,
    pub b_w_max_payload_bytes: usize,
    pub b_y_count: usize,
    pub b_y_sum: i64,
    pub b_v_values: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawMode10Analysis {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub compression_guess: String,
    pub width_pixels: Option<usize>,
    pub row_bytes_rgb: Option<usize>,
    pub w_rows: usize,
    pub nonzero_w_rows: usize,
    pub zero_w_rows: usize,
    pub w_payload_bytes: usize,
    pub max_w_payload_bytes: usize,
    pub y_commands: usize,
    pub y_skipped_rows: usize,
    pub decoded_blank_w_rows: usize,
    pub decoded_nonblank_w_rows: usize,
    pub zero_w_blank_rows: usize,
    pub zero_w_nonblank_rows: usize,
    pub longest_blank_w_run: usize,
    pub longest_nonblank_zero_w_run: usize,
    pub estimated_uncompressed_rgb_bytes: Option<u64>,
    pub payload_ratio_basis_points: Option<u64>,
    pub decode_errors: Vec<String>,
}

pub fn inspect_raw_file(path: &Path, leading_bytes: usize) -> Result<RawInspection> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(inspect_raw_bytes(path, &bytes, leading_bytes))
}

pub fn analyze_raw_file(path: &Path) -> Result<RawMode10Analysis> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(analyze_raw_bytes(path, &bytes))
}

pub fn inspect_raw_bytes(path: &Path, bytes: &[u8], leading_bytes: usize) -> RawInspection {
    let pcl3gui_offset = find_first(bytes, b"PCL3GUI");

    RawInspection {
        path: path.to_path_buf(),
        size_bytes: bytes.len() as u64,
        first_bytes_hex: format_first_bytes_hex(bytes, leading_bytes),
        first_nonzero_offset: bytes.iter().position(|byte| *byte != 0),
        pjl_start_offset: find_first(bytes, b"@PJL"),
        pcl3gui_offset,
        pcl_summary: pcl3gui_offset.and_then(|offset| summarize_pcl_after_pcl3gui(bytes, offset)),
        markers: detect_markers(bytes),
    }
}

pub fn analyze_raw_bytes(path: &Path, bytes: &[u8]) -> RawMode10Analysis {
    let inspection = inspect_raw_bytes(path, bytes, 0);
    let Some(summary) = inspection.pcl_summary.as_ref() else {
        return RawMode10Analysis {
            path: path.to_path_buf(),
            size_bytes: bytes.len() as u64,
            compression_guess: "no PCL3GUI raster section detected".to_string(),
            width_pixels: None,
            row_bytes_rgb: None,
            w_rows: 0,
            nonzero_w_rows: 0,
            zero_w_rows: 0,
            w_payload_bytes: 0,
            max_w_payload_bytes: 0,
            y_commands: 0,
            y_skipped_rows: 0,
            decoded_blank_w_rows: 0,
            decoded_nonblank_w_rows: 0,
            zero_w_blank_rows: 0,
            zero_w_nonblank_rows: 0,
            longest_blank_w_run: 0,
            longest_nonblank_zero_w_run: 0,
            estimated_uncompressed_rgb_bytes: None,
            payload_ratio_basis_points: None,
            decode_errors: Vec::new(),
        };
    };

    let width_pixels = summary
        .raster_width_s
        .and_then(|width| usize::try_from(width).ok())
        .filter(|width| *width > 0);
    let row_bytes_rgb = width_pixels.and_then(|width| width.checked_mul(3));
    let mut analysis = RawMode10Analysis {
        path: path.to_path_buf(),
        size_bytes: bytes.len() as u64,
        compression_guess: "PCL3GUI raster rows detected; compression method still unknown"
            .to_string(),
        width_pixels,
        row_bytes_rgb,
        w_rows: 0,
        nonzero_w_rows: 0,
        zero_w_rows: 0,
        w_payload_bytes: 0,
        max_w_payload_bytes: 0,
        y_commands: 0,
        y_skipped_rows: 0,
        decoded_blank_w_rows: 0,
        decoded_nonblank_w_rows: 0,
        zero_w_blank_rows: 0,
        zero_w_nonblank_rows: 0,
        longest_blank_w_run: 0,
        longest_nonblank_zero_w_run: 0,
        estimated_uncompressed_rgb_bytes: None,
        payload_ratio_basis_points: None,
        decode_errors: Vec::new(),
    };

    let mut mode10 = match width_pixels {
        Some(width) => Mode10State::new(width).ok(),
        None => None,
    };
    let mut cursor = summary.start_offset;
    let mut in_raster = false;
    let mut current_blank_run = 0usize;
    let mut current_nonblank_zero_run = 0usize;

    while let Some(relative) = bytes[cursor..].iter().position(|byte| *byte == 0x1b) {
        let offset = cursor + relative;
        let Some(command) = parse_pcl_command(bytes, offset) else {
            break;
        };

        if command.command.starts_with(b"\x1b*r") && command.final_byte == b'A' {
            in_raster = true;
        } else if command.command == b"\x1b*rC" {
            break;
        } else if in_raster && command.command.starts_with(b"\x1b*b") {
            analyze_raster_command(
                &mut analysis,
                &mut mode10,
                &command,
                &mut current_blank_run,
                &mut current_nonblank_zero_run,
            );
        }

        cursor = command.next_offset;
    }

    let total_rows = analysis
        .w_rows
        .checked_add(analysis.y_skipped_rows)
        .unwrap_or(usize::MAX);
    if let Some(row_bytes) = row_bytes_rgb {
        let estimated = (total_rows as u64).saturating_mul(row_bytes as u64);
        analysis.estimated_uncompressed_rgb_bytes = Some(estimated);
        if estimated > 0 {
            analysis.payload_ratio_basis_points =
                Some((analysis.w_payload_bytes as u64).saturating_mul(10_000) / estimated);
        }
    }
    analysis.compression_guess = compression_guess(&analysis);
    analysis
}

pub fn render_inspection(inspection: &RawInspection) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "path: {}", inspection.path.display());
    let _ = writeln!(output, "size_bytes: {}", inspection.size_bytes);
    let _ = writeln!(output, "first_bytes_hex: {}", inspection.first_bytes_hex);
    writeln_optional_usize(
        &mut output,
        "first_nonzero_offset",
        inspection.first_nonzero_offset,
    );
    writeln_optional_usize(&mut output, "pjl_start_offset", inspection.pjl_start_offset);
    writeln_optional_usize(&mut output, "pcl3gui_offset", inspection.pcl3gui_offset);

    if let Some(summary) = &inspection.pcl_summary {
        let _ = writeln!(output, "pcl_summary:");
        let _ = writeln!(output, "- start_offset: {}", summary.start_offset);
        let _ = writeln!(output, "- command_count: {}", summary.command_count);
        writeln_optional_i32(&mut output, "- paper_size_code", summary.paper_size_code);
        writeln_optional_i32(&mut output, "- resolution_dpi", summary.resolution_dpi);
        writeln_optional_i32(&mut output, "- raster_width_s", summary.raster_width_s);
        let _ = writeln!(
            output,
            "- raster_start_count: {}",
            summary.raster_start_count
        );
        let _ = writeln!(output, "- raster_end_count: {}", summary.raster_end_count);
        let _ = writeln!(output, "- b_w_count: {}", summary.b_w_count);
        let _ = writeln!(output, "- b_w_zero_count: {}", summary.b_w_zero_count);
        let _ = writeln!(output, "- b_w_payload_bytes: {}", summary.b_w_payload_bytes);
        let _ = writeln!(
            output,
            "- b_w_max_payload_bytes: {}",
            summary.b_w_max_payload_bytes
        );
        let _ = writeln!(output, "- b_y_count: {}", summary.b_y_count);
        let _ = writeln!(output, "- b_y_sum: {}", summary.b_y_sum);
        let _ = writeln!(output, "- b_v_values: {:?}", summary.b_v_values);
    } else {
        let _ = writeln!(output, "pcl_summary: none detected");
    }

    if inspection.markers.is_empty() {
        let _ = writeln!(output, "markers: none detected");
    } else {
        let _ = writeln!(output, "markers:");
        for marker in &inspection.markers {
            let _ = writeln!(output, "- offset {}: {}", marker.offset, marker.name);
        }
    }

    output
}

pub fn render_mode10_analysis(analysis: &RawMode10Analysis) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "path: {}", analysis.path.display());
    let _ = writeln!(output, "size_bytes: {}", analysis.size_bytes);
    let _ = writeln!(output, "compression_guess: {}", analysis.compression_guess);
    writeln_optional_usize(&mut output, "width_pixels", analysis.width_pixels);
    writeln_optional_usize(&mut output, "row_bytes_rgb", analysis.row_bytes_rgb);
    let _ = writeln!(output, "w_rows: {}", analysis.w_rows);
    let _ = writeln!(output, "nonzero_w_rows: {}", analysis.nonzero_w_rows);
    let _ = writeln!(output, "zero_w_rows: {}", analysis.zero_w_rows);
    let _ = writeln!(output, "w_payload_bytes: {}", analysis.w_payload_bytes);
    let _ = writeln!(
        output,
        "max_w_payload_bytes: {}",
        analysis.max_w_payload_bytes
    );
    let _ = writeln!(output, "y_commands: {}", analysis.y_commands);
    let _ = writeln!(output, "y_skipped_rows: {}", analysis.y_skipped_rows);
    let _ = writeln!(
        output,
        "decoded_blank_w_rows: {}",
        analysis.decoded_blank_w_rows
    );
    let _ = writeln!(
        output,
        "decoded_nonblank_w_rows: {}",
        analysis.decoded_nonblank_w_rows
    );
    let _ = writeln!(output, "zero_w_blank_rows: {}", analysis.zero_w_blank_rows);
    let _ = writeln!(
        output,
        "zero_w_nonblank_rows: {}",
        analysis.zero_w_nonblank_rows
    );
    let _ = writeln!(
        output,
        "longest_blank_w_run: {}",
        analysis.longest_blank_w_run
    );
    let _ = writeln!(
        output,
        "longest_nonblank_zero_w_run: {}",
        analysis.longest_nonblank_zero_w_run
    );
    match analysis.estimated_uncompressed_rgb_bytes {
        Some(bytes) => {
            let _ = writeln!(output, "estimated_uncompressed_rgb_bytes: {bytes}");
        }
        None => {
            let _ = writeln!(output, "estimated_uncompressed_rgb_bytes: none");
        }
    }
    match analysis.payload_ratio_basis_points {
        Some(value) => {
            let _ = writeln!(
                output,
                "payload_vs_estimated_rgb: {}",
                render_basis_points(value)
            );
        }
        None => {
            let _ = writeln!(output, "payload_vs_estimated_rgb: none");
        }
    }
    if analysis.decode_errors.is_empty() {
        let _ = writeln!(output, "decode_errors: none");
    } else {
        let _ = writeln!(output, "decode_errors:");
        for error in analysis.decode_errors.iter().take(10) {
            let _ = writeln!(output, "- {error}");
        }
        if analysis.decode_errors.len() > 10 {
            let _ = writeln!(output, "- ... {} more", analysis.decode_errors.len() - 10);
        }
    }

    output
}

fn render_basis_points(value: u64) -> String {
    format!("{}.{:02}%", value / 100, value % 100)
}

fn writeln_optional_usize(output: &mut String, label: &str, value: Option<usize>) {
    match value {
        Some(value) => {
            let _ = writeln!(output, "{label}: {value}");
        }
        None => {
            let _ = writeln!(output, "{label}: none");
        }
    }
}

fn writeln_optional_i32(output: &mut String, label: &str, value: Option<i32>) {
    match value {
        Some(value) => {
            let _ = writeln!(output, "{label}: {value}");
        }
        None => {
            let _ = writeln!(output, "{label}: none");
        }
    }
}

fn format_first_bytes_hex(bytes: &[u8], leading_bytes: usize) -> String {
    let take = bytes.len().min(leading_bytes);
    bytes[..take]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn detect_markers(bytes: &[u8]) -> Vec<RawMarker> {
    const MARKERS: &[(&[u8], &str)] = &[
        (b"\x1b%-12345X", "PJL Universal Exit Language"),
        (b"@PJL", "PJL command"),
        (b"\x1bE", "PCL reset"),
        (b"PCL3GUI", "PCL3GUI text marker"),
        (b"PCL", "PCL text marker"),
        (b"SLJ1660_PLACEHOLDER", "local placeholder/debug stream"),
    ];

    let mut found = Vec::new();
    for (needle, name) in MARKERS {
        found.extend(find_all(bytes, needle).into_iter().map(|offset| RawMarker {
            offset,
            name: (*name).to_string(),
        }));
    }
    found.sort_by_key(|marker| marker.offset);
    found.dedup();
    found
}

fn find_all(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return Vec::new();
    }

    haystack
        .windows(needle.len())
        .enumerate()
        .filter_map(|(offset, window)| (window == needle).then_some(offset))
        .collect()
}

fn find_first(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    find_all(haystack, needle).into_iter().next()
}

fn summarize_pcl_after_pcl3gui(bytes: &[u8], pcl3gui_offset: usize) -> Option<PclSummary> {
    let start_offset = bytes[pcl3gui_offset..]
        .windows(2)
        .position(|window| window == b"\x1bE")
        .map(|offset| pcl3gui_offset + offset)?;

    let mut cursor = start_offset;
    let mut summary = PclSummary {
        start_offset,
        ..PclSummary::default()
    };

    while let Some(relative) = bytes[cursor..].iter().position(|byte| *byte == 0x1b) {
        let offset = cursor + relative;
        let Some(command) = parse_pcl_command(bytes, offset) else {
            break;
        };

        summary.command_count += 1;
        apply_pcl_command_stats(&mut summary, &command);
        cursor = command.next_offset;
    }

    Some(summary)
}

fn apply_pcl_command_stats(summary: &mut PclSummary, command: &PclCommand<'_>) {
    let bytes = command.command;

    if bytes.starts_with(b"\x1b&l") && command.final_byte == b'A' {
        summary.paper_size_code = command.numeric_value;
    }
    if bytes.starts_with(b"\x1b*t") && command.final_byte == b'R' {
        summary.resolution_dpi = command.numeric_value;
    }
    if bytes.starts_with(b"\x1b*r") && command.final_byte == b'S' {
        summary.raster_width_s = command.numeric_value;
    }
    if bytes.starts_with(b"\x1b*r") && command.final_byte == b'A' {
        summary.raster_start_count += 1;
    }
    if bytes == b"\x1b*rC" {
        summary.raster_end_count += 1;
    }
    if bytes.starts_with(b"\x1b*b") && command.final_byte == b'W' {
        summary.b_w_count += 1;
        if command.data_len == Some(0) {
            summary.b_w_zero_count += 1;
        }
        let payload_len = command.data.len();
        summary.b_w_payload_bytes += payload_len;
        summary.b_w_max_payload_bytes = summary.b_w_max_payload_bytes.max(payload_len);
    }
    if bytes.starts_with(b"\x1b*b") && command.final_byte == b'Y' {
        summary.b_y_count += 1;
        summary.b_y_sum += i64::from(command.numeric_value.unwrap_or_default());
    }
    if bytes.starts_with(b"\x1b*b") && command.final_byte == b'V' {
        if let Some(value) = command.numeric_value {
            summary.b_v_values.push(value);
        }
    }
}

fn analyze_raster_command(
    analysis: &mut RawMode10Analysis,
    mode10: &mut Option<Mode10State>,
    command: &PclCommand<'_>,
    current_blank_run: &mut usize,
    current_nonblank_zero_run: &mut usize,
) {
    if command.final_byte == b'Y' {
        analysis.y_commands += 1;
        let rows = command
            .numeric_value
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or_default();
        analysis.y_skipped_rows = analysis.y_skipped_rows.saturating_add(rows);
        *current_blank_run = 0;
        *current_nonblank_zero_run = 0;
        return;
    }

    if command.final_byte != b'W' {
        return;
    }

    analysis.w_rows += 1;
    analysis.w_payload_bytes = analysis.w_payload_bytes.saturating_add(command.data.len());
    analysis.max_w_payload_bytes = analysis.max_w_payload_bytes.max(command.data.len());
    if command.data.is_empty() {
        analysis.zero_w_rows += 1;
    } else {
        analysis.nonzero_w_rows += 1;
    }

    let Some(mode10) = mode10.as_mut() else {
        return;
    };

    match mode10.decode_row(command.data) {
        Ok(row) if row.blank => {
            analysis.decoded_blank_w_rows += 1;
            *current_blank_run += 1;
            analysis.longest_blank_w_run = analysis.longest_blank_w_run.max(*current_blank_run);
            *current_nonblank_zero_run = 0;
            if command.data.is_empty() {
                analysis.zero_w_blank_rows += 1;
            }
        }
        Ok(_) => {
            analysis.decoded_nonblank_w_rows += 1;
            *current_blank_run = 0;
            if command.data.is_empty() {
                analysis.zero_w_nonblank_rows += 1;
                *current_nonblank_zero_run += 1;
                analysis.longest_nonblank_zero_w_run = analysis
                    .longest_nonblank_zero_w_run
                    .max(*current_nonblank_zero_run);
            } else {
                *current_nonblank_zero_run = 0;
            }
        }
        Err(error) => {
            analysis.decode_errors.push(format!(
                "row {} payload {} bytes: {error:#}",
                analysis.w_rows,
                command.data.len()
            ));
            *current_blank_run = 0;
            *current_nonblank_zero_run = 0;
        }
    }
}

fn compression_guess(analysis: &RawMode10Analysis) -> String {
    let Some(row_bytes_rgb) = analysis.row_bytes_rgb else {
        return "no raster width; cannot classify row compression".to_string();
    };
    if analysis.w_rows == 0 {
        return "no raster row payloads; likely setup/blank-page stream".to_string();
    }
    if analysis.max_w_payload_bytes < row_bytes_rgb {
        return "Mode10-style PCL3GUI delta/RLE row compression; not JPEG/FLATE/JBIG stream blocks"
            .to_string();
    }
    if analysis.max_w_payload_bytes >= row_bytes_rgb {
        return "row payloads approach raw RGB row size; compression is weak or absent".to_string();
    }
    "PCL3GUI row compression detected, but Mode10 decode had errors".to_string()
}

#[derive(Debug, Clone)]
struct PclCommand<'a> {
    command: &'a [u8],
    final_byte: u8,
    numeric_value: Option<i32>,
    data_len: Option<usize>,
    data: &'a [u8],
    next_offset: usize,
}

fn parse_pcl_command(bytes: &[u8], offset: usize) -> Option<PclCommand<'_>> {
    if bytes.get(offset) != Some(&0x1b) {
        return None;
    }

    let second = *bytes.get(offset + 1)?;
    let final_offset = match second {
        b'&' | b'*' => find_command_final(bytes, offset + 3)?,
        b'%' => find_command_final(bytes, offset + 2)?,
        _ => offset + 1,
    };

    let command = bytes.get(offset..=final_offset)?;
    let final_byte = *command.last()?;
    let numeric_value = parse_trailing_number(command);
    let data_len = if final_byte == b'W' {
        numeric_value.and_then(|value| usize::try_from(value).ok())
    } else {
        None
    };
    let data_start = final_offset + 1;
    let data_end = data_start + data_len.unwrap_or_default();
    let data = bytes.get(data_start..data_end)?;

    Some(PclCommand {
        command,
        final_byte,
        numeric_value,
        data_len,
        data,
        next_offset: data_end,
    })
}

fn find_command_final(bytes: &[u8], mut offset: usize) -> Option<usize> {
    while offset < bytes.len() {
        if bytes[offset].is_ascii_alphabetic() {
            return Some(offset);
        }
        offset += 1;
    }
    None
}

fn parse_trailing_number(command: &[u8]) -> Option<i32> {
    if command.len() < 2 {
        return None;
    }

    let end = command.len() - 1;
    let mut start = end;
    while start > 0 && command[start - 1].is_ascii_digit() {
        start -= 1;
    }
    if start > 0 && command[start - 1] == b'-' {
        start -= 1;
    }
    if start == end {
        return None;
    }

    std::str::from_utf8(&command[start..end])
        .ok()?
        .parse::<i32>()
        .ok()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn formats_hex_prefix() {
        let inspection = inspect_raw_bytes(Path::new("sample.raw"), b"\x1bEabc", 3);
        assert_eq!(inspection.size_bytes, 5);
        assert_eq!(inspection.first_bytes_hex, "1b 45 61");
        assert_eq!(inspection.first_nonzero_offset, Some(0));
    }

    #[test]
    fn detects_known_markers() {
        let bytes = b"\x1b%-12345X@PJL ENTER LANGUAGE=PCL\n\x1bE";
        let inspection = inspect_raw_bytes(Path::new("sample.raw"), bytes, 64);
        let marker_names = inspection
            .markers
            .iter()
            .map(|marker| marker.name.as_str())
            .collect::<Vec<_>>();
        assert!(marker_names.contains(&"PJL Universal Exit Language"));
        assert!(marker_names.contains(&"PJL command"));
        assert!(marker_names.contains(&"PCL reset"));
    }

    #[test]
    fn summarizes_pcl3gui_raster_commands() {
        let bytes = [
            vec![0; 8],
            b"\x1bE\x1b%-12345X@PJL ENTER LANGUAGE=PCL3GUI\n\x1bE".to_vec(),
            b"\x1b&l26A\x1b&u600D\x1b*t600R\x1b*r4891S\x1b*r1A".to_vec(),
            b"\x1b*b2Y\x1b*b3Wabc\x1b*b0W\x1b*b0V\x1b*rC".to_vec(),
        ]
        .concat();

        let inspection = inspect_raw_bytes(Path::new("sample.raw"), &bytes, 16);
        let summary = inspection.pcl_summary.unwrap();
        assert_eq!(inspection.first_nonzero_offset, Some(8));
        assert_eq!(summary.paper_size_code, Some(26));
        assert_eq!(summary.resolution_dpi, Some(600));
        assert_eq!(summary.raster_width_s, Some(4891));
        assert_eq!(summary.raster_start_count, 1);
        assert_eq!(summary.raster_end_count, 1);
        assert_eq!(summary.b_w_count, 2);
        assert_eq!(summary.b_w_zero_count, 1);
        assert_eq!(summary.b_w_payload_bytes, 3);
        assert_eq!(summary.b_w_max_payload_bytes, 3);
        assert_eq!(summary.b_y_count, 1);
        assert_eq!(summary.b_y_sum, 2);
        assert_eq!(summary.b_v_values, vec![0]);
    }

    #[test]
    fn analyzes_mode10_rows_and_repeats() {
        let bytes = [
            b"\x1b%-12345X@PJL ENTER LANGUAGE=PCL3GUI\n\x1bE".to_vec(),
            b"\x1b*t600R\x1b*r2S\x1b*r1A".to_vec(),
            b"\x1b*b1Y\x1b*b4W\x80\x00\x00\x00\x1b*b0W\x1b*b2Y\x1b*rC".to_vec(),
        ]
        .concat();

        let analysis = analyze_raw_bytes(Path::new("sample.raw"), &bytes);
        assert!(analysis.compression_guess.contains("Mode10-style"));
        assert_eq!(analysis.width_pixels, Some(2));
        assert_eq!(analysis.row_bytes_rgb, Some(6));
        assert_eq!(analysis.w_rows, 2);
        assert_eq!(analysis.nonzero_w_rows, 1);
        assert_eq!(analysis.zero_w_rows, 1);
        assert_eq!(analysis.y_skipped_rows, 3);
        assert_eq!(analysis.decoded_blank_w_rows, 0);
        assert_eq!(analysis.decoded_nonblank_w_rows, 2);
        assert_eq!(analysis.zero_w_nonblank_rows, 1);
        assert_eq!(analysis.longest_nonblank_zero_w_run, 1);
        assert_eq!(analysis.decode_errors, Vec::<String>::new());
    }

    #[test]
    fn analyzes_blank_zero_repeat() {
        let bytes = [
            b"\x1b%-12345X@PJL ENTER LANGUAGE=PCL3GUI\n\x1bE".to_vec(),
            b"\x1b*t600R\x1b*r2S\x1b*r1A\x1b*b0W\x1b*rC".to_vec(),
        ]
        .concat();

        let analysis = analyze_raw_bytes(Path::new("sample.raw"), &bytes);
        assert_eq!(analysis.w_rows, 1);
        assert_eq!(analysis.zero_w_rows, 1);
        assert_eq!(analysis.decoded_blank_w_rows, 1);
        assert_eq!(analysis.zero_w_blank_rows, 1);
        assert_eq!(analysis.longest_blank_w_run, 1);
    }
}
