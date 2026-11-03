//! 真实 D50 的 HID 通道。
//!
//! 协议说 Vendor HID「有效区 64 字节、不使用 Report ID」。Windows 的 hidapi
//! 一律把第一个字节当 Report ID，所以无 Report ID 的设备通常要显式补 `0x00`。
//! 这一点必须在真机上确认：写错的表现不是报错，而是设备完全不回帧。因此
//! 是否补前缀做成运行时开关（`--no-report-id-prefix`），别为一个字节重编译。

use super::{Device, Link, LinkError, REPORT_LEN, SessionEvent};
use hidapi::HidApi;
use std::time::{Duration, Instant};

/// 枚举到的一个 Vendor HID 候选。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// 与 `--device-index` 对应的序号，仅代表本次枚举顺序。
    pub index: usize,
    pub vendor_id: u16,
    pub product_id: u16,
    /// 很多 HID 设备根本不上报序列号，这里可能是 `None`。
    pub serial: Option<String>,
    pub product: Option<String>,
    pub usage_page: u16,
    pub interface: i32,
}

impl Candidate {
    pub fn label(&self) -> String {
        match (&self.product, &self.serial) {
            (Some(product), Some(serial)) => format!("{product} ({serial})"),
            (Some(product), None) => product.clone(),
            (None, Some(serial)) => serial.clone(),
            (None, None) => "(未命名设备)".into(),
        }
    }
}

/// 握手成功后的自报身份。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probed {
    pub device_type: String,
    pub serial: String,
    pub firmware: String,
}

pub struct HidLink {
    device: hidapi::HidDevice,
    prefix: bool,
}

impl Link for HidLink {
    fn write_report(&mut self, report: &[u8; REPORT_LEN]) -> Result<(), LinkError> {
        let mut buffer = Vec::with_capacity(REPORT_LEN + 1);
        if self.prefix {
            buffer.push(0x00);
        }
        buffer.extend_from_slice(report);
        self.device
            .write(&buffer)
            .map(|_| ())
            .map_err(|error| LinkError::Io(error.to_string()))
    }

    fn read_report(&mut self, timeout_ms: i32) -> Result<Option<[u8; REPORT_LEN]>, LinkError> {
        let mut buffer = [0u8; REPORT_LEN + 1];
        match self.device.read_timeout(&mut buffer, timeout_ms) {
            Ok(0) => Ok(None),
            Ok(read) => {
                // hidapi 返回的字节流是否带 Report ID 由设备描述符决定；
                // 两种长度都接受，只取前 64 字节。
                let start = if read > REPORT_LEN {
                    read - REPORT_LEN
                } else {
                    0
                };
                let mut report = [0u8; REPORT_LEN];
                let length = (read - start).min(REPORT_LEN);
                report[..length].copy_from_slice(&buffer[start..start + length]);
                Ok(Some(report))
            }
            Err(error) => Err(LinkError::Io(error.to_string())),
        }
    }
}

/// 列出所有 Vendor HID（usage page `0xFF00`）。
///
/// 不按 VID/PID 过滤：协议没定义它们，同一台机器上还可能插着别的 Ulanzi 设备。
/// 到底是不是 D50，要等握手时设备自己报 `DeviceType`。
pub fn candidates() -> Result<Vec<Candidate>, String> {
    let api = HidApi::new().map_err(|error| format!("HID_INIT_FAILED: {error}"))?;
    let mut out = Vec::new();
    for info in api.device_list() {
        if info.usage_page() != 0xFF00 {
            continue;
        }
        out.push(Candidate {
            index: out.len(),
            vendor_id: info.vendor_id(),
            product_id: info.product_id(),
            serial: info.serial_number().map(str::to_owned),
            product: info.product_string().map(str::to_owned),
            usage_page: info.usage_page(),
            interface: info.interface_number(),
        });
    }
    Ok(out)
}

pub fn open_index(index: usize, prefix: bool) -> Result<Box<dyn Link>, String> {
    let api = HidApi::new().map_err(|error| format!("HID_INIT_FAILED: {error}"))?;
    let mut current = 0;
    for info in api.device_list() {
        if info.usage_page() != 0xFF00 {
            continue;
        }
        if current == index {
            let device = api
                .open_path(info.path())
                .map_err(|error| format!("HID_OPEN_FAILED: {error}"))?;
            return Ok(Box::new(HidLink { device, prefix }));
        }
        current += 1;
    }
    Err(format!("HID_INDEX_OUT_OF_RANGE: {index}"))
}

/// 打开并按序列号选择；序列号来自设备自报，不是从别处查来的。
pub fn open(serial: Option<&str>, prefix: bool) -> Result<Box<dyn Link>, String> {
    let Some(serial) = serial else {
        return open_index(0, prefix);
    };
    for candidate in candidates()? {
        if candidate.serial.as_deref() == Some(serial) {
            return open_index(candidate.index, prefix);
        }
    }
    Err(format!("D50_SERIAL_NOT_FOUND: {serial}"))
}

/// 打开一个候选做一次握手，确认它到底是不是 D50。
pub fn probe_index(index: usize, prefix: bool) -> Result<Probed, String> {
    let link = open_index(index, prefix)?;
    let device = Device::spawn(link);
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        match device.recv_event_timeout(Duration::from_millis(200)) {
            Some(SessionEvent::DeviceInfo(info)) => {
                return Ok(Probed {
                    device_type: info.device_type,
                    serial: info.serial_number,
                    firmware: info.d_version,
                });
            }
            Some(SessionEvent::LinkLost(reason)) => {
                return Err(format!(
                    "握手失败（{reason}）：这大概率不是 D50，或 Report 前缀要换一下"
                ));
            }
            _ => {}
        }
    }
    Err("4 秒内没有完成握手：检查设备是否被别的程序占用，或试 --no-report-id-prefix".into())
}
