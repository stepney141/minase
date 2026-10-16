//! 対局と教師情報を保存するMNSD版2の二進形式。
//! 多バイト数値はリトルエンディアンである。以下の範囲は区画の先頭を0とし、上端を含まない。
//!
//! # ヘッダ（152バイト）
//!
//! | 位置 | 型 | 内容と値域 |
//! |---|---|---|
//! | 0..4 | `u8[4]` | 識別子`MNSD` |
//! | 4..8 | u32 | 版2 |
//! | 8..40 | `u8[32]` | UTF-8の規則セット名。NULを含まない最大32バイトを0で埋める |
//! | 40..80 | `u8[40]` | 生成コミット。大小文字を許す40桁のASCII 16進数字 |
//! | 80..112 | `u8[32]` | 教師の重みの検査和。駒得評価では全0 |
//! | 112 | u8 | 教師の種類。αβ探索0、モンテカルロ木探索（MCTS）1 |
//! | 113..120 | `u8[7]` | 予約。全0 |
//! | 120..128 | u64 | 教師の規模。αβのノード上限またはMCTSの探索回数 |
//! | 128..136 | u64 | 生成の基本シード |
//! | 136..144 | u64 | 対局数 |
//! | 144..152 | u64 | 手順の項目の総数 |
//!
//! 規則セット名は[`parse_rule_set`]と[`Rules::from_codes`]の両方を通る必要がある。
//! NUL以後の非0バイト、不正なUTF-8、未知の規則、規則の欠落・重複・矛盾を拒否する。
//! 規模とシードはu64全域を許す。件数は[`Writer::finish`]で確定する。
//!
//! # 対局ヘッダ（161バイト）
//!
//! | 位置 | 型 | 内容と値域 |
//! |---|---|---|
//! | 0..4 | u32 | 対局番号。0も許す。ファイル内では厳密な昇順 |
//! | 4 | u8 | 由来。乱数開始の自己対局0、実戦開始の自己対局1、実戦対局2、合成3 |
//! | 5 | u8 | 結果。先手勝ち0、後手勝ち1、引き分け2 |
//! | 6 | u8 | 終局理由。下記の0〜11 |
//! | 7 | u8 | 予約。0 |
//! | 8..10 | u16 | 開始局面までの着手数。0〜65,535手 |
//! | 10..14 | u32 | 項目数n。1以上で、開始手数+n−1が65,535以下 |
//! | 14..158 | `u8[144]` | `Square::dense_index`順の盤。下記の駒コードと保留印 |
//! | 158 | u8 | 開始局面の手番。先手0、後手1 |
//! | 159 | u8 | 先獅子の対象升。dense_indexの0〜143、なければ255 |
//! | 160 | u8 | 獅子を取った麒麟が同じ着手で成った印。0か1 |
//!
//! 理由0は対局に由来しないことを表し、合成かつ結果2の場合に限る。
//! 合成以外は理由0を使えない。勝ちの理由は王駒の捕獲1、反復2、駒枯れ3、裸玉4、
//! 合法手なし5、詰み6、投了7で、結果0か1の場合に限る。
//! 引き分けの理由は反復8、駒枯れ9、裸玉10、合意11で、結果2の場合に限る。
//! 結果は来歴として保持する。再生だけで決まらない投了・合意もあるため、再生結果との一致は要求しない。
//!
//! 空升は0、駒は`1 + PieceKind::index() + 29×成りビット + 64×陣営`で表す。
//! 先手は1〜58、後手は65〜122のうち、`PieceCode`の検査付きコンストラクタが受理する組だけを許す。
//! 最上位ビット128は成り権保留を表す。空升や保留不適格な駒の印を拒否する。
//! 復元は`PositionBuilder`で配置と保留印を戻し、`set_lion_capture`と`validate`で検査する。
//! 対象升が255なら麒麟成りの印は0とし、印は盤から復元した値と一致しなければならない。
//! 王駒の存在など到達可能性は要求しないので、検査を通る合成局面も保存できる。
//!
//! # 手順の項目と方策の候補
//!
//! k番目の項目は開始局面からk手後の局面を表す。最後の項目だけが指した手を持たない。
//! 固定部はαβで7バイト、MCTSで11バイトであり、その直後に候補を並べる。
//!
//! | 位置 | 型 | 内容と値域 |
//! |---|---|---|
//! | 0..2 | u16 | 指した手の番号。なしは65,535 |
//! | 2..4 | i16 / u16 | 手番側の価値。αβはセンチポーン、MCTSは期待得点で1を65,535とする |
//! | 4 | u8 | 下位2ビットは除外理由。なし0、詰みの帯1、戦術的な局面2、反復の局面3 |
//! | 同上 | 同上 | ビット2は価値なし、ビット3は指した手が最善手、ビット4〜7は0 |
//! | 5..7 | u16 | 方策の候補数。0なら方策なし、ただしビット3が立つ場合を除く |
//! | 7..11 | u32 | MCTSだけの根の総訪問回数。探索しなかった局面では0 |
//!
//! 価値なしなら価値欄は0。ビット3は指した手がある場合だけ使い、候補数は0とする。
//! この印は指した手を確率1とする方策である。価値の種類と訪問回数の有無はヘッダと一致させる。
//! 候補1つは4バイトで、0..2が着手番号u16、2..4が確率u16である。
//! 候補は着手番号の厳密な昇順で、重複、合法手数以上の番号、確率0、確率和が65,535でない列を拒否する。
//! メモリ上の候補も同じ順序で渡す。候補が明示されない合法手の確率は0である。
//!
//! 着手番号は、開始局面からファイルの規則で再生した[`Game::legal_moves`]を並べた0始まりの位置である。
//! R2・R3の禁止手は履歴から除かれる。開始局面より前の履歴は含まない。
//! 整列鍵は移動元dense_index、経由升dense_index（なければ255）、到達升dense_index、成り0か1を
//! 上位から並べた4バイトの符号なし整数とする。鍵の重複と合法手数65,535以上を拒否する。
//! 書き込みでは番号から戻した着手との一致も検査する。着手または候補がある項目だけ合法手を生成する。
//! 終局後にさらに指した手がある記録は拒否する。
//!
//! [`quantize_policy`]は非負の有限な重みを正の和で正規化し、65,535倍を切り捨てる。
//! 不足分は端数の大きい順に1ずつ配り、同率なら番号の小さい順を優先する。
//! 0になった候補を除いて番号順に返す。番号の重複、負・非有限の重み、和0は拒否する。
//! 有限な重みの総和のオーバーフローを避けるため、正規化前に最大重みで全体を割る。
//!
//! 読み込みでは途中切れ、予約欄の非0、未定義値、件数の不一致、末尾の余分なバイトを拒否する。
//! 最終対局を返す前に総数と末尾を検査し、空ファイルはヘッダの直後に末尾を検査する。
//! 入出力に失敗した読み書き器は再利用できない。書き出し先は空のストリームに限る。

use core::fmt;
use std::io::{self, Read, Seek, SeekFrom, Write};

use minase_core::piece::PIECE_KIND_COUNT;
use minase_core::rules::{RuleSetParseError, RulesError, parse_rule_set};
use minase_core::{
    Color, DrawReason, Game, GameError, GameResult, Move, PieceCode, PieceKind, Position,
    PositionBuildError, PositionBuilder, PositionError, Rules, Square, WinReason,
};

/// ファイル識別子。
pub const MAGIC: [u8; 4] = *b"MNSD";
/// 形式の版。
pub const FORMAT_VERSION: u32 = 2;
/// ヘッダ長（バイト）。
pub const HEADER_LEN: usize = 152;
/// 対局ヘッダ長（バイト）。
pub const GAME_HEADER_LEN: usize = 161;

/// 途中で切れた区画。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    /// ファイルヘッダ。
    Header,
    /// 対局ヘッダ。
    GameHeader,
    /// 項目の固定部。
    Entry,
    /// 方策の候補。
    Candidate,
}

/// 形式の検査、局面の復元、入出力の失敗。
#[derive(Debug)]
pub enum Error {
    /// 識別子が異なる。
    InvalidMagic([u8; 4]),
    /// 対応していない版。
    UnsupportedVersion(u32),
    /// 規則セット名が32バイトを超えるかNULを含む。
    InvalidRuleSetName,
    /// 規則セット名がUTF-8かNUL埋めの規約に違反する。
    InvalidEncodedRuleSetName,
    /// 規則セットの構文が不正。
    RuleSetParse(RuleSetParseError),
    /// 規則セットの組み合わせが不正。
    Rules(RulesError),
    /// 生成コミットが40桁の16進ASCIIではない。
    InvalidGenerationCommit,
    /// 教師の種類が0か1ではない。
    InvalidTeacherKind(u8),
    /// 予約欄に非0の値がある。
    NonzeroReservedBytes,
    /// 由来が0〜3ではない。
    InvalidOrigin(u8),
    /// 結果が0〜2ではない。
    InvalidOutcome(u8),
    /// 終局理由が0〜11ではない。
    InvalidEndReason(u8),
    /// 結果と終局理由が整合しない。
    OutcomeReasonMismatch,
    /// 合成の由来と対局に由来しない結果が対応しない。
    OriginOutcomeMismatch,
    /// 項目が1つもない。
    EmptyGame,
    /// 最後の局面の手数がu16を超える。
    PlyOverflow,
    /// 対局番号が直前の番号以下。
    GameNumberNotIncreasing,
    /// 保存用の駒コードが不正。
    InvalidPieceCode(u8),
    /// 手番が0か1ではない。
    InvalidSideToMove(u8),
    /// 先獅子の升が0〜143か255ではない。
    InvalidLionSquare(u8),
    /// 麒麟成りの印が0か1ではない。
    InvalidLionPromotionFlag(u8),
    /// 先獅子の升なしで麒麟成りの印がある。
    LionPromotionWithoutSquare,
    /// 麒麟成りの印が復元局面と異なる。
    LionPromotionMismatch,
    /// 局面の構築に失敗した。
    PositionBuild(PositionBuildError),
    /// 局面の不変条件に違反した。
    Position(PositionError),
    /// 指した手が最後の項目にある。
    MoveOnLastEntry,
    /// 最後以外の項目に指した手がない。
    MissingPlayedMove,
    /// 項目の上位4ビットに未定義の印がある。
    InvalidEntryFlags(u8),
    /// 価値なしの項目の価値欄が非0。
    NonzeroAbsentValue,
    /// 価値の種類がファイルの教師と異なる。
    ValueKindMismatch,
    /// αβの項目に訪問回数がある。
    UnexpectedVisits,
    /// MCTSの項目に訪問回数がない。
    MissingVisits,
    /// 最善手が指した手と同じ印に指した手がない。
    BestMoveWithoutPlayedMove,
    /// 最善手が指した手と同じ印に候補の列がある。
    BestMoveWithCandidates,
    /// 候補の列が空。方策なしには`Policy::None`を使う。
    EmptyPolicy,
    /// 候補数がu16に収まらない。
    TooManyCandidates,
    /// 方策の候補が昇順でない。
    CandidatesNotSorted,
    /// 方策の候補が重複している。
    DuplicateCandidate,
    /// 候補の確率が0。
    ZeroProbability,
    /// 候補の確率和が65,535ではない。
    InvalidProbabilitySum,
    /// 合法手の番号が範囲外。
    InvalidMoveIndex(u16),
    /// 書こうとした着手が合法手と一致しない。
    IllegalMove(Move),
    /// 合法手に同一の整列鍵がある。
    DuplicateMoveKey,
    /// 合法手数が65,535以上。
    TooManyLegalMoves,
    /// 終局後に指した手がある。
    MoveAfterGameOver,
    /// 手順の再生に失敗した。
    Replay(GameError),
    /// 量子化の重みが負か非有限。
    InvalidWeight,
    /// 量子化の重みの和が0。
    ZeroWeightSum,
    /// 量子化の番号が重複。
    DuplicateWeightIndex,
    /// 浮動小数点の誤差により最大剰余法の配分ができない。
    QuantizationPrecision,
    /// 件数がu64に収まらない。
    CountOverflow,
    /// ヘッダの項目総数と対局ごとの合計が異なる。
    EntryCountMismatch,
    /// 必要な区画が途中で切れている。
    Truncated(Section),
    /// 最終対局の後にバイトが残っている。
    TrailingBytes,
    /// 書き出し先が空でない。
    NonemptyOutput,
    /// 以前の入出力・復号失敗後の再利用。
    StreamFailed,
    /// 入出力の失敗。
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RuleSetParse(e) => write!(f, "invalid rule set: {e}"),
            Self::Rules(e) => write!(f, "invalid rules: {e}"),
            Self::PositionBuild(e) => write!(f, "cannot build position: {e}"),
            Self::Position(e) => write!(f, "invalid position: {e}"),
            Self::Replay(e) => write!(f, "cannot replay game: {e}"),
            Self::Io(e) => e.fmt(f),
            _ => write!(f, "invalid MNSD v2 data: {self:?}"),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::RuleSetParse(e) => Some(e),
            Self::Rules(e) => Some(e),
            Self::PositionBuild(e) => Some(e),
            Self::Position(e) => Some(e),
            Self::Replay(e) => Some(e),
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// 教師の価値と探索規模の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TeacherKind {
    /// αβ探索。
    AlphaBeta = 0,
    /// モンテカルロ木探索。
    Mcts = 1,
}

/// 検証済みのファイルヘッダ。件数は書き込み完了時に設定される。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    rule_set: String,
    rules: Rules,
    generation_commit: String,
    network_checksum: [u8; 32],
    teacher_kind: TeacherKind,
    teacher_search_budget: u64,
    seed: u64,
    game_count: u64,
    entry_count: u64,
}
impl Header {
    /// 規則セットと来歴を検査し、件数0のヘッダを作る。
    /// `teacher_search_budget`は教師の探索規模で、αβ探索ならノード上限、モンテカルロ木探索なら探索回数を表す。
    pub fn new(
        rule_set: String,
        generation_commit: String,
        network_checksum: [u8; 32],
        teacher_kind: TeacherKind,
        teacher_search_budget: u64,
        seed: u64,
    ) -> Result<Self, Error> {
        if rule_set.len() > 32 || rule_set.as_bytes().contains(&0) {
            return Err(Error::InvalidRuleSetName);
        }
        let codes = parse_rule_set(&rule_set).map_err(Error::RuleSetParse)?;
        let rules = Rules::from_codes(&codes).map_err(Error::Rules)?;
        if generation_commit.len() != 40
            || !generation_commit.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(Error::InvalidGenerationCommit);
        }
        Ok(Self {
            rule_set,
            rules,
            generation_commit,
            network_checksum,
            teacher_kind,
            teacher_search_budget,
            seed,
            game_count: 0,
            entry_count: 0,
        })
    }
    /// 規則セット名を返す。
    pub fn rule_set(&self) -> &str {
        &self.rule_set
    }
    /// 解釈した規則集合を返す。
    pub fn rules(&self) -> Rules {
        self.rules
    }
    /// 生成コミットを返す。
    pub fn generation_commit(&self) -> &str {
        &self.generation_commit
    }
    /// 教師の重みの検査和を返す。
    pub fn network_checksum(&self) -> &[u8; 32] {
        &self.network_checksum
    }
    /// 教師の種類を返す。
    pub fn teacher_kind(&self) -> TeacherKind {
        self.teacher_kind
    }
    /// 教師の探索規模を返す。αβ探索ならノード上限、モンテカルロ木探索なら探索回数である。
    pub fn teacher_search_budget(&self) -> u64 {
        self.teacher_search_budget
    }
    /// 基本シードを返す。
    pub fn seed(&self) -> u64 {
        self.seed
    }
    /// 対局数を返す。
    pub fn game_count(&self) -> u64 {
        self.game_count
    }
    /// 項目の総数を返す。
    pub fn entry_count(&self) -> u64 {
        self.entry_count
    }

    fn write(&self, w: &mut impl Write) -> Result<(), Error> {
        w.write_all(&MAGIC)?;
        w.write_all(&FORMAT_VERSION.to_le_bytes())?;
        let mut name = [0; 32];
        for (dst, src) in name.iter_mut().zip(self.rule_set.bytes()) {
            *dst = src;
        }
        w.write_all(&name)?;
        w.write_all(self.generation_commit.as_bytes())?;
        w.write_all(&self.network_checksum)?;
        w.write_all(&[self.teacher_kind as u8])?;
        w.write_all(&[0; 7])?;
        for value in [
            self.teacher_search_budget,
            self.seed,
            self.game_count,
            self.entry_count,
        ] {
            w.write_all(&value.to_le_bytes())?;
        }
        Ok(())
    }
    fn read(r: &mut impl Read) -> Result<Self, Error> {
        let bytes = input::<HEADER_LEN>(r, Section::Header)?;
        let mut r = bytes.as_slice();
        let magic = input::<4>(&mut r, Section::Header)?;
        if magic != MAGIC {
            return Err(Error::InvalidMagic(magic));
        }
        let version = u32::from_le_bytes(input(&mut r, Section::Header)?);
        if version != FORMAT_VERSION {
            return Err(Error::UnsupportedVersion(version));
        }
        let name = input::<32>(&mut r, Section::Header)?;
        let mut rule_set = Vec::new();
        let mut padding = false;
        for byte in name {
            if byte == 0 {
                padding = true;
            } else if padding {
                return Err(Error::InvalidEncodedRuleSetName);
            } else {
                rule_set.push(byte);
            }
        }
        let rule_set = String::from_utf8(rule_set).map_err(|_| Error::InvalidEncodedRuleSetName)?;
        let commit = String::from_utf8(input::<40>(&mut r, Section::Header)?.to_vec())
            .map_err(|_| Error::InvalidGenerationCommit)?;
        let checksum = input(&mut r, Section::Header)?;
        let [kind] = input(&mut r, Section::Header)?;
        let kind = match kind {
            0 => TeacherKind::AlphaBeta,
            1 => TeacherKind::Mcts,
            x => return Err(Error::InvalidTeacherKind(x)),
        };
        if input::<7>(&mut r, Section::Header)? != [0; 7] {
            return Err(Error::NonzeroReservedBytes);
        }
        let teacher_search_budget = u64::from_le_bytes(input(&mut r, Section::Header)?);
        let seed = u64::from_le_bytes(input(&mut r, Section::Header)?);
        let mut header = Self::new(
            rule_set,
            commit,
            checksum,
            kind,
            teacher_search_budget,
            seed,
        )?;
        header.game_count = u64::from_le_bytes(input(&mut r, Section::Header)?);
        header.entry_count = u64::from_le_bytes(input(&mut r, Section::Header)?);
        Ok(header)
    }
}

/// 教師データの局面を作った経路。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
#[repr(u8)]
pub enum Origin {
    /// 乱数開始の自己対局。
    RandomSelfplay = 0,
    /// 実戦開始の自己対局。
    HumanOpeningSelfplay = 1,
    /// 実戦対局。
    HumanGame = 2,
    /// 対局に由来しない合成局面。
    Synthetic = 3,
}
impl Origin {
    /// 永続形式の由来コードを検査する。
    pub fn from_code(value: u8) -> Result<Self, Error> {
        match value {
            0 => Ok(Self::RandomSelfplay),
            1 => Ok(Self::HumanOpeningSelfplay),
            2 => Ok(Self::HumanGame),
            3 => Ok(Self::Synthetic),
            x => Err(Error::InvalidOrigin(x)),
        }
    }
}

/// 対局結果、または対局に由来しないことを表す。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// 実際の対局の結果と終局理由。
    Game(GameResult),
    /// 合成局面であり対局結果がない。
    NotFromGame,
}
impl Outcome {
    fn codes(self) -> (u8, u8) {
        match self {
            Self::NotFromGame => (2, 0),
            Self::Game(GameResult::Win { winner, reason }) => (
                winner as u8,
                match reason {
                    WinReason::RoyalCapture => 1,
                    WinReason::Repetition => 2,
                    WinReason::PieceExhaustion => 3,
                    WinReason::BareKing => 4,
                    WinReason::Stalemate => 5,
                    WinReason::Mate => 6,
                    WinReason::Resignation => 7,
                },
            ),
            Self::Game(GameResult::Draw { reason }) => (
                2,
                match reason {
                    DrawReason::Repetition => 8,
                    DrawReason::PieceExhaustion => 9,
                    DrawReason::BareKing => 10,
                    DrawReason::Agreement => 11,
                },
            ),
        }
    }
    fn decode(result: u8, reason: u8) -> Result<Self, Error> {
        if result > 2 {
            return Err(Error::InvalidOutcome(result));
        }
        let winner = if result == 0 {
            Color::Black
        } else {
            Color::White
        };
        let value = match reason {
            0 => Self::NotFromGame,
            1..=7 => Self::Game(GameResult::Win {
                winner,
                reason: match reason {
                    1 => WinReason::RoyalCapture,
                    2 => WinReason::Repetition,
                    3 => WinReason::PieceExhaustion,
                    4 => WinReason::BareKing,
                    5 => WinReason::Stalemate,
                    6 => WinReason::Mate,
                    _ => WinReason::Resignation,
                },
            }),
            8..=11 => Self::Game(GameResult::Draw {
                reason: match reason {
                    8 => DrawReason::Repetition,
                    9 => DrawReason::PieceExhaustion,
                    10 => DrawReason::BareKing,
                    _ => DrawReason::Agreement,
                },
            }),
            x => return Err(Error::InvalidEndReason(x)),
        };
        if value.codes().0 != result {
            return Err(Error::OutcomeReasonMismatch);
        }
        Ok(value)
    }
    fn validate_origin(self, origin: Origin) -> Result<(), Error> {
        if (origin == Origin::Synthetic) != (self == Self::NotFromGame) {
            return Err(Error::OriginOutcomeMismatch);
        }
        Ok(())
    }
}

/// 手番側の価値。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value {
    /// 探索値を持たない。
    None,
    /// αβの探索値（センチポーン）。
    AlphaBeta(i16),
    /// MCTSの期待得点。1を65,535とする。
    Mcts(u16),
}
/// 学習用途ごとの選別に使う除外理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Exclusion {
    /// 除外しない。
    None = 0,
    /// 詰みの帯。
    MateBand = 1,
    /// 戦術的な局面。
    Tactical = 2,
    /// 反復の局面。
    Repetition = 3,
}
/// 方策の指定。候補の列は[`Entry::new`]で検査される。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Policy {
    /// 方策がない。
    None,
    /// 指した手が最善手で、確率1。
    PlayedMove,
    /// 整列鍵の昇順の候補と量子化した確率。
    Candidates(Vec<(Move, u16)>),
}
/// 1局面の着手と教師情報。ファイルの教師との整合は書き込み時に検査する。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    played_move: Option<Move>,
    value: Value,
    exclusion: Exclusion,
    policy: Policy,
    visits: Option<u32>,
}
impl Entry {
    /// 方策の順序、確率和、最善手の印を検査して項目を作る。
    pub fn new(
        played_move: Option<Move>,
        value: Value,
        exclusion: Exclusion,
        policy: Policy,
        visits: Option<u32>,
    ) -> Result<Self, Error> {
        match &policy {
            Policy::PlayedMove if played_move.is_none() => {
                return Err(Error::BestMoveWithoutPlayedMove);
            }
            Policy::Candidates(candidates) => {
                if candidates.is_empty() {
                    return Err(Error::EmptyPolicy);
                }
                if candidates.len() > usize::from(u16::MAX) {
                    return Err(Error::TooManyCandidates);
                }
                validate_probabilities(candidates.iter().map(|(mv, p)| (move_key(*mv), *p)))?;
            }
            _ => {}
        }
        Ok(Self {
            played_move,
            value,
            exclusion,
            policy,
            visits,
        })
    }
    /// 指した手を返す。
    pub fn played_move(&self) -> Option<Move> {
        self.played_move
    }
    /// 価値を返す。
    pub fn value(&self) -> Value {
        self.value
    }
    /// 除外理由を返す。
    pub fn exclusion(&self) -> Exclusion {
        self.exclusion
    }
    /// 方策を返す。
    pub fn policy(&self) -> &Policy {
        &self.policy
    }
    /// 根の総訪問回数を返す。
    pub fn visits(&self) -> Option<u32> {
        self.visits
    }
    fn validate_teacher(&self, kind: TeacherKind) -> Result<(), Error> {
        if matches!(
            (kind, self.value),
            (TeacherKind::AlphaBeta, Value::Mcts(_)) | (TeacherKind::Mcts, Value::AlphaBeta(_))
        ) {
            return Err(Error::ValueKindMismatch);
        }
        match (kind, self.visits) {
            (TeacherKind::AlphaBeta, Some(_)) => Err(Error::UnexpectedVisits),
            (TeacherKind::Mcts, None) => Err(Error::MissingVisits),
            _ => Ok(()),
        }
    }
}

/// 開始局面と途切れない手順を持つ対局記録。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameRecord {
    game_number: u32,
    origin: Origin,
    outcome: Outcome,
    start_ply: u16,
    start_position: Position,
    entries: Vec<Entry>,
}
impl GameRecord {
    /// 局面、由来、手数、および最後だけ着手なしであることを検査する。
    /// 規則・履歴に依存する着手と教師の検査は[`Writer::write_game`]で行う。
    pub fn new(
        game_number: u32,
        origin: Origin,
        outcome: Outcome,
        start_ply: u16,
        start_position: Position,
        entries: Vec<Entry>,
    ) -> Result<Self, Error> {
        outcome.validate_origin(origin)?;
        start_position.validate().map_err(Error::Position)?;
        validate_entry_count(start_ply, entries.len() as u64)?;
        for (i, entry) in entries.iter().enumerate() {
            validate_played_move(entry.played_move.is_some(), i + 1 == entries.len())?;
        }
        Ok(Self {
            game_number,
            origin,
            outcome,
            start_ply,
            start_position,
            entries,
        })
    }
    /// 対局番号を返す。
    pub fn game_number(&self) -> u32 {
        self.game_number
    }
    /// 由来を返す。
    pub fn origin(&self) -> Origin {
        self.origin
    }
    /// 対局結果を返す。
    pub fn outcome(&self) -> Outcome {
        self.outcome
    }
    /// 開始局面の手数を返す。
    pub fn start_ply(&self) -> u16 {
        self.start_ply
    }
    /// 開始局面を返す。
    pub fn start_position(&self) -> &Position {
        &self.start_position
    }
    /// 手順の項目を返す。
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
}

/// 復号した対局と各項目の局面。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedGame {
    record: GameRecord,
    positions: Vec<Position>,
}
impl DecodedGame {
    /// 復号した対局を返す。
    pub fn record(&self) -> &GameRecord {
        &self.record
    }
    /// 項目kの局面を添字kで保持する列を返す。項目数と同じ長さである。
    pub fn positions(&self) -> &[Position] {
        &self.positions
    }
    /// 対局と復元局面の所有権を返す。
    pub fn into_parts(self) -> (GameRecord, Vec<Position>) {
        (self.record, self.positions)
    }
}

fn move_key(mv: Move) -> u32 {
    u32::from_be_bytes([
        mv.from.dense_index() as u8,
        mv.mid.map_or(255, |s| s.dense_index() as u8),
        mv.to.dense_index() as u8,
        u8::from(mv.promote),
    ])
}
/// 履歴を含む合法手を形式の4バイト鍵で整列し、鍵の重複と手数の上限を検査する。
pub fn sorted_legal_moves(game: &Game) -> Result<Vec<Move>, Error> {
    let mut moves = game.legal_moves();
    if moves.len() >= usize::from(u16::MAX) {
        return Err(Error::TooManyLegalMoves);
    }
    moves.sort_unstable_by_key(|mv| move_key(*mv));
    let mut previous = None;
    for mv in &moves {
        let key = move_key(*mv);
        if previous == Some(key) {
            return Err(Error::DuplicateMoveKey);
        }
        previous = Some(key);
    }
    Ok(moves)
}

/// 最大剰余法で重みを量子化し、番号順・非0・和65,535の候補を返す。
pub fn quantize_policy(weights: &[(u16, f64)]) -> Result<Vec<(u16, u16)>, Error> {
    let mut sorted = weights.to_vec();
    sorted.sort_unstable_by_key(|(index, _)| *index);
    let mut previous = None;
    let mut maximum = 0.0_f64;
    for &(index, weight) in &sorted {
        if previous == Some(index) {
            return Err(Error::DuplicateWeightIndex);
        }
        previous = Some(index);
        if !weight.is_finite() || weight < 0.0 {
            return Err(Error::InvalidWeight);
        }
        maximum = maximum.max(weight);
    }
    if maximum == 0.0 {
        return Err(Error::ZeroWeightSum);
    }
    let sum: f64 = sorted.iter().map(|(_, weight)| weight / maximum).sum();
    let mut candidates = Vec::new();
    let mut allocated = 0_u32;
    for (index, weight) in sorted {
        let scaled = (weight / maximum) / sum * f64::from(u16::MAX);
        let floor = scaled.floor() as u16;
        allocated = allocated
            .checked_add(u32::from(floor))
            .ok_or(Error::QuantizationPrecision)?;
        candidates.push((index, floor, scaled.fract()));
    }
    let missing = u32::from(u16::MAX)
        .checked_sub(allocated)
        .ok_or(Error::QuantizationPrecision)?;
    if missing as usize > candidates.len() {
        return Err(Error::QuantizationPrecision);
    }
    candidates.sort_unstable_by(|a, b| b.2.total_cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    for (_, value, _) in candidates.iter_mut().take(missing as usize) {
        *value = value.checked_add(1).ok_or(Error::QuantizationPrecision)?;
    }
    candidates.sort_unstable_by_key(|c| c.0);
    Ok(candidates
        .into_iter()
        .filter(|c| c.1 != 0)
        .map(|(i, p, _)| (i, p))
        .collect())
}

fn validate_probabilities(candidates: impl IntoIterator<Item = (u32, u16)>) -> Result<(), Error> {
    let mut previous = None;
    let mut sum = 0_u64;
    for (index, probability) in candidates {
        if previous == Some(index) {
            return Err(Error::DuplicateCandidate);
        }
        if previous.is_some_and(|p| p > index) {
            return Err(Error::CandidatesNotSorted);
        }
        if probability == 0 {
            return Err(Error::ZeroProbability);
        }
        previous = Some(index);
        sum += u64::from(probability);
    }
    if sum != u64::from(u16::MAX) {
        return Err(Error::InvalidProbabilitySum);
    }
    Ok(())
}
fn validate_entry_count(ply: u16, count: u64) -> Result<(), Error> {
    if count == 0 {
        return Err(Error::EmptyGame);
    }
    if count > u64::from(u16::MAX - ply) + 1 {
        return Err(Error::PlyOverflow);
    }
    Ok(())
}
fn validate_played_move(has_move: bool, last: bool) -> Result<(), Error> {
    match (has_move, last) {
        (true, true) => Err(Error::MoveOnLastEntry),
        (false, false) => Err(Error::MissingPlayedMove),
        _ => Ok(()),
    }
}
fn validate_game_number(previous: Option<u32>, next: u32) -> Result<(), Error> {
    if previous.is_some_and(|p| p >= next) {
        return Err(Error::GameNumberNotIncreasing);
    }
    Ok(())
}
fn input<const N: usize>(r: &mut impl Read, section: Section) -> Result<[u8; N], Error> {
    let mut bytes = [0; N];
    r.read_exact(&mut bytes).map_err(|e| {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            Error::Truncated(section)
        } else {
            Error::Io(e)
        }
    })?;
    Ok(bytes)
}
fn decode_piece(value: u8) -> Result<PieceCode, Error> {
    let (color, payload) = match value {
        1..=58 => (Color::Black, value - 1),
        65..=122 => (Color::White, value - 65),
        _ => return Err(Error::InvalidPieceCode(value)),
    };
    let kind = PieceKind::from_index(payload % PIECE_KIND_COUNT as u8)
        .ok_or(Error::InvalidPieceCode(value))?;
    if payload >= PIECE_KIND_COUNT as u8 {
        PieceCode::new_promoted(color, kind)
    } else {
        PieceCode::new(color, kind)
    }
    .ok_or(Error::InvalidPieceCode(value))
}
/// 升順に144個の駒コードを返す。最上位ビットは成り権保留を表す。
/// 駒種または陣営が不正な場合は局面エラーを返す。
pub fn encode_board(position: &Position) -> Result<[u8; 144], Error> {
    let mut board = [0; 144];
    for square in Square::all() {
        let code = match position.piece_at(square) {
            None => 0,
            Some(piece) => {
                let kind = piece
                    .kind()
                    .ok_or(Error::Position(PositionError::InvalidPieceCode { square }))?;
                let color = piece
                    .color()
                    .ok_or(Error::Position(PositionError::InvalidPieceCode { square }))?;
                1 + kind.index() as u8
                    + PIECE_KIND_COUNT as u8 * u8::from(piece.is_promoted())
                    + 64 * color as u8
            }
        };
        board[square.dense_index()] =
            code | (u8::from(position.promotion_deferred().contains(square)) << 7);
    }
    Ok(board)
}
fn write_position(w: &mut impl Write, position: &Position) -> Result<(), Error> {
    w.write_all(&encode_board(position)?)?;
    let trigger = position.lion_taken_by_non_lion();
    w.write_all(&[
        position.side_to_move() as u8,
        trigger.map_or(255, |t| t.square.dense_index() as u8),
        u8::from(trigger.is_some_and(|t| t.by_kirin_promotion)),
    ])?;
    Ok(())
}
fn read_position(r: &mut impl Read) -> Result<Position, Error> {
    let board = input::<144>(r, Section::GameHeader)?;
    let [side, lion, flag] = input(r, Section::GameHeader)?;
    let side = match side {
        0 => Color::Black,
        1 => Color::White,
        x => return Err(Error::InvalidSideToMove(x)),
    };
    let lion = if lion == 255 {
        None
    } else {
        Some(Square::from_dense(usize::from(lion)).ok_or(Error::InvalidLionSquare(lion))?)
    };
    if flag > 1 {
        return Err(Error::InvalidLionPromotionFlag(flag));
    }
    decode_position(&board, side, lion, flag == 1)
}

/// 駒コード、手番、先獅子の升と麒麟成りの印から検査済みの局面を復元する。
/// 駒コード、成り権保留、先獅子の印と局面の不変条件に違反する入力は拒否する。
pub fn decode_position(
    board: &[u8; 144],
    side: Color,
    lion: Option<Square>,
    lion_promotion: bool,
) -> Result<Position, Error> {
    if lion.is_none() && lion_promotion {
        return Err(Error::LionPromotionWithoutSquare);
    }
    let mut builder = PositionBuilder::new(side);
    for (square, &code) in Square::all().zip(board) {
        let piece = code & 127;
        if piece != 0 {
            builder
                .put(square, decode_piece(piece)?)
                .map_err(Error::PositionBuild)?;
        }
        if code & 128 != 0 {
            builder
                .mark_promotion_deferred(square)
                .map_err(Error::PositionBuild)?;
        }
    }
    let mut position = builder.finish().map_err(Error::PositionBuild)?;
    position.set_lion_capture(lion).map_err(Error::Position)?;
    position.validate().map_err(Error::Position)?;
    if position
        .lion_taken_by_non_lion()
        .is_some_and(|t| t.by_kirin_promotion)
        != lion_promotion
    {
        return Err(Error::LionPromotionMismatch);
    }
    Ok(position)
}
fn move_index(moves: &[Move], mv: Move) -> Result<u16, Error> {
    let index = moves
        .binary_search_by_key(&move_key(mv), |m| move_key(*m))
        .map_err(|_| Error::IllegalMove(mv))?;
    if moves.get(index) != Some(&mv) {
        return Err(Error::IllegalMove(mv));
    }
    u16::try_from(index).map_err(|_| Error::TooManyLegalMoves)
}
fn move_at(moves: &[Move], index: u16) -> Result<Move, Error> {
    moves
        .get(usize::from(index))
        .copied()
        .ok_or(Error::InvalidMoveIndex(index))
}
fn legal_moves_for_entry(
    game: &Game,
    has_move: bool,
    has_candidates: bool,
) -> Result<Vec<Move>, Error> {
    if has_move && game.result().is_some() {
        return Err(Error::MoveAfterGameOver);
    }
    if has_move || has_candidates {
        sorted_legal_moves(game)
    } else {
        Ok(Vec::new())
    }
}
fn write_entry(
    w: &mut impl Write,
    entry: &Entry,
    game: &mut Game,
    kind: TeacherKind,
) -> Result<(), Error> {
    entry.validate_teacher(kind)?;
    let candidates = match &entry.policy {
        Policy::Candidates(c) => c.as_slice(),
        _ => &[],
    };
    let moves = legal_moves_for_entry(game, entry.played_move.is_some(), !candidates.is_empty())?;
    let played = match entry.played_move {
        Some(mv) => move_index(&moves, mv)?,
        None => u16::MAX,
    };
    w.write_all(&played.to_le_bytes())?;
    let value = match entry.value {
        Value::None => [0; 2],
        Value::AlphaBeta(v) => v.to_le_bytes(),
        Value::Mcts(v) => v.to_le_bytes(),
    };
    w.write_all(&value)?;
    let flags = entry.exclusion as u8
        | (u8::from(entry.value == Value::None) << 2)
        | (u8::from(entry.policy == Policy::PlayedMove) << 3);
    w.write_all(&[flags])?;
    w.write_all(
        &u16::try_from(candidates.len())
            .map_err(|_| Error::TooManyCandidates)?
            .to_le_bytes(),
    )?;
    if let Some(visits) = entry.visits {
        w.write_all(&visits.to_le_bytes())?;
    }
    for &(mv, probability) in candidates {
        w.write_all(&move_index(&moves, mv)?.to_le_bytes())?;
        w.write_all(&probability.to_le_bytes())?;
    }
    if let Some(mv) = entry.played_move {
        game.play(mv).map_err(Error::Replay)?;
    }
    Ok(())
}
fn read_entry(
    r: &mut impl Read,
    game: &mut Game,
    kind: TeacherKind,
    last: bool,
) -> Result<Entry, Error> {
    let played = u16::from_le_bytes(input(r, Section::Entry)?);
    let value_bytes = input::<2>(r, Section::Entry)?;
    let [flags] = input(r, Section::Entry)?;
    let count = u16::from_le_bytes(input(r, Section::Entry)?);
    let visits = match kind {
        TeacherKind::AlphaBeta => None,
        TeacherKind::Mcts => Some(u32::from_le_bytes(input(r, Section::Entry)?)),
    };
    if flags & 0xf0 != 0 {
        return Err(Error::InvalidEntryFlags(flags));
    }
    let value = if flags & 4 != 0 {
        if value_bytes != [0; 2] {
            return Err(Error::NonzeroAbsentValue);
        }
        Value::None
    } else {
        match kind {
            TeacherKind::AlphaBeta => Value::AlphaBeta(i16::from_le_bytes(value_bytes)),
            TeacherKind::Mcts => Value::Mcts(u16::from_le_bytes(value_bytes)),
        }
    };
    if flags & 8 != 0 {
        if played == u16::MAX {
            return Err(Error::BestMoveWithoutPlayedMove);
        }
        if count != 0 {
            return Err(Error::BestMoveWithCandidates);
        }
    }
    validate_played_move(played != u16::MAX, last)?;
    let moves = legal_moves_for_entry(game, played != u16::MAX, count != 0)?;
    let played_move = if played == u16::MAX {
        None
    } else {
        Some(move_at(&moves, played)?)
    };
    let mut candidates = Vec::new();
    for _ in 0..count {
        let index = u16::from_le_bytes(input(r, Section::Candidate)?);
        let probability = u16::from_le_bytes(input(r, Section::Candidate)?);
        candidates.push((move_at(&moves, index)?, probability));
    }
    let policy = if flags & 8 != 0 {
        Policy::PlayedMove
    } else if count == 0 {
        Policy::None
    } else {
        Policy::Candidates(candidates)
    };
    let exclusion = match flags & 3 {
        0 => Exclusion::None,
        1 => Exclusion::MateBand,
        2 => Exclusion::Tactical,
        _ => Exclusion::Repetition,
    };
    let entry = Entry::new(played_move, value, exclusion, policy, visits)?;
    if let Some(mv) = played_move {
        game.play(mv).map_err(Error::Replay)?;
    }
    Ok(entry)
}

/// 対局を検証して書き、完了時に件数を確定する書き込み器。
pub struct Writer<W> {
    inner: W,
    header: Header,
    previous_number: Option<u32>,
    failed: bool,
}
impl<W: Write + Seek> Writer<W> {
    /// 空の出力先に対局数0・項目数0の仮ヘッダを書く。
    pub fn new(mut inner: W, mut header: Header) -> Result<Self, Error> {
        if inner.seek(SeekFrom::End(0))? != 0 {
            return Err(Error::NonemptyOutput);
        }
        header.game_count = 0;
        header.entry_count = 0;
        header.write(&mut inner)?;
        Ok(Self {
            inner,
            header,
            previous_number: None,
            failed: false,
        })
    }
    /// 全項目を再生して検査した後、1局を書き出す。検査エラーでは出力を変更しない。
    pub fn write_game(&mut self, record: &GameRecord) -> Result<(), Error> {
        if self.failed {
            return Err(Error::StreamFailed);
        }
        validate_game_number(self.previous_number, record.game_number)?;
        let games = self
            .header
            .game_count
            .checked_add(1)
            .ok_or(Error::CountOverflow)?;
        let entries = self
            .header
            .entry_count
            .checked_add(record.entries.len() as u64)
            .ok_or(Error::CountOverflow)?;
        let mut bytes = Vec::new();
        bytes.write_all(&record.game_number.to_le_bytes())?;
        let (outcome, reason) = record.outcome.codes();
        bytes.write_all(&[record.origin as u8, outcome, reason, 0])?;
        bytes.write_all(&record.start_ply.to_le_bytes())?;
        bytes.write_all(&(record.entries.len() as u32).to_le_bytes())?;
        write_position(&mut bytes, &record.start_position)?;
        let mut game = Game::from_position(self.header.rules, record.start_position.clone());
        for entry in &record.entries {
            write_entry(&mut bytes, entry, &mut game, self.header.teacher_kind)?;
        }
        if let Err(error) = self.inner.write_all(&bytes) {
            self.failed = true;
            return Err(Error::Io(error));
        }
        self.header.game_count = games;
        self.header.entry_count = entries;
        self.previous_number = Some(record.game_number);
        Ok(())
    }
    /// ヘッダへ件数を書き戻してフラッシュし、末尾に位置する出力先を返す。
    pub fn finish(mut self) -> Result<W, Error> {
        if self.failed {
            return Err(Error::StreamFailed);
        }
        self.inner.seek(SeekFrom::Start(0))?;
        self.header.write(&mut self.inner)?;
        self.inner.seek(SeekFrom::End(0))?;
        self.inner.flush()?;
        Ok(self.inner)
    }
}

/// 対局を逐次復号し、手順を再生して各項目の局面を返す読み込み器。
pub struct Reader<R> {
    inner: R,
    header: Header,
    read_games: u64,
    read_entries: u64,
    previous_number: Option<u32>,
    failed: bool,
}
impl<R: Read> Reader<R> {
    /// ヘッダを検査する。対局数0なら項目数0と直後の末尾も検査する。
    pub fn new(mut inner: R) -> Result<Self, Error> {
        let header = Header::read(&mut inner)?;
        let mut reader = Self {
            inner,
            header,
            read_games: 0,
            read_entries: 0,
            previous_number: None,
            failed: false,
        };
        if reader.header.game_count == 0 {
            reader.check_end()?;
        }
        Ok(reader)
    }
    /// 復号したヘッダを返す。
    pub fn header(&self) -> &Header {
        &self.header
    }
    /// 次の対局を返す。最終対局を返す前に項目総数と末尾を検査する。
    pub fn read_game(&mut self) -> Result<Option<DecodedGame>, Error> {
        if self.failed {
            return Err(Error::StreamFailed);
        }
        let result = self.read_next();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    /// 入力の所有権を返す。未読の対局は検査しない。
    pub fn into_inner(self) -> R {
        self.inner
    }
    fn read_next(&mut self) -> Result<Option<DecodedGame>, Error> {
        if self.read_games == self.header.game_count {
            return Ok(None);
        }
        let bytes = input::<GAME_HEADER_LEN>(&mut self.inner, Section::GameHeader)?;
        let mut r = bytes.as_slice();
        let number = u32::from_le_bytes(input(&mut r, Section::GameHeader)?);
        validate_game_number(self.previous_number, number)?;
        let [origin, outcome, reason, reserved] = input(&mut r, Section::GameHeader)?;
        let origin = Origin::from_code(origin)?;
        let outcome = Outcome::decode(outcome, reason)?;
        outcome.validate_origin(origin)?;
        if reserved != 0 {
            return Err(Error::NonzeroReservedBytes);
        }
        let ply = u16::from_le_bytes(input(&mut r, Section::GameHeader)?);
        let count = u32::from_le_bytes(input(&mut r, Section::GameHeader)?);
        validate_entry_count(ply, u64::from(count))?;
        let total = self
            .read_entries
            .checked_add(u64::from(count))
            .ok_or(Error::CountOverflow)?;
        if total > self.header.entry_count {
            return Err(Error::EntryCountMismatch);
        }
        let start = read_position(&mut r)?;
        let mut game = Game::from_position(self.header.rules, start.clone());
        let mut entries = Vec::new();
        let mut positions = Vec::new();
        for index in 0..count {
            positions.push(game.position().clone());
            entries.push(read_entry(
                &mut self.inner,
                &mut game,
                self.header.teacher_kind,
                index + 1 == count,
            )?);
        }
        let record = GameRecord::new(number, origin, outcome, ply, start, entries)?;
        self.read_entries = total;
        self.read_games = self.read_games.checked_add(1).ok_or(Error::CountOverflow)?;
        self.previous_number = Some(number);
        if self.read_games == self.header.game_count {
            self.check_end()?;
        }
        Ok(Some(DecodedGame { record, positions }))
    }
    fn check_end(&mut self) -> Result<(), Error> {
        if self.read_entries != self.header.entry_count {
            return Err(Error::EntryCountMismatch);
        }
        let mut byte = [0];
        loop {
            match self.inner.read(&mut byte) {
                Ok(0) => return Ok(()),
                Ok(_) => return Err(Error::TrailingBytes),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(Error::Io(e)),
            }
        }
    }
}
