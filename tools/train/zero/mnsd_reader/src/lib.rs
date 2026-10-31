//! MNSDを再生し、価値を持ち除外されていない局面をファイル順に返す。
//!
//! Pythonの`read(path: str) -> dict`は、次のキーを持つ辞書を返す。
//! ヘッダの`origin`はint（0〜3）、対局が0局ならNoneである。
//! `teacher_kind`（αβ0、MCTS1）、`teacher_search_budget`、`seed`、`game_count`、
//! `entry_count`はint、`rule_set`と`generation_commit`はstr、`engine_default`はbool、
//! `network_checksum`は32バイトのbytesである。
//! `engine_default`は規則名ではなく、解釈した規則と`Rules::ENGINE_DEFAULT`の一致を表す。
//! `scan(path)`は手順を再生せずにtraining_countとvalidation_countを数える。
//! `read_selected(path, training_indices, include_validation)`は全手順を再生し、
//! ファイル内の訓練局面の通し番号で指定した局面と、指定時だけ検証局面を返す。
//! `read_games(path, games)`は基本シードと対局番号の組の集合を受け取り、
//! 訓練・検証の分割を問わず、指定した対局の採用局面だけを返す。
//! いずれも件数は抽出前の値であり、完全な形式検査は再生時に行う。
//!
//! `control_planes(boards, sides)`はuint8のNumPy配列`(n,144)`と`(n,)`を受け取り、
//! 入力の面95〜158をfloat32のNumPy配列`(n,64,144)`として返す。
//! 不正な駒コード、手番、形、型、および成り権の保留はValueErrorとする。
//!
//! 読み込み関数が返す配列はすべてリトルエンディアンのbytesであり、NumPyの`frombuffer`で読める。
//! nは保持した局面数、mはその全候補数、lはその全合法手数、gはファイルの全対局数を表す。
//! 合法手は反復の履歴を考えず、`MoveGenerator::generate_moves`で生成する。
//! 各局面の手は`records::sorted_legal_moves`と同じ4バイト鍵の順で並べる。
//!
//! | キー | 要素型 | 形と意味 |
//! |---|---|---|
//! | `game_numbers` | u32 | g。局面が選別されなかった対局も含む全対局番号 |
//! | `games` | u32 | n。各局面の対局番号 |
//! | `plies` | u16 | n。開始手数と項目の添字の和 |
//! | `boards` | u8 | n×144。MNSDの駒コード。保留印も保持 |
//! | `sides` | u8 | n。先手0、後手1 |
//! | `lions` | u8 | n。先獅子のdense_index。なしは255 |
//! | `values` | i32 | n。αβはセンチポーン、MCTSは0〜65,535 |
//! | `outcomes` | u8 | n。先手勝ち0、後手勝ち1、引き分け2 |
//! | `visits` | u32 | n。根の総訪問回数。αβは0 |
//! | `policy_offsets` | u64 | n+1。各局面の候補区間。先頭0、末尾m |
//! | `policy_from` | u8 | m。移動元のdense_index |
//! | `policy_mid` | u8 | m。経由升のdense_index。なしは255 |
//! | `policy_to` | u8 | m。移動先のdense_index |
//! | `policy_promote` | u8 | m。成り0か1 |
//! | `policy_probabilities` | u16 | m。65,535を1とする確率 |
//! | `legal_offsets` | u64 | n+1。各局面の合法手区間。先頭0、末尾l |
//! | `legal_from` | u8 | l。移動元のdense_index |
//! | `legal_mid` | u8 | l。経由升のdense_index。なしは255 |
//! | `legal_to` | u8 | l。移動先のdense_index |
//! | `legal_promote` | u8 | l。成り0か1 |

use std::{collections::HashSet, fmt, fs::File, io, io::BufReader, path::Path};

use minase::training::records::{self, Exclusion, Header, Origin, Outcome, Policy, Reader, Value};
use minase_core::{GameResult, Move, MoveGenerator};

mod control;
#[cfg(feature = "python")]
mod python;
mod scan;
#[cfg(test)]
mod tests;

pub use control::control_planes;
pub use scan::scan_file;

/// 入出力、MNSDの形式、および学習用の内容検査の失敗。
#[derive(Debug)]
pub enum Error {
    /// ファイルの入出力に失敗した。
    Io(io::Error),
    /// MNSDの復号または局面の復元に失敗した。
    Format(records::Error),
    /// 対局に由来しない合成局面を含む。
    SyntheticOrigin { game: u32 },
    /// 同じファイル内で対局の由来が異なる。
    MixedOrigins {
        game: u32,
        expected: Origin,
        actual: Origin,
    },
    /// 開始手数と項目の添字の和がu16に収まらない。
    PlyOverflow { game: u32, index: usize },
    /// 抽出の番号が厳密な昇順でないか、訓練局面数の範囲外である。
    InvalidSelection,
    /// 盤と手番の配列長が合わないか、利きの出力要素数を表せない。
    InvalidControlShape,
    /// 利きの面の出力を確保できない。
    AllocateControl(std::collections::TryReserveError),
    /// ネットの入力では扱わない成り権の保留印がある。
    PromotionDeferred { position: usize, square: usize },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "cannot read MNSD: {error}"),
            Self::Format(error) => write!(f, "cannot decode MNSD: {error}"),
            Self::SyntheticOrigin { game } => {
                write!(f, "game {game}: synthetic origin is unsupported")
            }
            Self::MixedOrigins {
                game,
                expected,
                actual,
            } => {
                write!(f, "game {game}: mixed origins ({expected:?}, {actual:?})")
            }
            Self::PlyOverflow { game, index } => {
                write!(f, "game {game}, entry {index}: ply exceeds u16")
            }
            Self::InvalidSelection => write!(
                f,
                "training indices must be strictly increasing and within the training position count"
            ),
            Self::InvalidControlShape => write!(
                f,
                "boards must contain 144 squares per side and the output size must fit in memory"
            ),
            Self::AllocateControl(error) => write!(f, "cannot allocate control planes: {error}"),
            Self::PromotionDeferred { position, square } => write!(
                f,
                "position {position}, square {square}: deferred promotion cannot be encoded"
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Format(error) => Some(error),
            Self::AllocateControl(error) => Some(error),
            _ => None,
        }
    }
}

impl From<records::Error> for Error {
    fn from(error: records::Error) -> Self {
        match error {
            records::Error::Io(error) => Self::Io(error),
            error => Self::Format(error),
        }
    }
}

/// ヘッダ、全対局番号、および抽出前の訓練と検証の局面数。
#[derive(Debug)]
pub struct FileInfo {
    pub header: Header,
    pub origin: Option<Origin>,
    pub game_numbers: Vec<u32>,
    pub training_count: u64,
    pub validation_count: u64,
}

impl FileInfo {
    fn new(header: Header) -> Self {
        Self {
            header,
            origin: None,
            game_numbers: Vec::new(),
            training_count: 0,
            validation_count: 0,
        }
    }

    fn add_game(&mut self, number: u32, origin: Origin) -> Result<(), Error> {
        if origin == Origin::Synthetic {
            return Err(Error::SyntheticOrigin { game: number });
        }
        if let Some(expected) = self.origin
            && expected != origin
        {
            return Err(Error::MixedOrigins {
                game: number,
                expected,
                actual: origin,
            });
        }
        self.origin = Some(origin);
        self.game_numbers.push(number);
        Ok(())
    }
}

/// 復号済みのファイル情報と、入力順の局面、候補、合法手の列。
#[derive(Debug)]
pub struct Records {
    pub info: FileInfo,
    pub games: Vec<u32>,
    pub plies: Vec<u16>,
    pub boards: Vec<u8>,
    pub sides: Vec<u8>,
    pub lions: Vec<u8>,
    pub values: Vec<i32>,
    pub outcomes: Vec<u8>,
    pub visits: Vec<u32>,
    pub policy_offsets: Vec<u64>,
    pub policy_from: Vec<u8>,
    pub policy_mid: Vec<u8>,
    pub policy_to: Vec<u8>,
    pub policy_promote: Vec<u8>,
    pub policy_probabilities: Vec<u16>,
    pub legal_offsets: Vec<u64>,
    pub legal_from: Vec<u8>,
    pub legal_mid: Vec<u8>,
    pub legal_to: Vec<u8>,
    pub legal_promote: Vec<u8>,
}

impl Records {
    fn new(header: Header) -> Self {
        Self {
            info: FileInfo::new(header),
            games: Vec::new(),
            plies: Vec::new(),
            boards: Vec::new(),
            sides: Vec::new(),
            lions: Vec::new(),
            values: Vec::new(),
            outcomes: Vec::new(),
            visits: Vec::new(),
            policy_offsets: vec![0],
            policy_from: Vec::new(),
            policy_mid: Vec::new(),
            policy_to: Vec::new(),
            policy_promote: Vec::new(),
            policy_probabilities: Vec::new(),
            legal_offsets: vec![0],
            legal_from: Vec::new(),
            legal_mid: Vec::new(),
            legal_to: Vec::new(),
            legal_promote: Vec::new(),
        }
    }

    fn push_candidate(&mut self, mv: Move, probability: u16) {
        self.policy_from.push(mv.from.dense_index() as u8);
        self.policy_mid
            .push(mv.mid.map_or(255, |square| square.dense_index() as u8));
        self.policy_to.push(mv.to.dense_index() as u8);
        self.policy_promote.push(u8::from(mv.promote));
        self.policy_probabilities.push(probability);
    }
}

/// 1ファイルを末尾まで検査し、価値を持ち除外されていない項目を返す。
///
/// 最後の項目も同じ条件で選別するため、投了を決めた探索の価値と方策は残る。
/// `Policy::PlayedMove`は確率65,535の候補1つへ展開する。
///
/// # Errors
/// 入出力、形式、由来の混在、合成の由来、および手数の桁あふれを専用エラーで返す。
pub fn read_file(path: &Path) -> Result<Records, Error> {
    read_positions(path, |_, _, _| true)
}

/// 全手順を再生し、基本シードと対局番号の組が集合に含まれる対局の採用局面を返す。
///
/// 訓練・検証の分割を問わず、価値を持ち除外されていない項目を入力順に保持する。
/// 集合は複数ファイルの対局を含めてよく、このファイルにない組は選択されない。
/// 空集合なら局面は保持せず、ファイル全体のヘッダ・対局番号・件数だけを返す。
/// 復号器が一時的に保持する1対局分を除き、局面のメモリは選択した局面数に比例する。
///
/// # Errors
/// 選択しなかった対局も再生し、`read_file`と同じ形式・由来の検査を行う。
pub fn read_games(path: &Path, games: &HashSet<(u64, u32)>) -> Result<Records, Error> {
    read_positions(path, |key, _, _| games.contains(&key))
}

/// 全手順を再生し、指定した訓練局面と、指定時だけ検証局面を保持する。
///
/// 訓練局面の番号は、価値があり除外されていない非検証局面に0から振る。
/// 復号器が一時的に保持する1対局分を除き、局面のメモリは保持件数に比例する。
///
/// # Errors
/// `read_file`の失敗に加え、番号の重複、順序違反、範囲外を返す。
pub fn read_selected(
    path: &Path,
    training_indices: &[u64],
    include_validation: bool,
) -> Result<Records, Error> {
    if training_indices.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(Error::InvalidSelection);
    }
    let mut selected = training_indices.iter().peekable();
    let output = read_positions(path, |_, validation, index| {
        if validation {
            include_validation
        } else if selected.peek().is_some_and(|&&next| next == index) {
            selected.next();
            true
        } else {
            false
        }
    })?;
    if selected.next().is_some() {
        return Err(Error::InvalidSelection);
    }
    Ok(output)
}

/// PythonとRust学習器のhash64と同じ折り返し演算で検証の対局を判定する。
fn is_validation(seed: u64, game: u32) -> bool {
    let mut value = seed ^ u64::from(game).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (value ^ (value >> 31)).is_multiple_of(20)
}

fn read_positions(
    path: &Path,
    mut keep: impl FnMut((u64, u32), bool, u64) -> bool,
) -> Result<Records, Error> {
    let file = File::open(path).map_err(Error::Io)?;
    let mut reader = Reader::new(BufReader::new(file))?;
    let mut output = Records::new(reader.header().clone());
    let generator = MoveGenerator::new(output.info.header.rules().moves);
    let mut legal = Vec::new();
    while let Some(game) = reader.read_game()? {
        let record = game.record();
        let number = record.game_number();
        let key = (output.info.header.seed(), number);
        output.info.add_game(number, record.origin())?;
        let validation = is_validation(output.info.header.seed(), number);
        let outcome = match record.outcome() {
            Outcome::Game(GameResult::Win { winner, .. }) => winner as u8,
            Outcome::Game(GameResult::Draw { .. }) => 2,
            Outcome::NotFromGame => return Err(Error::SyntheticOrigin { game: number }),
        };
        for (index, (entry, position)) in record.entries().iter().zip(game.positions()).enumerate()
        {
            if entry.exclusion() != Exclusion::None {
                continue;
            }
            let value = match entry.value() {
                Value::None => continue,
                Value::AlphaBeta(value) => i32::from(value),
                Value::Mcts(value) => i32::from(value),
            };
            let index_in_split = if validation {
                let index = output.info.validation_count;
                output.info.validation_count += 1;
                index
            } else {
                let index = output.info.training_count;
                output.info.training_count += 1;
                index
            };
            if !keep(key, validation, index_in_split) {
                continue;
            }
            let ply = u16::try_from(index)
                .ok()
                .and_then(|index| record.start_ply().checked_add(index))
                .ok_or(Error::PlyOverflow {
                    game: number,
                    index,
                })?;
            let visits = match (output.info.header.teacher_kind(), entry.visits()) {
                (records::TeacherKind::AlphaBeta, None) => 0,
                (records::TeacherKind::Mcts, Some(visits)) => visits,
                (records::TeacherKind::AlphaBeta, Some(_)) => {
                    return Err(records::Error::UnexpectedVisits.into());
                }
                (records::TeacherKind::Mcts, None) => {
                    return Err(records::Error::MissingVisits.into());
                }
            };
            output.games.push(number);
            output.plies.push(ply);
            output.boards.extend(records::encode_board(position)?);
            output.sides.push(position.side_to_move() as u8);
            output.lions.push(
                position
                    .lion_capture_square()
                    .map_or(255, |square| square.dense_index() as u8),
            );
            output.values.push(value);
            output.outcomes.push(outcome);
            output.visits.push(visits);
            match entry.policy() {
                Policy::None => {}
                Policy::PlayedMove => {
                    let mv = entry
                        .played_move()
                        .ok_or(records::Error::BestMoveWithoutPlayedMove)?;
                    output.push_candidate(mv, u16::MAX);
                }
                Policy::Candidates(candidates) => {
                    for &(mv, probability) in candidates {
                        output.push_candidate(mv, probability);
                    }
                }
            }
            output
                .policy_offsets
                .push(output.policy_probabilities.len() as u64);
            legal.clear();
            generator.generate_moves(position, &mut legal);
            legal.sort_unstable_by_key(|mv| {
                (
                    mv.from.dense_index(),
                    mv.mid.map_or(255, |square| square.dense_index()),
                    mv.to.dense_index(),
                    mv.promote,
                )
            });
            for mv in &legal {
                output.legal_from.push(mv.from.dense_index() as u8);
                output
                    .legal_mid
                    .push(mv.mid.map_or(255, |square| square.dense_index() as u8));
                output.legal_to.push(mv.to.dense_index() as u8);
                output.legal_promote.push(u8::from(mv.promote));
            }
            output.legal_offsets.push(output.legal_from.len() as u64);
        }
    }
    Ok(output)
}
