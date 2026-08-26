//! 自己対局から生成する評価関数学習データの二進形式。

use core::fmt;
use std::io::{self, Read, Seek, SeekFrom, Write};

use crate::core::piece::PIECE_KIND_COUNT;
use crate::{
    BOARD_SQUARE_COUNT, Color, GameResult, IllegalMove, Move, MoveGenerator, PieceCode, PieceKind,
    Position, PositionBuildError, PositionBuilder, PositionError, Square,
};

/// 学習データファイルの識別子。
pub const MAGIC: [u8; 4] = *b"MNSD";
/// 学習データ形式の版。
pub const FORMAT_VERSION: u32 = 1;
/// 学習データヘッダのバイト数。
pub const HEADER_LEN: usize = 136;
/// 学習データレコードのバイト数。
pub const RECORD_LEN: usize = 160;

/// 規則セット名欄のバイト数。
const RULE_SET_LEN: usize = 32;
/// 生成コミット欄のバイト数。
const COMMIT_LEN: usize = 40;
/// ネット検査和欄のバイト数。
const CHECKSUM_LEN: usize = 32;
/// 先獅子状態がないことを示す升コード。
const NO_LION_SQUARE: u8 = u8::MAX;

/// 学習データの復号、局面復元または入出力の失敗。
#[derive(Debug)]
pub enum Error {
    /// ヘッダの長さが固定長と異なる。
    InvalidHeaderLength {
        /// 実際のバイト数。
        actual: usize,
    },
    /// レコードの長さが固定長と異なる。
    InvalidRecordLength {
        /// 実際のバイト数。
        actual: usize,
    },
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
    /// ヘッダが示すレコード長が固定長と異なる。
    UnsupportedRecordLength {
        /// 読み取ったレコード長。
        actual: u32,
    },
    /// 規則セット名が固定欄へ収まらないかNULを含む。
    InvalidRuleSetName,
    /// 規則セット名欄がUTF-8またはNUL埋めの規約を満たさない。
    InvalidEncodedRuleSetName,
    /// 生成コミットが40桁の16進ASCIIではない。
    InvalidGenerationCommit,
    /// ファイル長の算出が整数範囲を超えた。
    FileLengthOverflow,
    /// ファイル長がヘッダのレコード数と一致しない。
    FileLengthMismatch {
        /// ヘッダから算出したバイト数。
        expected: u64,
        /// 実際のバイト数。
        actual: u64,
    },
    /// 盤面欄の駒コードが永続形式の定義域外にある。
    InvalidPieceCode {
        /// 問題の升。
        square: Square,
        /// 問題の駒コード。
        value: u8,
    },
    /// 手番欄が0または1ではない。
    InvalidSideToMove {
        /// 読み取った値。
        value: u8,
    },
    /// 先獅子の対象升が0から143または255ではない。
    InvalidLionSquare {
        /// 読み取った値。
        value: u8,
    },
    /// 先獅子の麒麟成りフラグが0または1ではない。
    InvalidLionPromotionFlag {
        /// 読み取った値。
        value: u8,
    },
    /// 対象升がない先獅子状態に麒麟成りフラグが立っている。
    LionPromotionWithoutSquare,
    /// 結果欄が0から2の範囲外にある。
    InvalidOutcome {
        /// 読み取った値。
        value: u8,
    },
    /// 予約欄に0以外の値がある。
    NonzeroReservedBytes,
    /// 局面の組み立てに失敗した。
    PositionBuild(PositionBuildError),
    /// 復元局面が不変条件を満たさない。
    Position(PositionError),
    /// 記録された麒麟成りフラグと復元結果が一致しない。
    LionPromotionMismatch {
        /// 記録された値。
        recorded: bool,
        /// 盤面から復元した値。
        restored: bool,
    },
    /// 入出力に失敗した。
    Io(io::Error),
}

impl fmt::Display for Error {
    /// エラーの説明を整形する。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidHeaderLength { actual } => {
                write!(
                    formatter,
                    "invalid header length: expected {HEADER_LEN}, got {actual}"
                )
            }
            Self::InvalidRecordLength { actual } => {
                write!(
                    formatter,
                    "invalid record length: expected {RECORD_LEN}, got {actual}"
                )
            }
            Self::InvalidMagic { actual } => write!(formatter, "invalid file magic: {actual:?}"),
            Self::UnsupportedVersion { actual } => {
                write!(formatter, "unsupported format version: {actual}")
            }
            Self::UnsupportedRecordLength { actual } => {
                write!(formatter, "unsupported record length: {actual}")
            }
            Self::InvalidRuleSetName => formatter.write_str("invalid rule-set name"),
            Self::InvalidEncodedRuleSetName => formatter.write_str("invalid encoded rule-set name"),
            Self::InvalidGenerationCommit => formatter
                .write_str("generation commit must be exactly 40 hexadecimal ASCII characters"),
            Self::FileLengthOverflow => formatter.write_str("training-data file length overflow"),
            Self::FileLengthMismatch { expected, actual } => write!(
                formatter,
                "training-data file length mismatch: expected {expected}, got {actual}"
            ),
            Self::InvalidPieceCode { square, value } => {
                write!(formatter, "invalid piece code {value} at {square:?}")
            }
            Self::InvalidSideToMove { value } => {
                write!(formatter, "invalid side-to-move code: {value}")
            }
            Self::InvalidLionSquare { value } => {
                write!(formatter, "invalid lion-capture square code: {value}")
            }
            Self::InvalidLionPromotionFlag { value } => {
                write!(formatter, "invalid lion-promotion flag: {value}")
            }
            Self::LionPromotionWithoutSquare => {
                formatter.write_str("lion-promotion flag requires a lion-capture square")
            }
            Self::InvalidOutcome { value } => write!(formatter, "invalid outcome code: {value}"),
            Self::NonzeroReservedBytes => {
                formatter.write_str("record reserved bytes must all be zero")
            }
            Self::PositionBuild(error) => write!(formatter, "cannot restore position: {error}"),
            Self::Position(error) => write!(formatter, "restored position is invalid: {error}"),
            Self::LionPromotionMismatch { recorded, restored } => write!(
                formatter,
                "lion-promotion flag mismatch: recorded {recorded}, restored {restored}"
            ),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for Error {
    /// 原因となった下位エラーを返す。
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::PositionBuild(error) => Some(error),
            Self::Position(error) => Some(error),
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

impl From<PositionBuildError> for Error {
    /// 局面構築エラーを学習データエラーへ変換する。
    fn from(error: PositionBuildError) -> Self {
        Self::PositionBuild(error)
    }
}

impl From<PositionError> for Error {
    /// 局面不変条件エラーを学習データエラーへ変換する。
    fn from(error: PositionError) -> Self {
        Self::Position(error)
    }
}

/// 学習データファイルの来歴とレコード配置を表すヘッダ。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Header {
    /// 採用した規則セットの表示文字列。
    rule_set: String,
    /// 生成元コミットの完全ハッシュ。
    generation_commit: String,
    /// 生成時に使ったネットの検査和。
    network_checksum: [u8; CHECKSUM_LEN],
    /// 教師探索のノード上限。
    teacher_nodes: u32,
    /// データ生成の基本シード。
    seed: u64,
    /// ファイルに格納したレコード数。
    record_count: u64,
}

impl Header {
    /// 指定した来歴から検証済みヘッダを作る。
    pub fn new(
        rule_set: String,
        generation_commit: String,
        network_checksum: [u8; CHECKSUM_LEN],
        teacher_nodes: u32,
        seed: u64,
        record_count: u64,
    ) -> Result<Self, Error> {
        validate_rule_set_name(&rule_set)?;
        validate_generation_commit(&generation_commit)?;
        Ok(Self {
            rule_set,
            generation_commit,
            network_checksum,
            teacher_nodes,
            seed,
            record_count,
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

    /// 教師探索のノード上限を返す。
    pub const fn teacher_nodes(&self) -> u32 {
        self.teacher_nodes
    }

    /// データ生成の基本シードを返す。
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// ファイルに格納したレコード数を返す。
    pub const fn record_count(&self) -> u64 {
        self.record_count
    }

    /// ヘッダを固定長の二進表現へ符号化する。
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut bytes = [0_u8; HEADER_LEN];
        bytes[0..4].copy_from_slice(&MAGIC);
        bytes[4..8].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        bytes[8..12].copy_from_slice(&(RECORD_LEN as u32).to_le_bytes());
        let rule_bytes = self.rule_set.as_bytes();
        bytes[12..12 + rule_bytes.len()].copy_from_slice(rule_bytes);
        bytes[44..84].copy_from_slice(self.generation_commit.as_bytes());
        bytes[84..116].copy_from_slice(&self.network_checksum);
        bytes[116..120].copy_from_slice(&self.teacher_nodes.to_le_bytes());
        bytes[120..128].copy_from_slice(&self.seed.to_le_bytes());
        bytes[128..136].copy_from_slice(&self.record_count.to_le_bytes());
        bytes
    }

    /// 固定長の二進表現を検証してヘッダへ復号する。
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != HEADER_LEN {
            return Err(Error::InvalidHeaderLength {
                actual: bytes.len(),
            });
        }
        let actual_magic = bytes[0..4].try_into().expect("slice length is fixed");
        if actual_magic != MAGIC {
            return Err(Error::InvalidMagic {
                actual: actual_magic,
            });
        }
        let version = read_u32(bytes, 4);
        if version != FORMAT_VERSION {
            return Err(Error::UnsupportedVersion { actual: version });
        }
        let record_length = read_u32(bytes, 8);
        if record_length != RECORD_LEN as u32 {
            return Err(Error::UnsupportedRecordLength {
                actual: record_length,
            });
        }

        let encoded_rule_set = &bytes[12..44];
        let rule_end = encoded_rule_set
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(RULE_SET_LEN);
        if encoded_rule_set[rule_end..].iter().any(|&byte| byte != 0) {
            return Err(Error::InvalidEncodedRuleSetName);
        }
        let rule_set = std::str::from_utf8(&encoded_rule_set[..rule_end])
            .map_err(|_| Error::InvalidEncodedRuleSetName)?
            .to_owned();
        let generation_commit = std::str::from_utf8(&bytes[44..84])
            .map_err(|_| Error::InvalidGenerationCommit)?
            .to_owned();
        let network_checksum = bytes[84..116].try_into().expect("slice length is fixed");
        Self::new(
            rule_set,
            generation_commit,
            network_checksum,
            read_u32(bytes, 116),
            read_u64(bytes, 120),
            read_u64(bytes, 128),
        )
    }
}

/// 手番側から見た終局結果。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Outcome {
    /// 手番側の負け。
    Loss,
    /// 引き分け。
    Draw,
    /// 手番側の勝ち。
    Win,
}

impl Outcome {
    /// 対局結果を指定した陣営の視点へ変換する。
    pub fn from_game_result(result: GameResult, side: Color) -> Self {
        match result {
            GameResult::Win { winner, .. } if winner == side => Self::Win,
            GameResult::Win { .. } => Self::Loss,
            GameResult::Draw { .. } => Self::Draw,
        }
    }

    /// 二進形式の結果コードを返す。
    const fn code(self) -> u8 {
        match self {
            Self::Loss => 0,
            Self::Draw => 1,
            Self::Win => 2,
        }
    }

    /// 二進形式の結果コードを復号する。
    const fn from_code(value: u8) -> Result<Self, Error> {
        match value {
            0 => Ok(Self::Loss),
            1 => Ok(Self::Draw),
            2 => Ok(Self::Win),
            _ => Err(Error::InvalidOutcome { value }),
        }
    }
}

/// 1探索局面とその教師値を表す固定長レコード。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Record {
    /// dense index順の盤面コード。
    board: [u8; BOARD_SQUARE_COUNT],
    /// 記録局面の手番側。
    side_to_move: Color,
    /// 先獅子の対象升。
    lion_square: Option<Square>,
    /// 獅子を取った麒麟が同じ着手で成ったかどうか。
    lion_by_kirin_promotion: bool,
    /// 手番側視点の探索値。
    score: i16,
    /// 手番側から見た終局結果。
    outcome: Outcome,
    /// 1から始まる対局番号。
    game_number: u32,
    /// 記録時点の手数。
    ply: u16,
}

impl Record {
    /// 局面と教師情報からレコードを作る。
    pub fn from_position(
        position: &Position,
        score: i16,
        outcome: Outcome,
        game_number: u32,
        ply: u16,
    ) -> Self {
        let mut board = [0_u8; BOARD_SQUARE_COUNT];
        for square in Square::all() {
            if let Some(piece) = position.piece_at(square) {
                board[square.dense_index()] = encode_piece(piece);
            }
        }
        let lion_trigger = position.lion_taken_by_non_lion();
        Self {
            board,
            side_to_move: position.side_to_move(),
            lion_square: lion_trigger.map(|trigger| trigger.square),
            lion_by_kirin_promotion: lion_trigger.is_some_and(|trigger| trigger.by_kirin_promotion),
            score,
            outcome,
            game_number,
            ply,
        }
    }

    /// 記録局面の手番側を返す。
    pub const fn side_to_move(&self) -> Color {
        self.side_to_move
    }

    /// 先獅子の対象升を返す。
    pub const fn lion_square(&self) -> Option<Square> {
        self.lion_square
    }

    /// 先獅子の麒麟成りフラグを返す。
    pub const fn lion_by_kirin_promotion(&self) -> bool {
        self.lion_by_kirin_promotion
    }

    /// 手番側視点の探索値を返す。
    pub const fn score(&self) -> i16 {
        self.score
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

    /// レコードを固定長の二進表現へ符号化する。
    pub fn encode(&self) -> [u8; RECORD_LEN] {
        let mut bytes = [0_u8; RECORD_LEN];
        bytes[..BOARD_SQUARE_COUNT].copy_from_slice(&self.board);
        bytes[144] = self.side_to_move as u8;
        bytes[145] = self
            .lion_square
            .map_or(NO_LION_SQUARE, |square| square.dense_index() as u8);
        bytes[146] = u8::from(self.lion_by_kirin_promotion);
        bytes[147..149].copy_from_slice(&self.score.to_le_bytes());
        bytes[149] = self.outcome.code();
        bytes[150..154].copy_from_slice(&self.game_number.to_le_bytes());
        bytes[154..156].copy_from_slice(&self.ply.to_le_bytes());
        bytes
    }

    /// 固定長の二進表現を検証してレコードへ復号する。
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != RECORD_LEN {
            return Err(Error::InvalidRecordLength {
                actual: bytes.len(),
            });
        }
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
            value => return Err(Error::InvalidSideToMove { value }),
        };
        let lion_square = match bytes[145] {
            NO_LION_SQUARE => None,
            value => {
                Some(Square::from_dense(value as usize).ok_or(Error::InvalidLionSquare { value })?)
            }
        };
        let lion_by_kirin_promotion = match bytes[146] {
            0 => false,
            1 => true,
            value => return Err(Error::InvalidLionPromotionFlag { value }),
        };
        if lion_square.is_none() && lion_by_kirin_promotion {
            return Err(Error::LionPromotionWithoutSquare);
        }
        let outcome = Outcome::from_code(bytes[149])?;
        if bytes[156..160].iter().any(|&byte| byte != 0) {
            return Err(Error::NonzeroReservedBytes);
        }
        Ok(Self {
            board,
            side_to_move,
            lion_square,
            lion_by_kirin_promotion,
            score: i16::from_le_bytes(bytes[147..149].try_into().expect("slice length is fixed")),
            outcome,
            game_number: read_u32(bytes, 150),
            ply: u16::from_le_bytes(bytes[154..156].try_into().expect("slice length is fixed")),
        })
    }

    /// レコードから局面を復元し、不変条件と先獅子状態を検査する。
    pub fn to_position(&self) -> Result<Position, Error> {
        let mut builder = PositionBuilder::new(self.side_to_move);
        for square in Square::all() {
            let value = self.board[square.dense_index()];
            if value != 0 {
                builder.put(square, decode_piece(square, value)?)?;
            }
        }
        let mut position = builder.finish()?;
        position.set_lion_capture(self.lion_square)?;
        position.validate()?;
        let restored_flag = position
            .lion_taken_by_non_lion()
            .is_some_and(|trigger| trigger.by_kirin_promotion);
        if restored_flag != self.lion_by_kirin_promotion {
            return Err(Error::LionPromotionMismatch {
                recorded: self.lion_by_kirin_promotion,
                restored: restored_flag,
            });
        }
        Ok(position)
    }
}

/// 合法な最善手が捕獲または成りなら`true`を返す。
///
/// `best_move`が指定局面の合法手でなければ`IllegalMove`を返し、局面は変更しない。
pub fn best_move_is_tactical(
    position: &Position,
    generator: &MoveGenerator,
    best_move: Move,
) -> Result<bool, IllegalMove> {
    let mut legal_moves = Vec::new();
    generator.generate_moves(position, &mut legal_moves);
    if !legal_moves.contains(&best_move) {
        return Err(IllegalMove(best_move));
    }
    Ok(best_move.promote
        || position
            .captured_squares(best_move)
            .into_iter()
            .any(|square| square.is_some()))
}

/// ヘッダを検証し、レコードを逐次復号する読み込み器。
pub struct Reader<R> {
    /// 読み込み元。
    inner: R,
    /// 復号済みヘッダ。
    header: Header,
    /// 未読レコード数。
    remaining: u64,
}

impl<R: Read + Seek> Reader<R> {
    /// 読み込み元のヘッダとファイル長を検証する。
    pub fn new(mut inner: R) -> Result<Self, Error> {
        let actual_length = inner.seek(SeekFrom::End(0))?;
        inner.seek(SeekFrom::Start(0))?;
        let mut header_bytes = [0_u8; HEADER_LEN];
        inner.read_exact(&mut header_bytes)?;
        let header = Header::decode(&header_bytes)?;
        let records_length = header
            .record_count
            .checked_mul(RECORD_LEN as u64)
            .ok_or(Error::FileLengthOverflow)?;
        let expected_length = (HEADER_LEN as u64)
            .checked_add(records_length)
            .ok_or(Error::FileLengthOverflow)?;
        if actual_length != expected_length {
            return Err(Error::FileLengthMismatch {
                expected: expected_length,
                actual: actual_length,
            });
        }
        let remaining = header.record_count;
        Ok(Self {
            inner,
            header,
            remaining,
        })
    }

    /// 復号済みヘッダを返す。
    pub const fn header(&self) -> &Header {
        &self.header
    }

    /// 次のレコードを復号し、末尾なら`None`を返す。
    pub fn read_record(&mut self) -> Result<Option<Record>, Error> {
        if self.remaining == 0 {
            return Ok(None);
        }
        let mut bytes = [0_u8; RECORD_LEN];
        self.inner.read_exact(&mut bytes)?;
        let record = Record::decode(&bytes)?;
        self.remaining -= 1;
        Ok(Some(record))
    }

    /// 読み込み元を返す。
    pub fn into_inner(self) -> R {
        self.inner
    }
}

impl<R: Read + Seek> Iterator for Reader<R> {
    type Item = Result<Record, Error>;

    /// 次のレコードを復号する。
    fn next(&mut self) -> Option<Self::Item> {
        match self.read_record() {
            Ok(Some(record)) => Some(Ok(record)),
            Ok(None) => None,
            Err(error) => {
                self.remaining = 0;
                Some(Err(error))
            }
        }
    }

    /// 未読レコード数から正確な残数を返す。
    fn size_hint(&self) -> (usize, Option<usize>) {
        match usize::try_from(self.remaining) {
            Ok(remaining) => (remaining, Some(remaining)),
            Err(_) => (usize::MAX, None),
        }
    }
}

/// ヘッダと固定長レコードを逐次書き出す書き込み器。
pub struct Writer<W> {
    /// 書き出し先。
    inner: W,
    /// 完了時にレコード数を書き戻すヘッダ。
    header: Header,
    /// 書き出し済みレコード数。
    record_count: u64,
}

impl<W: Write + Seek> Writer<W> {
    /// レコード数0の仮ヘッダを書いて書き出しを始める。
    pub fn new(mut inner: W, mut header: Header) -> Result<Self, Error> {
        header.record_count = 0;
        inner.write_all(&header.encode())?;
        Ok(Self {
            inner,
            header,
            record_count: 0,
        })
    }

    /// 1レコードを書き出す。
    pub fn write_record(&mut self, record: &Record) -> Result<(), Error> {
        self.inner.write_all(&record.encode())?;
        self.record_count = self
            .record_count
            .checked_add(1)
            .ok_or(Error::FileLengthOverflow)?;
        Ok(())
    }

    /// レコード数をヘッダへ書き戻し、書き出し先を返す。
    pub fn finish(mut self) -> Result<W, Error> {
        self.header.record_count = self.record_count;
        self.inner.seek(SeekFrom::Start(0))?;
        self.inner.write_all(&self.header.encode())?;
        self.inner.flush()?;
        Ok(self.inner)
    }
}

/// 規則セット名が固定欄へ符号化できることを検査する。
fn validate_rule_set_name(rule_set: &str) -> Result<(), Error> {
    if rule_set.len() > RULE_SET_LEN || rule_set.as_bytes().contains(&0) {
        Err(Error::InvalidRuleSetName)
    } else {
        Ok(())
    }
}

/// コミットハッシュが完全ハッシュの表記規約を満たすことを検査する。
fn validate_generation_commit(commit: &str) -> Result<(), Error> {
    if commit.len() == COMMIT_LEN && commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(Error::InvalidGenerationCommit)
    }
}

/// 内部駒コードを安定した永続形式へ符号化する。
fn encode_piece(piece: PieceCode) -> u8 {
    let kind = piece.kind().expect("a board piece must have a kind");
    let color = piece.color().expect("a board piece must have a color");
    1 + kind.index() as u8
        + PIECE_KIND_COUNT as u8 * u8::from(piece.is_promoted())
        + 64 * color as u8
}

/// 永続形式の駒コードを内部駒コードへ復号する。
fn decode_piece(square: Square, value: u8) -> Result<PieceCode, Error> {
    let (color, payload) = match value {
        1..=58 => (Color::Black, value - 1),
        65..=122 => (Color::White, value - 65),
        _ => return Err(Error::InvalidPieceCode { square, value }),
    };
    let promoted = payload >= PIECE_KIND_COUNT as u8;
    let kind_index = payload % PIECE_KIND_COUNT as u8;
    let kind =
        PieceKind::from_index(kind_index).ok_or(Error::InvalidPieceCode { square, value })?;
    if promoted {
        PieceCode::new_promoted(color, kind).ok_or(Error::InvalidPieceCode { square, value })
    } else {
        PieceCode::new(color, kind).ok_or(Error::InvalidPieceCode { square, value })
    }
}

/// 指定オフセットからリトルエンディアンの`u32`を読む。
fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("slice length is fixed"),
    )
}

/// 指定オフセットからリトルエンディアンの`u64`を読む。
fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("slice length is fixed"),
    )
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::test_util::{position, position_from_codes, sq};
    use crate::{DrawReason, WinReason};

    /// テスト用の有効なヘッダを返す。
    fn header(record_count: u64) -> Header {
        Header::new(
            "L0,P0,R1,E0".to_owned(),
            "0123456789abcdef0123456789abcdef01234567".to_owned(),
            [0; CHECKSUM_LEN],
            100_000,
            7,
            record_count,
        )
        .unwrap()
    }

    /// 局面をレコードの符号化と復号を経て往復させる。
    fn round_trip(position: &Position) -> Position {
        let record = Record::from_position(position, -123, Outcome::Draw, 4, 57);
        Record::decode(&record.encode())
            .unwrap()
            .to_position()
            .unwrap()
    }

    // 公開境界は未検証の着手をパニックせず型付きエラーとして拒否する。
    #[test]
    fn tactical_move_classification_rejects_a_move_without_an_origin_piece() {
        let position = Position::empty(Color::Black);
        let invalid = Move {
            from: sq(0, 0),
            mid: None,
            to: sq(0, 1),
            promote: false,
        };
        let before = position.clone();

        assert_eq!(
            best_move_is_tactical(&position, &MoveGenerator::standard(), invalid),
            Err(IllegalMove(invalid))
        );
        assert_eq!(position, before);
    }

    // evaluation.md 203〜215行。盤面、手番、成り状態およびzobristを固定長形式が保存する。
    #[test]
    fn positions_round_trip_with_initial_promoted_and_white_to_move_states() {
        let initial = Position::initial();
        assert_eq!(round_trip(&initial), initial);

        let promoted = position_from_codes(
            Color::Black,
            &[
                (
                    sq(1, 1),
                    PieceCode::new_promoted(Color::Black, PieceKind::WhiteHorse).unwrap(),
                ),
                (
                    sq(4, 4),
                    PieceCode::new_promoted(Color::White, PieceKind::Lion).unwrap(),
                ),
            ],
        );
        assert_eq!(round_trip(&promoted), promoted);

        let white_to_move = position(
            Color::White,
            &[
                (sq(5, 0), Color::Black, PieceKind::King),
                (sq(6, 11), Color::White, PieceKind::King),
            ],
        );
        assert_eq!(round_trip(&white_to_move), white_to_move);
    }

    // evaluation.md 207〜208行。先獅子の対象升と麒麟成り原因を盤面から復元する。
    #[test]
    fn lion_trigger_round_trips_with_and_without_kirin_promotion() {
        let mut ordinary = position(Color::Black, &[(sq(4, 4), Color::White, PieceKind::Bishop)]);
        ordinary.set_lion_capture(Some(sq(4, 4))).unwrap();
        assert_eq!(round_trip(&ordinary), ordinary);

        let trigger_square = sq(7, 7);
        let mut promoted_kirin = position_from_codes(
            Color::Black,
            &[(
                trigger_square,
                PieceCode::new_promoted(Color::White, PieceKind::Lion).unwrap(),
            )],
        );
        promoted_kirin
            .set_lion_capture(Some(trigger_square))
            .unwrap();
        let restored = round_trip(&promoted_kirin);
        assert_eq!(restored, promoted_kirin);
        assert!(
            restored
                .lion_taken_by_non_lion()
                .unwrap()
                .by_kirin_promotion
        );
    }

    // evaluation.md 205行。陣営の間の空き領域と上限外の値は駒に復号できない。
    #[test]
    fn persistent_piece_codes_use_disjoint_color_ranges_and_reject_gaps() {
        for color in Color::ALL {
            for kind in PieceKind::ALL {
                let expected_range = match color {
                    Color::Black => 1..=58,
                    Color::White => 65..=122,
                };
                let unpromoted_code = 1 + kind.index() as u8 + 64 * color as u8;
                assert!(expected_range.contains(&unpromoted_code));
                if let Some(piece) = PieceCode::new(color, kind) {
                    assert_eq!(decode_piece(sq(0, 0), unpromoted_code).unwrap(), piece);
                } else {
                    assert!(matches!(
                        decode_piece(sq(0, 0), unpromoted_code),
                        Err(Error::InvalidPieceCode { .. })
                    ));
                    let mut encoded =
                        Record::from_position(&Position::initial(), 0, Outcome::Draw, 1, 0)
                            .encode();
                    encoded[0] = unpromoted_code;
                    assert!(matches!(
                        Record::decode(&encoded),
                        Err(Error::InvalidPieceCode { .. })
                    ));
                }
                if let Some(promoted) = PieceCode::new_promoted(color, kind) {
                    let promoted_code = encode_piece(promoted);
                    assert!(expected_range.contains(&promoted_code));
                    assert_eq!(decode_piece(sq(0, 0), promoted_code).unwrap(), promoted);
                }
            }
        }

        for value in (59..=64).chain(123..=u8::MAX) {
            assert!(matches!(
                decode_piece(sq(0, 0), value),
                Err(Error::InvalidPieceCode { .. })
            ));

            let mut encoded =
                Record::from_position(&Position::initial(), 0, Outcome::Draw, 1, 0).encode();
            encoded[0] = value;
            assert!(matches!(
                Record::decode(&encoded),
                Err(Error::InvalidPieceCode { .. })
            ));
        }
    }

    // evaluation.md 200〜201行。識別子、版、レコード長および物理長を受理条件にする。
    #[test]
    fn header_and_file_length_mismatches_are_rejected() {
        let valid = header(0).encode();
        for (offset, replacement) in [(0, 0_u8), (4, 2), (8, 159)] {
            let mut invalid = valid;
            invalid[offset] = replacement;
            assert!(Header::decode(&invalid).is_err());
        }

        let bytes = header(1).encode().to_vec();
        assert!(matches!(
            Reader::new(Cursor::new(bytes)),
            Err(Error::FileLengthMismatch { .. })
        ));
    }

    // evaluation.md 210行。結果コードは手番側の勝敗と引き分けの3値である。
    #[test]
    fn game_results_map_to_side_relative_outcomes() {
        let black_win = GameResult::Win {
            winner: Color::Black,
            reason: WinReason::Mate,
        };
        assert_eq!(
            Outcome::from_game_result(black_win, Color::Black),
            Outcome::Win
        );
        assert_eq!(
            Outcome::from_game_result(black_win, Color::White),
            Outcome::Loss
        );
        let draw = GameResult::Draw {
            reason: DrawReason::Repetition,
        };
        assert_eq!(Outcome::from_game_result(draw, Color::Black), Outcome::Draw);
        assert_eq!(Outcome::from_game_result(draw, Color::White), Outcome::Draw);
    }
}
