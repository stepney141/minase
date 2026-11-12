//! ランダム対局の結果と手数を集計して表示する。

use std::{io, num::NonZeroU64, time::Duration};

use super::Error;

use minase_core::{Color, DrawReason, GameResult, WinReason};

use super::{
    game::{CompletedGame, Outcome},
    text::{move_text, outcome_text},
};

/// 集計に使う勝利理由の一覧。
const WIN_REASONS: [WinReason; 7] = [
    WinReason::RoyalCapture,
    WinReason::Repetition,
    WinReason::PieceExhaustion,
    WinReason::BareKing,
    WinReason::Stalemate,
    WinReason::Mate,
    WinReason::Resignation,
];
/// 集計に使う引き分け理由の一覧。
const DRAW_REASONS: [DrawReason; 4] = [
    DrawReason::Repetition,
    DrawReason::PieceExhaustion,
    DrawReason::BareKing,
    DrawReason::Agreement,
];

/// 全局の結果と手数を集計する。
#[derive(Default)]
pub(super) struct Summary {
    /// 集計した局数。
    games: u64,
    /// 対局者ごとの勝利数。
    wins: [u64; 2],
    /// 引き分け数。
    draws: u64,
    /// 勝利理由ごとの件数。
    win_reasons: [u64; WIN_REASONS.len()],
    /// 引き分け理由ごとの件数。
    draw_reasons: [u64; DRAW_REASONS.len()],
    /// 手数上限による打ち切り数。
    cutoffs: u64,
    /// 全局の手数合計。
    total_plies: u128,
    /// 最短の手数。
    min_plies: Option<u32>,
    /// 最長の手数。
    max_plies: u32,
}

impl Summary {
    /// 1局の結果を集計へ加える。
    fn record(&mut self, completed: &CompletedGame) {
        self.games += 1;
        self.total_plies += u128::from(completed.plies);
        self.min_plies = Some(
            self.min_plies
                .map_or(completed.plies, |minimum| minimum.min(completed.plies)),
        );
        self.max_plies = self.max_plies.max(completed.plies);

        match completed.outcome {
            Outcome::Finished(GameResult::Win { winner, reason }) => {
                self.wins[winner.index()] += 1;
                self.win_reasons[win_reason_index(reason)] += 1;
            }
            Outcome::Finished(GameResult::Draw { reason }) => {
                self.draws += 1;
                self.draw_reasons[draw_reason_index(reason)] += 1;
            }
            Outcome::Cutoff => self.cutoffs += 1,
        }
    }

    /// 全局の集計を表示する。
    pub(super) fn print(
        &self,
        elapsed: Duration,
        output: &mut dyn FnMut(&str) -> io::Result<()>,
    ) -> Result<(), Error> {
        let minimum = self.min_plies.ok_or(Error::NoGames)?;
        let mut output = |line: &str| output(line).map_err(Error::Output);
        let average = self.total_plies as f64 / self.games as f64;
        output(&format!("summary: games={}", self.games))?;
        output(&format!(
            "results: BlackWins={} WhiteWins={} Draws={} Cutoffs={}",
            self.wins[Color::Black.index()],
            self.wins[Color::White.index()],
            self.draws,
            self.cutoffs
        ))?;
        output("win reasons:")?;
        for reason in WIN_REASONS {
            output(&format!(
                "  {reason:?}={}",
                self.win_reasons[win_reason_index(reason)]
            ))?;
        }
        output("draw reasons:")?;
        for reason in DRAW_REASONS {
            output(&format!(
                "  {reason:?}={}",
                self.draw_reasons[draw_reason_index(reason)]
            ))?;
        }
        output(&format!(
            "plies: min={} max={} average={average:.3}",
            minimum, self.max_plies
        ))?;
        output(&format!("elapsed: {:.6} s", elapsed.as_secs_f64()))?;
        Ok(())
    }
}

/// 勝利理由に対応する集計添字を返す。
const fn win_reason_index(reason: WinReason) -> usize {
    match reason {
        WinReason::RoyalCapture => 0,
        WinReason::Repetition => 1,
        WinReason::PieceExhaustion => 2,
        WinReason::BareKing => 3,
        WinReason::Stalemate => 4,
        WinReason::Mate => 5,
        WinReason::Resignation => 6,
    }
}

/// 引き分け理由に対応する集計添字を返す。
const fn draw_reason_index(reason: DrawReason) -> usize {
    match reason {
        DrawReason::Repetition => 0,
        DrawReason::PieceExhaustion => 1,
        DrawReason::BareKing => 2,
        DrawReason::Agreement => 3,
    }
}

/// 1局の結果と、指定時は全手順を表示して集計へ加える。
pub(super) fn report_game(
    game_number: NonZeroU64,
    completed: CompletedGame,
    verbose: bool,
    summary: &mut Summary,
    output: &mut dyn FnMut(&str) -> io::Result<()>,
) -> io::Result<()> {
    output(&format!(
        "game {game_number}: plies={} result={}",
        completed.plies,
        outcome_text(completed.outcome)
    ))?;
    if verbose {
        output(&format!("game {game_number} moves:"))?;
        for (index, &mv) in completed.moves.iter().enumerate() {
            output(&format!("  {}: {}", index + 1, move_text(mv)))?;
        }
    }
    summary.record(&completed);
    Ok(())
}
