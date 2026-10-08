//! 自己対局1局を、探索が評価を必要とするたびに止まる状態機械として進める。

use crate::selfplay_gen::opening::{Opening, starting_game};
use minase::datagen::invalid_data;
use minase::nn::{mnzt, policy};
use minase::search::SearchSnapshot;
use minase::search::mcts::{SearchedMove, Step, SteppedGumbelSearch};
use minase::training::records::Outcome;
use minase_core::rng::{XorShift64, derive_seed};
use minase_core::{Color, Game, GameResult, Move, Position, Rules};
use std::{
    io,
    num::{NonZeroU16, NonZeroU32, NonZeroU64},
};

/// 局のシードから投了の免除の抽選に使う系列の番号。開始局面の乱択は系列0を使う。
const PLAYTHROUGH_STREAM: u64 = 1;

/// 局のシードからGumbel乱数に使う系列の番号。
const GUMBEL_STREAM: u64 = 2;

/// 確率1に対応する方策の量子化の単位数（MNZTの確率の量子化と同じ）。
const PROBABILITY_UNITS: u32 = u16::MAX as u32;

/// すべての対局に共通する探索と終局の設定。
#[derive(Clone, Copy, Debug)]
pub(super) struct GameSettings {
    /// 採用する規則セット。
    pub(super) rules: Rules,
    /// Gumbel探索の1手の探索回数n。
    pub(super) simulations: NonZeroU32,
    /// 1局の手数の上限。開始局面の元の手数を含めて数える。
    pub(super) max_ply: NonZeroU16,
    /// 投了する根の勝率の閾値。
    pub(super) resign_threshold: f32,
    /// 投了せずに最後まで指させる対局の割合。
    pub(super) resign_playthrough: f64,
}

/// 対局の終わり方。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Ending {
    /// 規則または投了で終局した。
    Finished(GameResult),
    /// 手数の上限に達した。
    MaxPly,
}

/// 終えた1局の記録と、塊のサマリに集計する値。
#[derive(Debug)]
pub(super) struct FinishedGame {
    /// 塊の中での1から始まる対局番号。
    pub(super) number: u32,
    /// 学習データの記録。手数の上限に達した対局では空。
    pub(super) records: Vec<mnzt::Record>,
    /// 終わり方。
    pub(super) ending: Ending,
    /// 開始局面の元の手数を含めた終局時の手数。
    pub(super) plies: u32,
    /// 投了せずに最後まで指させた対局か。
    pub(super) playthrough: bool,
    /// 最後まで指させた対局で、投了の条件が最初に成立した側。
    pub(super) resign_trigger: Option<Color>,
}

/// 対局結果が付く前の、探索した1局面の記録。
struct Searched {
    /// 探索した局面。
    position: Position,
    /// 開始局面の元の手数を含めた手数。
    ply: u16,
    /// 手番側の根の勝率。
    value: f32,
    /// 方策の目標。
    policy: Vec<mnzt::PolicyEntry>,
}

/// 進行中の1局。
pub(super) struct SelfPlayGame {
    /// 塊の中での1から始まる対局番号。
    number: u32,
    /// 審判層の対局。
    game: Game,
    /// 実戦の開始局面の元の手数。乱択の開始局面では0。
    ply_offset: u32,
    /// Gumbel乱数の生成器。
    rng: XorShift64,
    /// 投了せずに最後まで指させる対局か。
    playthrough: bool,
    /// 最後まで指させる対局で、投了の条件が最初に成立した側。
    resign_trigger: Option<Color>,
    /// 探索した局面。
    searched: Vec<Searched>,
    /// 現在の手の探索。手を指した直後は`None`。
    search: Option<SteppedGumbelSearch>,
}

impl SelfPlayGame {
    /// 局のシードから開始局面と投了の免除を決めて対局を始める。
    pub(super) fn new(
        number: u32,
        game_seed: NonZeroU64,
        opening: Option<&Opening>,
        settings: &GameSettings,
    ) -> io::Result<Self> {
        let game = starting_game(settings.rules, game_seed, opening)?;
        let draw = derive_seed(game_seed.get(), PLAYTHROUGH_STREAM).get();
        Ok(Self {
            number,
            game,
            ply_offset: opening.map_or(0, |opening| opening.ply),
            rng: XorShift64::new(derive_seed(game_seed.get(), GUMBEL_STREAM)),
            playthrough: unit_interval(draw) < settings.resign_playthrough,
            resign_trigger: None,
            searched: Vec::new(),
            search: None,
        })
    }

    /// 次に評価が必要な葉を出すか、対局が終わるまで進める。
    ///
    /// 評価が必要なら真を返し、入力平面は[`SelfPlayGame::pending_input`]が返す。
    /// 終局または手数の上限で偽を返す。
    pub(super) fn advance(&mut self, settings: &GameSettings) -> io::Result<bool> {
        loop {
            let search = match self.search.as_mut() {
                Some(search) => search,
                None => {
                    if self.game.result().is_some()
                        || self.game.ply_count() + self.ply_offset
                            >= u32::from(settings.max_ply.get())
                    {
                        return Ok(false);
                    }
                    let snapshot = SearchSnapshot::from_game(&self.game)
                        .map_err(|error| invalid_data(format!("game {}: {error}", self.number)))?;
                    let search = SteppedGumbelSearch::new(&snapshot, settings.simulations)
                        .map_err(|error| invalid_data(format!("game {}: {error}", self.number)))?;
                    self.search.insert(search)
                }
            };
            match search
                .advance(&mut self.rng)
                .map_err(|error| invalid_data(format!("game {}: {error}", self.number)))?
            {
                Step::Evaluate => return Ok(true),
                Step::Finished(searched) => {
                    self.search = None;
                    self.conclude_search(searched, settings)?;
                }
            }
        }
    }

    /// 評価待ちの葉の入力平面を返す。
    pub(super) fn pending_input(&self) -> &[f32] {
        self.search
            .as_ref()
            .map_or(&[], SteppedGumbelSearch::pending_input)
    }

    /// 評価待ちの葉の方策logitと勝率を探索へ渡す。
    pub(super) fn supply(&mut self, policy: &[f32], value: f32) -> io::Result<()> {
        let search = self.search.as_mut().ok_or_else(|| {
            invalid_data(format!("game {} has no pending evaluation", self.number))
        })?;
        search
            .supply(policy, value)
            .map_err(|error| invalid_data(format!("game {}: {error}", self.number)))
    }

    /// 探索した局面を記録し、投了の条件を調べてから手を指す。
    fn conclude_search(
        &mut self,
        searched: SearchedMove,
        settings: &GameSettings,
    ) -> io::Result<()> {
        let side = self.game.position().side_to_move();
        let ply = u16::try_from(self.game.ply_count() + self.ply_offset)
            .map_err(|_| invalid_data(format!("game {} ply does not fit in u16", self.number)))?;
        self.searched.push(Searched {
            position: self.game.position().clone(),
            ply,
            value: searched.root_value,
            policy: policy_entries(&searched.policy_target, side)
                .map_err(|error| invalid_data(format!("game {}: {error}", self.number)))?,
        });
        let reject = |error| invalid_data(format!("game {} rejected a move: {error}", self.number));
        if searched.root_value < settings.resign_threshold {
            if !self.playthrough {
                self.game.resign(side).map_err(reject)?;
                return Ok(());
            }
            self.resign_trigger.get_or_insert(side);
        }
        self.game.play(searched.best_move).map_err(reject)?;
        Ok(())
    }

    /// 対局を終え、終局していれば対局結果を付けた記録を返す。
    pub(super) fn finish(self) -> io::Result<FinishedGame> {
        let (ending, records) = match self.game.result() {
            Some(result) => {
                let records = self
                    .searched
                    .into_iter()
                    .map(|searched| {
                        let outcome =
                            Outcome::from_game_result(result, searched.position.side_to_move());
                        mnzt::Record::from_position(
                            &searched.position,
                            outcome,
                            self.number,
                            searched.ply,
                            searched.value,
                            searched.policy,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| invalid_data(format!("game {}: {error}", self.number)))?;
                (Ending::Finished(result), records)
            }
            None => (Ending::MaxPly, Vec::new()),
        };
        Ok(FinishedGame {
            number: self.number,
            records,
            ending,
            plies: self.game.ply_count() + self.ply_offset,
            playthrough: self.playthrough,
            resign_trigger: self.resign_trigger,
        })
    }
}

/// 64ビットの乱数を[0, 1)の値へ写す。
fn unit_interval(value: u64) -> f64 {
    (value >> 11) as f64 / (1_u64 << 53) as f64
}

/// 方策の目標を、確率の和が量子化の後も1になるように、ラベルと確率の組へ変換する。
///
/// 確率を65535単位へ切り捨て、不足する単位を切り捨てた端数の大きい順に1単位ずつ配る
/// （最大剰余方式）。各手を独立に丸めると、合法手が70手程度で和の誤差が1e-4を超え得るからである。
fn policy_entries(
    target: &[(Move, f32)],
    side: Color,
) -> Result<Vec<mnzt::PolicyEntry>, Box<dyn std::error::Error + Send + Sync>> {
    let scaled: Vec<f64> = target
        .iter()
        .map(|&(_, probability)| f64::from(probability) * f64::from(PROBABILITY_UNITS))
        .collect();
    let total: f64 = scaled.iter().sum();
    if !total.is_finite() || total <= 0.0 {
        return Err("the policy target has no positive finite mass".into());
    }
    // 方策の目標はsoftmaxなので和は1に近いが、f32の丸めの分を正規化してから配る。
    let scaled: Vec<f64> = scaled
        .into_iter()
        .map(|value| value * f64::from(PROBABILITY_UNITS) / total)
        .collect();
    let mut units: Vec<u32> = scaled.iter().map(|value| value.floor() as u32).collect();
    let assigned: u32 = units.iter().sum();
    let mut order: Vec<usize> = (0..units.len()).collect();
    order.sort_by(|&left, &right| {
        let fraction = |index: usize| scaled[index] - scaled[index].floor();
        fraction(right)
            .total_cmp(&fraction(left))
            .then(left.cmp(&right))
    });
    for &index in order
        .iter()
        .take(PROBABILITY_UNITS.saturating_sub(assigned) as usize)
    {
        units[index] += 1;
    }
    target
        .iter()
        .zip(units)
        .map(|(&(mv, _), units)| {
            let label = policy::label(mv, side)?;
            Ok(mnzt::PolicyEntry::new(
                label,
                units as f32 / PROBABILITY_UNITS as f32,
            )?)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase_core::Rules;

    /// 量子化した確率の和はちょうど1になり、各手の誤差は1単位未満である。
    #[test]
    fn policy_entries_sum_to_exactly_one() {
        let game = Game::new(Rules::ENGINE_DEFAULT);
        let moves = game.legal_moves();
        let count = moves.len();
        let uniform: Vec<(Move, f32)> = moves.iter().map(|&mv| (mv, 1.0 / count as f32)).collect();
        let skewed: Vec<(Move, f32)> = moves
            .iter()
            .enumerate()
            .map(|(index, &mv)| (mv, (index + 1) as f32 * 2.0 / (count * (count + 1)) as f32))
            .collect();
        for target in [uniform, skewed] {
            let entries = policy_entries(&target, Color::Black).unwrap();
            let units: u32 = entries
                .iter()
                .map(|entry| (entry.probability() * PROBABILITY_UNITS as f32).round() as u32)
                .sum();
            assert_eq!(units, PROBABILITY_UNITS);
            for (entry, &(mv, probability)) in entries.iter().zip(&target) {
                assert_eq!(entry.label(), policy::label(mv, Color::Black).unwrap());
                assert!((entry.probability() - probability).abs() <= 1.0 / 65535.0);
            }
        }
    }

    /// 乱数を[0, 1)へ写す。
    #[test]
    fn unit_interval_stays_below_one() {
        assert_eq!(unit_interval(0), 0.0);
        assert!(unit_interval(u64::MAX) < 1.0);
    }
}
