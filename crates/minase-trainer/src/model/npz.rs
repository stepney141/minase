use super::{Error, Result, validate_initial};
use minase::eval::features::FEATURE_COUNT;
use std::path::Path;

/// 2端点のf32配列をmiddlegame、endgameの順に、圧縮なしのNPZへ符号化する。
pub fn encode_pst_npz(initial: &[[f32; 2]]) -> Result<Vec<u8>> {
    validate_initial(initial)?;
    let mut zip = Vec::new();
    let mut central = Vec::new();
    for (endpoint, name) in ["middlegame.npy", "endgame.npy"].iter().enumerate() {
        let mut header = "{'descr': '<f4', 'fortran_order': False, 'shape': (13680,), }".to_owned();
        while !(10 + header.len() + 1).is_multiple_of(64) {
            header.push(' ');
        }
        header.push('\n');
        let mut array = b"\x93NUMPY\x01\x00".to_vec();
        array.extend((header.len() as u16).to_le_bytes());
        array.extend(header.as_bytes());
        for row in initial {
            array.extend(row[endpoint].to_le_bytes());
        }
        let offset = zip.len() as u32;
        let size = array.len() as u32;
        let checksum = crc32(&array);
        zip.extend(b"PK\x03\x04");
        for value in [20_u16, 0, 0, 0, 33] {
            zip.extend(value.to_le_bytes());
        }
        for value in [checksum, size, size] {
            zip.extend(value.to_le_bytes());
        }
        zip.extend((name.len() as u16).to_le_bytes());
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
        for value in [name.len() as u16, 0, 0, 0, 0] {
            central.extend(value.to_le_bytes());
        }
        central.extend(0_u32.to_le_bytes());
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let offset = zip.len() as u32;
    let size = central.len() as u32;
    zip.extend(central);
    zip.extend(b"PK\x05\x06");
    for value in [0_u16, 0, 2, 2] {
        zip.extend(value.to_le_bytes());
    }
    zip.extend(size.to_le_bytes());
    zip.extend(offset.to_le_bytes());
    zip.extend(0_u16.to_le_bytes());
    Ok(zip)
}

/// 圧縮なしnpzのmiddlegame.npyとendgame.npyを読む。NPY 1.0の<f4[13680]専用で、他のメンバーは読み飛ばす。
///
/// NumPyが小さいファイルにも使うZIP64ローカルサイズを扱う。暗号化、データ
/// デスクリプタ、圧縮、および異なるデータ型・配列形状・配列順は拒否する。
/// 2端点の値が有限で保存範囲内にあり、左右の鏡映対で一致することも検査する。
pub fn load_initial(path: impl AsRef<Path>) -> Result<Vec<[f32; 2]>> {
    let initial = parse(&std::fs::read(path)?)?;
    validate_initial(&initial)?;
    Ok(initial)
}

fn parse(bytes: &[u8]) -> Result<Vec<[f32; 2]>> {
    let mut reader = Reader::new(bytes);
    let mut endpoints: [Option<Vec<f32>>; 2] = [None, None];
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
        // J75のnpzはFMの配列（V、a、mask）も持つので、2端点以外のメンバーは読み飛ばす。
        let index = match name {
            b"middlegame.npy" => 0,
            b"endgame.npy" => 1,
            _ => continue,
        };
        if endpoints[index].is_some() {
            return Err(Error::Format("duplicate NPZ endpoint"));
        }
        endpoints[index] = Some(npy(data)?);
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
    let [Some(mg), Some(eg)] = endpoints else {
        return Err(Error::Format("missing NPZ endpoint"));
    };
    Ok(mg.into_iter().zip(eg).map(|(mg, eg)| [mg, eg]).collect())
}

fn npy(bytes: &[u8]) -> Result<Vec<f32>> {
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
    while !remaining.trim_start().starts_with('}') {
        let key = string(&mut remaining)?;
        token(&mut remaining, ":")?;
        let index = match key {
            "descr" => {
                if string(&mut remaining)? != "<f4" {
                    return Err(Error::Format("NPY dtype"));
                }
                0
            }
            "fortran_order" => {
                token(&mut remaining, "False")?;
                1
            }
            "shape" => {
                for value in ["(", "13680", ",", ")"] {
                    token(&mut remaining, value)?;
                }
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
    let data = r.take(FEATURE_COUNT * 4)?;
    if !r.remaining().is_empty() {
        return Err(Error::Format("NPY array length"));
    }
    Ok(data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&b| f32::from_le_bytes(b))
        .collect())
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
pub(super) struct Reader<'a> {
    bytes: &'a [u8],
}
impl<'a> Reader<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
    pub(super) fn remaining(&self) -> &'a [u8] {
        self.bytes
    }
    pub(super) fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        if len > self.bytes.len() {
            return Err(Error::Format("truncated input"));
        }
        let (value, rest) = self.bytes.split_at(len);
        self.bytes = rest;
        Ok(value)
    }
    pub(super) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("checked length"),
        ))
    }
    pub(super) fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("checked length"),
        ))
    }
    pub(super) fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    pub(super) fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.u64()?))
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
    fn npy_rejects_dtype_order_shape_version_and_truncation() {
        fn array(header: &str) -> Vec<u8> {
            let mut b = b"\x93NUMPY\x01\x00".to_vec();
            b.extend((header.len() as u16).to_le_bytes());
            b.extend(header.as_bytes());
            b.extend(vec![0; FEATURE_COUNT * 4]);
            b
        }
        let header = "{'descr': '<f4', 'fortran_order': False, 'shape': (13680,), }\n";
        let bytes = array(header);
        assert_eq!(npy(&bytes).unwrap(), vec![0.0; FEATURE_COUNT]);
        for wrong in [
            header.replace("<f4", ">f4"),
            header.replace("False", "True"),
            header.replace("13680", "13681"),
            header.replace("'shape'", "'unknown'"),
        ] {
            assert!(npy(&array(&wrong)).is_err());
        }
        for cut in [0, 7, 9, bytes.len() - 1] {
            assert!(npy(&bytes[..cut]).is_err());
        }
        let mut wrong = bytes;
        wrong[6] = 2;
        assert!(npy(&wrong).is_err());
    }
}
