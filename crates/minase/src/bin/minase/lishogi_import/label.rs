//! 再生した局面の探索と教師ラベルの付与。

use super::{cli::Common, filter::winner, input::InputGame, replay::replay};
use minase::datagen::{
    self, data_error,
    game::{CompletedGame, CompletedRecord},
    statistics::Statistics,
};
use minase::search::{DEFAULT_THREADS, SearchLimits, SearchSnapshot, TranspositionTable, search};
use minase::training::records::{Outcome, Record, best_move_is_tactical};
use minase_core::notation::sfen::{SetupPosition, to_extended_sfen};
use minase_core::{Game, MoveGenerator, Rules};
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeMap, io};

/// `minase data lishogi games`の報告の`games`欄に、棋譜IDをキーとして格納するJSONオブジェクトである。
/// 受理した対局について全欄を出力し、任意値がない欄は省略せずnullにする。
/// `openings`サブコマンドでは、このオブジェクトを出力しない。
/// 対局番号、レーティング、手数および局面数はJSON数値で、いずれも整数である。
#[derive(Serialize)]
pub(super) struct GameDetails {
    /// `game`は受理順に1から付ける対局番号を表す整数のJSON数値で、範囲は1〜4,294,967,295である。
    /// 学習データの`Record::game_number`と一致する識別番号であり、単位はない。
    pub(super) game: u32,
    /// `sente_rating`は入力の先手レーティングを表す整数のJSON数値またはnullで、単位はレーティング点である。
    /// 型の範囲はi32の−2,147,483,648〜2,147,483,647で、受理対局では必ず値がある。
    pub(super) sente_rating: Option<i32>,
    /// `gote_rating`は入力の後手レーティングを表す整数のJSON数値またはnullで、単位はレーティング点である。
    /// 型の範囲はi32の−2,147,483,648〜2,147,483,647で、受理対局では必ず値がある。
    pub(super) gote_rating: Option<i32>,
    /// `speed`は持ち時間の区分を表すJSON文字列またはnullで、単位はない。
    /// 入力の空でない`speed`を優先し、なければ`clock`がオブジェクトなら`"realTime"`とする。
    /// それ以外で`daysPerTurn`が非nullなら`"correspondence"`とする。
    /// 受理対局では必ず値があるが、入力の`speed`の語彙は制限しない。
    pub(super) speed: Option<String>,
    /// `clock`は入力の`clock`をそのまま保持する任意のJSON値で、省略またはnullの入力はnullにする。
    /// 時計情報の内部構造、数値の範囲および単位は本形式では規定せず、検査も換算もしない。
    pub(super) clock: Option<Value>,
    /// `days_per_turn`は入力の`daysPerTurn`をそのまま保持する任意のJSON値で、省略またはnullの入力はnullにする。
    /// 通信対局の1手あたりの持ち時間を保持するが、型、範囲および単位は本形式では規定せず、検査も換算もしない。
    pub(super) days_per_turn: Option<Value>,
    /// `status`は入力の終局状態を保持するJSON文字列またはnullで、単位はない。
    /// 受理対局では`"resign"`または`"royalsLost"`である。
    pub(super) status: Option<String>,
    /// `winner`は入力の勝者を保持するJSON文字列またはnullで、単位はない。
    /// 受理対局では先手を表す`"sente"`または後手を表す`"gote"`である。
    pub(super) winner: Option<String>,
    /// `plies`は入力の`moves`を空白で分割した着手数を表す非負整数のJSON数値で、片側の1着手を1手と数える。
    /// 再生で終局した後に入力に残る着手も含める。
    pub(super) plies: usize,
    /// `recorded_positions`はこの対局から学習データへ書いた局面数を表す非負整数のJSON数値で、0も出力する。
    pub(super) recorded_positions: usize,
    /// `excluded_positions`は局面の除外理由をキー、局面数を非負整数のJSON数値で持つJSONオブジェクトである。
    /// `mate_band`は探索値の絶対値が29,000センチポーン以上、`tactical`は最善手が捕獲または成りの場合を表す。
    /// `repetition`は現在局面の探索キーが同じ対局の再生履歴に既出の場合を表す。
    /// `mate_band`、`tactical`、`repetition`の順で最初に該当した理由だけに数える。
    /// 全3種類のキーを件数0でも出力し、局面数の型の範囲はu64である。
    pub(super) excluded_positions: BTreeMap<&'static str, u64>,
}

pub(super) struct Job {
    pub(super) input: InputGame,
    pub(super) number: u32,
}

pub(super) struct Processed {
    pub(super) completed: CompletedGame,
    pub(super) openings: Vec<(u32, String)>,
    pub(super) score_exclusions: usize,
    pub(super) no_legal_moves: usize,
    pub(super) candidates: usize,
}

pub(super) fn process_job(
    job: &Job,
    common: &Common,
    openings: bool,
    table: &mut TranspositionTable,
) -> io::Result<Processed> {
    let nnue = minase::eval::weights().map_err(data_error)?;
    let limits = SearchLimits::new(None, Some(u64::from(common.nodes.get())), None, None)
        .map_err(data_error)?;
    let mut records = Vec::new();
    let mut stats = Statistics::default();
    let mut retained = Vec::new();
    let mut score_exclusions = 0;
    let mut no_legal_moves = 0;
    let replay = replay(&job.input, openings).map_err(data_error)?;
    let mut histories = minase::search::HistoryTables::new(DEFAULT_THREADS);
    for candidate in &replay.positions {
        let position = &candidate.position;
        let game = Game::from_position(Rules::ENGINE_DEFAULT, position.clone());
        if openings && game.legal_moves().is_empty() {
            no_legal_moves += 1;
            continue;
        }
        let snapshot = SearchSnapshot::from_game(&game).map_err(data_error)?;
        table.clear();
        histories.clear();
        let result = search(
            &nnue,
            &snapshot,
            &limits,
            DEFAULT_THREADS,
            table,
            &mut histories,
        )
        .map_err(data_error)?;
        if openings {
            if result.score.unsigned_abs() > 500 {
                score_exclusions += 1;
                continue;
            }
            let setup = SetupPosition::new(
                position.clone(),
                position.lion_capture_square(),
                u32::from(candidate.ply) + 1,
            )
            .map_err(data_error)?;
            retained.push((u32::from(candidate.ply), to_extended_sfen(&setup)));
        } else if result.score.unsigned_abs() >= datagen::MATE_BAND_START {
            stats.excluded_mate_band += 1;
        } else if best_move_is_tactical(
            position,
            &MoveGenerator::new(Rules::ENGINE_DEFAULT.moves),
            result.best_move,
        )
        .map_err(data_error)?
        {
            stats.excluded_tactical += 1;
        } else if candidate.repeated {
            stats.excluded_repetition += 1;
        } else {
            let outcome = if winner(&job.input) == Some(position.side_to_move()) {
                Outcome::Win
            } else {
                Outcome::Loss
            };
            records.push(CompletedRecord {
                record: Record::from_position(
                    position,
                    i16::try_from(result.score).map_err(data_error)?,
                    outcome,
                    job.number,
                    candidate.ply,
                    minase::training::records::Origin::HumanGame,
                ),
                search_key: position.zobrist() ^ position.rights_zobrist(),
            });
        }
    }
    stats.recorded_positions = records.len() as u64;
    Ok(Processed {
        completed: CompletedGame {
            game_number: job.number,
            records,
            stats,
        },
        openings: retained,
        score_exclusions,
        no_legal_moves,
        candidates: replay.positions.len(),
    })
}
