use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use rusb::{Device, DeviceDescriptor, DeviceHandle, Direction, GlobalContext, TransferType};

pub const DEFAULT_VENDOR_ID: u16 = 0x04e8;
pub const DEFAULT_PRODUCT_ID: u16 = 0x3954;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbTarget {
    pub vendor_id: u16,
    pub product_id: u16,
    pub serial_number: Option<String>,
    pub interface_number: Option<u8>,
    pub endpoint_address: Option<u8>,
}

impl Default for UsbTarget {
    fn default() -> Self {
        Self {
            vendor_id: DEFAULT_VENDOR_ID,
            product_id: DEFAULT_PRODUCT_ID,
            serial_number: None,
            interface_number: None,
            endpoint_address: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbDeviceInfo {
    pub vendor_id: u16,
    pub product_id: u16,
    pub bus_number: u8,
    pub address: u8,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial_number: Option<String>,
    pub interfaces: Vec<UsbInterfaceInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbInterfaceInfo {
    pub number: u8,
    pub alt_setting: u8,
    pub class_code: u8,
    pub sub_class_code: u8,
    pub protocol_code: u8,
    pub endpoints: Vec<UsbEndpointInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbEndpointInfo {
    pub address: u8,
    pub direction: Direction,
    pub transfer_type: TransferType,
    pub max_packet_size: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferStats {
    pub bytes_sent: usize,
    pub chunks: usize,
}

pub trait UsbTransport {
    fn send_all(&mut self, bytes: &[u8]) -> Result<TransferStats>;
}

pub fn send_bytes<T: UsbTransport>(transport: &mut T, bytes: &[u8]) -> Result<TransferStats> {
    if bytes.is_empty() {
        bail!("raw stream is empty; refusing to send");
    }
    transport.send_all(bytes)
}

pub fn list_usb_devices() -> Result<Vec<UsbDeviceInfo>> {
    let devices = rusb::devices().context("libusb could not list USB devices")?;
    let timeout = Duration::from_millis(250);

    devices
        .iter()
        .map(|device| usb_device_info(&device, timeout))
        .collect()
}

fn usb_device_info(device: &Device<GlobalContext>, timeout: Duration) -> Result<UsbDeviceInfo> {
    let descriptor = device.device_descriptor()?;
    let mut handle = device.open().ok();
    let language = handle
        .as_mut()
        .and_then(|handle| handle.read_languages(timeout).ok())
        .and_then(|languages| languages.into_iter().next());

    let manufacturer = handle
        .as_mut()
        .zip(language)
        .and_then(|(handle, language)| {
            handle
                .read_manufacturer_string(language, &descriptor, timeout)
                .ok()
        });
    let product = handle
        .as_mut()
        .zip(language)
        .and_then(|(handle, language)| {
            handle
                .read_product_string(language, &descriptor, timeout)
                .ok()
        });
    let serial_number = handle
        .as_mut()
        .zip(language)
        .and_then(|(handle, language)| {
            handle
                .read_serial_number_string(language, &descriptor, timeout)
                .ok()
        });

    Ok(UsbDeviceInfo {
        vendor_id: descriptor.vendor_id(),
        product_id: descriptor.product_id(),
        bus_number: device.bus_number(),
        address: device.address(),
        manufacturer,
        product,
        serial_number,
        interfaces: collect_interfaces(device, &descriptor),
    })
}

fn collect_interfaces(
    device: &Device<GlobalContext>,
    descriptor: &DeviceDescriptor,
) -> Vec<UsbInterfaceInfo> {
    let mut interfaces = Vec::new();
    for config_index in 0..descriptor.num_configurations() {
        let Ok(config) = device.config_descriptor(config_index) else {
            continue;
        };

        for interface in config.interfaces() {
            for interface_descriptor in interface.descriptors() {
                let endpoints = interface_descriptor
                    .endpoint_descriptors()
                    .map(|endpoint| UsbEndpointInfo {
                        address: endpoint.address(),
                        direction: endpoint.direction(),
                        transfer_type: endpoint.transfer_type(),
                        max_packet_size: endpoint.max_packet_size(),
                    })
                    .collect();

                interfaces.push(UsbInterfaceInfo {
                    number: interface_descriptor.interface_number(),
                    alt_setting: interface_descriptor.setting_number(),
                    class_code: interface_descriptor.class_code(),
                    sub_class_code: interface_descriptor.sub_class_code(),
                    protocol_code: interface_descriptor.protocol_code(),
                    endpoints,
                });
            }
        }
    }

    interfaces
}

pub struct LibusbTransport {
    handle: DeviceHandle<GlobalContext>,
    interface_number: u8,
    endpoint_address: u8,
    timeout: Duration,
    chunk_size: usize,
    chunk_delay: Duration,
}

impl LibusbTransport {
    pub fn open(target: UsbTarget, timeout: Duration, chunk_size: usize) -> Result<Self> {
        if chunk_size == 0 {
            bail!("chunk size must be greater than zero");
        }

        let devices = rusb::devices().context("libusb could not list USB devices")?;
        for device in devices.iter() {
            let descriptor = device.device_descriptor()?;
            if descriptor.vendor_id() != target.vendor_id
                || descriptor.product_id() != target.product_id
            {
                continue;
            }

            let endpoint = find_bulk_out_endpoint(
                &device,
                &descriptor,
                target.interface_number,
                target.endpoint_address,
            )
            .with_context(|| {
                format!(
                    "device {:04x}:{:04x} has no matching bulk OUT endpoint",
                    target.vendor_id, target.product_id
                )
            })?;

            let mut handle = device.open().with_context(|| {
                format!(
                    "could not open USB device {:04x}:{:04x}; macOS permissions or another process may be blocking access",
                    target.vendor_id, target.product_id
                )
            })?;

            if let Some(expected_serial) = target.serial_number.as_deref() {
                let actual_serial = read_serial(&mut handle, &descriptor, timeout);
                if actual_serial.as_deref() != Some(expected_serial) {
                    continue;
                }
            }

            handle
                .claim_interface(endpoint.interface_number)
                .map_err(|error| {
                    anyhow::anyhow!(
                        "could not claim USB interface {} on {:04x}:{:04x}: {error}. \
                     On macOS this commonly means an existing driver, print queue, or another \
                     process has claimed the interface. Close printer tools and CUPS queues, \
                     reconnect the USB cable, and retry. This MVP does not install kexts or use \
                     privileged driver-detach hacks.",
                        endpoint.interface_number,
                        target.vendor_id,
                        target.product_id
                    )
                })?;

            return Ok(Self {
                handle,
                interface_number: endpoint.interface_number,
                endpoint_address: endpoint.endpoint_address,
                timeout,
                chunk_size,
                chunk_delay: Duration::ZERO,
            });
        }

        bail!(
            "SL-J1660 USB target {:04x}:{:04x} was not found. Run `slj1660 list-usb` and verify the printer is connected.",
            target.vendor_id,
            target.product_id
        )
    }

    pub fn interface_number(&self) -> u8 {
        self.interface_number
    }

    pub fn endpoint_address(&self) -> u8 {
        self.endpoint_address
    }

    pub fn set_chunk_delay(&mut self, chunk_delay: Duration) {
        self.chunk_delay = chunk_delay;
    }
}

impl UsbTransport for LibusbTransport {
    fn send_all(&mut self, bytes: &[u8]) -> Result<TransferStats> {
        let mut bytes_sent = 0;
        let mut chunks = 0;

        while bytes_sent < bytes.len() {
            let end = (bytes_sent + self.chunk_size).min(bytes.len());
            let written = self
                .handle
                .write_bulk(self.endpoint_address, &bytes[bytes_sent..end], self.timeout)
                .with_context(|| {
                    format!(
                        "USB bulk write failed after {} byte(s) on endpoint 0x{:02x}",
                        bytes_sent, self.endpoint_address
                    )
                })?;

            if written == 0 {
                bail!("USB bulk write returned 0 bytes; aborting to avoid an infinite loop");
            }

            bytes_sent += written;
            chunks += 1;

            if bytes_sent < bytes.len() && !self.chunk_delay.is_zero() {
                thread::sleep(self.chunk_delay);
            }
        }

        Ok(TransferStats { bytes_sent, chunks })
    }
}

impl Drop for LibusbTransport {
    fn drop(&mut self) {
        let _ = self.handle.release_interface(self.interface_number);
    }
}

#[derive(Debug, Clone, Copy)]
struct BulkOutEndpoint {
    interface_number: u8,
    endpoint_address: u8,
    interface_class: u8,
}

fn find_bulk_out_endpoint(
    device: &Device<GlobalContext>,
    descriptor: &DeviceDescriptor,
    interface_filter: Option<u8>,
    endpoint_filter: Option<u8>,
) -> Result<BulkOutEndpoint> {
    let mut candidates = Vec::new();

    for config_index in 0..descriptor.num_configurations() {
        let config = device.config_descriptor(config_index)?;
        for interface in config.interfaces() {
            for interface_descriptor in interface.descriptors() {
                if interface_filter
                    .is_some_and(|number| number != interface_descriptor.interface_number())
                {
                    continue;
                }

                for endpoint in interface_descriptor.endpoint_descriptors() {
                    if endpoint_filter.is_some_and(|address| address != endpoint.address()) {
                        continue;
                    }

                    if endpoint.direction() == Direction::Out
                        && endpoint.transfer_type() == TransferType::Bulk
                    {
                        candidates.push(BulkOutEndpoint {
                            interface_number: interface_descriptor.interface_number(),
                            endpoint_address: endpoint.address(),
                            interface_class: interface_descriptor.class_code(),
                        });
                    }
                }
            }
        }
    }

    if candidates.is_empty() {
        bail!("no matching bulk OUT endpoint found");
    }

    candidates
        .iter()
        .find(|candidate| candidate.interface_class == 0x07)
        .copied()
        .or_else(|| candidates.first().copied())
        .context("no matching bulk OUT endpoint found")
}

fn read_serial(
    handle: &mut DeviceHandle<GlobalContext>,
    descriptor: &DeviceDescriptor,
    timeout: Duration,
) -> Option<String> {
    let language = handle.read_languages(timeout).ok()?.into_iter().next()?;
    handle
        .read_serial_number_string(language, descriptor, timeout)
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct MockTransport {
        sent: Vec<u8>,
        chunk_size: usize,
    }

    impl UsbTransport for MockTransport {
        fn send_all(&mut self, bytes: &[u8]) -> Result<TransferStats> {
            self.sent.extend_from_slice(bytes);
            Ok(TransferStats {
                bytes_sent: bytes.len(),
                chunks: bytes.len().div_ceil(self.chunk_size.max(1)),
            })
        }
    }

    #[test]
    fn sends_bytes_through_transport_abstraction() {
        let mut transport = MockTransport {
            sent: Vec::new(),
            chunk_size: 2,
        };
        let stats = send_bytes(&mut transport, b"abcd").unwrap();
        assert_eq!(stats.bytes_sent, 4);
        assert_eq!(stats.chunks, 2);
        assert_eq!(transport.sent, b"abcd");
    }

    #[test]
    fn refuses_empty_streams() {
        let mut transport = MockTransport::default();
        let error = send_bytes(&mut transport, b"").unwrap_err();
        assert!(error.to_string().contains("empty"));
    }
}
