//! 段階9「付け直しの出力」「付け直しの再開」のMNRS交換形式。

use std::fmt;
use std::io::{self, BufRead, Read, Seek, SeekFrom};

use sha2::{Digest, Sha256};

/// MNRSヘッダの固定長。
pub const HEADER_LEN: usize = 240;
/// MNRS記録の固定長。
pub const ENTRY_LEN: usize = 16;

/// 付け直し形式または対象一覧の不正。
#[derive(Debug)]
pub enum Error {
    /// 固定長または記録境界が不正。
    InvalidLength,
    /// 識別子がMNRSでない。
    InvalidMagic,
    /// 未対応の版。
    UnsupportedVersion,
    /// 指定欄の値が不正。
    InvalidField(&'static str),
    /// 予約欄に非0の値がある。
    NonzeroReserved,
    /// 未定義の記録状態。
    InvalidStatus(u8),
    /// 状態が使わない欄に非0の値がある。
    NonzeroInactiveField,
    /// 再開条件の指定欄が一致しない。
    HeaderMismatch(&'static str),
    /// 対象一覧の指定行が不正、範囲外、または昇順でない。
    InvalidTargetLine(u64),
    /// 対象一覧と記録の状態が一致しない。
    TargetStatusMismatch(u64),
    /// 入出力の失敗。
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength => f.write_str("invalid MNRS length or incomplete entry"),
            Self::InvalidMagic => f.write_str("invalid MNRS magic"),
            Self::UnsupportedVersion => f.write_str("unsupported MNRS version"),
            Self::InvalidField(field) => write!(f, "invalid MNRS field: {field}"),
            Self::NonzeroReserved => f.write_str("MNRS reserved bytes must be zero"),
            Self::InvalidStatus(value) => write!(f, "undefined MNRS status: {value}"),
            Self::NonzeroInactiveField => f.write_str("inactive MNRS entry fields must be zero"),
            Self::HeaderMismatch(field) => write!(f, "MNRS header mismatch: {field}"),
            Self::InvalidTargetLine(line) => write!(
                f,
                "invalid target at line {line}: expected an ascending, unique decimal index within the MNSD"
            ),
            Self::TargetStatusMismatch(index) => {
                write!(f, "MNRS target status mismatch at record {index}")
            }
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// 付け直しの元データと探索条件。符号化時にも全欄を検証する。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RescoreHeader {
    /// 元MNSD全体のSHA-256。
    pub mnsd_sha256: [u8; 32],
    /// 元MNSDの記録数。
    pub record_count: u64,
    /// 対象番号をu64のLEで連結した列のSHA-256。
    pub targets_sha256: [u8; 32],
    /// 対象の記録数。
    pub target_count: u64,
    /// MNPTの重み本体の検査和。
    pub network_checksum: [u8; 32],
    /// 探索のノード上限。
    pub nodes: u32,
    /// 規則セット名。
    pub rule_set: String,
    /// 各ワーカーの置換表容量(MB)。
    pub hash_mb: u32,
    /// 生成コミットの40桁16進表現。
    pub generation_commit: String,
    /// 実行バイナリ全体のSHA-256。
    pub binary_sha256: [u8; 32],
}

impl RescoreHeader {
    /// 全欄を検証し、240バイトへ符号化する。探索条件は常に局面単独の0。
    pub fn encode(&self) -> Result<[u8; HEADER_LEN], Error> {
        if self.target_count > self.record_count {
            return Err(Error::InvalidField("target_count"));
        }
        if self.nodes == 0 {
            return Err(Error::InvalidField("nodes"));
        }
        if self.hash_mb == 0 {
            return Err(Error::InvalidField("hash_mb"));
        }
        if self.rule_set.is_empty() || self.rule_set.len() > 32 || self.rule_set.contains('\0') {
            return Err(Error::InvalidField("rule_set"));
        }
        if !is_hex(&self.generation_commit, 40) {
            return Err(Error::InvalidField("generation_commit"));
        }
        self.record_count
            .checked_mul(ENTRY_LEN as u64)
            .and_then(|n| n.checked_add(HEADER_LEN as u64))
            .ok_or(Error::InvalidLength)?;
        let mut bytes = [0; HEADER_LEN];
        bytes[..4].copy_from_slice(b"MNRS");
        bytes[4..8].copy_from_slice(&1_u32.to_le_bytes());
        bytes[8..40].copy_from_slice(&self.mnsd_sha256);
        bytes[40..48].copy_from_slice(&self.record_count.to_le_bytes());
        bytes[48..80].copy_from_slice(&self.targets_sha256);
        bytes[80..88].copy_from_slice(&self.target_count.to_le_bytes());
        bytes[88..120].copy_from_slice(&self.network_checksum);
        bytes[120..124].copy_from_slice(&self.nodes.to_le_bytes());
        bytes[124..124 + self.rule_set.len()].copy_from_slice(self.rule_set.as_bytes());
        bytes[156..160].copy_from_slice(&self.hash_mb.to_le_bytes());
        bytes[164..204].copy_from_slice(self.generation_commit.as_bytes());
        bytes[204..236].copy_from_slice(&self.binary_sha256);
        Ok(bytes)
    }

    /// ヘッダの全欄、予約欄、および固定長を検証して復号する。
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let b: &[u8; HEADER_LEN] = bytes.try_into().map_err(|_| Error::InvalidLength)?;
        if &b[..4] != b"MNRS" {
            return Err(Error::InvalidMagic);
        }
        if u32::from_le_bytes(array(b, 4)) != 1 {
            return Err(Error::UnsupportedVersion);
        }
        if b[160] != 0 {
            return Err(Error::InvalidField("search_condition"));
        }
        if b[161..164].iter().chain(&b[236..240]).any(|&v| v != 0) {
            return Err(Error::NonzeroReserved);
        }
        let rule_bytes = &b[124..156];
        let end = rule_bytes.iter().position(|&v| v == 0).unwrap_or(32);
        if rule_bytes[end..].iter().any(|&v| v != 0) {
            return Err(Error::InvalidField("rule_set"));
        }
        let header = Self {
            mnsd_sha256: array(b, 8),
            record_count: u64::from_le_bytes(array(b, 40)),
            targets_sha256: array(b, 48),
            target_count: u64::from_le_bytes(array(b, 80)),
            network_checksum: array(b, 88),
            nodes: u32::from_le_bytes(array(b, 120)),
            rule_set: std::str::from_utf8(&rule_bytes[..end])
                .map_err(|_| Error::InvalidField("rule_set"))?
                .to_owned(),
            hash_mb: u32::from_le_bytes(array(b, 156)),
            generation_commit: std::str::from_utf8(&b[164..204])
                .map_err(|_| Error::InvalidField("generation_commit"))?
                .to_owned(),
            binary_sha256: array(b, 204),
        };
        header.encode()?;
        Ok(header)
    }

    /// 元データの同一性を検証する。
    pub fn validate_source(&self, checksum: &[u8; 32], count: u64) -> Result<(), Error> {
        if self.mnsd_sha256 != *checksum {
            return Err(Error::HeaderMismatch("mnsd_sha256"));
        }
        if self.record_count != count {
            return Err(Error::HeaderMismatch("record_count"));
        }
        Ok(())
    }

    /// 全再開条件を照合し、最初に異なる欄を返す。
    pub fn validate_resume(&self, expected: &Self) -> Result<(), Error> {
        self.validate_source(&expected.mnsd_sha256, expected.record_count)?;
        macro_rules! compare {
            ($($field:ident),+ $(,)?) => { $(if self.$field != expected.$field { return Err(Error::HeaderMismatch(stringify!($field))); })+ };
        }
        compare!(
            targets_sha256,
            target_count,
            network_checksum,
            nodes,
            rule_set,
            hash_mb,
            generation_commit,
            binary_sha256
        );
        Ok(())
    }
}

/// 付け直し記録の状態。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RescoreStatus {
    /// 対象に含まれない。
    NotTarget,
    /// 深さ1以上を完了した。
    Rescored,
    /// 深さ1を完了しなかった。
    Incomplete,
}

/// 検証済みの付け直し記録。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RescoreEntry {
    status: RescoreStatus,
    tactical: bool,
    score: i16,
    depth: u32,
    nodes: u64,
}

impl RescoreEntry {
    /// 対象外の全0記録を作る。
    pub const fn not_target() -> Self {
        Self {
            status: RescoreStatus::NotTarget,
            tactical: false,
            score: 0,
            depth: 0,
            nodes: 0,
        }
    }
    /// 深さ1を完了しなかった記録を作る。
    pub const fn incomplete(nodes: u64) -> Self {
        Self {
            status: RescoreStatus::Incomplete,
            nodes,
            ..Self::not_target()
        }
    }
    /// 完了した探索の記録を作る。深さ0は拒否する。
    pub fn rescored(tactical: bool, score: i16, depth: u32, nodes: u64) -> Result<Self, Error> {
        if depth == 0 {
            return Err(Error::InvalidField("depth"));
        }
        Ok(Self {
            status: RescoreStatus::Rescored,
            tactical,
            score,
            depth,
            nodes,
        })
    }
    /// 記録の状態を返す。
    pub const fn status(&self) -> RescoreStatus {
        self.status
    }
    /// 最善手が捕獲または成りかを返す。
    pub const fn tactical(&self) -> bool {
        self.tactical
    }
    /// 探索値を返す。状態1以外は0。
    pub const fn score(&self) -> i16 {
        self.score
    }
    /// 完了した深さを返す。
    pub const fn depth(&self) -> u32 {
        self.depth
    }
    /// 実際の探索ノード数を返す。
    pub const fn nodes(&self) -> u64 {
        self.nodes
    }
    /// 16バイトへ符号化する。
    pub fn encode(&self) -> [u8; ENTRY_LEN] {
        let mut b = [0; ENTRY_LEN];
        b[0] = match self.status {
            RescoreStatus::NotTarget => 0,
            RescoreStatus::Rescored => 1,
            RescoreStatus::Incomplete => 2,
        };
        b[1] = u8::from(self.tactical);
        b[2..4].copy_from_slice(&self.score.to_le_bytes());
        b[4..8].copy_from_slice(&self.depth.to_le_bytes());
        b[8..16].copy_from_slice(&self.nodes.to_le_bytes());
        b
    }
    /// 状態ごとの使用欄と値を検証して復号する。
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let b: &[u8; ENTRY_LEN] = bytes.try_into().map_err(|_| Error::InvalidLength)?;
        let nodes = u64::from_le_bytes(array(b, 8));
        match b[0] {
            0 if b[1..].iter().any(|&v| v != 0) => Err(Error::NonzeroInactiveField),
            0 => Ok(Self::not_target()),
            1 => {
                if b[1] > 1 {
                    return Err(Error::InvalidField("tactical"));
                }
                Self::rescored(
                    b[1] == 1,
                    i16::from_le_bytes(array(b, 2)),
                    u32::from_le_bytes(array(b, 4)),
                    nodes,
                )
            }
            2 if b[1..8].iter().any(|&v| v != 0) => Err(Error::NonzeroInactiveField),
            2 => Ok(Self::incomplete(nodes)),
            value => Err(Error::InvalidStatus(value)),
        }
    }
}

/// 完了済みまたは記録境界で中断されたMNRSを逐次読む。
pub struct RescoreReader<R> {
    inner: R,
    header: RescoreHeader,
    written: u64,
    remaining: u64,
}

impl<R: Read + Seek> RescoreReader<R> {
    /// ヘッダとファイル長を検証する。途中の記録や余分な記録は拒否する。
    pub fn new(mut inner: R) -> Result<Self, Error> {
        let length = inner.seek(SeekFrom::End(0))?;
        let body = length
            .checked_sub(HEADER_LEN as u64)
            .ok_or(Error::InvalidLength)?;
        if body % ENTRY_LEN as u64 != 0 {
            return Err(Error::InvalidLength);
        }
        inner.seek(SeekFrom::Start(0))?;
        let mut bytes = [0; HEADER_LEN];
        inner.read_exact(&mut bytes)?;
        let header = RescoreHeader::decode(&bytes)?;
        let written = body / ENTRY_LEN as u64;
        if written > header.record_count {
            return Err(Error::InvalidLength);
        }
        Ok(Self {
            inner,
            header,
            written,
            remaining: written,
        })
    }
    /// 検証済みヘッダを返す。
    pub const fn header(&self) -> &RescoreHeader {
        &self.header
    }
    /// 書き込み済み記録数を返す。
    pub const fn written_count(&self) -> u64 {
        self.written
    }
    /// 全記録が書き込まれているかを返す。
    pub fn is_complete(&self) -> bool {
        self.written == self.header.record_count
    }
    /// 次の記録を検証して読む。
    pub fn read_entry(&mut self) -> Result<Option<RescoreEntry>, Error> {
        if self.remaining == 0 {
            return Ok(None);
        }
        let mut bytes = [0; ENTRY_LEN];
        self.inner.read_exact(&mut bytes)?;
        let entry = RescoreEntry::decode(&bytes)?;
        self.remaining -= 1;
        Ok(Some(entry))
    }
    /// 読み込み元を返す。
    pub fn into_inner(self) -> R {
        self.inner
    }
}

/// 対象一覧を検証して読む。空ファイルは対象0件を表す。
pub fn read_targets(input: impl BufRead, record_count: u64) -> Result<Vec<u64>, Error> {
    let mut targets = Vec::new();
    for (index, line) in input.lines().enumerate() {
        let line = line?;
        let invalid = || Error::InvalidTargetLine(index as u64 + 1);
        if line.is_empty() || !line.bytes().all(|v| v.is_ascii_digit()) {
            return Err(invalid());
        }
        let target = line.parse::<u64>().map_err(|_| invalid())?;
        if target >= record_count || targets.last().is_some_and(|&previous| target <= previous) {
            return Err(invalid());
        }
        targets.push(target);
    }
    Ok(targets)
}

/// 対象番号列の正規化されたSHA-256を計算する。
pub fn targets_checksum(targets: &[u64]) -> [u8; 32] {
    let mut hash = Sha256::new();
    for target in targets {
        hash.update(target.to_le_bytes());
    }
    hash.finalize().into()
}

/// 現在位置から末尾までのSHA-256を一定量のメモリで計算する。
pub fn sha256(mut input: impl Read) -> io::Result<[u8; 32]> {
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let size = input.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        hash.update(&buffer[..size]);
    }
    Ok(hash.finalize().into())
}

pub(super) fn is_hex(text: &str, length: usize) -> bool {
    text.len() == length && text.bytes().all(|v| v.is_ascii_hexdigit())
}

/// 検査済み固定長配列の欄を取り出す。呼出し位置と長さは形式の定数。
fn array<const N: usize>(bytes: &[u8], offset: usize) -> [u8; N] {
    let mut value = [0; N];
    value.copy_from_slice(&bytes[offset..offset + N]);
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn header() -> RescoreHeader {
        RescoreHeader {
            mnsd_sha256: [0x11; 32],
            record_count: 3,
            targets_sha256: [0x22; 32],
            target_count: 2,
            network_checksum: [0x33; 32],
            nodes: 200,
            rule_set: "engine-default".to_owned(),
            hash_mb: 16,
            generation_commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            binary_sha256: [0x44; 32],
        }
    }

    // formats-phase4.md第1節のオフセットから独立に組み立てた固定データ。
    #[test]
    fn header_matches_exchange_layout() {
        let mut fixture = Vec::new();
        fixture.extend(b"MNRS");
        fixture.extend(1_u32.to_le_bytes());
        fixture.extend([0x11; 32]);
        fixture.extend(3_u64.to_le_bytes());
        fixture.extend([0x22; 32]);
        fixture.extend(2_u64.to_le_bytes());
        fixture.extend([0x33; 32]);
        fixture.extend(200_u32.to_le_bytes());
        fixture.extend(b"engine-default");
        fixture.extend([0; 18]);
        fixture.extend(16_u32.to_le_bytes());
        fixture.extend([0; 4]);
        fixture.extend(b"0123456789abcdef0123456789abcdef01234567");
        fixture.extend([0x44; 32]);
        fixture.extend([0; 4]);
        assert_eq!(fixture.len(), 240);
        assert_eq!(header().encode().unwrap().as_slice(), fixture);
        assert_eq!(RescoreHeader::decode(&fixture).unwrap(), header());
    }

    #[test]
    fn entries_match_exchange_layout_and_reject_inactive_values() {
        let fixture = [1, 1, 0x85, 0xff, 3, 0, 0, 0, 200, 0, 0, 0, 0, 0, 0, 0];
        let entry = RescoreEntry::rescored(true, -123, 3, 200).unwrap();
        assert_eq!(entry.encode(), fixture);
        assert_eq!(RescoreEntry::decode(&fixture).unwrap(), entry);
        assert_eq!(RescoreEntry::not_target().encode(), [0; 16]);
        for index in 1..16 {
            let mut bytes = [0; 16];
            bytes[index] = 1;
            assert!(matches!(
                RescoreEntry::decode(&bytes),
                Err(Error::NonzeroInactiveField)
            ));
        }
        let mut incomplete = [0; 16];
        incomplete[0] = 2;
        incomplete[8] = 200;
        assert_eq!(
            RescoreEntry::decode(&incomplete).unwrap(),
            RescoreEntry::incomplete(200)
        );
        for index in 1..8 {
            let mut bytes = incomplete;
            bytes[index] = 1;
            assert!(matches!(
                RescoreEntry::decode(&bytes),
                Err(Error::NonzeroInactiveField)
            ));
        }
        for status in 3..=255 {
            let mut bytes = [0; 16];
            bytes[0] = status;
            assert!(matches!(
                RescoreEntry::decode(&bytes),
                Err(Error::InvalidStatus(_))
            ));
        }
        let mut bytes = fixture;
        bytes[1] = 2;
        assert!(matches!(
            RescoreEntry::decode(&bytes),
            Err(Error::InvalidField("tactical"))
        ));
        let mut bytes = fixture;
        bytes[4..8].fill(0);
        assert!(matches!(
            RescoreEntry::decode(&bytes),
            Err(Error::InvalidField("depth"))
        ));
        for length in 0..16 {
            assert!(RescoreEntry::decode(&fixture[..length]).is_err());
        }
    }

    #[test]
    fn header_rejects_undefined_fields_and_reserved_bytes() {
        let encoded = header().encode().unwrap();
        for index in [161, 162, 163, 236, 237, 238, 239] {
            let mut bytes = encoded;
            bytes[index] = 1;
            assert!(matches!(
                RescoreHeader::decode(&bytes),
                Err(Error::NonzeroReserved)
            ));
        }
        for index in [0, 4, 160, 164, 124, 140] {
            let mut bytes = encoded;
            bytes[index] = 255;
            assert!(RescoreHeader::decode(&bytes).is_err(), "offset {index}");
        }
        for range in [120..124, 156..160] {
            let mut bytes = encoded;
            bytes[range].fill(0);
            assert!(RescoreHeader::decode(&bytes).is_err());
        }
        let mut bytes = encoded;
        bytes[80..88].copy_from_slice(&4_u64.to_le_bytes());
        assert!(matches!(
            RescoreHeader::decode(&bytes),
            Err(Error::InvalidField("target_count"))
        ));
        let mut bytes = encoded;
        bytes[40..48].fill(255);
        assert!(matches!(
            RescoreHeader::decode(&bytes),
            Err(Error::InvalidLength)
        ));
        for length in 0..240 {
            assert!(RescoreHeader::decode(&encoded[..length]).is_err());
        }
    }

    #[test]
    fn resume_requires_every_header_field_and_source_identity() {
        let expected = header();
        let mut changed = expected.clone();
        changed.mnsd_sha256[0] ^= 1;
        assert!(matches!(
            changed.validate_source(&expected.mnsd_sha256, 3),
            Err(Error::HeaderMismatch("mnsd_sha256"))
        ));
        assert!(matches!(
            expected.validate_source(&expected.mnsd_sha256, 4),
            Err(Error::HeaderMismatch("record_count"))
        ));
        macro_rules! change {
            ($field:ident, $value:expr) => {{
                let mut changed = expected.clone(); changed.$field = $value;
                assert!(matches!(changed.validate_resume(&expected), Err(Error::HeaderMismatch(name)) if name == stringify!($field)));
            }};
        }
        change!(mnsd_sha256, [1; 32]);
        change!(record_count, 4);
        change!(targets_sha256, [1; 32]);
        change!(target_count, 1);
        change!(network_checksum, [1; 32]);
        change!(nodes, 201);
        change!(rule_set, "L0,P0,R1,E0".into());
        change!(hash_mb, 32);
        change!(generation_commit, "a".repeat(40));
        change!(binary_sha256, [1; 32]);
    }

    #[test]
    fn reader_distinguishes_complete_interrupted_and_torn_files() {
        for count in 0..=3 {
            let mut bytes = header().encode().unwrap().to_vec();
            bytes.extend(vec![0; count * 16]);
            let mut reader = RescoreReader::new(Cursor::new(&bytes)).unwrap();
            assert_eq!(reader.written_count(), count as u64);
            assert_eq!(reader.is_complete(), count == 3);
            for _ in 0..count {
                assert_eq!(
                    reader.read_entry().unwrap(),
                    Some(RescoreEntry::not_target())
                );
            }
            assert_eq!(reader.read_entry().unwrap(), None);
            for tail in 1..16 {
                let mut torn = bytes.clone();
                torn.extend(vec![0; tail]);
                assert!(matches!(
                    RescoreReader::new(Cursor::new(torn)),
                    Err(Error::InvalidLength)
                ));
            }
        }
        let mut bytes = header().encode().unwrap().to_vec();
        bytes.extend([0; 64]);
        assert!(matches!(
            RescoreReader::new(Cursor::new(bytes)),
            Err(Error::InvalidLength)
        ));
    }

    #[test]
    fn targets_are_strictly_ordered_decimal_indices_and_hash_little_endian_numbers() {
        assert_eq!(read_targets(Cursor::new(b"0\n2\n"), 3).unwrap(), [0, 2]);
        assert_eq!(read_targets(Cursor::new(b"0\r\n2"), 3).unwrap(), [0, 2]);
        assert!(read_targets(Cursor::new(b""), 0).unwrap().is_empty());
        for invalid in [
            "\n",
            "0\n\n",
            "1\n0",
            "1\n1",
            "3",
            "-1",
            "+1",
            " 1",
            "1 ",
            "18446744073709551616",
            "１",
        ] {
            assert!(
                matches!(
                    read_targets(Cursor::new(invalid), 3),
                    Err(Error::InvalidTargetLine(_))
                ),
                "{invalid:?}"
            );
        }
        let raw = [0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(targets_checksum(&[0, 2]), sha256(Cursor::new(raw)).unwrap());
    }
}
