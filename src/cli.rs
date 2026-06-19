use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{ArgGroup, Parser, Subcommand};

use crate::encoder::{PlaceholderEncoder, PrinterStreamEncoder};
use crate::ipp::{
    default_printer_path, default_project_root, default_script_path, default_spool_dir, serve_ipp,
    IppServerConfig,
};
use crate::raster::{GhostscriptRasterizer, RasterOptions, Rasterizer};
use crate::raw::{analyze_raw_file, inspect_raw_file, render_inspection, render_mode10_analysis};
use crate::usb::{
    list_usb_devices, send_bytes, LibusbTransport, UsbTarget, DEFAULT_PRODUCT_ID, DEFAULT_VENDOR_ID,
};

const DEFAULT_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_CHUNK_SIZE: usize = 16 * 1024;

#[derive(Debug, Parser)]
#[command(
    name = "slj1660",
    about = "User-space print-only MVP tools for Samsung SL-J1660",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// List USB devices and highlight likely Samsung SL-J1660 matches.
    ListUsb,

    /// Send an already-captured raw printer stream over USB.
    SendRaw {
        /// Raw stream captured from a known-good driver.
        path: PathBuf,

        /// USB vendor ID to match, in decimal or 0x-prefixed hex.
        #[arg(long, default_value = "0x04e8", value_parser = parse_u16)]
        vendor_id: u16,

        /// USB product ID to match, in decimal or 0x-prefixed hex.
        #[arg(long, default_value = "0x3954", value_parser = parse_u16)]
        product_id: u16,

        /// Optional USB serial number to disambiguate multiple printers.
        #[arg(long)]
        serial: Option<String>,

        /// Optional USB interface number override, in decimal or 0x-prefixed hex.
        #[arg(long, value_parser = parse_u8)]
        interface: Option<u8>,

        /// Optional USB bulk OUT endpoint override, in decimal or 0x-prefixed hex.
        #[arg(long, value_parser = parse_u8)]
        endpoint: Option<u8>,

        /// Bulk transfer chunk size in bytes.
        #[arg(long, default_value_t = DEFAULT_CHUNK_SIZE)]
        chunk_size: usize,

        /// USB write timeout in milliseconds.
        #[arg(long, default_value_t = DEFAULT_TIMEOUT_MS)]
        timeout_ms: u64,

        /// Delay between USB bulk transfer chunks in milliseconds.
        #[arg(long, default_value_t = 0)]
        chunk_delay_ms: u64,
    },

    /// Inspect a raw printer stream without sending it.
    InspectRaw {
        /// Raw stream to inspect.
        path: PathBuf,

        /// Number of leading bytes to print as hex.
        #[arg(long, default_value_t = 64)]
        bytes: usize,
    },

    /// Decode Mode10 row-compression stats from a raw PCL3GUI stream.
    AnalyzeRaw {
        /// Raw stream to analyze.
        path: PathBuf,
    },

    /// Validate PDF rasterization or emit a placeholder debug stream.
    #[command(group(
        ArgGroup::new("print_action")
            .required(true)
            .args(["dry_run", "output_raw"])
    ))]
    PrintPdf {
        /// PDF file to rasterize.
        path: PathBuf,

        /// Validate PDF-to-raster only; do not write output and do not touch USB.
        #[arg(long)]
        dry_run: bool,

        /// Write an incomplete placeholder/debug raw stream for inspection.
        #[arg(long, value_name = "OUT_RAW")]
        output_raw: Option<PathBuf>,
    },

    /// Run a local IPP printer-app daemon that accepts PDF jobs and sends them to USB.
    ServeIpp {
        /// Address for the local IPP server.
        #[arg(long, default_value = "127.0.0.1:8631")]
        bind: SocketAddr,

        /// HTTP path exposed as the printer URI.
        #[arg(long, default_value = "/printers/slj1660")]
        printer_path: String,

        /// Project root containing scripts/ and fixtures/.
        #[arg(long)]
        project_root: Option<PathBuf>,

        /// PDF-to-Mode10 script path. Defaults to scripts/print-pdf-mode10.py.
        #[arg(long)]
        script: Option<PathBuf>,

        /// Directory for accepted PDF jobs and generated raw streams.
        #[arg(long)]
        spool_dir: Option<PathBuf>,

        /// Optional USB serial number to disambiguate multiple SL-J1660 devices.
        #[arg(long)]
        serial: Option<String>,

        /// Render jobs to raw files but do not send them to USB.
        #[arg(long)]
        dry_run: bool,

        /// Skip best-effort LEDM alert/resume acknowledgement requests.
        #[arg(long)]
        no_confirm_alerts: bool,

        /// Bulk transfer chunk size in bytes for generated raw pages.
        #[arg(long, default_value_t = DEFAULT_CHUNK_SIZE)]
        chunk_size: usize,

        /// USB write timeout in milliseconds.
        #[arg(long, default_value_t = 30_000)]
        timeout_ms: u64,

        /// Delay between USB bulk transfer chunks in milliseconds.
        #[arg(long, default_value_t = 0)]
        chunk_delay_ms: u64,

        /// Maximum PDF pages to render for one accepted IPP job.
        #[arg(long, default_value_t = 20)]
        max_pages: u32,
    },
}

pub fn run_from_env() -> Result<()> {
    run(Cli::parse())
}

pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::ListUsb => run_list_usb(),
        Commands::SendRaw {
            path,
            vendor_id,
            product_id,
            serial,
            interface,
            endpoint,
            chunk_size,
            timeout_ms,
            chunk_delay_ms,
        } => run_send_raw(
            path,
            vendor_id,
            product_id,
            serial,
            interface,
            endpoint,
            chunk_size,
            timeout_ms,
            chunk_delay_ms,
        ),
        Commands::InspectRaw { path, bytes } => run_inspect_raw(path, bytes),
        Commands::AnalyzeRaw { path } => run_analyze_raw(path),
        Commands::PrintPdf {
            path,
            dry_run,
            output_raw,
        } => run_print_pdf(path, dry_run, output_raw),
        Commands::ServeIpp {
            bind,
            printer_path,
            project_root,
            script,
            spool_dir,
            serial,
            dry_run,
            no_confirm_alerts,
            chunk_size,
            timeout_ms,
            chunk_delay_ms,
            max_pages,
        } => run_serve_ipp(
            bind,
            printer_path,
            project_root,
            script,
            spool_dir,
            serial,
            dry_run,
            no_confirm_alerts,
            chunk_size,
            timeout_ms,
            chunk_delay_ms,
            max_pages,
        ),
    }
}

fn run_list_usb() -> Result<()> {
    let devices = list_usb_devices().context("failed to enumerate USB devices")?;

    if devices.is_empty() {
        println!("No USB devices found.");
        return Ok(());
    }

    println!("USB devices:");
    for device in devices {
        let target_marker =
            if device.vendor_id == DEFAULT_VENDOR_ID && device.product_id == DEFAULT_PRODUCT_ID {
                " [likely SL-J1660 target]"
            } else {
                ""
            };

        println!(
            "- {:04x}:{:04x} bus {} address {}{}",
            device.vendor_id, device.product_id, device.bus_number, device.address, target_marker
        );
        println!(
            "  manufacturer: {}",
            device.manufacturer.as_deref().unwrap_or("<unavailable>")
        );
        println!(
            "  product: {}",
            device.product.as_deref().unwrap_or("<unavailable>")
        );
        println!(
            "  serial: {}",
            device.serial_number.as_deref().unwrap_or("<unavailable>")
        );

        for interface in device.interfaces {
            println!(
                "  interface {} alt {} class {:02x} subclass {:02x} protocol {:02x}",
                interface.number,
                interface.alt_setting,
                interface.class_code,
                interface.sub_class_code,
                interface.protocol_code
            );
            for endpoint in interface.endpoints {
                println!(
                    "    endpoint 0x{:02x} {:?} {:?} max_packet_size {}",
                    endpoint.address,
                    endpoint.direction,
                    endpoint.transfer_type,
                    endpoint.max_packet_size
                );
            }
        }
    }

    Ok(())
}

fn run_send_raw(
    path: PathBuf,
    vendor_id: u16,
    product_id: u16,
    serial: Option<String>,
    interface: Option<u8>,
    endpoint: Option<u8>,
    chunk_size: usize,
    timeout_ms: u64,
    chunk_delay_ms: u64,
) -> Result<()> {
    if chunk_size == 0 {
        bail!("chunk size must be greater than zero");
    }

    let bytes = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    if bytes.is_empty() {
        bail!(
            "{} is empty; refusing to send an empty raw stream",
            path.display()
        );
    }

    let target = UsbTarget {
        vendor_id,
        product_id,
        serial_number: serial,
        interface_number: interface,
        endpoint_address: endpoint,
    };
    let timeout = Duration::from_millis(timeout_ms);
    let mut transport = LibusbTransport::open(target, timeout, chunk_size)
        .context("failed to open USB transport")?;
    transport.set_chunk_delay(Duration::from_millis(chunk_delay_ms));
    let stats = send_bytes(&mut transport, &bytes).context("failed to send raw stream")?;

    println!(
        "Sent {} bytes in {} USB bulk transfer chunk(s) to endpoint 0x{:02x}.",
        stats.bytes_sent,
        stats.chunks,
        transport.endpoint_address()
    );
    println!(
        "Claimed interface {} on {:04x}:{:04x}.",
        transport.interface_number(),
        vendor_id,
        product_id
    );
    Ok(())
}

fn run_inspect_raw(path: PathBuf, bytes: usize) -> Result<()> {
    let inspection = inspect_raw_file(&path, bytes)?;
    print!("{}", render_inspection(&inspection));
    Ok(())
}

fn run_analyze_raw(path: PathBuf) -> Result<()> {
    let analysis = analyze_raw_file(&path)?;
    print!("{}", render_mode10_analysis(&analysis));
    Ok(())
}

fn run_print_pdf(path: PathBuf, dry_run: bool, output_raw: Option<PathBuf>) -> Result<()> {
    if !path.exists() {
        bail!("PDF does not exist: {}", path.display());
    }

    let rasterizer = GhostscriptRasterizer::from_environment();
    let options = RasterOptions::black_and_white_300dpi();
    let raster = rasterizer
        .rasterize(&path, &options)
        .with_context(|| format!("failed to rasterize {}", path.display()))?;

    if dry_run {
        println!(
            "Dry run OK: rasterized first page to {}x{} at {} dpi ({} byte PBM payload).",
            raster.width,
            raster.height,
            raster.dpi,
            raster.data.len()
        );
        println!("No USB transfer was attempted.");
    }

    if let Some(out_path) = output_raw {
        let encoder = PlaceholderEncoder::default();
        let encoded = encoder.encode(&raster)?;
        fs::write(&out_path, &encoded.bytes)
            .with_context(|| format!("failed to write {}", out_path.display()))?;
        if let Some(warning) = encoded.warning {
            println!("warning: {warning}");
        }
        println!(
            "Wrote placeholder debug stream to {} ({} bytes).",
            out_path.display(),
            encoded.bytes.len()
        );
        println!("Do not send this placeholder stream to the printer.");
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_serve_ipp(
    bind: SocketAddr,
    printer_path: String,
    project_root: Option<PathBuf>,
    script: Option<PathBuf>,
    spool_dir: Option<PathBuf>,
    serial: Option<String>,
    dry_run: bool,
    no_confirm_alerts: bool,
    chunk_size: usize,
    timeout_ms: u64,
    chunk_delay_ms: u64,
    max_pages: u32,
) -> Result<()> {
    let project_root = project_root.unwrap_or_else(default_project_root);
    let script_path = script.unwrap_or_else(|| default_script_path(&project_root));
    let printer_path = if printer_path.is_empty() {
        default_printer_path()
    } else {
        printer_path
    };

    serve_ipp(IppServerConfig {
        bind,
        printer_path,
        project_root,
        script_path,
        spool_dir: spool_dir.unwrap_or_else(default_spool_dir),
        serial_number: serial,
        dry_run,
        confirm_alerts: !no_confirm_alerts,
        chunk_size,
        chunk_delay: Duration::from_millis(chunk_delay_ms),
        timeout: Duration::from_millis(timeout_ms),
        max_pages,
    })
}

fn parse_u16(value: &str) -> Result<u16, String> {
    let trimmed = value.trim();
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        u16::from_str_radix(hex, 16).map_err(|error| error.to_string())
    } else {
        trimmed.parse::<u16>().map_err(|error| error.to_string())
    }
}

fn parse_u8(value: &str) -> Result<u8, String> {
    let parsed = parse_u16(value)?;
    u8::try_from(parsed).map_err(|_| format!("{value} is outside the u8 range"))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn parses_required_commands() {
        let list = Cli::try_parse_from(["slj1660", "list-usb"]).unwrap();
        assert!(matches!(list.command, Commands::ListUsb));

        let inspect =
            Cli::try_parse_from(["slj1660", "inspect-raw", "fixtures/capture.raw"]).unwrap();
        assert!(matches!(inspect.command, Commands::InspectRaw { .. }));

        let analyze =
            Cli::try_parse_from(["slj1660", "analyze-raw", "fixtures/capture.raw"]).unwrap();
        assert!(matches!(analyze.command, Commands::AnalyzeRaw { .. }));

        let print =
            Cli::try_parse_from(["slj1660", "print-pdf", "sample.pdf", "--dry-run"]).unwrap();
        assert!(matches!(
            print.command,
            Commands::PrintPdf { dry_run: true, .. }
        ));

        let serve = Cli::try_parse_from(["slj1660", "serve-ipp", "--dry-run"]).unwrap();
        assert!(matches!(
            serve.command,
            Commands::ServeIpp { dry_run: true, .. }
        ));
    }

    #[test]
    fn print_pdf_requires_an_action() {
        let error = Cli::try_parse_from(["slj1660", "print-pdf", "sample.pdf"]).unwrap_err();
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn parses_hex_ids() {
        assert_eq!(parse_u16("0x04e8").unwrap(), 0x04e8);
        assert_eq!(parse_u16("14676").unwrap(), 14676);
    }
}
