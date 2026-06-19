use std::env;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use rusb::{request_type, Direction, Recipient, RequestType};

const VENDOR_ID: u16 = 0x04e8;
const PRODUCT_ID: u16 = 0x3954;
const PRINTER_INTERFACE: u8 = 1;
const REQUEST_GET_DEVICE_ID: u8 = 0;
const REQUEST_GET_PORT_STATUS: u8 = 1;
const REQUEST_SOFT_RESET: u8 = 2;

fn main() -> Result<()> {
    let do_reset = env::args().any(|arg| arg == "--soft-reset");
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
            .claim_interface(PRINTER_INTERFACE)
            .with_context(|| format!("failed to claim printer interface {PRINTER_INTERFACE}"))?;

        let device_id_index = u16::from(PRINTER_INTERFACE) << 8;
        let device_id = get_device_id(&handle, device_id_index, timeout)?;
        println!("GET_DEVICE_ID index=0x{device_id_index:04x}:");
        println!("{}", String::from_utf8_lossy(&device_id));

        for index in [u16::from(PRINTER_INTERFACE), device_id_index] {
            match get_port_status(&handle, index, timeout) {
                Ok(status) => println!(
                    "GET_PORT_STATUS index=0x{index:04x}: 0x{status:02x} ({})",
                    decode_port_status(status)
                ),
                Err(error) => println!("GET_PORT_STATUS index=0x{index:04x}: error: {error:#}"),
            }
        }

        if do_reset {
            let index = u16::from(PRINTER_INTERFACE);
            soft_reset(&handle, index, timeout)?;
            println!("SOFT_RESET index=0x{index:04x}: OK");
        }

        return Ok(());
    }

    bail!("SL-J1660 {:04x}:{:04x} not found", VENDOR_ID, PRODUCT_ID)
}

fn get_device_id(
    handle: &rusb::DeviceHandle<rusb::GlobalContext>,
    index: u16,
    timeout: Duration,
) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; 1024];
    let len = handle
        .read_control(
            request_type(Direction::In, RequestType::Class, Recipient::Interface),
            REQUEST_GET_DEVICE_ID,
            0,
            index,
            &mut buf,
            timeout,
        )
        .context("GET_DEVICE_ID failed")?;
    buf.truncate(len);
    Ok(buf)
}

fn get_port_status(
    handle: &rusb::DeviceHandle<rusb::GlobalContext>,
    index: u16,
    timeout: Duration,
) -> Result<u8> {
    let mut buf = [0u8; 1];
    handle
        .read_control(
            request_type(Direction::In, RequestType::Class, Recipient::Interface),
            REQUEST_GET_PORT_STATUS,
            0,
            index,
            &mut buf,
            timeout,
        )
        .context("GET_PORT_STATUS failed")?;
    Ok(buf[0])
}

fn soft_reset(
    handle: &rusb::DeviceHandle<rusb::GlobalContext>,
    index: u16,
    timeout: Duration,
) -> Result<()> {
    handle
        .write_control(
            request_type(Direction::Out, RequestType::Class, Recipient::Interface),
            REQUEST_SOFT_RESET,
            0,
            index,
            &[],
            timeout,
        )
        .context("SOFT_RESET failed")?;
    Ok(())
}

fn decode_port_status(status: u8) -> String {
    let mut parts = Vec::new();
    if status & 0x08 != 0 {
        parts.push("not-error");
    } else {
        parts.push("error");
    }
    if status & 0x10 != 0 {
        parts.push("select");
    } else {
        parts.push("not-select");
    }
    if status & 0x20 != 0 {
        parts.push("paper-empty-or-feed-attention");
    } else {
        parts.push("paper-present");
    }
    parts.join(", ")
}
