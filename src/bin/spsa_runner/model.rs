//! 反復ごとの摂動、候補側の得点集計、および更新。

use super::params::Parameter;
use minase::harness::{CompletedPair, FailureCounts, GameRecord, OpeningRecord, TerminationRecord};
use minase::rng::{XorShift64, derive_seed, splitmix64};
use serde::{Deserialize, Serialize};

pub(super) const ALPHA: f64 = 0.602;
pub(super) const GAMMA: f64 = 0.101;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Settings {
    pub parameters: Vec<Parameter>,
    pub alpha: f64,
    pub gamma: f64,
    pub a: f64,
    pub iterations: u64,
    pub pairs_per_iteration: usize,
    pub seed: u64,
    pub concurrency: usize,
}

impl Settings {
    pub fn initial_theta(&self) -> Vec<f64> {
        self.parameters.iter().map(|p| p.start).collect()
    }
}

pub(super) fn rates(settings: &Settings, p: &Parameter, k: u64) -> (f64, f64) {
    let n = settings.iterations as f64;
    let k = k as f64;
    let c = p.c_end * n.powf(settings.gamma) / k.powf(settings.gamma);
    let a = p.r_end * p.c_end.powi(2) * (settings.a + n).powf(settings.alpha)
        / (settings.a + k).powf(settings.alpha);
    (c, a)
}

pub(super) fn uniform(rng: &mut XorShift64) -> f64 {
    (rng.next() >> 11) as f64 / ((1_u64 << 53) as f64)
}

fn iteration_rng(seed: u64, k: u64) -> XorShift64 {
    // 対局の基本シード＋通し番号とは別の用途として混合する。
    XorShift64::new(derive_seed(splitmix64(seed ^ 0x5350_5341_5f54_554e), k))
}

pub(super) fn flips(settings: &Settings, k: u64) -> Vec<i8> {
    let mut rng = iteration_rng(settings.seed, k);
    settings
        .parameters
        .iter()
        .map(|_| if rng.next() & 1 == 0 { -1 } else { 1 })
        .collect()
}

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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AbnormalGame {
    pub opening: OpeningRecord,
    pub game: GameRecord,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PairObservation {
    pub values: PairValues,
    pub category: Option<u8>,
    // 設計書の「各局の結果」を残す。正常局の着手列は持たない。
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
