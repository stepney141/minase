//! 学習局面の生成結果と書き出したレコードの集計。

use std::collections::HashSet;

use crate::{Color, DrawReason, GameResult, WinReason};

use super::game::CompletedRecord;

/// 注入オフセットのヒストグラム区間数。
pub const INJECTION_HISTOGRAM_BINS: usize = 8;
/// 探索値の取り得る値の数。
pub const SCORE_VALUE_COUNT: usize = 65_536;

/// データ生成全体または1局分の統計。
#[derive(Default, PartialEq, Eq, Debug)]
pub struct Statistics {
    /// 手数上限で破棄した対局数。
    pub discarded_games: u64,
    /// 先手勝ちの対局数。
    pub black_wins: u64,
    /// 後手勝ちの対局数。
    pub white_wins: u64,
    /// 引き分けの対局数。
    pub draws: u64,
    /// 王駒捕獲による勝利数。
    pub royal_capture_wins: u64,
    /// 反復裁定による勝利数。
    pub repetition_wins: u64,
    /// 駒枯れによる勝利数。
    pub piece_exhaustion_wins: u64,
    /// 裸玉による勝利数。
    pub bare_king_wins: u64,
    /// 合法手なしによる勝利数。
    pub stalemate_wins: u64,
    /// 詰みによる勝利数。
    pub mate_wins: u64,
    /// 投了による勝利数。
    pub resignation_wins: u64,
    /// 反復裁定による引き分け数。
    pub repetition_draws: u64,
    /// 駒枯れによる引き分け数。
    pub piece_exhaustion_draws: u64,
    /// 裸玉による引き分け数。
    pub bare_king_draws: u64,
    /// 合意による引き分け数。
    pub agreement_draws: u64,
    /// 探索した局面数。
    pub searched_positions: u64,
    /// 記録境界以後に探索した局面数。
    pub recordable_positions: u64,
    /// ファイルへ記録した局面数。
    pub recorded_positions: u64,
    /// 詰み帯の探索値による除外数。
    pub excluded_mate_band: u64,
    /// 捕獲または成りの最善手による除外数。
    pub excluded_tactical: u64,
    /// 現局面の再出現による除外数。
    pub excluded_repetition: u64,
    /// ランダム手を含む全対局の総手数。
    pub total_plies: u64,
    /// 探索で指した総手数。
    pub searched_plies: u64,
    /// 探索が訪問したノード合計。
    pub searched_nodes: u64,
    /// 予定したランダム着手の合計。
    pub planned_injections: u64,
    /// 実施したランダム着手の合計。
    pub performed_injections: u64,
    /// 実施した注入オフセットを10手幅で数えた度数。
    pub injection_offset_histogram: [u64; INJECTION_HISTOGRAM_BINS],
}

impl Statistics {
    /// 1局分の統計を合計へ加える。
    pub(super) fn merge(&mut self, other: &Self) {
        self.discarded_games += other.discarded_games;
        self.black_wins += other.black_wins;
        self.white_wins += other.white_wins;
        self.draws += other.draws;
        self.royal_capture_wins += other.royal_capture_wins;
        self.repetition_wins += other.repetition_wins;
        self.piece_exhaustion_wins += other.piece_exhaustion_wins;
        self.bare_king_wins += other.bare_king_wins;
        self.stalemate_wins += other.stalemate_wins;
        self.mate_wins += other.mate_wins;
        self.resignation_wins += other.resignation_wins;
        self.repetition_draws += other.repetition_draws;
        self.piece_exhaustion_draws += other.piece_exhaustion_draws;
        self.bare_king_draws += other.bare_king_draws;
        self.agreement_draws += other.agreement_draws;
        self.searched_positions += other.searched_positions;
        self.recordable_positions += other.recordable_positions;
        self.recorded_positions += other.recorded_positions;
        self.excluded_mate_band += other.excluded_mate_band;
        self.excluded_tactical += other.excluded_tactical;
        self.excluded_repetition += other.excluded_repetition;
        self.total_plies += other.total_plies;
        self.searched_plies += other.searched_plies;
        self.searched_nodes += other.searched_nodes;
        self.planned_injections += other.planned_injections;
        self.performed_injections += other.performed_injections;
        for (total, count) in self
            .injection_offset_histogram
            .iter_mut()
            .zip(other.injection_offset_histogram)
        {
            *total += count;
        }
    }

    /// 終局理由と勝敗を集計する。
    pub fn record_result(&mut self, result: GameResult) {
        match result {
            GameResult::Win { winner, reason } => {
                match winner {
                    Color::Black => self.black_wins += 1,
                    Color::White => self.white_wins += 1,
                }
                match reason {
                    WinReason::RoyalCapture => self.royal_capture_wins += 1,
                    WinReason::Repetition => self.repetition_wins += 1,
                    WinReason::PieceExhaustion => self.piece_exhaustion_wins += 1,
                    WinReason::BareKing => self.bare_king_wins += 1,
                    WinReason::Stalemate => self.stalemate_wins += 1,
                    WinReason::Mate => self.mate_wins += 1,
                    WinReason::Resignation => self.resignation_wins += 1,
                }
            }
            GameResult::Draw { reason } => {
                self.draws += 1;
                match reason {
                    DrawReason::Repetition => self.repetition_draws += 1,
                    DrawReason::PieceExhaustion => self.piece_exhaustion_draws += 1,
                    DrawReason::BareKing => self.bare_king_draws += 1,
                    DrawReason::Agreement => self.agreement_draws += 1,
                }
            }
        }
    }
}

/// 書き出したレコードから対局横断で求める統計。
pub struct RecordedStatistics {
    /// i16の全探索値に対応する度数表。
    pub score_frequencies: Vec<u64>,
    /// 既出局面の探索キー。
    pub search_keys: HashSet<u64>,
    /// 以前の対局にも現れた局面数。
    pub duplicate_positions: u64,
}

impl Default for RecordedStatistics {
    fn default() -> Self {
        Self {
            score_frequencies: vec![0; SCORE_VALUE_COUNT],
            search_keys: HashSet::new(),
            duplicate_positions: 0,
        }
    }
}

impl RecordedStatistics {
    /// 対局番号順に書き出す1レコードを集計する。
    pub(super) fn record(&mut self, completed: &CompletedRecord) {
        self.score_frequencies[score_index(completed.record.score())] += 1;
        if !self.search_keys.insert(completed.search_key) {
            self.duplicate_positions += 1;
        }
    }
}

/// i16の探索値を昇順の度数表添字へ変換する。
pub fn score_index(score: i16) -> usize {
    usize::try_from(i32::from(score) - i32::from(i16::MIN))
        .expect("an i16 score index must be non-negative")
}
