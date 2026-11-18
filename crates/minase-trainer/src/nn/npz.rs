//! 圧縮なしNPZとNPY 1.0の配列を、値やバイト順を変えずに読み書きする。
//!
//! # NPZとNPYの配置
//!
//! NPZは配列ごとの`<キー>.npy`を指定順に収めた無圧縮ZIPで、独自の版欄はない。
//! キーは空でない重複しないASCII文字列とし、`/`、`\`、NULを含めない。
//! 各メンバーは30バイトのローカルヘッダ、ファイル名、NPY本体の順に置く。
//! ローカルヘッダの0は`PK\x03\x04`、4は必要版u16の20、6はフラグu16の0、8は圧縮方式u16の0である。
//! 10は時刻u16の0、12は日付u16の33、14はNPY全体のCRC-32のu32である。
//! 18と22はNPY長u32、26はファイル名長u16、28は拡張領域長u16の0である。
//! 末尾には各メンバーの中央ディレクトリと22バイトの終端記録を置き、ZIPの整数はリトルエンディアンとする。
//! 書き手はZIP32を生成し、読み手はローカルヘッダのZIP64サイズも受け付ける。
//! 中央ディレクトリの各項目は46バイトのヘッダとファイル名からなる。
//! ヘッダの0は`PK\x01\x02`、4と6は版u16の20、8と10はフラグと圧縮方式u16の0、12は時刻u16の0、14は日付u16の33である。
//! 16はCRC-32のu32、20と24はNPY長u32、28はファイル名長u16である。
//! 30、32、34、36はそれぞれ拡張領域長、コメント長、ディスク番号、内部属性のu16の0、38は外部属性u32の0である。
//! 42は対応するローカルヘッダのファイル先頭からのオフセットu32である。
//! 終端記録の0は`PK\x05\x06`、4と6はディスク番号u16の0、8と10は配列数u16である。
//! 12は中央ディレクトリのバイト長u32、16はその開始位置u32、20はコメント長u16の0である。
//!
//! NPYの0〜5は`\x93NUMPY`、6と7は版の1と0、8〜9はリトルエンディアンu16のヘッダ文字列長である。
//! 10以降はASCII辞書で、`descr`にdtype、`fortran_order`に`False`、`shape`に各軸長のタプルを書く。
//! 空のタプルはスカラーを表す。
//! 辞書の後ろを空白で埋めてLFで終え、データの開始を64バイト境界へそろえる。
//! データは行優先で、要素のバイト順は`descr`に従い、バイト数は要素幅と各軸長の積に一致させる。
//! 受理するdtypeは[`Array::validate`]に列挙し、ここでは値の変換や有限性の検査は行わない。

use super::{Error, Result};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
};

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
        validate_layout(&self.dtype, &self.shape, self.data.len())
    }
}

fn validate_layout(dtype: &str, shape: &[usize], data_len: usize) -> Result<()> {
    let item_size = match dtype.as_bytes() {
        [b'|' | b'<' | b'>' | b'=', b'b' | b'i' | b'u', b'1'] => 1,
        [b'<' | b'>' | b'=', b'i' | b'u' | b'f', b'2'] => 2,
        [b'<' | b'>' | b'=', b'i' | b'u' | b'f', b'4'] => 4,
        [b'<' | b'>' | b'=', b'i' | b'u' | b'f' | b'c', b'8'] => 8,
        [b'<' | b'>' | b'=', b'c', b'1', b'6'] => 16,
        _ => return Err(Error::Format("unsupported numeric NPY dtype")),
    };
    let length = shape.iter().try_fold(item_size, |size: usize, &dimension| {
        size.checked_mul(dimension)
    });
    if length != Some(data_len) {
        return Err(Error::Format("NPY array length"));
    }
    Ok(())
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
    let mut bytes = npy_header(&value.dtype, &value.shape)?;
    bytes.extend(&value.data);
    Ok(bytes)
}

fn npy_header(dtype: &str, shape: &[usize]) -> Result<Vec<u8>> {
    let shape = shape
        .iter()
        .map(|n| format!("{n},"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut header =
        format!("{{'descr': '{dtype}', 'fortran_order': False, 'shape': ({shape}), }}");
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
    Ok(bytes)
}

pub(crate) struct Writer<W> {
    output: W,
    offset: u64,
    central: Vec<u8>,
    keys: std::collections::HashSet<String>,
}
impl<W: Write> Writer<W> {
    pub(crate) fn new(output: W) -> Self {
        Self {
            output,
            offset: 0,
            central: Vec::new(),
            keys: std::collections::HashSet::new(),
        }
    }
    pub(crate) fn member(
        &mut self,
        key: &str,
        dtype: &str,
        shape: &[usize],
        data_len: usize,
        write_data: impl Fn(&mut dyn Write) -> std::io::Result<()>,
    ) -> Result<()> {
        if key.is_empty()
            || !key.is_ascii()
            || key.contains(['/', '\\', '\0'])
            || !self.keys.insert(key.to_owned())
        {
            return Err(Error::Format("invalid or duplicate NPZ key"));
        }
        u16::try_from(self.keys.len()).map_err(|_| Error::Format("too many NPZ arrays"))?;
        validate_layout(dtype, shape, data_len)?;
        let name = format!("{key}.npy");
        let name_len = u16::try_from(name.len()).map_err(|_| Error::Format("NPZ key too long"))?;
        let header = npy_header(dtype, shape)?;
        let offset =
            u32::try_from(self.offset).map_err(|_| Error::Format("NPZ size exceeds u32"))?;
        let size = header
            .len()
            .checked_add(data_len)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or(Error::Format("NPY size exceeds u32"))?;
        let mut crc = Crc32::new();
        crc.write_all(&header)?;
        write_data(&mut crc)?;
        let checksum = crc.finish();
        self.output.write_all(b"PK\x03\x04")?;
        for value in [20_u16, 0, 0, 0, 33] {
            self.output.write_all(&value.to_le_bytes())?;
        }
        for value in [checksum, size, size] {
            self.output.write_all(&value.to_le_bytes())?;
        }
        self.output.write_all(&name_len.to_le_bytes())?;
        self.output.write_all(&0_u16.to_le_bytes())?;
        self.output.write_all(name.as_bytes())?;
        self.output.write_all(&header)?;
        write_data(&mut self.output)?;
        self.offset += 30 + u64::from(name_len) + u64::from(size);
        self.central.extend(b"PK\x01\x02");
        for value in [20_u16, 20, 0, 0, 0, 33] {
            self.central.extend(value.to_le_bytes());
        }
        for value in [checksum, size, size] {
            self.central.extend(value.to_le_bytes());
        }
        for value in [name_len, 0, 0, 0, 0] {
            self.central.extend(value.to_le_bytes());
        }
        self.central.extend(0_u32.to_le_bytes());
        self.central.extend(offset.to_le_bytes());
        self.central.extend(name.as_bytes());
        Ok(())
    }
    pub(crate) fn finish(mut self) -> Result<()> {
        let count =
            u16::try_from(self.keys.len()).map_err(|_| Error::Format("too many NPZ arrays"))?;
        let offset =
            u32::try_from(self.offset).map_err(|_| Error::Format("NPZ size exceeds u32"))?;
        let size = u32::try_from(self.central.len())
            .map_err(|_| Error::Format("NPZ directory exceeds u32"))?;
        self.output.write_all(&self.central)?;
        self.output.write_all(b"PK\x05\x06")?;
        for value in [0_u16, 0, count, count] {
            self.output.write_all(&value.to_le_bytes())?;
        }
        self.output.write_all(&size.to_le_bytes())?;
        self.output.write_all(&offset.to_le_bytes())?;
        self.output.write_all(&0_u16.to_le_bytes())?;
        Ok(())
    }
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

pub(crate) fn read(input: impl Read) -> Result<BTreeMap<String, Array>> {
    decode_members(input, |_| true)
}

struct StreamReader<R> {
    input: R,
    position: u64,
}
impl<R: Read> StreamReader<R> {
    fn take(&mut self, len: usize) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(len)
            .map_err(|_| Error::Format("ZIP size"))?;
        // Read through a bounded stack buffer so truncated inputs do not touch a
        // potentially huge allocation before discovering the missing payload.
        let mut remaining = len;
        let mut chunk = [0; 8192];
        while remaining != 0 {
            let count = remaining.min(chunk.len());
            self.input.read_exact(&mut chunk[..count]).map_err(|e| {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    Error::Format("truncated input")
                } else {
                    Error::Io(e)
                }
            })?;
            bytes.extend_from_slice(&chunk[..count]);
            remaining -= count;
        }
        self.position += len as u64;
        Ok(bytes)
    }
}

fn decode_members(
    input: impl Read,
    include: impl Fn(&[u8]) -> bool,
) -> Result<BTreeMap<String, Array>> {
    let mut reader = StreamReader { input, position: 0 };
    let mut arrays = BTreeMap::new();
    let mut members = 0_u16;
    let mut signature = reader.take(4)?;
    while signature == b"PK\x03\x04" {
        members = members
            .checked_add(1)
            .ok_or(Error::Format("too many NPZ members"))?;
        let mut header = signature;
        header.extend(reader.take(26)?);
        let header = header.as_slice();
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
            let mut extras = Reader::new(&extra);
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
        if crc32(&data) != crc {
            return Err(Error::Format("ZIP CRC32"));
        }
        if !include(&name) {
            signature = reader.take(4)?;
            continue;
        }
        let key = std::str::from_utf8(&name)
            .map_err(|_| Error::Format("NPZ key encoding"))?
            .strip_suffix(".npy")
            .ok_or(Error::Format("NPZ member is not NPY"))?;
        if key.is_empty()
            || !key.is_ascii()
            || key.contains(['/', '\\', '\0'])
            || arrays
                .insert(key.to_owned(), decode_npy_owned(data)?)
                .is_some()
        {
            return Err(Error::Format("invalid or duplicate NPZ key"));
        }
        signature = reader.take(4)?;
    }
    // 読む内容はローカルヘッダで確定するが、切断されたzipは受け入れない。
    let central_offset = reader.position - 4;
    for _ in 0..members {
        let mut header = signature;
        header.extend(reader.take(42)?);
        let header = header.as_slice();
        if &header[..4] != b"PK\x01\x02" || u16_at(header, 8) != 0 || u16_at(header, 10) != 0 {
            return Err(Error::Format("ZIP central directory"));
        }
        reader.take(
            u16_at(header, 28) as usize + u16_at(header, 30) as usize + u16_at(header, 32) as usize,
        )?;
        signature = reader.take(4)?;
    }
    let central_size = reader.position - 4 - central_offset;
    let mut end = signature;
    end.extend(reader.take(18)?);
    let end = end.as_slice();
    if &end[..4] != b"PK\x05\x06"
        || u16_at(end, 4) != 0
        || u16_at(end, 6) != 0
        || u16_at(end, 8) != members
        || u16_at(end, 10) != members
        || u64::from(u32_at(end, 12)) != central_size
        || u64::from(u32_at(end, 16)) != central_offset
    {
        return Err(Error::Format("ZIP end directory"));
    }
    reader.take(u16_at(end, 20) as usize)?;
    if reader.input.read(&mut [0])? != 0 {
        return Err(Error::Format("trailing ZIP bytes"));
    }
    Ok(arrays)
}

/// NPY 1.0の数値配列を検証し、値を変えずに復号する。
pub fn decode_npy(bytes: &[u8]) -> Result<Array> {
    let (dtype, shape, offset) = decode_npy_header(bytes)?;
    Ok(Array {
        dtype,
        shape,
        data: bytes[offset..].to_vec(),
    })
}

fn decode_npy_owned(mut bytes: Vec<u8>) -> Result<Array> {
    let (dtype, shape, offset) = decode_npy_header(&bytes)?;
    bytes.drain(..offset);
    Ok(Array {
        dtype,
        shape,
        data: bytes,
    })
}

fn decode_npy_header(bytes: &[u8]) -> Result<(String, Vec<usize>, usize)> {
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
    validate_layout(&dtype, &shape, r.remaining().len())?;
    Ok((dtype, shape, bytes.len() - r.remaining().len()))
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
struct Crc32(u32);
impl Crc32 {
    fn new() -> Self {
        Self(!0)
    }
    fn update(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 ^= u32::from(byte);
            for _ in 0..8 {
                self.0 = (self.0 >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(self.0 & 1));
            }
        }
    }
    fn finish(self) -> u32 {
        !self.0
    }
}
impl Write for Crc32 {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = Crc32::new();
    crc.update(bytes);
    crc.finish()
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
