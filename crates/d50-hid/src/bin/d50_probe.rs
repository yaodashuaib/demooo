//! 原始 HID 探针：把写进去的字节、hidapi 的返回、读到的字节全部打出来。
//!
//! 握手失败只有三种可能：写不进去、写得进去但没人回、回的东西不对。
//! 这个工具就是用来区分这三种的，不参与生产路径。

use clap::Parser;
use hidapi::HidApi;
use std::time::{Duration, Instant};

#[derive(Debug, Parser)]
#[command(
    name = "d50-probe",
    about = "D50 原始 HID 收发探针（只诊断，不改设备状态）"
)]
struct Args {
    /// 候选序号，来自 --list-devices。
    #[arg(long, default_value_t = 0)]
    index: usize,
    /// 每种写法的读等待时间。
    #[arg(long, default_value_t = 600)]
    read_timeout_ms: i32,
    /// 每种写法读几次。
    #[arg(long, default_value_t = 6)]
    reads: usize,
    /// 在「全部 HID 接口」里按下标选设备（配合 --all 的序号）。
    #[arg(long)]
    any_index: Option<usize>,
    /// 只读不写地听这么久（秒）：用来确认设备自己会不会发输入报告。
    #[arg(long, default_value_t = 0)]
    listen_seconds: u64,
    /// 只发心跳并持续读这么久（秒），用来看设备会不会主动说话。
    #[arg(long, default_value_t = 0)]
    soak_seconds: u64,
    /// 列出全部 HID 接口（不只 0xFF00），确认厂商集合是哪一个。
    #[arg(long)]
    all: bool,
    /// 只打印设备路径，交给 Windows API 读描述符用。
    #[arg(long)]
    paths: bool,
    /// 发一条「预览 Agent 提示灯」：只闪一次、不保存配置，用来判断下行通不通。
    #[arg(long)]
    flash: bool,
    /// 逐个 Report ID 试同一帧。
    #[arg(long)]
    ids: bool,
    /// 逐种帧摆法试一遍，看设备认哪一种。
    #[arg(long)]
    sweep: bool,
}

/// 一个 55 D5 帧的两种常用摆法。
fn frame(command: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0x55u8, 0xD5, command, 0x00];
    let length = (8 + payload.len()) as u16;
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(payload);
    let crc = crc16(&out);
    out.extend_from_slice(&crc.to_le_bytes());
    out
}

fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for byte in data {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 把一段内容摆成一个 65 字节 report：首字节是报告 ID，其余补 0。
fn report(report_id: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![report_id];
    out.extend_from_slice(body);
    out.resize(65, 0);
    out
}

/// 同一句 GET_PROTOCOL_VERSION 的几种可能摆法。
fn variants() -> Vec<(&'static str, Vec<u8>)> {
    let header = [0x55u8, 0xD5, 0x00, 0x00, 0x08, 0x00];
    let crc = crc16(&header).to_le_bytes();
    let mut doc = header.to_vec();
    doc.extend_from_slice(&crc);

    let mut len_as_payload = [0x55u8, 0xD5, 0x00, 0x00, 0x00, 0x00].to_vec();
    len_as_payload.extend_from_slice(&crc16(&len_as_payload.clone()).to_le_bytes());

    let mut crc_be = header.to_vec();
    crc_be.extend_from_slice(&[crc[1], crc[0]]);

    let mut crc_payload_only = header.to_vec();
    crc_payload_only.extend_from_slice(&0xFFFFu16.to_le_bytes());

    let mut sync_reversed = [0xD5u8, 0x55, 0x00, 0x00, 0x08, 0x00].to_vec();
    sync_reversed.extend_from_slice(&crc16(&sync_reversed.clone()).to_le_bytes());

    vec![
        ("doc（协议原文）", report(0x00, &doc)),
        ("长度=Payload", report(0x00, &len_as_payload)),
        ("CRC 大端", report(0x00, &crc_be)),
        ("CRC 只算 Payload", report(0x00, &crc_payload_only)),
        (
            "不带 CRC",
            report(0x00, &[0x55, 0xD5, 0x00, 0x00, 0x06, 0x00]),
        ),
        ("同步头 D5 55", report(0x00, &sync_reversed)),
        ("报告 ID=0x55 共用帧头", report(0x55, &doc[1..])),
        ("报告 ID=0x55 + 完整帧", report(0x55, &doc)),
    ]
}

/// `SET_SOUND_LIGHT_CONFIG` 的 Preview 分支：只按本次参数闪一次，不保存配置。
fn flash(api: &HidApi, info: &hidapi::DeviceInfo) {
    let payload = br#"{"Light":{"Brightness":80,"AgentPrompt":{"Enabled":true,"Color":{"R":255,"G":0,"B":0},"DurationMs":5000,"Preview":true}}}"#;
    let bytes = frame(0x12, payload);
    println!("== 预览 Agent 提示灯（红色呼吸 5 秒，不保存配置）");
    println!(
        "   payload {} 字节，帧 {} 字节 → {} 个 report",
        payload.len(),
        bytes.len(),
        bytes.len().div_ceil(64)
    );
    let device = match api.open_path(info.path()) {
        Ok(device) => device,
        Err(error) => {
            eprintln!("打开失败: {error}");
            return;
        }
    };
    for (index, chunk) in bytes.chunks(64).enumerate() {
        let mut report = vec![0x00u8];
        report.extend_from_slice(chunk);
        report.resize(65, 0);
        match device.write(&report) {
            Ok(written) => println!("   report {} 写 Ok({written})", index + 1),
            Err(error) => println!("   report {} 写失败: {error}", index + 1),
        }
    }
    let mut buffer = [0u8; 256];
    match device.read_timeout(&mut buffer, 800) {
        Ok(0) => println!("   之后 800 ms 内没有回音"),
        Ok(read) => println!("   读 {read} 字节: {}", hex(&buffer[..read])),
        Err(error) => println!("   读失败: {error}"),
    }
}

fn id_sweep(api: &HidApi, info: &hidapi::DeviceInfo, read_timeout_ms: i32) {
    let header = [0x55u8, 0xD5, 0x00, 0x00, 0x08, 0x00];
    let crc = crc16(&header).to_le_bytes();
    let mut doc = header.to_vec();
    doc.extend_from_slice(&crc);
    println!("== Report ID 扫描（设备声明 65 字节报告 = 1 字节 ID + 64 字节数据）");
    for id in [0x00u8, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x55] {
        let bytes = report(id, &doc);
        let device = match api.open_path(info.path()) {
            Ok(device) => device,
            Err(error) => {
                println!("   打开失败: {error}");
                return;
            }
        };
        let write = match device.write(&bytes) {
            Ok(written) => format!("Ok({written})"),
            Err(error) => format!("Err({error})"),
        };
        let mut buffer = [0u8; 256];
        let read = match device.read_timeout(&mut buffer, read_timeout_ms) {
            Ok(0) => "无回音".to_string(),
            Ok(read) => format!("{} 字节: {}", read, hex(&buffer[..read])),
            Err(error) => format!("读失败: {error}"),
        };
        println!("  ID 0x{id:02X} 写 {write:<10} 读 {read}");
    }
}

fn sweep(api: &HidApi, info: &hidapi::DeviceInfo, read_timeout_ms: i32) {
    println!("== 帧摆法扫描（每种读 {} ms）", read_timeout_ms);
    for (name, bytes) in variants() {
        let device = match api.open_path(info.path()) {
            Ok(device) => device,
            Err(error) => {
                println!("   打开失败: {error}");
                return;
            }
        };
        let write = match device.write(&bytes) {
            Ok(written) => format!("Ok({written})"),
            Err(error) => format!("Err({error})"),
        };
        let mut buffer = [0u8; 256];
        let read = match device.read_timeout(&mut buffer, read_timeout_ms) {
            Ok(0) => "无回音".to_string(),
            Ok(read) => format!("{} 字节: {}", read, hex(&buffer[..read])),
            Err(error) => format!("读失败: {error}"),
        };
        println!("  {name:<22} 写 {write:<10} 读 {read}");
        println!(
            "      发出: {}",
            hex(&bytes[..bytes
                .iter()
                .rposition(|b| *b != 0)
                .map_or(0, |i| i + 1)
                .min(bytes.len())])
        );
    }
}

fn main() {
    let args = Args::parse();
    let api = HidApi::new().expect("HID 初始化失败");

    if args.all {
        println!("全部 HID 接口：");
        for (index, device) in api.device_list().enumerate() {
            println!(
                "  [{index}] {:04X}:{:04X} usage {:04X}:{:04X} iface {:>3}  {:<28} {:?}",
                device.vendor_id(),
                device.product_id(),
                device.usage_page(),
                device.usage(),
                device.interface_number(),
                device.product_string().unwrap_or("-"),
                device.serial_number()
            );
        }
        println!();
    }

    let all: Vec<_> = api.device_list().collect();
    let devices: Vec<_> = all
        .iter()
        .copied()
        .filter(|d| d.usage_page() == 0xFF00)
        .collect();

    if args.paths {
        for device in &devices {
            println!("{}", device.path().to_string_lossy());
        }
        return;
    }
    let selected = match args.any_index {
        Some(index) => all.get(index).copied(),
        None => devices.get(args.index).copied(),
    };
    let Some(info) = selected else {
        eprintln!(
            "没有这个设备（Vendor HID {} 个，全部 {} 个）",
            devices.len(),
            all.len()
        );
        std::process::exit(2);
    };
    println!(
        "设备 {:04X}:{:04X}  usage_page 0x{:04X} usage 0x{:04X}  iface {}",
        info.vendor_id(),
        info.product_id(),
        info.usage_page(),
        info.usage(),
        info.interface_number()
    );
    println!("product {:?}", info.product_string());
    println!("serial  {:?}", info.serial_number());
    println!();

    let heartbeat = frame(0x02, &[0x00]);
    let version = frame(0x00, &[]);
    println!("heartbeat 帧 {}", hex(&heartbeat));
    println!("version   帧 {}", hex(&version));
    println!();

    if args.flash {
        flash(&api, info);
        return;
    }

    if args.ids {
        id_sweep(&api, info, args.read_timeout_ms);
        return;
    }

    if args.sweep {
        sweep(&api, info, args.read_timeout_ms);
        return;
    }

    if args.listen_seconds > 0 {
        println!("== 只读 {} 秒，不发任何字节", args.listen_seconds);
        let device = match api.open_path(info.path()) {
            Ok(device) => device,
            Err(error) => {
                eprintln!("打开失败: {error}");
                std::process::exit(2);
            }
        };
        let deadline = Instant::now() + Duration::from_secs(args.listen_seconds);
        let mut total = 0;
        while Instant::now() < deadline {
            let mut buffer = [0u8; 256];
            match device.read_timeout(&mut buffer, 200) {
                Ok(0) => {}
                Ok(read) => {
                    total += read;
                    println!("   读 {read} 字节: {}", hex(&buffer[..read]));
                }
                Err(error) => {
                    println!("   读失败: {error}");
                    break;
                }
            }
        }
        println!("   总共收到 {total} 字节");
        return;
    }

    if args.soak_seconds > 0 {
        println!(
            "== 持续心跳 {} 秒（每 800 ms 一次），看设备会不会主动说话",
            args.soak_seconds
        );
        let device = match api.open_path(info.path()) {
            Ok(device) => device,
            Err(error) => {
                eprintln!("打开失败: {error}");
                std::process::exit(2);
            }
        };
        let deadline = Instant::now() + Duration::from_secs(args.soak_seconds);
        let mut last_beat = Instant::now() - Duration::from_secs(1);
        let mut total = 0;
        while Instant::now() < deadline {
            if last_beat.elapsed() >= Duration::from_millis(800) {
                let mut buffer = vec![0x00];
                buffer.extend_from_slice(&heartbeat);
                buffer.resize(65, 0);
                let _ = device.write(&buffer);
                last_beat = Instant::now();
            }
            let mut buffer = [0u8; 128];
            match device.read_timeout(&mut buffer, 200) {
                Ok(0) => {}
                Ok(read) => {
                    total += read;
                    println!("   读 {read} 字节: {}", hex(&buffer[..read]));
                }
                Err(error) => {
                    println!("   读失败: {error}");
                    break;
                }
            }
        }
        println!("   总共收到 {total} 字节");
        return;
    }

    // 三种摆法，逐个试：谁写成功、谁有回音。
    let modes: [(&str, &dyn Fn(&[u8]) -> Vec<u8>); 3] = [
        ("A: 前置 0x00 报告 ID（65 字节）", &|f: &[u8]| {
            let mut v = vec![0x00];
            v.extend_from_slice(f);
            v.resize(65, 0);
            v
        }),
        ("B: 直接 64 字节（无报告 ID）", &|f: &[u8]| {
            let mut v = f.to_vec();
            v.resize(64, 0);
            v
        }),
        (
            "C: 前置 0x00，pad 到 64 总长（63 有效）",
            &|f: &[u8]| {
                let mut v = vec![0x00];
                v.extend_from_slice(f);
                v.resize(64, 0);
                v
            },
        ),
    ];

    for (name, wrap) in modes {
        println!("== {name}");
        let device = match api.open_path(info.path()) {
            Ok(device) => device,
            Err(error) => {
                println!("   打开失败: {error}");
                continue;
            }
        };
        for (label, bytes) in [
            ("heartbeat", wrap(&heartbeat)),
            ("version  ", wrap(&version)),
        ] {
            match device.write(&bytes) {
                Ok(written) => println!("   写 {label} {} 字节 → Ok({written})", bytes.len()),
                Err(error) => {
                    println!("   写 {label} {} 字节 → Err({error})", bytes.len());
                    continue;
                }
            }
        }
        let mut total = 0;
        for _ in 0..args.reads {
            let mut buffer = [0u8; 128];
            match device.read_timeout(&mut buffer, args.read_timeout_ms) {
                Ok(0) => {}
                Ok(read) => {
                    total += read;
                    println!("   读 {read} 字节: {}", hex(&buffer[..read]));
                }
                Err(error) => {
                    println!("   读失败: {error}");
                    break;
                }
            }
        }
        if total == 0 {
            println!("   无回音（{} × {} ms）", args.reads, args.read_timeout_ms);
        }
        println!();
    }
}
