//! tataraの符号付きLEB128区画。識別子は17バイトの`COMPRESSED_LEB128`。
//! 続くu32は圧縮バイト数で、値の個数以上、3倍以下を受理する。
//! 値はi64として10バイトまで復号し、i16の範囲を検査する。
//! 個数不足、途中切れ、桁あふれ、余りのバイトは別々に拒否する。

use super::{FormatError, Reader};
const MAGIC: &[u8] = b"COMPRESSED_LEB128";

pub(super) fn encode(bytes: &mut Vec<u8>, values: &[i16]) {
    bytes.extend(MAGIC);
    let length_at = bytes.len();
    bytes.extend([0; 4]);
    let start = bytes.len();
    for &value in values {
        let mut value = i64::from(value);
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            let sign = byte & 0x40 != 0;
            if (value == 0 && !sign) || (value == -1 && sign) {
                bytes.push(byte);
                break;
            }
            bytes.push(byte | 0x80);
        }
    }
    let length = (bytes.len() - start) as u32;
    bytes[length_at..length_at + 4].copy_from_slice(&length.to_le_bytes());
}

pub(super) fn decode(
    reader: &mut Reader<'_>,
    count: usize,
    section: &'static str,
) -> Result<Vec<i16>, FormatError> {
    if reader.take(MAGIC.len(), section)? != MAGIC {
        return Err(FormatError::LebMagic { section });
    }
    let size = reader.u32(section)? as usize;
    if size < count || size > 3 * count {
        return Err(FormatError::LebSize {
            section,
            actual: size,
            count,
        });
    }
    let mut block = Reader::new(reader.take(size, section)?);
    let mut values = Vec::with_capacity(count);
    for index in 0..count {
        if block.bytes.is_empty() {
            return Err(FormatError::LebCount {
                section,
                expected: count,
                actual: index,
            });
        }
        let mut value = 0_i64;
        for byte_index in 0..10 {
            let byte = block.take(1, section)?[0];
            let shift = byte_index * 7;
            if byte_index == 9 && byte != 0 && byte != 0x7f {
                return Err(FormatError::LebOverflow { section });
            }
            value |= i64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                if shift + 7 < 64 && byte & 0x40 != 0 {
                    value |= !0_i64 << (shift + 7);
                }
                values.push(
                    i16::try_from(value).map_err(|_| FormatError::LebRange { section, value })?,
                );
                break;
            }
        }
    }
    if !block.bytes.is_empty() {
        return Err(FormatError::LebTrailing {
            section,
            count: block.bytes.len(),
        });
    }
    Ok(values)
}
