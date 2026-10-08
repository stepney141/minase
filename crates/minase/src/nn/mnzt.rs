//! 方策の教師を持つ学習データの二進形式MNZT。
//!
//! 1つのファイルに、固定長のヘッダ、固定長レコードを並べた局面の表、および（ラベル、確率）の組を
//! 並べた方策の列を、この順に連続して置く。
//! 各レコードは、方策の列の中の自分の組の開始位置と個数を持つ。
//! 局面の符号化と結果コードは、評価関数の学習データ形式MNSD（[`crate::training::records`]）と共有する。
//! 数値はすべてリトルエンディアンで符号化する。

use core::fmt;
use std::io::{self, Read, Seek, SeekFrom, Write};

use minase_core::{BOARD_SQUARE_COUNT, Color, Position, PositionBuilder, Square};

use super::POLICY_SIZE;
use crate::training::records::{
    self, Outcome, decode_piece, encode_piece, read_u32, read_u64, validate_generation_commit,
    validate_rule_set_name,
};

/// 学習データファイルの識別子。
pub const MAGIC: [u8; 4] = *b"MNZT";
/// 学習データ形式の版。
pub const FORMAT_VERSION: u32 = 1;
/// ヘッダのバイト数。
pub const HEADER_LEN: usize = 176;
/// 局面の表の1レコードのバイト数。
pub const RECORD_LEN: usize = 168;
/// 方策の列の1組（ラベルと量子化した確率）のバイト数。
pub const POLICY_ENTRY_LEN: usize = 4;

/// 規則セット名欄のバイト数。
const RULE_SET_LEN: usize = 32;
/// 生成コミット欄のバイト数。
const COMMIT_LEN: usize = 40;
/// ネット検査和欄のバイト数。
const CHECKSUM_LEN: usize = 32;
/// 先獅子状態がないことを示す升コード。
const NO_LION_SQUARE: u8 = u8::MAX;
/// 確率1に対応する量子化値。
const PROBABILITY_SCALE: f32 = u16::MAX as f32;

/// 学習データの検証、局面復元または入出力の失敗。
#[derive(Debug)]
pub enum Error {
    /// ファイル識別子が一致しない。
    InvalidMagic {
        /// 読み取った識別子。
        actual: [u8; 4],
    },
    /// 形式の版が本実装の版と異なる。
    UnsupportedVersion {
        /// 読み取った版。
        actual: u32,
    },
    /// ヘッダが示すヘッダ長が固定長と異なる。
    UnsupportedHeaderLength {
        /// 読み取ったヘッダ長。
        actual: u32,
    },
    /// ヘッダが示すレコード長が固定長と異なる。
    UnsupportedRecordLength {
        /// 読み取ったレコード長。
        actual: u32,
    },
    /// 教師の種類のコードが0または1ではない。
    InvalidTeacherKind {
        /// 読み取った値。
        value: u8,
    },
    /// ヘッダの予約欄に0以外の値がある。
    NonzeroReservedBytes,
    /// ヘッダが示す局面の表の位置が、ヘッダの直後ではない。
    InvalidTableOffset {
        /// 読み取った位置。
        actual: u64,
    },
    /// ヘッダが示す方策の列の位置が、局面の表の直後ではない。
    InvalidPolicyOffset {
        /// 記録数から算出した位置。
        expected: u64,
        /// 読み取った位置。
        actual: u64,
    },
    /// ファイル長の算出が整数範囲を超えた。
    FileLengthOverflow,
    /// ファイル長がヘッダの記録数と組の総数から算出した長さと一致しない。
    FileLengthMismatch {
        /// ヘッダから算出したバイト数。
        expected: u64,
        /// 実際のバイト数。
        actual: u64,
    },
    /// 根の価値が0以上1以下の有限値ではない。
    InvalidValue {
        /// 問題の値。
        value: f32,
    },
    /// 方策のラベルが方策の出力の要素数以上である。
    InvalidLabel {
        /// 問題のラベル。
        label: u16,
    },
    /// 方策の確率が0以上1以下の有限値ではない。
    InvalidProbability {
        /// 問題の値。
        value: f32,
    },
    /// 1局面の方策の組の個数がレコードの個数欄（`u16`）に収まらない。
    PolicyTooLong {
        /// 組の個数。
        len: usize,
    },
    /// レコードが指す方策の組の範囲が、方策の列の外にある。
    PolicyRangeOutOfBounds {
        /// レコードが示す開始位置。
        start: u64,
        /// レコードが示す個数。
        count: u16,
        /// ヘッダが示す組の総数。
        policy_count: u64,
    },
    /// 指定したレコード番号が記録数以上である。
    IndexOutOfRange {
        /// 指定したレコード番号。
        index: u64,
        /// ヘッダが示す記録数。
        record_count: u64,
    },
    /// MNSDと共有する欄（規則セット名、生成コミット、局面、結果）の検証に失敗した。
    Encoding(records::Error),
    /// 入出力に失敗した。
    Io(io::Error),
}

impl fmt::Display for Error {
    /// エラーの説明を整形する。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic { actual } => write!(formatter, "invalid file magic: {actual:?}"),
            Self::UnsupportedVersion { actual } => {
                write!(formatter, "unsupported format version: {actual}")
            }
            Self::UnsupportedHeaderLength { actual } => {
                write!(formatter, "unsupported header length: {actual}")
            }
            Self::UnsupportedRecordLength { actual } => {
                write!(formatter, "unsupported record length: {actual}")
            }
            Self::InvalidTeacherKind { value } => {
                write!(formatter, "invalid teacher kind code: {value}")
            }
            Self::NonzeroReservedBytes => {
                formatter.write_str("header reserved bytes must all be zero")
            }
            Self::InvalidTableOffset { actual } => write!(
                formatter,
                "invalid position-table offset: expected {HEADER_LEN}, got {actual}"
            ),
            Self::InvalidPolicyOffset { expected, actual } => write!(
                formatter,
                "invalid policy-array offset: expected {expected}, got {actual}"
            ),
            Self::FileLengthOverflow => formatter.write_str("training-data file length overflow"),
            Self::FileLengthMismatch { expected, actual } => write!(
                formatter,
                "training-data file length mismatch: expected {expected}, got {actual}"
            ),
            Self::InvalidValue { value } => {
                write!(formatter, "root value must be in [0, 1], got {value}")
            }
            Self::InvalidLabel { label } => {
                write!(
                    formatter,
                    "policy label must be below {POLICY_SIZE}, got {label}"
                )
            }
            Self::InvalidProbability { value } => {
                write!(
                    formatter,
                    "policy probability must be in [0, 1], got {value}"
                )
            }
            Self::PolicyTooLong { len } => {
                write!(formatter, "too many policy entries for one record: {len}")
            }
            Self::PolicyRangeOutOfBounds {
                start,
                count,
                policy_count,
            } => write!(
                formatter,
                "policy range {start}+{count} exceeds the policy array of {policy_count} entries"
            ),
            Self::IndexOutOfRange {
                index,
                record_count,
            } => write!(
                formatter,
                "record index {index} out of range for {record_count} records"
            ),
            Self::Encoding(error) => error.fmt(formatter),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for Error {
    /// 原因となった下位エラーを返す。
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Encoding(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    /// 入出力エラーを学習データエラーへ変換する。
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<records::Error> for Error {
    /// MNSDと共有する欄の検証エラーを学習データエラーへ変換する。
    fn from(error: records::Error) -> Self {
        Self::Encoding(error)
    }
}

/// 方策の教師を作った探索の種類と規模。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Teacher {
    /// αβ探索のノード数の上限。
    AlphaBetaNodes(u64),
    /// MCTSの探索回数。
    MctsSimulations(u64),
}

impl Teacher {
    /// 二進形式の種類コードと値を返す。
    const fn encode(self) -> (u8, u64) {
        match self {
            Self::AlphaBetaNodes(nodes) => (0, nodes),
            Self::MctsSimulations(simulations) => (1, simulations),
        }
    }

    /// 二進形式の種類コードと値を復号する。
    const fn decode(kind: u8, value: u64) -> Result<Self, Error> {
        match kind {
            0 => Ok(Self::AlphaBetaNodes(value)),
            1 => Ok(Self::MctsSimulations(value)),
            _ => Err(Error::InvalidTeacherKind { value: kind }),
        }
    }
}

/// 学習データファイルの来歴と、局面の表および方策の列の大きさを表すヘッダ。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Header {
    /// 採用した規則セットの表示文字列。
    rule_set: String,
    /// 生成元コミットの完全ハッシュ。
    generation_commit: String,
    /// 生成時に使ったネットの検査和。ネットを使わない生成では全零。
    network_checksum: [u8; CHECKSUM_LEN],
    /// 方策の教師を作った探索。
    teacher: Teacher,
    /// データ生成の基本シード。
    seed: u64,
    /// 局面の表のレコード数。
    record_count: u64,
    /// 方策の列の組の総数。
    policy_count: u64,
}

impl Header {
    /// 指定した来歴から、記録数と組の総数が0の検証済みヘッダを作る。
    ///
    /// 記録数と組の総数は、[`Writer::finish`]が書き出した内容から確定する。
    pub fn new(
        rule_set: String,
        generation_commit: String,
        network_checksum: [u8; CHECKSUM_LEN],
        teacher: Teacher,
        seed: u64,
    ) -> Result<Self, Error> {
        validate_rule_set_name(&rule_set)?;
        validate_generation_commit(&generation_commit)?;
        Ok(Self {
            rule_set,
            generation_commit,
            network_checksum,
            teacher,
            seed,
            record_count: 0,
            policy_count: 0,
        })
    }

    /// 規則セット名を返す。
    pub fn rule_set(&self) -> &str {
        &self.rule_set
    }

    /// 生成元コミットの完全ハッシュを返す。
    pub fn generation_commit(&self) -> &str {
        &self.generation_commit
    }

    /// 生成時に使ったネットの検査和を返す。
    pub const fn network_checksum(&self) -> &[u8; CHECKSUM_LEN] {
        &self.network_checksum
    }

    /// 方策の教師を作った探索を返す。
    pub const fn teacher(&self) -> Teacher {
        self.teacher
    }

    /// データ生成の基本シードを返す。
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// 局面の表のレコード数を返す。
    pub const fn record_count(&self) -> u64 {
        self.record_count
    }

    /// 方策の列の組の総数を返す。
    pub const fn policy_count(&self) -> u64 {
        self.policy_count
    }

    /// ヘッダを固定長の二進表現へ符号化する。
    fn encode(&self) -> Result<[u8; HEADER_LEN], Error> {
        let layout = Layout::new(self.record_count, self.policy_count)?;
        let (teacher_kind, teacher_value) = self.teacher.encode();
        let mut bytes = [0_u8; HEADER_LEN];
        bytes[0..4].copy_from_slice(&MAGIC);
        bytes[4..8].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        bytes[8..12].copy_from_slice(&(HEADER_LEN as u32).to_le_bytes());
        bytes[12..16].copy_from_slice(&(RECORD_LEN as u32).to_le_bytes());
        let rule_bytes = self.rule_set.as_bytes();
        bytes[16..16 + rule_bytes.len()].copy_from_slice(rule_bytes);
        bytes[48..88].copy_from_slice(self.generation_commit.as_bytes());
        bytes[88..120].copy_from_slice(&self.network_checksum);
        bytes[120] = teacher_kind;
        bytes[128..136].copy_from_slice(&teacher_value.to_le_bytes());
        bytes[136..144].copy_from_slice(&self.seed.to_le_bytes());
        bytes[144..152].copy_from_slice(&self.record_count.to_le_bytes());
        bytes[152..160].copy_from_slice(&self.policy_count.to_le_bytes());
        bytes[160..168].copy_from_slice(&(HEADER_LEN as u64).to_le_bytes());
        bytes[168..176].copy_from_slice(&layout.policy_offset.to_le_bytes());
        Ok(bytes)
    }

    /// 固定長の二進表現を検証してヘッダへ復号する。
    fn decode(bytes: &[u8; HEADER_LEN]) -> Result<Self, Error> {
        let actual_magic: [u8; 4] = bytes[0..4].try_into().expect("slice length is fixed");
        if actual_magic != MAGIC {
            return Err(Error::InvalidMagic {
                actual: actual_magic,
            });
        }
        let version = read_u32(bytes, 4);
        if version != FORMAT_VERSION {
            return Err(Error::UnsupportedVersion { actual: version });
        }
        let header_length = read_u32(bytes, 8);
        if header_length != HEADER_LEN as u32 {
            return Err(Error::UnsupportedHeaderLength {
                actual: header_length,
            });
        }
        let record_length = read_u32(bytes, 12);
        if record_length != RECORD_LEN as u32 {
            return Err(Error::UnsupportedRecordLength {
                actual: record_length,
            });
        }

        let encoded_rule_set = &bytes[16..48];
        let rule_end = encoded_rule_set
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(RULE_SET_LEN);
        if encoded_rule_set[rule_end..].iter().any(|&byte| byte != 0) {
            return Err(records::Error::InvalidEncodedRuleSetName.into());
        }
        let rule_set = std::str::from_utf8(&encoded_rule_set[..rule_end])
            .map_err(|_| records::Error::InvalidEncodedRuleSetName)?
            .to_owned();
        let generation_commit = std::str::from_utf8(&bytes[48..48 + COMMIT_LEN])
            .map_err(|_| records::Error::InvalidGenerationCommit)?
            .to_owned();
        let network_checksum = bytes[88..120].try_into().expect("slice length is fixed");
        if bytes[121..128].iter().any(|&byte| byte != 0) {
            return Err(Error::NonzeroReservedBytes);
        }
        let teacher = Teacher::decode(bytes[120], read_u64(bytes, 128))?;

        let mut header = Self::new(
            rule_set,
            generation_commit,
            network_checksum,
            teacher,
            read_u64(bytes, 136),
        )?;
        header.record_count = read_u64(bytes, 144);
        header.policy_count = read_u64(bytes, 152);

        let table_offset = read_u64(bytes, 160);
        if table_offset != HEADER_LEN as u64 {
            return Err(Error::InvalidTableOffset {
                actual: table_offset,
            });
        }
        let layout = Layout::new(header.record_count, header.policy_count)?;
        let policy_offset = read_u64(bytes, 168);
        if policy_offset != layout.policy_offset {
            return Err(Error::InvalidPolicyOffset {
                expected: layout.policy_offset,
                actual: policy_offset,
            });
        }
        Ok(header)
    }
}

/// 記録数と組の総数から決まるファイル内の配置。
struct Layout {
    /// 方策の列の開始バイト位置。
    policy_offset: u64,
    /// 方策の列のバイト数。
    policy_length: u64,
    /// ファイル全体のバイト数。
    file_length: u64,
}

impl Layout {
    /// 配置を算出し、整数範囲を超える場合は`FileLengthOverflow`を返す。
    fn new(record_count: u64, policy_count: u64) -> Result<Self, Error> {
        let table_length = record_count
            .checked_mul(RECORD_LEN as u64)
            .ok_or(Error::FileLengthOverflow)?;
        let policy_offset = (HEADER_LEN as u64)
            .checked_add(table_length)
            .ok_or(Error::FileLengthOverflow)?;
        let policy_length = policy_count
            .checked_mul(POLICY_ENTRY_LEN as u64)
            .ok_or(Error::FileLengthOverflow)?;
        let file_length = policy_offset
            .checked_add(policy_length)
            .ok_or(Error::FileLengthOverflow)?;
        Ok(Self {
            policy_offset,
            policy_length,
            file_length,
        })
    }
}

/// 方策の列の1組。着手のラベルと、0以上1以下の確率を16ビットへ量子化した値を持つ。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PolicyEntry {
    /// 着手の方策ラベル。
    label: u16,
    /// 確率に65535を掛けて丸めた値。
    quantized_probability: u16,
}

impl PolicyEntry {
    /// ラベルと確率から組を作る。確率は`round(p * 65535)`で量子化する。
    ///
    /// ラベルが方策の出力の要素数以上なら`InvalidLabel`、
    /// 確率が0以上1以下の有限値でなければ`InvalidProbability`を返す。
    pub fn new(label: u16, probability: f32) -> Result<Self, Error> {
        validate_label(label)?;
        if !(0.0..=1.0).contains(&probability) {
            return Err(Error::InvalidProbability { value: probability });
        }
        Ok(Self {
            label,
            quantized_probability: (probability * PROBABILITY_SCALE).round() as u16,
        })
    }

    /// 着手の方策ラベルを返す。
    pub const fn label(self) -> u16 {
        self.label
    }

    /// 量子化した値から復元した確率を返す。
    pub fn probability(self) -> f32 {
        f32::from(self.quantized_probability) / PROBABILITY_SCALE
    }

    /// 組を固定長の二進表現へ符号化する。
    fn encode(self) -> [u8; POLICY_ENTRY_LEN] {
        let mut bytes = [0_u8; POLICY_ENTRY_LEN];
        bytes[0..2].copy_from_slice(&self.label.to_le_bytes());
        bytes[2..4].copy_from_slice(&self.quantized_probability.to_le_bytes());
        bytes
    }

    /// 固定長の二進表現を検証して組へ復号する。量子化した確率はどの値も有効である。
    fn decode(bytes: &[u8; POLICY_ENTRY_LEN]) -> Result<Self, Error> {
        let label = u16::from_le_bytes([bytes[0], bytes[1]]);
        validate_label(label)?;
        Ok(Self {
            label,
            quantized_probability: u16::from_le_bytes([bytes[2], bytes[3]]),
        })
    }
}

/// ラベルが方策の出力の要素数未満であることを検査する。
fn validate_label(label: u16) -> Result<(), Error> {
    if usize::from(label) < POLICY_SIZE {
        Ok(())
    } else {
        Err(Error::InvalidLabel { label })
    }
}

/// 1局面と、その価値および方策の教師を表すレコード。
#[derive(Clone, PartialEq, Debug)]
pub struct Record {
    /// dense index順の盤面コード。
    board: [u8; BOARD_SQUARE_COUNT],
    /// 記録局面の手番側。
    side_to_move: Color,
    /// 先獅子の対象升。
    lion_square: Option<Square>,
    /// 獅子を取った麒麟が同じ着手で成ったかどうか。
    lion_by_kirin_promotion: bool,
    /// 手番側から見た終局結果。
    outcome: Outcome,
    /// 1から始まる対局番号。
    game_number: u32,
    /// 記録時点の手数。
    ply: u16,
    /// 手番側から見た根の価値（勝率）。
    value: f32,
    /// 方策の教師の組。
    policy: Vec<PolicyEntry>,
}

impl Record {
    /// 局面と教師情報からレコードを作る。
    ///
    /// `value`は手番側の勝率であり、0以上1以下の有限値でなければ`InvalidValue`を返す。
    /// 探索値から勝率への変換は呼び出し側が行う。
    /// `policy`の個数が`u16::MAX`を超える場合は`PolicyTooLong`を返す。
    pub fn from_position(
        position: &Position,
        outcome: Outcome,
        game_number: u32,
        ply: u16,
        value: f32,
        policy: Vec<PolicyEntry>,
    ) -> Result<Self, Error> {
        validate_value(value)?;
        if u16::try_from(policy.len()).is_err() {
            return Err(Error::PolicyTooLong { len: policy.len() });
        }
        let mut board = [0_u8; BOARD_SQUARE_COUNT];
        for square in Square::all() {
            if let Some(piece) = position.piece_at(square) {
                board[square.dense_index()] = encode_piece(piece);
            }
        }
        let lion_trigger = position.lion_taken_by_non_lion();
        Ok(Self {
            board,
            side_to_move: position.side_to_move(),
            lion_square: lion_trigger.map(|trigger| trigger.square),
            lion_by_kirin_promotion: lion_trigger.is_some_and(|trigger| trigger.by_kirin_promotion),
            outcome,
            game_number,
            ply,
            value,
            policy,
        })
    }

    /// 手番側から見た終局結果を返す。
    pub const fn outcome(&self) -> Outcome {
        self.outcome
    }

    /// 対局番号を返す。
    pub const fn game_number(&self) -> u32 {
        self.game_number
    }

    /// 記録時点の手数を返す。
    pub const fn ply(&self) -> u16 {
        self.ply
    }

    /// 手番側から見た根の価値（勝率）を返す。
    pub const fn value(&self) -> f32 {
        self.value
    }

    /// 方策の教師の組を返す。
    pub fn policy(&self) -> &[PolicyEntry] {
        &self.policy
    }

    /// レコードから局面を復元し、不変条件と先獅子状態を検査する。
    pub fn to_position(&self) -> Result<Position, Error> {
        let mut builder = PositionBuilder::new(self.side_to_move);
        for square in Square::all() {
            let value = self.board[square.dense_index()];
            if value != 0 {
                builder
                    .put(square, decode_piece(square, value)?)
                    .map_err(records::Error::from)?;
            }
        }
        let mut position = builder.finish().map_err(records::Error::from)?;
        position
            .set_lion_capture(self.lion_square)
            .map_err(records::Error::from)?;
        position.validate().map_err(records::Error::from)?;
        let restored_flag = position
            .lion_taken_by_non_lion()
            .is_some_and(|trigger| trigger.by_kirin_promotion);
        if restored_flag != self.lion_by_kirin_promotion {
            return Err(records::Error::LionPromotionMismatch {
                recorded: self.lion_by_kirin_promotion,
                restored: restored_flag,
            }
            .into());
        }
        Ok(position)
    }

    /// 方策の列の開始位置を付けて、局面の表のレコードへ符号化する。
    fn encode(&self, policy_start: u64) -> [u8; RECORD_LEN] {
        let mut bytes = [0_u8; RECORD_LEN];
        bytes[..BOARD_SQUARE_COUNT].copy_from_slice(&self.board);
        bytes[144] = self.side_to_move as u8;
        bytes[145] = self
            .lion_square
            .map_or(NO_LION_SQUARE, |square| square.dense_index() as u8);
        bytes[146] = u8::from(self.lion_by_kirin_promotion);
        bytes[147] = self.outcome.code();
        bytes[148..152].copy_from_slice(&self.game_number.to_le_bytes());
        bytes[152..154].copy_from_slice(&self.ply.to_le_bytes());
        // 個数は構築時に`u16`へ収まることを検査済みである。
        bytes[154..156].copy_from_slice(&(self.policy.len() as u16).to_le_bytes());
        bytes[156..160].copy_from_slice(&self.value.to_le_bytes());
        bytes[160..168].copy_from_slice(&policy_start.to_le_bytes());
        bytes
    }

    /// 局面の表のレコードを検証し、方策を空にしたレコードと、方策の開始位置および個数へ復号する。
    fn decode(bytes: &[u8; RECORD_LEN]) -> Result<(Self, u64, u16), Error> {
        let mut board = [0_u8; BOARD_SQUARE_COUNT];
        board.copy_from_slice(&bytes[..BOARD_SQUARE_COUNT]);
        for square in Square::all() {
            let value = board[square.dense_index()];
            if value != 0 {
                decode_piece(square, value)?;
            }
        }
        let side_to_move = match bytes[144] {
            0 => Color::Black,
            1 => Color::White,
            value => return Err(records::Error::InvalidSideToMove { value }.into()),
        };
        let lion_square = match bytes[145] {
            NO_LION_SQUARE => None,
            value => Some(
                Square::from_dense(value as usize)
                    .ok_or(records::Error::InvalidLionSquare { value })?,
            ),
        };
        let lion_by_kirin_promotion = match bytes[146] {
            0 => false,
            1 => true,
            value => return Err(records::Error::InvalidLionPromotionFlag { value }.into()),
        };
        if lion_square.is_none() && lion_by_kirin_promotion {
            return Err(records::Error::LionPromotionWithoutSquare.into());
        }
        let outcome = Outcome::from_code(bytes[147])?;
        let value = f32::from_le_bytes(bytes[156..160].try_into().expect("slice length is fixed"));
        validate_value(value)?;
        let record = Self {
            board,
            side_to_move,
            lion_square,
            lion_by_kirin_promotion,
            outcome,
            game_number: read_u32(bytes, 148),
            ply: u16::from_le_bytes([bytes[152], bytes[153]]),
            value,
            policy: Vec::new(),
        };
        let policy_count = u16::from_le_bytes([bytes[154], bytes[155]]);
        Ok((record, read_u64(bytes, 160), policy_count))
    }
}

/// 根の価値が0以上1以下の有限値であることを検査する。
fn validate_value(value: f32) -> Result<(), Error> {
    if (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(Error::InvalidValue { value })
    }
}

/// ヘッダと配置を検証し、レコードを逐次または番号指定で復号する読み込み器。
pub struct Reader<R> {
    /// 読み込み元。
    inner: R,
    /// 復号済みヘッダ。
    header: Header,
    /// 方策の列の開始バイト位置。
    policy_offset: u64,
    /// 逐次読みで次に返すレコード番号。
    next_index: u64,
}

impl<R: Read + Seek> Reader<R> {
    /// 読み込み元のヘッダ、配置およびファイル長を検証する。
    pub fn new(mut inner: R) -> Result<Self, Error> {
        let actual_length = inner.seek(SeekFrom::End(0))?;
        inner.seek(SeekFrom::Start(0))?;
        let mut header_bytes = [0_u8; HEADER_LEN];
        inner.read_exact(&mut header_bytes)?;
        let header = Header::decode(&header_bytes)?;
        let layout = Layout::new(header.record_count, header.policy_count)?;
        if actual_length != layout.file_length {
            return Err(Error::FileLengthMismatch {
                expected: layout.file_length,
                actual: actual_length,
            });
        }
        Ok(Self {
            inner,
            header,
            policy_offset: layout.policy_offset,
            next_index: 0,
        })
    }

    /// 復号済みヘッダを返す。
    pub const fn header(&self) -> &Header {
        &self.header
    }

    /// 0から始まる番号のレコードを、方策の組とともに復号する。
    ///
    /// 番号が記録数以上なら`IndexOutOfRange`を返す。逐次読みの位置は変えない。
    pub fn read_at(&mut self, index: u64) -> Result<Record, Error> {
        if index >= self.header.record_count {
            return Err(Error::IndexOutOfRange {
                index,
                record_count: self.header.record_count,
            });
        }
        // 番号は記録数未満であり、記録数から算出した配置は整数範囲に収まることを検証済みである。
        let record_offset = HEADER_LEN as u64 + index * RECORD_LEN as u64;
        self.inner.seek(SeekFrom::Start(record_offset))?;
        let mut bytes = [0_u8; RECORD_LEN];
        self.inner.read_exact(&mut bytes)?;
        let (mut record, start, count) = Record::decode(&bytes)?;
        let in_bounds = start
            .checked_add(u64::from(count))
            .is_some_and(|end| end <= self.header.policy_count);
        if !in_bounds {
            return Err(Error::PolicyRangeOutOfBounds {
                start,
                count,
                policy_count: self.header.policy_count,
            });
        }
        // 範囲は組の総数以内であり、組の総数から算出した配置はファイル長と一致する。
        self.inner.seek(SeekFrom::Start(
            self.policy_offset + start * POLICY_ENTRY_LEN as u64,
        ))?;
        let mut policy_bytes = vec![0_u8; usize::from(count) * POLICY_ENTRY_LEN];
        self.inner.read_exact(&mut policy_bytes)?;
        record.policy = policy_bytes
            .as_chunks::<POLICY_ENTRY_LEN>()
            .0
            .iter()
            .map(PolicyEntry::decode)
            .collect::<Result<_, _>>()?;
        Ok(record)
    }

    /// 読み込み元を返す。
    pub fn into_inner(self) -> R {
        self.inner
    }
}

impl<R: Read + Seek> Iterator for Reader<R> {
    type Item = Result<Record, Error>;

    /// 次のレコードを復号する。失敗した後は`None`を返す。
    fn next(&mut self) -> Option<Self::Item> {
        if self.next_index >= self.header.record_count {
            return None;
        }
        let result = self.read_at(self.next_index);
        self.next_index = if result.is_ok() {
            self.next_index + 1
        } else {
            self.header.record_count
        };
        Some(result)
    }

    /// 未読レコード数から正確な残数を返す。
    fn size_hint(&self) -> (usize, Option<usize>) {
        match usize::try_from(self.header.record_count - self.next_index) {
            Ok(remaining) => (remaining, Some(remaining)),
            Err(_) => (usize::MAX, None),
        }
    }
}

/// レコードを1件ずつ受けて書き出す追記型の書き込み器。
///
/// 局面の表は書き出し先へ直接書き、方策の列は別の読み書き先`scratch`へ溜めて、
/// [`Writer::finish`]で局面の表の後ろへ連結する。
/// 方策の列をメモリに保持しないので、`scratch`に一時ファイルを渡せば
/// 1サイクル分の大きな方策の列も書き出せる。
pub struct Writer<W, S> {
    /// 書き出し先。
    inner: W,
    /// 方策の列を一時的に溜める読み書き先。
    scratch: S,
    /// 書き出し済みの記録数と組の総数を持つヘッダ。
    header: Header,
}

impl<W: Write + Seek, S: Read + Write + Seek> Writer<W, S> {
    /// 記録数0の仮ヘッダを書いて書き出しを始める。
    ///
    /// `inner`は先頭に位置した書き出し先とする。
    /// `scratch`は先頭へ移動してから使い、その中の既存の内容は上書きまたは無視する。
    /// `scratch`に一時ファイルを渡した場合、その削除は呼び出し側が行う。
    pub fn new(mut inner: W, mut scratch: S, mut header: Header) -> Result<Self, Error> {
        header.record_count = 0;
        header.policy_count = 0;
        inner.write_all(&header.encode()?)?;
        scratch.seek(SeekFrom::Start(0))?;
        Ok(Self {
            inner,
            scratch,
            header,
        })
    }

    /// 1レコードを書き出す。
    pub fn write_record(&mut self, record: &Record) -> Result<(), Error> {
        let record_count = self
            .header
            .record_count
            .checked_add(1)
            .ok_or(Error::FileLengthOverflow)?;
        let policy_count = self
            .header
            .policy_count
            .checked_add(record.policy.len() as u64)
            .ok_or(Error::FileLengthOverflow)?;
        Layout::new(record_count, policy_count)?;

        self.inner
            .write_all(&record.encode(self.header.policy_count))?;
        let policy_bytes: Vec<u8> = record
            .policy
            .iter()
            .flat_map(|entry| entry.encode())
            .collect();
        self.scratch.write_all(&policy_bytes)?;
        self.header.record_count = record_count;
        self.header.policy_count = policy_count;
        Ok(())
    }

    /// 方策の列を局面の表の後ろへ連結し、記録数と組の総数をヘッダへ書き戻して、書き出し先を返す。
    pub fn finish(mut self) -> Result<W, Error> {
        let layout = Layout::new(self.header.record_count, self.header.policy_count)?;
        self.scratch.flush()?;
        self.scratch.seek(SeekFrom::Start(0))?;
        let copied = io::copy(
            &mut (&mut self.scratch).take(layout.policy_length),
            &mut self.inner,
        )?;
        if copied != layout.policy_length {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
        }
        self.inner.seek(SeekFrom::Start(0))?;
        self.inner.write_all(&self.header.encode()?)?;
        self.inner.flush()?;
        Ok(self.inner)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use minase_core::test_util::{position, position_from_codes, sq};
    use minase_core::{PieceCode, PieceKind};

    /// テスト用の有効なヘッダを返す。
    fn header() -> Header {
        Header::new(
            "L0,P0,R1,E0".to_owned(),
            "0123456789abcdef0123456789abcdef01234567".to_owned(),
            [7; CHECKSUM_LEN],
            Teacher::MctsSimulations(64),
            11,
        )
        .unwrap()
    }

    /// レコード列をメモリ上のファイルへ書き出す。
    fn write_file(records: &[Record]) -> Vec<u8> {
        let mut writer =
            Writer::new(Cursor::new(Vec::new()), Cursor::new(Vec::new()), header()).unwrap();
        for record in records {
            writer.write_record(record).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    /// 指定した確率列から方策の組を作る。ラベルは添字の7倍とする。
    fn policy(probabilities: &[f32]) -> Vec<PolicyEntry> {
        probabilities
            .iter()
            .enumerate()
            .map(|(index, &probability)| PolicyEntry::new(index as u16 * 7, probability).unwrap())
            .collect()
    }

    /// 初期局面、成駒を含む後手番の局面、および先獅子つきの局面の3レコードを返す。
    fn sample_records() -> Vec<Record> {
        let initial = Position::initial();
        let white_to_move = position_from_codes(
            Color::White,
            &[
                (
                    sq(1, 1),
                    PieceCode::new_promoted(Color::Black, PieceKind::WhiteHorse).unwrap(),
                ),
                (
                    sq(5, 0),
                    PieceCode::new(Color::Black, PieceKind::King).unwrap(),
                ),
                (
                    sq(6, 11),
                    PieceCode::new(Color::White, PieceKind::King).unwrap(),
                ),
            ],
        );
        let trigger_square = sq(7, 7);
        let mut lion_trigger = position_from_codes(
            Color::Black,
            &[(
                trigger_square,
                PieceCode::new_promoted(Color::White, PieceKind::Lion).unwrap(),
            )],
        );
        lion_trigger.set_lion_capture(Some(trigger_square)).unwrap();
        vec![
            Record::from_position(
                &initial,
                Outcome::Win,
                1,
                0,
                0.5,
                policy(&[0.1, 0.2, 0.3, 0.4]),
            )
            .unwrap(),
            Record::from_position(
                &white_to_move,
                Outcome::Loss,
                1,
                17,
                0.123_456_7,
                policy(&[1.0 / 3.0, 2.0 / 3.0, 0.0]),
            )
            .unwrap(),
            Record::from_position(&lion_trigger, Outcome::Draw, 2, 4, 1.0, policy(&[1.0])).unwrap(),
        ]
    }

    /// 2つのレコードが局面、結果、対局番号、手数、価値および方策で一致することを検査する。
    fn assert_same_record(actual: &Record, expected: &Record, probabilities: &[f32]) {
        assert_eq!(
            actual.to_position().unwrap(),
            expected.to_position().unwrap()
        );
        assert_eq!(actual.outcome(), expected.outcome());
        assert_eq!(actual.game_number(), expected.game_number());
        assert_eq!(actual.ply(), expected.ply());
        assert_eq!(actual.value().to_bits(), expected.value().to_bits());
        assert_eq!(actual.policy(), expected.policy());
        assert_eq!(actual.policy().len(), probabilities.len());
        for (entry, &probability) in actual.policy().iter().zip(probabilities) {
            assert!((entry.probability() - probability).abs() <= 1.0 / 65535.0);
        }
    }

    // alphazero.md「学習データの形式」。局面、結果、対局番号、手数、価値および方策の列が往復で保たれる。
    #[test]
    fn records_round_trip_sequentially_and_by_index() {
        let records = sample_records();
        let probabilities: [&[f32]; 3] =
            [&[0.1, 0.2, 0.3, 0.4], &[1.0 / 3.0, 2.0 / 3.0, 0.0], &[1.0]];
        let mut reader = Reader::new(Cursor::new(write_file(&records))).unwrap();
        assert_eq!(reader.header().record_count(), 3);
        assert_eq!(reader.header().policy_count(), 8);
        assert_eq!(reader.header().teacher(), Teacher::MctsSimulations(64));
        assert_eq!(reader.header().rule_set(), "L0,P0,R1,E0");
        assert_eq!(reader.header().network_checksum(), &[7; CHECKSUM_LEN]);
        assert_eq!(reader.header().seed(), 11);

        for index in (0..3).rev() {
            let record = reader.read_at(index).unwrap();
            assert_same_record(
                &record,
                &records[index as usize],
                probabilities[index as usize],
            );
        }
        let sequential: Vec<Record> = reader.by_ref().collect::<Result<_, _>>().unwrap();
        assert_eq!(sequential, records);
        assert!(matches!(
            reader.read_at(3),
            Err(Error::IndexOutOfRange {
                index: 3,
                record_count: 3
            })
        ));
    }

    // 合法手が1手だけのSLのレコードは、確率1の組を1個だけ持ち、確率1が量子化で誤差なく戻る。
    #[test]
    fn single_move_policy_keeps_exact_probability_one() {
        let record = Record::from_position(
            &Position::initial(),
            Outcome::Draw,
            3,
            9,
            0.25,
            vec![PolicyEntry::new(35_855, 1.0).unwrap()],
        )
        .unwrap();
        let mut reader = Reader::new(Cursor::new(write_file(&[record]))).unwrap();
        let restored = reader.next().unwrap().unwrap();
        assert_eq!(restored.policy().len(), 1);
        assert_eq!(restored.policy()[0].label(), 35_855);
        assert_eq!(restored.policy()[0].probability(), 1.0);
        assert!(reader.next().is_none());
    }

    // 記録数0のファイルと、方策の個数0のレコードを含むファイルを読み書きできる。
    #[test]
    fn empty_file_and_empty_policy_are_accepted() {
        let bytes = write_file(&[]);
        assert_eq!(bytes.len(), HEADER_LEN);
        let mut reader = Reader::new(Cursor::new(bytes)).unwrap();
        assert_eq!(reader.header().record_count(), 0);
        assert_eq!(reader.header().policy_count(), 0);
        assert!(reader.next().is_none());
        assert!(reader.read_at(0).is_err());

        let records = vec![
            Record::from_position(&Position::initial(), Outcome::Win, 1, 0, 0.0, Vec::new())
                .unwrap(),
            Record::from_position(
                &Position::initial(),
                Outcome::Loss,
                1,
                1,
                1.0,
                policy(&[0.5, 0.5]),
            )
            .unwrap(),
            Record::from_position(&Position::initial(), Outcome::Win, 1, 2, 0.5, Vec::new())
                .unwrap(),
        ];
        let reader = Reader::new(Cursor::new(write_file(&records))).unwrap();
        let restored: Vec<Record> = reader.collect::<Result<_, _>>().unwrap();
        assert_eq!(restored, records);
    }

    // 形式の版を上げずに配置を変えると、既存のファイルと学習器の読み取りが壊れる。
    #[test]
    fn byte_lengths_are_fixed() {
        assert_eq!(HEADER_LEN, 176);
        assert_eq!(RECORD_LEN, 168);
        assert_eq!(POLICY_ENTRY_LEN, 4);
        let records = sample_records();
        let bytes = write_file(&records);
        assert_eq!(bytes.len(), 176 + 3 * 168 + 8 * 4);
        assert_eq!(&bytes[0..4], b"MNZT");
        assert_eq!(read_u64(&bytes, 160), 176);
        assert_eq!(read_u64(&bytes, 168), 176 + 3 * 168);
        // 2番目のレコードの方策の開始位置は、1番目のレコードの組の個数である。
        let second = HEADER_LEN + RECORD_LEN;
        assert_eq!(
            u16::from_le_bytes([bytes[second + 154], bytes[second + 155]]),
            3
        );
        assert_eq!(read_u64(&bytes, second + 160), 4);
    }

    // ヘッダの識別子、版、長さ、位置、教師の種類、予約欄、記録数またはファイル長が壊れたファイルを拒否する。
    #[test]
    fn corrupted_headers_are_rejected() {
        let valid = write_file(&sample_records());
        let corruptions: [(usize, u8); 10] = [
            (0, b'X'), // 識別子
            (4, 2),    // 版
            (8, 175),  // ヘッダ長
            (12, 160), // レコード長
            (120, 2),  // 教師の種類
            (121, 1),  // 予約欄
            (144, 4),  // 記録数
            (152, 9),  // 組の総数
            (160, 0),  // 局面の表の位置
            (168, 0),  // 方策の列の位置
        ];
        for (offset, replacement) in corruptions {
            let mut bytes = valid.clone();
            bytes[offset] = replacement;
            assert!(
                Reader::new(Cursor::new(bytes)).is_err(),
                "corruption at byte {offset} must be rejected"
            );
        }

        let mut truncated = valid.clone();
        truncated.pop();
        assert!(matches!(
            Reader::new(Cursor::new(truncated)),
            Err(Error::FileLengthMismatch { .. })
        ));
        assert!(Reader::new(Cursor::new(valid[..HEADER_LEN - 1].to_vec())).is_err());

        let mut overflowing = valid;
        overflowing[144..152].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(matches!(
            Reader::new(Cursor::new(overflowing)),
            Err(Error::FileLengthOverflow)
        ));
    }

    // レコードの方策の範囲、価値、または局面の欄が壊れたファイルは、読み取り時に`Err`を返す。
    #[test]
    fn corrupted_records_are_rejected() {
        let valid = write_file(&sample_records());
        let second = HEADER_LEN + RECORD_LEN;

        let mut out_of_range = valid.clone();
        out_of_range[second + 160..second + 168].copy_from_slice(&6_u64.to_le_bytes());
        let mut reader = Reader::new(Cursor::new(out_of_range)).unwrap();
        assert!(matches!(
            reader.read_at(1),
            Err(Error::PolicyRangeOutOfBounds {
                start: 6,
                count: 3,
                policy_count: 8
            })
        ));
        let mut overflowing_start = valid.clone();
        overflowing_start[second + 160..second + 168].copy_from_slice(&u64::MAX.to_le_bytes());
        let mut reader = Reader::new(Cursor::new(overflowing_start)).unwrap();
        assert!(matches!(
            reader.read_at(1),
            Err(Error::PolicyRangeOutOfBounds { .. })
        ));

        for value in [f32::NAN, -0.5, 1.5, f32::INFINITY] {
            let mut bad_value = valid.clone();
            bad_value[second + 156..second + 160].copy_from_slice(&value.to_le_bytes());
            let mut reader = Reader::new(Cursor::new(bad_value)).unwrap();
            assert!(matches!(reader.read_at(1), Err(Error::InvalidValue { .. })));
        }

        for (offset, replacement) in [(0, 59_u8), (144, 2), (145, 144), (146, 2), (147, 3)] {
            let mut bad_position = valid.clone();
            bad_position[second + offset] = replacement;
            let mut reader = Reader::new(Cursor::new(bad_position)).unwrap();
            assert!(
                matches!(reader.read_at(1), Err(Error::Encoding(_))),
                "corruption at record byte {offset} must be rejected"
            );
        }

        let first_policy = HEADER_LEN + 3 * RECORD_LEN;
        let mut bad_label = valid.clone();
        bad_label[first_policy..first_policy + 2]
            .copy_from_slice(&(POLICY_SIZE as u16).to_le_bytes());
        let mut reader = Reader::new(Cursor::new(bad_label)).unwrap();
        assert!(matches!(reader.read_at(0), Err(Error::InvalidLabel { .. })));

        // 逐次読みは壊れたレコードで`Err`を1回返した後に停止する。
        let mut bad_second = valid;
        bad_second[second + 147] = 3;
        let mut reader = Reader::new(Cursor::new(bad_second)).unwrap();
        assert!(reader.next().unwrap().is_ok());
        assert!(reader.next().unwrap().is_err());
        assert!(reader.next().is_none());
    }

    // 範囲外の価値と確率、および欄へ収まらない来歴を構築時に拒否する。
    #[test]
    fn constructors_reject_invalid_values() {
        for value in [f32::NAN, -f32::EPSILON, 1.0 + f32::EPSILON] {
            assert!(matches!(
                PolicyEntry::new(0, value),
                Err(Error::InvalidProbability { .. })
            ));
            assert!(matches!(
                PolicyEntry::new(POLICY_SIZE as u16, 0.5),
                Err(Error::InvalidLabel { .. })
            ));
            assert!(matches!(
                Record::from_position(&Position::initial(), Outcome::Draw, 1, 0, value, Vec::new()),
                Err(Error::InvalidValue { .. })
            ));
        }
        let too_long = vec![PolicyEntry::new(0, 0.0).unwrap(); usize::from(u16::MAX) + 1];
        assert!(matches!(
            Record::from_position(&Position::initial(), Outcome::Draw, 1, 0, 0.5, too_long),
            Err(Error::PolicyTooLong { .. })
        ));
        assert!(
            Header::new(
                "x".repeat(33),
                "0123456789abcdef0123456789abcdef01234567".to_owned(),
                [0; CHECKSUM_LEN],
                Teacher::AlphaBetaNodes(100_000),
                0,
            )
            .is_err()
        );
        assert!(
            Header::new(
                "L0,P0,R1,E0".to_owned(),
                "not-a-commit".to_owned(),
                [0; CHECKSUM_LEN],
                Teacher::AlphaBetaNodes(100_000),
                0,
            )
            .is_err()
        );
    }

    // 教師の種類と値はヘッダを往復し、SLのαβ探索のノード数も保たれる。
    #[test]
    fn alpha_beta_teacher_round_trips() {
        let header = Header::new(
            "engine-default".to_owned(),
            "fedcba9876543210fedcba9876543210fedcba98".to_owned(),
            [0; CHECKSUM_LEN],
            Teacher::AlphaBetaNodes(100_000),
            u64::MAX,
        )
        .unwrap();
        let writer = Writer::new(
            Cursor::new(Vec::new()),
            Cursor::new(Vec::new()),
            header.clone(),
        )
        .unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        let reader = Reader::new(Cursor::new(bytes)).unwrap();
        assert_eq!(reader.header(), &header);
    }

    // 先獅子の対象升と麒麟成りの原因が往復で保たれる。
    #[test]
    fn lion_trigger_round_trips() {
        let mut ordinary = position(Color::Black, &[(sq(4, 4), Color::White, PieceKind::Bishop)]);
        ordinary.set_lion_capture(Some(sq(4, 4))).unwrap();
        let record =
            Record::from_position(&ordinary, Outcome::Win, 1, 0, 0.75, Vec::new()).unwrap();
        let mut reader = Reader::new(Cursor::new(write_file(&[record]))).unwrap();
        assert_eq!(reader.read_at(0).unwrap().to_position().unwrap(), ordinary);

        let restored = Reader::new(Cursor::new(write_file(&sample_records())))
            .unwrap()
            .nth(2)
            .unwrap()
            .unwrap()
            .to_position()
            .unwrap();
        assert!(
            restored
                .lion_taken_by_non_lion()
                .unwrap()
                .by_kirin_promotion
        );
    }
}
