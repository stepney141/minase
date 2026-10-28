//! 1局の序盤、手ごとの探索、投了、および教師情報の記録。
//!
//! 対局番号はサイクル内の1からの通し番号であり、対局のシードは`derive_seed(基本シード, 対局番号)`とする。
//! 序盤の乱択には`derive_seed(対局のシード, 0)`を使い、初期局面から8〜16手を一様に選ぶ。
//! 序盤で終局した場合は、序盤のシードから再び`derive_seed(序盤のシード, 0)`を派生させて作り直す。
//! Gumbel乱数には`derive_seed(対局のシード, 1)`で初期化した1本の乱数列を全手番で使う。
//! `derive_seed(対局のシード, 2) % 100 < playthrough_percent`を満たす対局は投了させずに最後まで指させる。
//! シードと番号の扱いは`docs/adr/commit-zero-self-play-cycles-in-fixed-game-chunks.md`を参照。
//!
//! 各手番は新しい探索木からGumbel AlphaZeroのSequential Halvingを行い、最後に残った手を指す。
//! 温度による手の抽選とディリクレ雑音は使わない。
//! 根の評価後に指定したシミュレーション数を終えるか、根の勝敗が確定したら探索を終える。
//! 根の総訪問回数は根の辺の訪問回数の和であり、根自体の評価は数えない。
//! 探索中の引き分けの価値は、引き分け値の設定によらず0.5とする。
//! 木の容量16 MiBを使い切った場合はエラーにする。
//! 探索の構成は`docs/adr/batch-zero-self-play-leaves-across-many-games.md`を参照。
//!
//! 教師データ形式MNSDの価値は、根の手番側から見た期待得点で、勝ちを1、引き分けを0.5、負けを0とする。
//! 最後に残った辺が訪問済みなら訪問で得た期待得点の平均Qを、未訪問なら[`gumbel::mixed_value`]による混合価値を使う。
//! 有限な[0, 1]の値だけを受け付け、65,535倍して四捨五入した整数を記録する。
//! 方策は根の候補手`SearchSnapshot::root_moves`だけについて求める`softmax(logit + σq)`であり、
//! R1反復規則により根の候補から外れた合法手の確率は0になる。
//! 候補をMNSDの合法手の番号順に並べ、確率の合計が65,535になるよう最大剰余法で量子化し、0になった候補は記録しない。
//!
//! 対局記録は初期局面を開始局面、開始手数を0、由来を乱数開始の自己対局とする。
//! 手順の各項目は着手前の局面を表し、除外理由はすべて「なし」とする。
//! 序盤の項目は指した手だけを持ち、価値と方策はなし、訪問回数は0とする。
//! 探索した手の項目は指した手、上記の価値と方策、および根の総訪問回数を持つ。
//! 投了した局面も探索の価値、方策、および訪問回数を持つが、指した手は持たない。
//! 投了以外の終局では、指した手、価値、方策がなく、訪問回数が0の最終項目を追加する。
//!
//! 投了条件は、選んだ辺が訪問済みでQが1%未満、かつ根とその子孫に反復で打ち切った節点がないことである。
//! 反復の印は[`Node::has_approximate_result`]で調べ、反復の近似による勝敗の確定と引き分けの両方を含める。正確な裁定は含めない。
//! 選んだ辺が未訪問なら投了せず、最後まで指させる対局では条件の成立だけを記録する。
//! 最初に条件を満たした側が最終的に負けなかった場合を誤った投了とし、勝ち、引き分け、手数上限での打ち切りを含める。
//! 投了の扱いは`docs/adr/resign-zero-self-play-games-below-one-percent-after-bounding-false-resignations.md`を参照。
//!
//! 序盤を含めて1,000手に達しても未終局の対局は、学習用の記録をすべて破棄し、集計だけに残す。
//! `docs/adr/discard-zero-self-play-games-that-reach-1000-plies.md`を参照。

use minase::datagen::opening::generate_opening;
use minase::search::SearchSnapshot;
use minase::search::mcts::gumbel::{self, Gumbel};
use minase::search::mcts::tree::{Node, NodeState, SelectionRule};
use minase::search::mcts::{DrawValue, Driver, EvaluationRequest, Step};
use minase::training::records::{
    self, Entry, Exclusion, GameRecord, Origin, Outcome, Policy, Value,
};
use minase_core::rng::{XorShift64, derive_seed};
use minase_core::{Color, Game, GameResult, Move, Position, Rules};

use super::{Error, RESIGN_THRESHOLD, TREE_CAPACITY};

/// 全対局で共有する条件。手数上限は試験でも同じ経路から渡す。
#[derive(Clone, Copy)]
pub(super) struct Settings {
    pub(super) rules: Rules,
    pub(super) seed: u64,
    pub(super) simulations: u32,
    pub(super) playthrough_percent: u8,
    pub(super) max_ply: u32,
}

/// 終局または手数上限に到達した1局。破棄した局にも統計を残す。
pub(super) struct CompletedGame {
    pub(super) number: u32,
    pub(super) record: Option<GameRecord>,
    pub(super) resigned: bool,
    pub(super) playthrough: bool,
    pub(super) first_trigger: Option<Color>,
    pub(super) result: Option<GameResult>,
    pub(super) positions: u64,
    pub(super) plies: u32,
}

/// 次の推論待ちまで進められる対局と、その手番の探索。
pub(super) struct PlayingGame {
    number: u32,
    game: Game,
    settings: Settings,
    entries: Vec<Entry>,
    rng: XorShift64,
    playthrough: bool,
    first_trigger: Option<Color>,
    resigned: bool,
    positions: u64,
    search: Option<Search>,
}

impl PlayingGame {
    pub(super) fn new(number: u32, settings: Settings) -> Result<Self, Error> {
        let seed = derive_seed(settings.seed, u64::from(number));
        let (game, opening) = generate_opening(settings.rules, seed)?;
        let entries = opening
            .into_iter()
            .map(|mv| {
                Entry::new(
                    Some(mv),
                    Value::None,
                    Exclusion::None,
                    Policy::None,
                    Some(0),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            number,
            game,
            settings,
            entries,
            rng: XorShift64::new(derive_seed(seed.get(), 1)),
            playthrough: derive_seed(seed.get(), 2).get() % 100
                < u64::from(settings.playthrough_percent),
            first_trigger: None,
            resigned: false,
            positions: 0,
            search: None,
        })
    }

    /// 推論が必要になるか、終局・手数上限に到達するまで進める。
    /// `None`は対局の終了を表し、[`Self::finish`]で結果を取り出す。
    pub(super) fn advance(&mut self) -> Result<Option<EvaluationRequest>, Error> {
        while self.game.result().is_none() && self.game.ply_count() < self.settings.max_ply {
            if self.search.is_none() {
                self.search = Some(Search::new(
                    SearchSnapshot::from_game(&self.game)?,
                    &mut self.rng,
                    self.settings.simulations,
                )?);
            }
            let search = self.search.as_mut().ok_or(Error::MissingSearch)?;
            if search.finished() {
                let decision = search.decision(&self.game)?;
                self.search = None;
                let side = self.game.position().side_to_move();
                if decision.resign && self.first_trigger.is_none() {
                    self.first_trigger = Some(side);
                }
                self.resigned = decision.resign && !self.playthrough;
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
                self.positions += 1;
                if self.resigned {
                    self.game.resign(side)?;
                } else {
                    self.game.play(decision.mv)?;
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
        let result = self.game.result();
        let record = if let Some(result) = result {
            if !self.resigned {
                self.entries.push(Entry::new(
                    None,
                    Value::None,
                    Exclusion::None,
                    Policy::None,
                    Some(0),
                )?);
            }
            Some(GameRecord::new(
                self.number,
                Origin::RandomSelfplay,
                Outcome::Game(result),
                0,
                Position::initial(),
                self.entries,
            )?)
        } else {
            None
        };
        Ok(CompletedGame {
            number: self.number,
            record,
            resigned: self.resigned,
            playthrough: self.playthrough,
            first_trigger: self.first_trigger,
            result,
            positions: self.positions,
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
            gumbel: Gumbel::new(noise, simulations)?,
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

    fn decision(&self, game: &Game) -> Result<Decision, Error> {
        let root = self.driver.tree().root();
        let action = self.gumbel.action(root)?;
        let value = selected_value(root, action)?;
        let legal = records::sorted_legal_moves(game)?;
        let probabilities = gumbel::policy_target(root)?;
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
        Ok(Decision {
            mv: root.edges[action].mv,
            value: Value::Mcts((f64::from(value) * 65535.0).round() as u16),
            policy: Policy::Candidates(candidates),
            visits: root.edges.iter().map(|edge| edge.visits).sum(),
            resign: should_resign(root, action)?,
        })
    }
}

struct Decision {
    mv: Move,
    value: Value,
    policy: Policy,
    visits: u32,
    resign: bool,
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

/// 選んだ辺のQを返し、未訪問ならGumbelの混合価値を返す。
fn selected_value(root: &Node, action: usize) -> Result<f32, Error> {
    let edge = root.edges.get(action).ok_or(Error::InvalidAction)?;
    let value = match edge.q() {
        Some(q) => q,
        None => gumbel::mixed_value(root)?,
    };
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(Error::InvalidValue(value));
    }
    Ok(value)
}

/// 選んだ辺が訪問済みでQが1%未満、かつ根の部分木に反復の印がなければ投了する。
pub(super) fn should_resign(root: &Node, action: usize) -> Result<bool, Error> {
    let edge = root.edges.get(action).ok_or(Error::InvalidAction)?;
    Ok(edge.visits > 0
        && selected_value(root, action)? < RESIGN_THRESHOLD
        && !root.has_approximate_result())
}

/// 最初に投了条件を満たした側が、最後まで指しても負けなかったかを返す。
/// 未終局での破棄も負けとはみなさず、誤投了に数える。
pub(super) fn false_resignation(
    playthrough: bool,
    first_trigger: Option<Color>,
    result: Option<GameResult>,
) -> bool {
    playthrough && first_trigger.is_some_and(|side| {
        !matches!(result, Some(GameResult::Win { winner, .. }) if winner == side.opposite())
    })
}

#[cfg(test)]
mod tests;
