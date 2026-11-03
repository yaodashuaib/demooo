//! CRC-16/CCITT-FALSE：poly 0x1021、init 0xFFFF、不反转、xorout 0。

/// 按线上字节顺序计算；协议要求 CRC 覆盖 `SyncFrame | CommandID | FlagsSeq |
/// FrameLength | Payload`，不含 CRC 自身，因此调用方传入不含 CRC 的切片。
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
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

#[cfg(test)]
mod tests {
    use super::crc16;

    /// CRC-16/CCITT-FALSE 的标准校验值：输入 "123456789" 得 0x29B1。
    /// 这条向量能一次性排掉 poly/init/reflect 三种写错方式。
    #[test]
    fn known_check_value() {
        assert_eq!(crc16(b"123456789"), 0x29B1);
    }

    #[test]
    fn empty_is_init() {
        assert_eq!(crc16(&[]), 0xFFFF);
    }

    #[test]
    fn is_position_sensitive() {
        assert_ne!(crc16(&[0x01, 0x02]), crc16(&[0x02, 0x01]));
    }
}
