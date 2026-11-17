//! 自己対局の探索、序盤の抽選、投了、および教師情報の記録。
//!
//! 対局番号は1から数え、対局のシードは`derive_seed(基本シード, 対局番号)`で作る。
//! 派生番号0は配置の組み替え、1は全手番のGumbel乱数、2は訪問回数による抽選に使う。
//! `initial`は標準の初期局面、`shuffled-setup`は組み替えた配置から開始する。
//! 開始手数は0、由来は乱数開始の自己対局とする。
//! `docs/adr/start-zero-self-play-from-the-initial-position-and-sample-the-first-30-moves-by-root-visit-counts.md`を参照。
//!
//! 全手番で新しい木から指定回数のSequential Halvingを行い、勝敗が確定したら探索を終える。
//! `docs/adr/batch-zero-self-play-leaves-across-many-games-with-a-fresh-tree-per-move.md`を参照。
//! `docs/adr/search-every-zero-self-play-move-with-32-simulations-and-train-on-every-searched-position.md`を参照。
//! Gumbel乱数の倍率はすべて1とし、最初の30手は探索後に全辺の訪問回数に比例して抽選する。
//! `docs/adr/start-zero-self-play-from-the-initial-position-and-sample-the-first-30-moves-by-root-visit-counts.md`を参照。
//!
//! 記録する価値と投了に使う値は、抽選前の最終の選択の値とする。
//! 子の勝敗が確定していれば根の勝ちは1、負けは0、未確定ならQ、未訪問なら混合価値を使う。
//! 有限な[0, 1]の値を65,535倍して四捨五入し、MNSDの価値とする。
//! 方策は`softmax(logit + σ(q̂))`を最大剰余法で量子化し、合計を65,535にする。
//! 全探索局面に価値、方策、および根の総訪問回数を記録し、投了した局面だけ指した手を持たない。
//! 根の評価は訪問回数に数えず、探索中の引き分け値は0.5、木の容量は16 MiBとする。
//! `docs/adr/calibrate-the-zero-self-play-resignation-threshold-as-aobazero-does-and-record-the-gumbel-choice-value.md`を参照。
//!
//! 閾値は実行器がAobaZeroの較正の方式で決め、`--resign-threshold`で渡す。
//! 対局番号1、11、21…では投了せず、両者の全探索局面の値の最小値を較正用に残す。
//! ほかの対局では、着手前の手数が10以上で値が閾値より小さく、根と子孫に反復の近似の印がなければ投了する。
//! `docs/adr/calibrate-the-zero-self-play-resignation-threshold-as-aobazero-does-and-record-the-gumbel-choice-value.md`を参照。
//!
//! 着手後は規則上の終局、次の手番側の王手限定の3手以内の勝ち、手数上限の順に判定する。
//! 上限3,000手に達した未終局の対局は理由14の引き分けとして学習に残す。
//! 投了以外の終局では、手、価値、方策を持たず訪問回数0の最終項目を加える。
//! `docs/adr/draw-zero-self-play-games-at-3000-plies-and-keep-them-for-training.md`を参照。

use std::num::NonZeroUsize;

use minase::search::SearchSnapshot;
use minase::search::mate_in_three::{Verdict, mate_in_three};
use minase::search::mcts::gumbel::{self, Gumbel};
use minase::search::mcts::tree::{Node, NodeState, Outcome as TreeOutcome, SelectionRule};
use minase::search::mcts::{DrawValue, Driver, EvaluationRequest, Step};
use minase::training::records::{
    self, Entry, Exclusion, GameRecord, Origin, Outcome, Policy, Value,
};
use minase_core::rng::{XorShift64, derive_seed};
use minase_core::{Color, Game, Move, Position, Rules};
use serde::{Deserialize, Serialize};

use super::cli::StartPosition;
use super::{Error, GUMBEL_VALUE_SCALE, TREE_CAPACITY};

/// 全対局で共有する条件。手数上限は試験でも同じ経路から渡す。
#[derive(Clone, Copy)]
pub(super) struct Settings {
    pub(super) rules: Rules,
    pub(super) seed: u64,
    pub(super) start_position: StartPosition,
    pub(super) simulations: u32,
    pub(super) resign_threshold: f64,
    pub(super) max_ply: u32,
}

/// 終局または手数上限に到達した1局。
pub(super) struct CompletedGame {
    pub(super) number: u32,
    pub(super) record: GameRecord,
    pub(super) calibration: Option<ResignCalibrationGame>,
    pub(super) searches: u64,
    pub(super) plies: u32,
}

/// 投了禁止の対局から得た、実行器の較正に使う値。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct ResignCalibrationGame {
    pub(super) game: u32,
    pub(super) result: CalibrationResult,
    pub(super) min_value_black: f64,
    pub(super) min_value_white: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum CalibrationResult {
    Black,
    White,
    Draw,
}

/// 次の推論待ちまで進められる対局と、その手番の探索。
pub(super) struct PlayingGame {
    number: u32,
    game: Game,
    start_position: Position,
    settings: Settings,
    entries: Vec<Entry>,
    rng: XorShift64,
    action_rng: XorShift64,
    no_resign: bool,
    min_values: [Option<f32>; 2],
    resigned: bool,
    mate_winner: Option<Color>,
    searches: u64,
    search: Option<Search>,
}

impl PlayingGame {
    pub(super) fn new(number: u32, settings: Settings) -> Result<Self, Error> {
        let seed = derive_seed(settings.seed, u64::from(number));
        let start_position = match settings.start_position {
            StartPosition::Initial => Position::initial(),
            StartPosition::ShuffledSetup => {
                Position::shuffled_initial(&mut XorShift64::new(derive_seed(seed.get(), 0)))
            }
        };
        let game = Game::from_position(settings.rules, start_position.clone());
        let no_resign = number.checked_sub(1).ok_or(Error::UnexpectedGame(number))? % 10 == 0;
        Ok(Self {
            number,
            game,
            start_position,
            settings,
            entries: Vec::new(),
            rng: XorShift64::new(derive_seed(seed.get(), 1)),
            action_rng: XorShift64::new(derive_seed(seed.get(), 2)),
            no_resign,
            min_values: [None; 2],
            resigned: false,
            mate_winner: None,
            searches: 0,
            search: None,
        })
    }

    /// 推論が必要になるか、終局・手数上限に到達するまで進める。
    /// `None`は対局の終了を表し、[`Self::finish`]で結果を取り出す。
    pub(super) fn advance(&mut self) -> Result<Option<EvaluationRequest>, Error> {
        while self.game.result().is_none()
            && self.mate_winner.is_none()
            && self.game.ply_count() < self.settings.max_ply
        {
            if self.search.is_none() {
                self.search = Some(Search::new(
                    SearchSnapshot::from_game(&self.game)?,
                    &mut self.rng,
                    self.settings.simulations,
                )?);
            }
            let search = self.search.as_mut().ok_or(Error::MissingSearch)?;
            if search.finished() {
                let decision = search.decision(
                    &self.game,
                    &mut self.action_rng,
                    self.settings.resign_threshold,
                )?;
                self.searches += 1;
                self.search = None;
                let side = self.game.position().side_to_move();
                if self.no_resign {
                    let minimum = &mut self.min_values[side as usize];
                    *minimum = Some(match *minimum {
                        Some(previous) => previous.min(decision.raw_value),
                        None => decision.raw_value,
                    });
                }
                self.resigned = decision.resign && !self.no_resign;
                self.entries.push(Entry::new(
                    if self.resigned {
                        None
                    } else {
                        Some(decision.mv)
                    },
                    decision.value,
                    Exclusion::None,
                    decision.policy,
                    Some(decision.visits),
                )?);
                if self.resigned {
                    self.game.resign(side)?;
                } else {
                    self.game.play(decision.mv)?;
                    if self.game.result().is_none() && mate_in_three(&self.game)? == Verdict::Win {
                        self.mate_winner = Some(self.game.position().side_to_move());
                    }
                }
                continue;
            }
            match search
                .driver
                .select_leaf(SelectionRule::Gumbel(&search.gumbel))?
            {
                Step::Evaluate(request) => return Ok(Some(request)),
                Step::BackedUp => {}
                Step::Collision => return Err(Error::Collision),
                Step::Capacity => return Err(Error::Capacity),
            }
        }
        Ok(None)
    }

    pub(super) fn evaluate(
        &mut self,
        request: &EvaluationRequest,
        logits: &[f32],
        value: f32,
    ) -> Result<(), Error> {
        self.search
            .as_mut()
            .ok_or(Error::MissingSearch)?
            .driver
            .evaluate(request, logits, value)?;
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<CompletedGame, Error> {
        let result = match self.mate_winner {
            Some(winner) => Outcome::MateInThree { winner },
            None => match self.game.result() {
                Some(result) => Outcome::Game(result),
                None if self.game.ply_count() >= self.settings.max_ply => Outcome::MaxPlyDraw,
                None => return Err(Error::UnfinishedGame(self.number)),
            },
        };
        let calibration = if self.no_resign {
            let minimum = |side: Color| {
                self.min_values[side as usize].map(f64::from).ok_or(
                    Error::MissingCalibrationValue {
                        game: self.number,
                        side,
                    },
                )
            };
            Some(ResignCalibrationGame {
                game: self.number,
                result: match result {
                    Outcome::Game(minase_core::GameResult::Win { winner, .. })
                    | Outcome::MateInThree { winner } => match winner {
                        Color::Black => CalibrationResult::Black,
                        Color::White => CalibrationResult::White,
                    },
                    _ => CalibrationResult::Draw,
                },
                min_value_black: minimum(Color::Black)?,
                min_value_white: minimum(Color::White)?,
            })
        } else {
            None
        };
        if !self.resigned {
            self.entries.push(Entry::new(
                None,
                Value::None,
                Exclusion::None,
                Policy::None,
                Some(0),
            )?);
        }
        let record = GameRecord::new(
            self.number,
            Origin::RandomSelfplay,
            result,
            0,
            self.start_position,
            self.entries,
        )?;
        Ok(CompletedGame {
            number: self.number,
            record,
            calibration,
            searches: self.searches,
            plies: self.game.ply_count(),
        })
    }
}

/// 根の評価後の訪問回数で予算を数える、1手だけの探索。
struct Search {
    driver: Driver,
    gumbel: Gumbel,
    simulations: u32,
}

impl Search {
    fn new(
        snapshot: SearchSnapshot,
        rng: &mut XorShift64,
        simulations: u32,
    ) -> Result<Self, Error> {
        let noise = (0..snapshot.root_moves().len())
            .map(|_| gumbel_noise(rng.next()))
            .collect();
        Ok(Self {
            driver: Driver::new(snapshot, TREE_CAPACITY, DrawValue::SelfPlay)?,
            gumbel: Gumbel::new(noise, simulations, GUMBEL_VALUE_SCALE)?,
            simulations,
        })
    }

    fn finished(&self) -> bool {
        let root = self.driver.tree().root();
        matches!(root.state, NodeState::Proven(_))
            || root
                .edges
                .iter()
                .map(|edge| u64::from(edge.visits))
                .sum::<u64>()
                >= u64::from(self.simulations)
    }

    fn decision(
        &self,
        game: &Game,
        rng: &mut XorShift64,
        threshold: f64,
    ) -> Result<Decision, Error> {
        #[cfg(feature = "invariants")]
        self.driver.assert_finished();
        let root = self.driver.tree().root();
        let action = self.gumbel.action(root, 1.0)?;
        let value = selected_value(root, action)?;
        let played = explorative_action(root, action, game.ply_count(), rng);
        Ok(Decision {
            mv: root.edges[played].mv,
            raw_value: value,
            value: Value::Mcts((f64::from(value) * 65535.0).round() as u16),
            policy: self.policy(game)?,
            visits: root.edges.iter().map(|edge| edge.visits).sum(),
            resign: should_resign(root, value, game.ply_count(), threshold),
        })
    }

    fn policy(&self, game: &Game) -> Result<Policy, Error> {
        let root = self.driver.tree().root();
        let legal = records::sorted_legal_moves(game)?;
        let probabilities = self.gumbel.policy_target(root)?;
        let weights = root
            .edges
            .iter()
            .zip(probabilities)
            .map(|(edge, probability)| {
                let index = legal
                    .iter()
                    .position(|&mv| mv == edge.mv)
                    .ok_or(records::Error::IllegalMove(edge.mv))?;
                // sorted_legal_movesは合法手数がu16::MAX未満であることを保証する。
                Ok((index as u16, f64::from(probability)))
            })
            .collect::<Result<Vec<_>, records::Error>>()?;
        let candidates = records::quantize_policy(&weights)?
            .into_iter()
            .map(|(index, probability)| (legal[usize::from(index)], probability))
            .collect();
        Ok(Policy::Candidates(candidates))
    }
}

struct Decision {
    raw_value: f32,
    mv: Move,
    value: Value,
    policy: Policy,
    visits: u32,
    resign: bool,
}

/// 最初の30手は、逐次半減で外れた辺も含む訪問回数に比例して抽選する。
/// 訪問がない場合は、根の評価だけで確定した最終の選択を使う。
fn explorative_action(root: &Node, selected: usize, ply: u32, rng: &mut XorShift64) -> usize {
    if ply >= 30 {
        return selected;
    }
    let total = root.edges.iter().map(|edge| edge.visits as usize).sum();
    let Some(total) = NonZeroUsize::new(total) else {
        return selected;
    };
    let mut draw = rng.index(total);
    for (index, edge) in root.edges.iter().enumerate() {
        if draw < edge.visits as usize {
            return index;
        }
        draw -= edge.visits as usize;
    }
    unreachable!("the draw lies within the total visits")
}

/// 53ビットの一様乱数を、区間の中点からGumbel乱数へ写す。
fn gumbel_noise(random: u64) -> f32 {
    let unit = 1.0 / (1_u64 << 53) as f64;
    let u = ((random >> 11) as f64 + 0.5) * unit;
    // 最上端の中点1−2^-54はf64で1へ丸められるため、同じ対数をln_1pで求める。
    let log_u = if u == 1.0 {
        (-0.5 * unit).ln_1p()
    } else {
        u.ln()
    };
    -(-log_u).ln() as f32
}

/// 子の確定した勝敗を根の視点へ反転し、未確定ならQまたは未訪問の混合価値を返す。
fn selected_value(root: &Node, action: usize) -> Result<f32, Error> {
    let edge = root.edges.get(action).ok_or(Error::InvalidAction)?;
    let value = match edge.child.as_deref().map(|child| child.state) {
        Some(NodeState::Proven(proof)) => match proof.outcome {
            TreeOutcome::Loss => 1.0,
            TreeOutcome::Win => 0.0,
        },
        _ => match edge.q() {
            Some(q) => q,
            None => gumbel::mixed_value(root)?,
        },
    };
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(Error::InvalidValue(value));
    }
    Ok(value)
}

/// 10手目以降、閾値を厳密に下回り、反復の近似に依存しなければ投了する。
fn should_resign(root: &Node, value: f32, ply: u32, threshold: f64) -> bool {
    ply >= 10 && f64::from(value) < threshold && !root.has_approximate_result()
}

#[cfg(test)]
mod tests;
