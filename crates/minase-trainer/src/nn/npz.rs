//! 圧縮なしNPZとNPY 1.0の配列を、値やバイト順を変えずに読み書きする。

use super::{Error, Result};
use std::collections::BTreeMap;

/// 行優先配列のdtype、形状とそのままのデータバイト列。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Array {
    /// NPYの数値dtype。例は `<f4`、`>i2`、`|u1`。オブジェクト型は扱わない。
    pub dtype: String,
    /// 各軸の長さ。空ならスカラー。
    pub shape: Vec<usize>,
    /// dtypeで指定したバイト順の連続した要素。
    pub data: Vec<u8>,
}
impl Array {
    /// 数値dtypeと、形状から求めたデータ長だけを検査する。
    pub fn validate(&self) -> Result<()> {
        let item_size = match self.dtype.as_bytes() {
            [b'|' | b'<' | b'>' | b'=', b'b' | b'i' | b'u', b'1'] => 1,
            [b'<' | b'>' | b'=', b'i' | b'u' | b'f', b'2'] => 2,
            [b'<' | b'>' | b'=', b'i' | b'u' | b'f', b'4'] => 4,
            [b'<' | b'>' | b'=', b'i' | b'u' | b'f' | b'c', b'8'] => 8,
            [b'<' | b'>' | b'=', b'c', b'1', b'6'] => 16,
            _ => return Err(Error::Format("unsupported numeric NPY dtype")),
        };
        let length = self
            .shape
            .iter()
            .try_fold(item_size, |size: usize, &dimension| {
                size.checked_mul(dimension)
            });
        if length != Some(self.data.len()) {
            return Err(Error::Format("NPY array length"));
        }
        Ok(())
    }
}

/// 指定したASCIIキーの順序で、配列をNPZへ符号化する。値の変換や有限性の検査はしない。
pub fn encode(arrays: &[(&str, Array)]) -> Result<Vec<u8>> {
    let mut zip = Vec::new();
    let mut central = Vec::new();
    let count = u16::try_from(arrays.len()).map_err(|_| Error::Format("too many NPZ arrays"))?;
    let mut keys = std::collections::HashSet::new();
    for (key, value) in arrays {
        if key.is_empty()
            || !key.is_ascii()
            || key.contains(['/', '\\', '\0'])
            || !keys.insert(*key)
        {
            return Err(Error::Format("invalid or duplicate NPZ key"));
        }
        let name = format!("{key}.npy");
        let array = encode_npy(value)?;
        let offset = u32::try_from(zip.len()).map_err(|_| Error::Format("NPZ size exceeds u32"))?;
        let size = u32::try_from(array.len()).map_err(|_| Error::Format("NPY size exceeds u32"))?;
        let checksum = crc32(&array);
        zip.extend(b"PK\x03\x04");
        for value in [20_u16, 0, 0, 0, 33] {
            zip.extend(value.to_le_bytes());
        }
        for value in [checksum, size, size] {
            zip.extend(value.to_le_bytes());
        }
        zip.extend(
            (u16::try_from(name.len()).map_err(|_| Error::Format("NPZ key too long"))?)
                .to_le_bytes(),
        );
        zip.extend(0_u16.to_le_bytes());
        zip.extend(name.as_bytes());
        zip.extend(array);
        central.extend(b"PK\x01\x02");
        for value in [20_u16, 20, 0, 0, 0, 33] {
            central.extend(value.to_le_bytes());
        }
        for value in [checksum, size, size] {
            central.extend(value.to_le_bytes());
        }
        for value in [
            u16::try_from(name.len()).map_err(|_| Error::Format("NPZ key too long"))?,
            0,
            0,
            0,
            0,
        ] {
            central.extend(value.to_le_bytes());
        }
        central.extend(0_u32.to_le_bytes());
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let offset = u32::try_from(zip.len()).map_err(|_| Error::Format("NPZ size exceeds u32"))?;
    let size =
        u32::try_from(central.len()).map_err(|_| Error::Format("NPZ directory exceeds u32"))?;
    zip.extend(central);
    zip.extend(b"PK\x05\x06");
    for value in [0_u16, 0, count, count] {
        zip.extend(value.to_le_bytes());
    }
    zip.extend(size.to_le_bytes());
    zip.extend(offset.to_le_bytes());
    zip.extend(0_u16.to_le_bytes());
    Ok(zip)
}

/// NPY 1.0へ符号化する。NumPyと同じ64バイト境界へヘッダを整列する。
pub fn encode_npy(value: &Array) -> Result<Vec<u8>> {
    value.validate()?;
    let shape = value
        .shape
        .iter()
        .map(|n| format!("{n},"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut header = format!(
        "{{'descr': '{}', 'fortran_order': False, 'shape': ({shape}), }}",
        value.dtype
    );
    while !(10 + header.len() + 1).is_multiple_of(64) {
        header.push(' ');
    }
    header.push('\n');
    let mut bytes = b"\x93NUMPY\x01\x00".to_vec();
    bytes.extend(
        u16::try_from(header.len())
            .map_err(|_| Error::Format("NPY header too long"))?
            .to_le_bytes(),
    );
    bytes.extend(header.as_bytes());
    bytes.extend(&value.data);
    Ok(bytes)
}

/// ZIP64のローカルサイズも読み、検査和、dtype、形状と配列長を検査する。
pub fn decode(bytes: &[u8]) -> Result<BTreeMap<String, Array>> {
    decode_members(bytes, |_| true)
}

/// 指定キーの配列だけを読む。ほかのメンバーはZIPの検査和を検査して読み飛ばす。
pub fn decode_selected(bytes: &[u8], keys: &[&str]) -> Result<BTreeMap<String, Array>> {
    let names: Vec<_> = keys.iter().map(|key| format!("{key}.npy")).collect();
    decode_members(bytes, |name| names.iter().any(|key| key.as_bytes() == name))
}

fn decode_members(
    bytes: &[u8],
    include: impl Fn(&[u8]) -> bool,
) -> Result<BTreeMap<String, Array>> {
    let mut reader = Reader::new(bytes);
    let mut arrays = BTreeMap::new();
    let mut members = 0_u16;
    while reader.remaining().starts_with(b"PK\x03\x04") {
        members = members
            .checked_add(1)
            .ok_or(Error::Format("too many NPZ members"))?;
        let header = reader.take(30)?;
        let version = u16_at(header, 4);
        if ![10, 20, 45].contains(&version) || u16_at(header, 6) != 0 || u16_at(header, 8) != 0 {
            return Err(Error::Format("ZIP must be stored with no flags"));
        }
        let crc = u32_at(header, 14);
        let mut compressed = u64::from(u32_at(header, 18));
        let mut size = u64::from(u32_at(header, 22));
        let name = reader.take(u16_at(header, 26) as usize)?;
        let extra = reader.take(u16_at(header, 28) as usize)?;
        if compressed == u64::from(u32::MAX) || size == u64::from(u32::MAX) {
            let mut extras = Reader::new(extra);
            let mut found = false;
            while !extras.remaining().is_empty() {
                let h = extras.take(4)?;
                let field = extras.take(u16_at(h, 2) as usize)?;
                if u16_at(h, 0) == 1 {
                    if found {
                        return Err(Error::Format("duplicate ZIP64 size"));
                    }
                    let mut field = Reader::new(field);
                    if size == u64::from(u32::MAX) {
                        size = field.u64()?;
                    }
                    if compressed == u64::from(u32::MAX) {
                        compressed = field.u64()?;
                    }
                    found = true;
                }
            }
            if !found {
                return Err(Error::Format("missing ZIP64 size"));
            }
        }
        if compressed != size {
            return Err(Error::Format("stored ZIP sizes differ"));
        }
        let data = reader.take(usize::try_from(size).map_err(|_| Error::Format("ZIP size"))?)?;
        if crc32(data) != crc {
            return Err(Error::Format("ZIP CRC32"));
        }
        if !include(name) {
            continue;
        }
        let key = std::str::from_utf8(name)
            .map_err(|_| Error::Format("NPZ key encoding"))?
            .strip_suffix(".npy")
            .ok_or(Error::Format("NPZ member is not NPY"))?;
        if key.is_empty()
            || !key.is_ascii()
            || key.contains(['/', '\\', '\0'])
            || arrays.insert(key.to_owned(), decode_npy(data)?).is_some()
        {
            return Err(Error::Format("invalid or duplicate NPZ key"));
        }
    }
    // 読む内容はローカルヘッダで確定するが、切断されたzipは受け入れない。
    let central_offset = bytes.len() - reader.remaining().len();
    for _ in 0..members {
        let header = reader.take(46)?;
        if &header[..4] != b"PK\x01\x02" || u16_at(header, 8) != 0 || u16_at(header, 10) != 0 {
            return Err(Error::Format("ZIP central directory"));
        }
        reader.take(
            u16_at(header, 28) as usize + u16_at(header, 30) as usize + u16_at(header, 32) as usize,
        )?;
    }
    let central_size = bytes.len() - reader.remaining().len() - central_offset;
    let end = reader.take(22)?;
    if &end[..4] != b"PK\x05\x06"
        || u16_at(end, 4) != 0
        || u16_at(end, 6) != 0
        || u16_at(end, 8) != members
        || u16_at(end, 10) != members
        || u32_at(end, 12) as usize != central_size
        || u32_at(end, 16) as usize != central_offset
    {
        return Err(Error::Format("ZIP end directory"));
    }
    reader.take(u16_at(end, 20) as usize)?;
    if !reader.remaining().is_empty() {
        return Err(Error::Format("trailing ZIP bytes"));
    }
    Ok(arrays)
}

/// NPY 1.0の数値配列を検証し、値を変えずに復号する。
pub fn decode_npy(bytes: &[u8]) -> Result<Array> {
    let mut r = Reader::new(bytes);
    if r.take(8)? != b"\x93NUMPY\x01\x00" {
        return Err(Error::Format("NPY version must be 1.0"));
    }
    let length = u16_at(r.take(2)?, 0) as usize;
    let header =
        std::str::from_utf8(r.take(length)?).map_err(|_| Error::Format("NPY header encoding"))?;
    if !header.is_ascii() || !header.ends_with('\n') {
        return Err(Error::Format("NPY header newline"));
    }
    // NPYの限定した辞書文法だけを読み、Pythonコードとしては実行しない。
    let mut remaining = header;
    token(&mut remaining, "{")?;
    let mut seen = [false; 3];
    let mut dtype = String::new();
    let mut shape = Vec::new();
    while !remaining.trim_start().starts_with('}') {
        let key = string(&mut remaining)?;
        token(&mut remaining, ":")?;
        let index = match key {
            "descr" => {
                dtype = string(&mut remaining)?.to_owned();
                0
            }
            "fortran_order" => {
                token(&mut remaining, "False")?;
                1
            }
            "shape" => {
                token(&mut remaining, "(")?;
                while !remaining.trim_start().starts_with(')') {
                    remaining = remaining.trim_start();
                    let end = remaining
                        .find(|c: char| !c.is_ascii_digit())
                        .ok_or(Error::Format("NPY dimension"))?;
                    let dimension = remaining[..end]
                        .parse::<usize>()
                        .map_err(|_| Error::Format("NPY dimension"))?;
                    shape.push(dimension);
                    remaining = &remaining[end..];
                    if remaining.trim_start().starts_with(')') && shape.len() > 1 {
                        break;
                    }
                    token(&mut remaining, ",")?;
                }
                token(&mut remaining, ")")?;
                2
            }
            _ => return Err(Error::Format("unexpected NPY field")),
        };
        if seen[index] {
            return Err(Error::Format("duplicate NPY field"));
        }
        seen[index] = true;
        if remaining.trim_start().starts_with('}') {
            break;
        }
        token(&mut remaining, ",")?;
    }
    token(&mut remaining, "}")?;
    if seen.contains(&false) || !remaining.trim().is_empty() {
        return Err(Error::Format("missing or trailing NPY fields"));
    }
    let array = Array {
        dtype,
        shape,
        data: r.remaining().to_vec(),
    };
    array.validate()?;
    Ok(array)
}

fn token(input: &mut &str, expected: &str) -> Result<()> {
    *input = input
        .trim_start()
        .strip_prefix(expected)
        .ok_or(Error::Format("NPY header syntax"))?;
    Ok(())
}
fn string<'a>(input: &mut &'a str) -> Result<&'a str> {
    let text = input.trim_start();
    let quote = text.chars().next().ok_or(Error::Format("NPY string"))?;
    if !['\'', '"'].contains(&quote) {
        return Err(Error::Format("NPY quoted string"));
    }
    let text = &text[1..];
    let end = text.find(quote).ok_or(Error::Format("NPY closing quote"))?;
    *input = &text[end + 1..];
    Ok(&text[..end])
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("checked header"),
    )
}
struct Reader<'a> {
    bytes: &'a [u8],
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
    fn remaining(&self) -> &'a [u8] {
        self.bytes
    }
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        if len > self.bytes.len() {
            return Err(Error::Format("truncated input"));
        }
        let (value, rest) = self.bytes.split_at(len);
        self.bytes = rest;
        Ok(value)
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("checked length"),
        ))
    }
}
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npy_rejects_invalid_order_shape_version_and_truncation() {
        fn array(header: &str) -> Vec<u8> {
            let mut bytes = b"\x93NUMPY\x01\x00".to_vec();
            bytes.extend((header.len() as u16).to_le_bytes());
            bytes.extend(header.as_bytes());
            bytes.extend([0; 12]);
            bytes
        }
        let header = "{'descr': '<f4', 'fortran_order': False, 'shape': (3,), }\n";
        let bytes = array(header);
        assert_eq!(decode_npy(&bytes).unwrap().data, [0; 12]);
        for wrong in [
            header.replace("<f4", "|O"),
            header.replace("False", "True"),
            header.replace("(3,)", "(4,)"),
            header.replace("'shape'", "'unknown'"),
        ] {
            assert!(decode_npy(&array(&wrong)).is_err());
        }
        for cut in [0, 7, 9, bytes.len() - 1] {
            assert!(decode_npy(&bytes[..cut]).is_err());
        }
        let mut wrong = bytes;
        wrong[6] = 2;
        assert!(decode_npy(&wrong).is_err());
    }

    #[test]
    fn selected_arrays_skip_unrelated_payloads_but_verify_their_checksums() {
        let mut bytes = encode(&[
            (
                "value",
                Array {
                    dtype: "<f4".into(),
                    shape: vec![1],
                    data: (-0.0_f32).to_le_bytes().to_vec(),
                },
            ),
            (
                "unused",
                Array {
                    dtype: "|u1".into(),
                    shape: vec![1],
                    data: vec![17],
                },
            ),
        ])
        .unwrap();
        // 任意の余分なメンバーを許す契約。NPYとして読めないペイロードも読み飛ばす。
        let local = bytes.windows(4).rposition(|w| w == b"PK\x03\x04").unwrap();
        let data_start = local + 30 + u16_at(&bytes, local + 26) as usize;
        let data_len = u32_at(&bytes, local + 22) as usize;
        bytes[data_start] = 0;
        let checksum = crc32(&bytes[data_start..data_start + data_len]);
        bytes[local + 14..local + 18].copy_from_slice(&checksum.to_le_bytes());
        let central = bytes.windows(4).rposition(|w| w == b"PK\x01\x02").unwrap();
        bytes[central + 16..central + 20].copy_from_slice(&checksum.to_le_bytes());
        let selected = decode_selected(&bytes, &["value"]).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected["value"].data, (-0.0_f32).to_le_bytes());
        assert!(decode(&bytes).is_err());
        bytes[data_start + data_len - 1] ^= 1;
        assert!(decode_selected(&bytes, &["value"]).is_err());
    }
}
