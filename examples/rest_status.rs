use std::time::Duration;
use std::{env, process};

use anyhow::{bail, Context, Result};

const VENDOR_ID: u16 = 0x04e8;
const PRODUCT_ID: u16 = 0x3954;
const REST_INTERFACE: u8 = 3;
const REST_OUT_ENDPOINT: u8 = 0x0a;
const REST_IN_ENDPOINT: u8 = 0x8b;

fn main() -> Result<()> {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "/DevMgmt/ProductStatusDyn.xml".to_string());
    if !path.starts_with('/') {
        eprintln!(
            "usage: {} [/DevMgmt/Resource.xml]",
            env::args().next().unwrap()
        );
        process::exit(2);
    }

    let timeout = Duration::from_millis(5_000);
    let devices = rusb::devices().context("failed to enumerate USB devices")?;

    for device in devices.iter() {
        let descriptor = device.device_descriptor()?;
        if descriptor.vendor_id() != VENDOR_ID || descriptor.product_id() != PRODUCT_ID {
            continue;
        }

        let handle = device
            .open()
            .context("failed to open SL-J1660 USB device")?;
        handle
            .claim_interface(REST_INTERFACE)
            .with_context(|| format!("failed to claim REST interface {REST_INTERFACE}"))?;

        let request = format!("GET {path} HTTP/1.1\r\nHOST: localhost\r\n\r\n");
        let written = handle
            .write_bulk(REST_OUT_ENDPOINT, request.as_bytes(), timeout)
            .context("failed to write REST status request")?;
        eprintln!("wrote {written} byte REST request to endpoint 0x{REST_OUT_ENDPOINT:02x}");

        let mut response = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match handle.read_bulk(REST_IN_ENDPOINT, &mut buf, timeout) {
                Ok(0) => break,
                Ok(len) => response.extend_from_slice(&buf[..len]),
                Err(rusb::Error::Timeout) => break,
                Err(error) => return Err(error).context("failed to read REST status response"),
            }
        }

        if response.is_empty() {
            eprintln!("no REST response before timeout");
        } else {
            print!("{}", String::from_utf8_lossy(&response));
        }
        return Ok(());
    }

    bail!("SL-J1660 {:04x}:{:04x} not found", VENDOR_ID, PRODUCT_ID)
}
