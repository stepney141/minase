//! 対局結果から保存形式への変換。

use crate::harness::engine::{EngineEvaluation, EngineProcess, EngineResourceUsage, EngineScore};
use crate::harness::failure::EngineFailure;
use crate::harness::game::{GameOutcome, PlayedGame};
use crate::harness::records::{
    EvaluationRecord, FailureKind, GameRecord, ScoreRecord, StoredColor, TerminationRecord,
    TurnRecord,
};
use crate::{Color, GameResult};
use std::num::NonZeroU64;
use std::time::{Duration, Instant};

/// 1局の集計結果と監査記録。
pub struct RecordedGame {
    /// 既存の得点計算と表示に使う終局結果。
    pub played: PlayedGame,
    /// 永続化する構造化記録。
    pub record: GameRecord,
}

/// 手番を保存形式へ変換する。
pub const fn stored_color(color: Color) -> StoredColor {
    match color {
        Color::Black => StoredColor::Black,
        Color::White => StoredColor::White,
    }
}

/// エンジン異常を保存形式へ変換する。
pub(in crate::harness) const fn stored_failure(failure: EngineFailure) -> FailureKind {
    match failure {
        EngineFailure::IllegalMove => FailureKind::IllegalMove,
        EngineFailure::Crash => FailureKind::Crash,
        EngineFailure::Timeout => FailureKind::Timeout,
        EngineFailure::TimeForfeit => FailureKind::TimeForfeit,
        EngineFailure::RejectedMove => FailureKind::RejectedMove,
    }
}

/// 保存形式の異常分類を集計用の分類へ戻す。
pub const fn failure_from_stored(reason: FailureKind) -> EngineFailure {
    match reason {
        FailureKind::IllegalMove => EngineFailure::IllegalMove,
        FailureKind::Crash => EngineFailure::Crash,
        FailureKind::Timeout => EngineFailure::Timeout,
        FailureKind::TimeForfeit => EngineFailure::TimeForfeit,
        FailureKind::RejectedMove => EngineFailure::RejectedMove,
    }
}

/// エンジン評価値へ視点を付けて保存形式へ変換する。
pub(in crate::harness) fn evaluation_record(
    evaluation: Option<EngineEvaluation>,
    perspective: Color,
) -> Option<EvaluationRecord> {
    evaluation.map(|evaluation| EvaluationRecord {
        perspective: stored_color(perspective),
        depth: evaluation.depth,
        score: match evaluation.score {
            EngineScore::Cp(value) => ScoreRecord::Cp { value },
            EngineScore::MateIn(moves) => ScoreRecord::MateIn { moves },
            EngineScore::MatedIn(moves) => ScoreRecord::MatedIn { moves },
        },
        bound: evaluation.bound,
    })
}

/// `Duration`を保存形式のナノ秒へ変換する。
pub(in crate::harness) fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).expect("an engine response duration must fit in u64 ns")
}

/// 終局結果を保存形式へ変換する。
fn termination_record(game: PlayedGame) -> TerminationRecord {
    match game {
        PlayedGame::Finished {
            outcome: GameOutcome::Adjudicated(GameResult::Win { winner, reason }),
            ..
        } => TerminationRecord::AdjudicatedWin {
            winner: stored_color(winner),
            reason: format!("{reason:?}"),
        },
        PlayedGame::Finished {
            outcome: GameOutcome::Adjudicated(GameResult::Draw { reason }),
            ..
        } => TerminationRecord::AdjudicatedDraw {
            reason: format!("{reason:?}"),
        },
        PlayedGame::Finished {
            outcome: GameOutcome::Forfeit { winner, reason },
            ..
        } => TerminationRecord::Forfeit {
            loser: stored_color(winner.opposite()),
            reason: stored_failure(reason),
        },
        PlayedGame::Finished {
            outcome: GameOutcome::Resigned { winner },
            ..
        } => TerminationRecord::Resigned {
            loser: stored_color(winner.opposite()),
        },
        PlayedGame::Cutoff { .. } => TerminationRecord::Cutoff,
    }
}

/// 1局の終局結果と収集済み着手をまとめる。
#[allow(clippy::too_many_arguments)]
pub(in crate::harness) fn recorded_game(
    played: PlayedGame,
    candidate_color: Color,
    candidate_seed: NonZeroU64,
    baseline_seed: NonZeroU64,
    turns: Vec<TurnRecord>,
    started: Instant,
    mut candidate_process: Option<&mut EngineProcess>,
    mut baseline_process: Option<&mut EngineProcess>,
) -> RecordedGame {
    if let Some(process) = candidate_process.as_mut() {
        process.stop_ponder();
    }
    if let Some(process) = baseline_process.as_mut() {
        process.stop_ponder();
    }
    let candidate_usage = candidate_process.map_or_else(EngineResourceUsage::default, |process| {
        process.resource_usage()
    });
    let baseline_usage = baseline_process.map_or_else(EngineResourceUsage::default, |process| {
        process.resource_usage()
    });
    RecordedGame {
        played,
        record: GameRecord {
            candidate_color: stored_color(candidate_color),
            candidate_seed: candidate_seed.get(),
            baseline_seed: baseline_seed.get(),
            wall_time_ns: duration_ns(started.elapsed()),
            candidate_cpu_time_ns: candidate_usage.cpu_time_ns,
            baseline_cpu_time_ns: baseline_usage.cpu_time_ns,
            candidate_peak_rss_bytes: candidate_usage.peak_rss_bytes,
            baseline_peak_rss_bytes: baseline_usage.peak_rss_bytes,
            turns,
            termination: termination_record(played),
        },
    }
}
