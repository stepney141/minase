//! 同時摂動確率近似法（SPSA）の摂動、ペア得点、および係数更新。
//!
//! 利得は反復とともに減衰し、既定の指数は[`ALPHA`]と[`GAMMA`]である。
//! 安定化定数Aはセッション開始時の総反復数の0.1倍とする。
//! 係数ファイルの既定の終端利得は[`super::summary::parameter_line`]で生成する。
//! 理由は`docs/adr/decay-spsa-gains-over-30000-pair-sessions.md`にある。
//!
//! 反復の保存ファイルと各欄は書き手の[`super::storage`]に定める。

use super::params::Parameter;
use minase::harness::{CompletedPair, FailureCounts, GameRecord, OpeningRecord, TerminationRecord};
use minase_core::rng::{XorShift64, derive_seed, splitmix64};
use serde::{Deserialize, Serialize};
use std::io;

/// 学習率の減衰指数αの既定値。利得の式は[`rates`]に従う。
pub(super) const ALPHA: f64 = 0.602;
/// 摂動幅の減衰指数γの既定値。利得の式は[`rates`]に従う。
pub(super) const GAMMA: f64 = 0.101;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Settings {
    /// 係数ファイルの各行を表す調整対象の列。
    pub parameters: Vec<Parameter>,
    /// 学習率の減衰指数α。
    pub alpha: f64,
    /// 摂動幅の減衰指数γ。
    pub gamma: f64,
    /// 安定化定数A。セッション作成時に総反復数の0.1倍へ固定する。
    pub a: f64,
    /// 開始時に固定する総反復数N。利得の式に入るため再開で変更できない。
    pub iterations: u64,
    /// 1反復で対局する先後入替ペアの数。
    pub pairs_per_iteration: usize,
    /// 対局と摂動の乱数列を派生させる基本シード。
    pub seed: u64,
    /// 同時に対局させるペアの数。
    pub concurrency: usize,
}

impl Settings {
    pub fn initial_theta(&self) -> Vec<f64> {
        self.parameters.iter().map(|p| p.start).collect()
    }
}

/// 外部の記録からも更新則を計算できるよう、反復数と利得の前提を検査する。
///
/// 総反復数、1反復のペア数、および同時対局数は正で、総局数は`i64`に収まる必要がある。
/// 総反復数`u64::MAX`も拒否する。
/// 全係数の最初と最後の反復で、摂動幅、学習率、およびその比が有限の正値であることを検査する。
/// 比に1反復の最大得点差を掛けた値も有限でなければならない。
pub(super) fn validate_settings(settings: &Settings) -> io::Result<()> {
    let total = settings
        .iterations
        .checked_mul(settings.pairs_per_iteration as u64)
        .and_then(|n| n.checked_mul(2));
    if settings.iterations == 0
        || settings.iterations == u64::MAX
        || settings.pairs_per_iteration == 0
        || settings.concurrency == 0
        || total.is_none_or(|n| n > i64::MAX as u64)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "session size is zero or exceeds supported counters",
        ));
    }
    for p in &settings.parameters {
        for k in [1, settings.iterations] {
            let (c, a) = rates(settings, p, k);
            if !c.is_finite()
                || c <= 0.0
                || !a.is_finite()
                || a <= 0.0
                || !(a / c).is_finite()
                || a / c <= 0.0
                || !((a / c) * (2.0 * settings.pairs_per_iteration as f64)).is_finite()
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("non-finite or underflowing SPSA rates: {}", p.name),
                ));
            }
        }
    }
    Ok(())
}

/// 1起算の反復`k`の摂動幅`c_k`と学習率`a_k`を返す。
///
/// 総反復数をN、安定化定数をAとすると、`c_k = c_end * N^γ / k^γ`、
/// `a_k = r_end * c_end² * (A + N)^α / (A + k)^α`である。
/// fishtestの`spsa_workflow.py`と同じ式である。
pub(super) fn rates(settings: &Settings, p: &Parameter, k: u64) -> (f64, f64) {
    let n = settings.iterations as f64;
    let k = k as f64;
    let c = p.c_end * n.powf(settings.gamma) / k.powf(settings.gamma);
    let a = p.r_end * p.c_end.powi(2) * (settings.a + n).powf(settings.alpha)
        / (settings.a + k).powf(settings.alpha);
    (c, a)
}

/// [0, 1)の一様乱数を53ビットで作る。消費順は[`issue`]に従う。
pub(super) fn uniform(rng: &mut XorShift64) -> f64 {
    (rng.next() >> 11) as f64 / ((1_u64 << 53) as f64)
}

fn iteration_rng(seed: u64, k: u64) -> XorShift64 {
    // 対局の基本シード＋通し番号とは別の用途として混合する。
    XorShift64::new(derive_seed(splitmix64(seed ^ 0x5350_5341_5f54_554e), k))
}

/// [`issue`]と同じ基本シードと反復番号から、全係数の摂動の符号±1を再現する。
pub(super) fn flips(settings: &Settings, k: u64) -> Vec<i8> {
    let mut rng = iteration_rng(settings.seed, k);
    settings
        .parameters
        .iter()
        .map(|_| if rng.next() & 1 == 0 { -1 } else { 1 })
        .collect()
}

/// θ±c·flipを係数の範囲へ切り詰め、同じ一様乱数`u`を共有して整数化する。
/// 切り詰めた値に`u`を足して切り捨てた後、再び係数の整数範囲へ切り詰める。
pub(super) fn round_pair(p: &Parameter, theta: f64, c: f64, flip: i8, u: f64) -> (i32, i32) {
    let clip = |x: f64| x.clamp(f64::from(p.min), f64::from(p.max));
    let round = |x| ((clip(x) + u).floor() as i32).clamp(p.min, p.max);
    (
        round(theta + c * f64::from(flip)),
        round(theta - c * f64::from(flip)),
    )
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PairValues {
    pub number: u64,
    pub plus: Vec<i32>,
    pub minus: Vec<i32>,
}

pub(super) struct Issued {
    pub k: u64,
    pub flip: Vec<i8>,
    pub pending: std::collections::VecDeque<PairValues>,
    pub finished: Vec<PairObservation>,
}

/// 発行時点のθから反復`k`の全ペアのθ+とθ−を作る。
///
/// 乱数列は`derive_seed(splitmix64(seed ^ 0x5350_5341_5f54_554e), k)`で作り、対局用と分ける。
/// 先に全係数の符号±1を引き、続いてペアごと、係数ごとに[`uniform`]を1回引いて[`round_pair`]へ渡す。
/// ペアの通し番号は`(k - 1) * pairs_per_iteration + j`で、`j`は1起算である。
/// 対局のシードはこの番号から派生させ、θ+を候補側とする。
/// 符号と丸めの乱数は基本シードと`k`だけで決まるため、乱数の状態は保存しない。
/// 理由は`docs/adr/play-theta-plus-against-theta-minus-in-eight-pair-iterations.md`にある。
pub(super) fn issue(settings: &Settings, theta: &[f64], k: u64) -> Issued {
    let mut rng = iteration_rng(settings.seed, k);
    let flip: Vec<_> = settings
        .parameters
        .iter()
        .map(|_| if rng.next() & 1 == 0 { -1 } else { 1 })
        .collect();
    let mut pending = std::collections::VecDeque::new();
    for j in 1..=settings.pairs_per_iteration {
        let (plus, minus) = settings
            .parameters
            .iter()
            .zip(theta)
            .zip(&flip)
            .map(|((p, &x), &f)| round_pair(p, x, rates(settings, p, k).0, f, uniform(&mut rng)))
            .unzip();
        pending.push_back(PairValues {
            number: (k - 1) * settings.pairs_per_iteration as u64 + j as u64,
            plus,
            minus,
        });
    }
    Issued {
        k,
        flip,
        pending,
        finished: Vec::new(),
    }
}

/// 反則負けで終わった局の開始手順と全記録。
/// SPSA対局は採否の根拠とせず、監査は異常の原因を調べるために行う。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AbnormalGame {
    pub opening: OpeningRecord,
    pub game: GameRecord,
}

/// ペアの整数係数、分類、両局の終局理由、および異常の記録。
/// 正常な局の着手列は保存せず、反則負けの局だけ[`AbnormalGame`]へ全記録を残す。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PairObservation {
    pub values: PairValues,
    pub category: Option<u8>,
    /// 正常局も含む両局の終局理由。着手列の保存範囲は[`PairObservation`]に従う。
    pub terminations: [TerminationRecord; 2],
    pub failures: FailureCounts,
    pub abnormal_games: Vec<AbnormalGame>,
}

impl PairObservation {
    pub fn from_completed(values: PairValues, pair: CompletedPair) -> Self {
        let terminations = pair.record.games.each_ref().map(|g| g.termination.clone());
        let abnormal_games = pair
            .record
            .games
            .into_iter()
            .filter(|g| matches!(g.termination, TerminationRecord::Forfeit { .. }))
            .map(|game| AbnormalGame {
                opening: pair.record.opening.clone(),
                game,
            })
            .collect();
        Self {
            values,
            category: pair.record.category,
            terminations,
            failures: pair.result.failures,
            abnormal_games,
        }
    }
    /// 分類から2を引き、θ+側の勝ち数と負け数の差を返す。
    /// 手数上限に達して分類を持たないペアは0とする。
    pub fn difference(&self) -> i64 {
        self.category.map_or(0, |category| i64::from(category) - 2)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IterationRecord {
    pub k: u64,
    pub application: u64,
    pub flip: Vec<i8>,
    pub pairs: Vec<PairObservation>,
    pub d: i64,
    pub failures: FailureCounts,
    pub theta: Vec<f64>,
}

/// 完了時点のθへ`θ_i ← clip(θ_i + (a_k / c_k) * D * flip_i)`を適用する。
///
/// Dは反復内の全ペアの[`PairObservation::difference`]の和である。
/// `clip`は係数ごとの範囲への切り詰めであり、利得は[`rates`]から求める。
/// 発行時点のθには戻さない。
pub(super) fn update(settings: &Settings, theta: &[f64], k: u64, flip: &[i8], d: i64) -> Vec<f64> {
    settings
        .parameters
        .iter()
        .zip(theta)
        .zip(flip)
        .map(|((p, &x), &f)| {
            let (c, a) = rates(settings, p, k);
            (x + (a / c) * d as f64 * f64::from(f)).clamp(f64::from(p.min), f64::from(p.max))
        })
        .collect()
}

impl Issued {
    /// 完了ペアを番号順に並べ、得点差の和で[`update`]して反復記録を作る。
    /// `theta`には完了時点の値を渡す。保存が確定するまで次の発行には使わない。
    pub fn complete(
        mut self,
        settings: &Settings,
        theta: &[f64],
        application: u64,
    ) -> IterationRecord {
        self.finished.sort_by_key(|p| p.values.number);
        let d = self.finished.iter().map(PairObservation::difference).sum();
        let mut failures = FailureCounts::default();
        for pair in &self.finished {
            failures.add(pair.failures);
        }
        IterationRecord {
            k: self.k,
            application,
            theta: update(settings, theta, self.k, &self.flip, d),
            flip: self.flip,
            pairs: self.finished,
            d,
            failures,
        }
    }
}
