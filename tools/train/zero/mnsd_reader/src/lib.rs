//! MNSDを再生し、価値を持ち除外されていない局面をファイル順に返す。
//!
//! Pythonの`read(path: str) -> dict`は、次のキーを持つ辞書を返す。
//! ヘッダの`origin`はint（0〜3）、対局が0局ならNoneである。
//! `teacher_kind`（αβ0、MCTS1）、`teacher_search_budget`、`seed`、`game_count`、
//! `entry_count`はint、`rule_set`と`generation_commit`はstr、`engine_default`はbool、
//! `network_checksum`は32バイトのbytesである。
//! `engine_default`は規則名ではなく、解釈した規則と`Rules::ENGINE_DEFAULT`の一致を表す。
//!
//! 配列はすべてリトルエンディアンのbytesであり、NumPyの`frombuffer`で読める。
//! nは選別後の局面数、mは全候補数、gは対局数を表す。
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

use std::{fmt, fs::File, io, io::BufReader, path::Path};

use minase::training::records::{self, Exclusion, Header, Origin, Outcome, Policy, Reader, Value};
use minase_core::{GameResult, Move};

#[cfg(feature = "python")]
mod python;
#[cfg(test)]
mod tests;

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
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Format(error) => Some(error),
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

/// 復号済みのヘッダと、入力順の局面・候補の列。各列の型と意味はモジュールの表に従う。
#[derive(Debug)]
pub struct Records {
    pub header: Header,
    pub origin: Option<Origin>,
    pub game_numbers: Vec<u32>,
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
}

impl Records {
    fn new(header: Header) -> Self {
        Self {
            header,
            origin: None,
            game_numbers: Vec::new(),
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
    let file = File::open(path).map_err(Error::Io)?;
    let mut reader = Reader::new(BufReader::new(file))?;
    let mut output = Records::new(reader.header().clone());
    while let Some(game) = reader.read_game()? {
        let record = game.record();
        let number = record.game_number();
        if record.origin() == Origin::Synthetic {
            return Err(Error::SyntheticOrigin { game: number });
        }
        if let Some(expected) = output.origin
            && expected != record.origin()
        {
            return Err(Error::MixedOrigins {
                game: number,
                expected,
                actual: record.origin(),
            });
        }
        output.origin = Some(record.origin());
        output.game_numbers.push(number);
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
            let ply = u16::try_from(index)
                .ok()
                .and_then(|index| record.start_ply().checked_add(index))
                .ok_or(Error::PlyOverflow {
                    game: number,
                    index,
                })?;
            let visits = match (output.header.teacher_kind(), entry.visits()) {
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
        }
    }
    Ok(output)
}
