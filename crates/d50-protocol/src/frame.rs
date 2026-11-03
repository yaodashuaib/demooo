//! 应用帧编解码、字节流组帧与多帧重组。
//!
//! 帧布局：
//! `SyncFrame(2)=0x55 0xD5 | CommandID(1) | FlagsSeq(1) | FrameLength(2,LE) | Payload(N) | CRC16(2,LE)`
//! `FrameLength` 是整帧字节数，所以 `PayloadLength = FrameLength - 8`。

use crate::crc::crc16;
use thiserror::Error;

pub const SYNC: [u8; 2] = [0x55, 0xD5];
pub const HEADER_LEN: usize = 6;
pub const TRAILER_LEN: usize = 2;
/// 空 Payload 帧的长度：ACK 就是这一种。
pub const MIN_FRAME_LEN: usize = HEADER_LEN + TRAILER_LEN;
pub const MAX_FRAME_LEN: usize = 65535;
pub const MAX_PAYLOAD_LEN: usize = MAX_FRAME_LEN - MIN_FRAME_LEN;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    Complete = 0,
    First = 1,
    Middle = 2,
    End = 3,
}

impl FrameType {
    /// 从 `FlagsSeq` 高两位取帧类型；低六位是序号。
    pub fn from_flags(flags: u8) -> Self {
        match (flags >> 6) & 0x03 {
            0 => Self::Complete,
            1 => Self::First,
            2 => Self::Middle,
            _ => Self::End,
        }
    }

    pub fn bits(self) -> u8 {
        (self as u8) << 6
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProtocolError {
    /// 帧不完整、校验失败或字段越界；协议要求这类帧静默丢弃。
    #[error("INVALID_FRAME")]
    Frame,
    /// 调用方参数非法（序号、长度、Payload 内容）。
    #[error("INVALID_PARAMETER")]
    Parameter,
    /// 多帧序号或 CommandID 不连续。
    #[error("SEQUENCE_ERROR")]
    Sequence,
    #[error("PAYLOAD_TOO_LARGE")]
    TooLarge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub command: u8,
    pub frame_type: FrameType,
    pub seq: u8,
    pub payload: Vec<u8>,
}

impl Frame {
    /// ACK 的定义是「同 CommandID、同 FlagsSeq、空 Payload」。
    pub fn is_ack(&self) -> bool {
        self.payload.is_empty()
    }

    /// Response 是带非空 Payload 的同 CommandID 帧，不再另发 ACK。
    pub fn is_response(&self) -> bool {
        !self.payload.is_empty()
    }
}

/// 把一帧序列化成线上字节。序号高位和帧类型由本函数负责拼装。
pub fn encode(
    command: u8,
    frame_type: FrameType,
    seq: u8,
    payload: &[u8],
) -> Result<Vec<u8>, ProtocolError> {
    let len = MIN_FRAME_LEN + payload.len();
    if payload.len() > MAX_PAYLOAD_LEN || len > MAX_FRAME_LEN {
        return Err(ProtocolError::TooLarge);
    }
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(&SYNC);
    out.push(command);
    out.push(frame_type.bits() | (seq & 0x3F));
    out.extend_from_slice(&(len as u16).to_le_bytes());
    out.extend_from_slice(payload);
    let crc = crc16(&out);
    out.extend_from_slice(&crc.to_le_bytes());
    Ok(out)
}

/// 解一整帧；长度必须与 `FrameLength` 完全一致，多一个字节都不算。
pub fn decode(buf: &[u8]) -> Result<Frame, ProtocolError> {
    if buf.len() < MIN_FRAME_LEN || buf[..2] != SYNC {
        return Err(ProtocolError::Frame);
    }
    let len = usize::from(u16::from_le_bytes([buf[4], buf[5]]));
    if len != buf.len() || len < MIN_FRAME_LEN {
        return Err(ProtocolError::Frame);
    }
    let crc = u16::from_le_bytes([buf[len - 2], buf[len - 1]]);
    if crc16(&buf[..len - 2]) != crc {
        return Err(ProtocolError::Frame);
    }
    Ok(Frame {
        command: buf[2],
        frame_type: FrameType::from_flags(buf[3]),
        seq: buf[3] & 0x3F,
        payload: buf[HEADER_LEN..len - TRAILER_LEN].to_vec(),
    })
}

/// 字节流组帧器：把任意切分的 report 拼回应用帧。
///
/// HID report 边界与应用帧边界无关，所以这里只认 `SyncFrame + FrameLength`。
/// 坏帧按协议要求静默丢弃——但只丢一个字节再重新找同步头，避免把后面的好帧
/// 一起吞掉。
#[derive(Default)]
pub struct Scanner {
    buf: Vec<u8>,
}

impl Scanner {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn buffered(&self) -> usize {
        self.buf.len()
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub fn next_frame(&mut self) -> Option<Frame> {
        loop {
            let Some(start) = self.find_sync() else {
                // 没有同步头时只保留可能是半个同步头的末字节。
                let keep = self.buf.last().copied() == Some(SYNC[0]);
                self.buf.clear();
                if keep {
                    self.buf.push(SYNC[0]);
                }
                return None;
            };
            if start > 0 {
                self.buf.drain(..start);
            }
            if self.buf.len() < HEADER_LEN {
                return None;
            }
            let len = usize::from(u16::from_le_bytes([self.buf[4], self.buf[5]]));
            if len < MIN_FRAME_LEN {
                // FrameLength 本身非法：丢掉这个同步头，从下一个字节重新找。
                self.buf.drain(..1);
                continue;
            }
            if self.buf.len() < len {
                return None;
            }
            match decode(&self.buf[..len]) {
                Ok(frame) => {
                    self.buf.drain(..len);
                    return Some(frame);
                }
                Err(_) => {
                    self.buf.drain(..1);
                }
            }
        }
    }

    fn find_sync(&self) -> Option<usize> {
        if self.buf.len() < 2 {
            return None;
        }
        (0..self.buf.len() - 1).find(|&i| self.buf[i] == SYNC[0] && self.buf[i + 1] == SYNC[1])
    }
}

/// 多帧重组：同一 CommandID、序号从 FIRST=0 起逐个 +1，满 63 回 0。
#[derive(Default)]
pub struct Reassembler {
    active: Option<(u8, u8, Vec<u8>)>,
}

impl Reassembler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_assembling(&self) -> bool {
        self.active.is_some()
    }

    /// 返回拼装完成的 Payload；未完成时返回 `None`。
    ///
    /// 新的 `FIRST/Seq=0` 会放弃旧 Transfer（协议允许发送方从头重来）。
    /// 序号跳变或 CommandID 变化视为协议错误，并丢弃当前进度。
    pub fn push(&mut self, frame: Frame) -> Result<Option<(u8, Vec<u8>)>, ProtocolError> {
        match frame.frame_type {
            FrameType::Complete => {
                if self.active.take().is_some() {
                    return Err(ProtocolError::Sequence);
                }
                Ok(Some((frame.command, frame.payload)))
            }
            FrameType::First => {
                if frame.seq != 0 {
                    return Err(ProtocolError::Sequence);
                }
                self.active = Some((frame.command, frame.seq, frame.payload));
                Ok(None)
            }
            FrameType::Middle | FrameType::End => {
                let Some((command, seq, mut buf)) = self.active.take() else {
                    return Err(ProtocolError::Sequence);
                };
                if command != frame.command || frame.seq != seq.wrapping_add(1) & 0x3F {
                    return Err(ProtocolError::Sequence);
                }
                buf.extend_from_slice(&frame.payload);
                if frame.frame_type == FrameType::End {
                    Ok(Some((command, buf)))
                } else {
                    self.active = Some((command, frame.seq, buf));
                    Ok(None)
                }
            }
        }
    }
}

/// 把一段业务 Payload 切成若干应用帧的视图（不复制字节）。
///
/// 单帧放得下就用 `COMPLETE`；放不下才用 `FIRST/MIDDLE/END`。协议明确要求
/// 同一条命令不能永久绑定某一种方式，所以这是按长度决定的。
pub fn split_payload(
    payload: &[u8],
    max_payload: usize,
) -> Result<Vec<(FrameType, u8, &[u8])>, ProtocolError> {
    if max_payload == 0 || max_payload > MAX_PAYLOAD_LEN {
        return Err(ProtocolError::Parameter);
    }
    if payload.len() <= max_payload {
        return Ok(vec![(FrameType::Complete, 0, payload)]);
    }
    let mut out = Vec::new();
    let mut seq: u8 = 0;
    for (index, chunk) in payload.chunks(max_payload).enumerate() {
        let total = payload.len().div_ceil(max_payload);
        let frame_type = if index == 0 {
            FrameType::First
        } else if index + 1 == total {
            FrameType::End
        } else {
            FrameType::Middle
        };
        out.push((frame_type, seq, chunk));
        seq = seq.wrapping_add(1) & 0x3F;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(command: u8, frame_type: FrameType, seq: u8, payload: &[u8]) -> Vec<u8> {
        encode(command, frame_type, seq, payload).unwrap()
    }

    #[test]
    fn round_trip_complete() {
        let bytes = frame(0x41, FrameType::Complete, 0, &[1, 2, 3]);
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.command, 0x41);
        assert_eq!(decoded.frame_type, FrameType::Complete);
        assert_eq!(decoded.seq, 0);
        assert_eq!(decoded.payload, vec![1, 2, 3]);
    }

    #[test]
    fn frame_length_is_total_bytes() {
        let bytes = frame(0x22, FrameType::Complete, 0, &[9; 10]);
        assert_eq!(bytes.len(), 18);
        assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), 18);
    }

    #[test]
    fn ack_is_empty_payload() {
        let bytes = frame(0x01, FrameType::Complete, 0, &[]);
        assert_eq!(bytes.len(), MIN_FRAME_LEN);
        let decoded = decode(&bytes).unwrap();
        assert!(decoded.is_ack());
        assert!(!decoded.is_response());
    }

    #[test]
    fn flags_seq_packs_type_and_sequence() {
        let bytes = frame(0x20, FrameType::Middle, 5, &[0]);
        assert_eq!(bytes[3], 0x80 | 5);
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.frame_type, FrameType::Middle);
        assert_eq!(decoded.seq, 5);
    }

    #[test]
    fn corrupted_crc_is_rejected() {
        let mut bytes = frame(0x01, FrameType::Complete, 0, &[1, 2]);
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        assert_eq!(decode(&bytes), Err(ProtocolError::Frame));
    }

    #[test]
    fn trailing_byte_is_rejected() {
        let mut bytes = frame(0x01, FrameType::Complete, 0, &[]);
        bytes.push(0);
        assert_eq!(decode(&bytes), Err(ProtocolError::Frame));
    }

    #[test]
    fn empty_input_is_rejected() {
        assert_eq!(decode(&[]), Err(ProtocolError::Frame));
    }

    #[test]
    fn scanner_handles_every_split_point() {
        let bytes = frame(0x22, FrameType::Complete, 0, &[7; 20]);
        for split in 0..=bytes.len() {
            let mut scanner = Scanner::new();
            scanner.feed(&bytes[..split]);
            let first = scanner.next_frame();
            scanner.feed(&bytes[split..]);
            let second = scanner.next_frame();
            let got = first.or(second).expect("frame must survive split");
            assert_eq!(got.payload, vec![7; 20], "split at {split}");
        }
    }

    #[test]
    fn scanner_byte_by_byte() {
        let bytes = frame(0x21, FrameType::Complete, 0, &[1, 2, 3, 4]);
        let mut scanner = Scanner::new();
        let mut got = None;
        for byte in &bytes {
            scanner.feed(&[*byte]);
            if let Some(f) = scanner.next_frame() {
                got = Some(f);
            }
        }
        assert_eq!(got.unwrap().command, 0x21);
    }

    #[test]
    fn scanner_skips_garbage_and_bad_frames() {
        let mut stream = vec![0x00, 0x11, 0x22];
        let mut bad = frame(0x01, FrameType::Complete, 0, &[1]);
        bad[2] ^= 0xFF; // 破坏 CRC
        stream.extend_from_slice(&bad);
        stream.extend_from_slice(&frame(0x02, FrameType::Complete, 0, &[5]));
        let mut scanner = Scanner::new();
        scanner.feed(&stream);
        let got = scanner.next_frame().expect("good frame after garbage");
        assert_eq!(got.command, 0x02);
        assert_eq!(got.payload, vec![5]);
        assert!(scanner.next_frame().is_none());
    }

    #[test]
    fn scanner_keeps_half_sync_head() {
        let mut scanner = Scanner::new();
        scanner.feed(&[0xAA, 0x55]);
        // 先让扫描器跑一轮，它才会丢掉前导垃圾、只留下可能是半个同步头的末字节。
        assert!(scanner.next_frame().is_none());
        assert_eq!(scanner.buffered(), 1);
        scanner.feed(&[0xD5]);
        let bytes = frame(0x00, FrameType::Complete, 0, &[]);
        scanner.feed(&bytes[2..]);
        assert_eq!(scanner.next_frame().unwrap().command, 0x00);
    }

    #[test]
    fn scanner_rejects_impossible_length_and_resyncs() {
        let mut stream = vec![0x55, 0xD5, 0x01, 0x00, 0x02, 0x00];
        stream.extend_from_slice(&frame(0x03, FrameType::Complete, 0, &[9]));
        let mut scanner = Scanner::new();
        scanner.feed(&stream);
        assert_eq!(scanner.next_frame().unwrap().command, 0x03);
    }

    #[test]
    fn split_small_payload_uses_complete() {
        let chunks = split_payload(&[1, 2, 3], 16).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].0, FrameType::Complete);
    }

    #[test]
    fn split_exact_boundary_stays_complete() {
        let chunks = split_payload(&[0; 16], 16).unwrap();
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn split_overflow_uses_first_and_end() {
        let payload: Vec<u8> = (0..33).collect();
        let chunks = split_payload(&payload, 16).unwrap();
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].0, FrameType::First);
        assert_eq!(chunks[1].0, FrameType::Middle);
        assert_eq!(chunks[2].0, FrameType::End);
        assert_eq!(chunks[0].1, 0);
        assert_eq!(chunks[1].1, 1);
        assert_eq!(chunks[2].1, 2);
    }

    #[test]
    fn reassembler_round_trips_multi_frame() {
        let payload: Vec<u8> = (0..50).collect();
        let chunks = split_payload(&payload, 12).unwrap();
        let mut reassembler = Reassembler::new();
        let mut result = None;
        for (frame_type, seq, chunk) in chunks {
            let bytes = frame(0x20, frame_type, seq, chunk);
            let decoded = decode(&bytes).unwrap();
            if let Some(done) = reassembler.push(decoded).unwrap() {
                result = Some(done);
            }
        }
        let (command, body) = result.expect("must complete");
        assert_eq!(command, 0x20);
        assert_eq!(body, payload);
    }

    #[test]
    fn reassembler_sequence_wrap() {
        let payload = vec![0u8; 8 * 70];
        let chunks = split_payload(&payload, 8).unwrap();
        let mut reassembler = Reassembler::new();
        let mut done = None;
        for (frame_type, seq, chunk) in chunks {
            let decoded = decode(&frame(0xE0, frame_type, seq, chunk)).unwrap();
            if let Some(value) = reassembler.push(decoded).unwrap() {
                done = Some(value);
            }
        }
        assert_eq!(done.unwrap().1.len(), payload.len());
    }

    #[test]
    fn reassembler_rejects_out_of_order() {
        let mut reassembler = Reassembler::new();
        let first = decode(&frame(0x20, FrameType::First, 0, &[1])).unwrap();
        assert!(reassembler.push(first).unwrap().is_none());
        let skipped = decode(&frame(0x20, FrameType::End, 2, &[2])).unwrap();
        assert_eq!(reassembler.push(skipped), Err(ProtocolError::Sequence));
        assert!(!reassembler.is_assembling());
    }

    #[test]
    fn reassembler_restarts_on_new_first() {
        let mut reassembler = Reassembler::new();
        reassembler
            .push(decode(&frame(0x20, FrameType::First, 0, &[1])).unwrap())
            .unwrap();
        reassembler
            .push(decode(&frame(0x20, FrameType::First, 0, &[2])).unwrap())
            .unwrap();
        let done = reassembler
            .push(decode(&frame(0x20, FrameType::End, 1, &[3])).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(done.1, vec![2, 3]);
    }

    #[test]
    fn reassembler_rejects_command_change() {
        let mut reassembler = Reassembler::new();
        reassembler
            .push(decode(&frame(0x20, FrameType::First, 0, &[1])).unwrap())
            .unwrap();
        let swapped = decode(&frame(0x21, FrameType::End, 1, &[2])).unwrap();
        assert_eq!(reassembler.push(swapped), Err(ProtocolError::Sequence));
    }

    #[test]
    fn oversize_payload_is_refused() {
        let payload = vec![0u8; MAX_PAYLOAD_LEN + 1];
        assert_eq!(
            encode(0x20, FrameType::Complete, 0, &payload),
            Err(ProtocolError::TooLarge)
        );
    }
}
