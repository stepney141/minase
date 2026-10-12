//! PythonのPST比較診断で定義された全検証集合の損失と標本誤差。

use super::{
    Result,
    pst_reference::{Features, Models, Weights},
    pst_sample::{BANDS, band},
};
use crate::{
    data::{Dataset, GameKey, estimate_k, outcome_code},
    nn::bce_with_logits,
};
use serde::Serialize;
use std::collections::HashMap;

pub(super) fn teacher_ks(dataset: &Dataset, scores: &[f64]) -> Result<Vec<f64>> {
    if dataset.training_indices().is_empty() {
        return Err("teacher calibration requires training records".into());
    }
    dataset
        .teacher_lambdas()
        .iter()
        .enumerate()
        .map(|(class, &mix)| {
            if mix == 0.0 {
                return Ok(f64::NAN);
            }
            let mut selected = Vec::new();
            let mut outcomes = Vec::new();
            for &index in dataset.training_indices() {
                if dataset.teacher_class(index)? as usize == class {
                    selected.push(scores[index as usize]);
                    outcomes.push(outcome_code(dataset.record(index)?.outcome()));
                }
            }
            Ok(estimate_k(&selected, &outcomes)?)
        })
        .collect()
}

#[derive(Serialize)]
pub(super) struct OutcomeMetrics {
    records: usize,
    games: usize,
    output_k: f64,
    bce_position_mean: Option<f64>,
    bce_game_mean: Option<f64>,
    sign_agreement: Option<f64>,
    sign_matches: usize,
    sign_records: usize,
    sign_excluded: usize,
    draw_records: usize,
    zero_score_records: usize,
}

#[derive(Default)]
struct Outcomes<'a> {
    games: HashMap<GameKey<'a>, usize>,
    totals: Vec<(f64, usize)>,
    loss_sum: f64,
    records: usize,
    draws: usize,
    zeros: usize,
    excluded: usize,
    eligible: usize,
    matches: usize,
}
impl<'a> Outcomes<'a> {
    fn add(&mut self, key: GameKey<'a>, score: i32, outcome: u8, k: f64) {
        let loss = bce_with_logits(f64::from(score) / k, f64::from(outcome) / 2.0);
        self.records += 1;
        self.loss_sum += loss;
        let game = *self.games.entry(key).or_insert_with(|| {
            self.totals.push((0.0, 0));
            self.totals.len() - 1
        });
        self.totals[game].0 += loss;
        self.totals[game].1 += 1;
        self.draws += usize::from(outcome == 1);
        self.zeros += usize::from(score == 0);
        if outcome == 1 || score == 0 {
            self.excluded += 1;
        } else {
            self.eligible += 1;
            self.matches += usize::from((score > 0) == (outcome == 2));
        }
    }
    fn finish(self, output_k: f64) -> OutcomeMetrics {
        OutcomeMetrics {
            records: self.records,
            games: self.games.len(),
            output_k,
            bce_position_mean: (self.records != 0).then(|| self.loss_sum / self.records as f64),
            bce_game_mean: (!self.totals.is_empty()).then(|| {
                self.totals
                    .iter()
                    .map(|(sum, count)| sum / *count as f64)
                    .sum::<f64>()
                    / self.totals.len() as f64
            }),
            sign_agreement: (self.eligible != 0)
                .then(|| self.matches as f64 / self.eligible as f64),
            sign_matches: self.matches,
            sign_records: self.eligible,
            sign_excluded: self.excluded,
            draw_records: self.draws,
            zero_score_records: self.zeros,
        }
    }
}

pub(super) struct Validation {
    pub losses: Models<Vec<[f64; BANDS]>>,
    pub outcomes: Models<OutcomeMetrics>,
}

pub(super) fn validation(
    dataset: &Dataset,
    models: &Models<Weights>,
    scores: &[f64],
    ks: &[f64],
) -> Result<Validation> {
    let mut losses = Models {
        base: vec![[0.0; BANDS]; ks.len()],
        candidate: vec![[0.0; BANDS]; ks.len()],
    };
    let mut outcomes = Models::<Outcomes<'_>>::default();
    for &index in dataset.validation_indices() {
        let record = dataset.record(index)?;
        let features = Features::new(&record);
        let class = dataset.teacher_class(index)? as usize;
        let b = band(features.q);
        let outcome = outcome_code(record.outcome());
        let mix = dataset.teacher_lambdas()[class];
        let result = f64::from(outcome) / 2.0;
        // build_targetsはf64で混合して一度f32へ丸め、その後損失計算でf64へ戻す。
        let target = f64::from(if mix == 0.0 {
            result
        } else {
            mix / (1.0 + (-scores[index as usize] / ks[class]).exp()) + (1.0 - mix) * result
        } as f32);
        let key = dataset.game_key(index)?;
        for (model, sums, outcomes) in [
            (&models.base, &mut losses.base, &mut outcomes.base),
            (
                &models.candidate,
                &mut losses.candidate,
                &mut outcomes.candidate,
            ),
        ] {
            let score = model.evaluate(&features);
            sums[class][b] += bce_with_logits(f64::from(score) / model.k(), target);
            outcomes.add(key, score, outcome, model.k());
        }
    }
    Ok(Validation {
        losses,
        outcomes: Models {
            base: outcomes.base.finish(models.base.k()),
            candidate: outcomes.candidate.finish(models.candidate.k()),
        },
    })
}

#[derive(Debug, Serialize)]
pub(super) struct ErrorSummary {
    mae_scaled_cp: f64,
    mae_raw_cp: f64,
    correlation: Option<f64>,
    correlation_reason: Option<&'static str>,
}

pub(super) fn error_summary(predicted: &[f64], scaled: &[f64], raw: &[f64]) -> ErrorSummary {
    assert!(
        !predicted.is_empty() && predicted.len() == scaled.len() && predicted.len() == raw.len()
    );
    let reason = if predicted.len() < 2 {
        Some("fewer than 2 samples")
    } else if scaled.iter().all(|&v| v == scaled[0]) {
        Some("constant teacher scores")
    } else if predicted.iter().all(|&v| v == predicted[0]) {
        Some("constant predicted scores")
    } else {
        None
    };
    let n = predicted.len() as f64;
    let correlation = reason.is_none().then(|| {
        let px = predicted.iter().sum::<f64>() / n;
        let py = scaled.iter().sum::<f64>() / n;
        let (mut xx, mut yy, mut xy) = (0.0, 0.0, 0.0);
        for (&x, &y) in predicted.iter().zip(scaled) {
            xx += (x - px).powi(2);
            yy += (y - py).powi(2);
            xy += (x - px) * (y - py);
        }
        (xy / xx.sqrt() / yy.sqrt()).clamp(-1.0, 1.0)
    });
    ErrorSummary {
        mae_scaled_cp: predicted
            .iter()
            .zip(scaled)
            .map(|(p, t)| (p - t).abs())
            .sum::<f64>()
            / n,
        mae_raw_cp: predicted
            .iter()
            .zip(raw)
            .map(|(p, t)| (p - t).abs())
            .sum::<f64>()
            / n,
        correlation,
        correlation_reason: reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-12, "{a} != {b}");
    }
    #[test]
    fn correlation_is_centered_and_undefined_reasons_have_python_priority() {
        // 中心化平方和は2と14、内積5。相関=5/sqrt(28)。
        let s = error_summary(&[1.0, 2.0, 3.0], &[1.0, 2.0, 6.0], &[0.0, 0.0, 0.0]);
        close(s.mae_scaled_cp, 1.0);
        close(s.mae_raw_cp, 2.0);
        close(s.correlation.unwrap(), 5.0 / 28.0_f64.sqrt());
        for (p, t, reason) in [
            (vec![1.0], vec![2.0], "fewer than 2 samples"),
            (vec![1.0, 1.0], vec![2.0, 2.0], "constant teacher scores"),
            (vec![1.0, 1.0], vec![2.0, 3.0], "constant predicted scores"),
        ] {
            let summary = error_summary(&p, &t, &t);
            assert_eq!(summary.correlation, None);
            assert_eq!(summary.correlation_reason, Some(reason));
        }
        close(
            error_summary(&[3.0, 2.0, 1.0], &[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0])
                .correlation
                .unwrap(),
            -1.0,
        );
    }
    #[test]
    fn game_means_weight_games_equally_and_sign_exclusions_are_a_union() {
        let mut outcomes = Outcomes::default();
        // Aは3局面すべてlog(2)、Bは1局面log(1+exp(1))。局面平均と対局平均は異なる。
        for _ in 0..3 {
            outcomes.add(GameKey::Human("A"), 0, 1, 100.0);
        }
        outcomes.add(GameKey::Human("B"), 100, 0, 100.0);
        let report = outcomes.finish(100.0);
        let a = 2.0_f64.ln();
        let b = 1.0_f64.exp().ln_1p();
        close(report.bce_position_mean.unwrap(), (3.0 * a + b) / 4.0);
        close(report.bce_game_mean.unwrap(), (a + b) / 2.0);
        assert_eq!(
            (
                report.games,
                report.records,
                report.draw_records,
                report.zero_score_records,
                report.sign_excluded,
                report.sign_records,
                report.sign_matches
            ),
            (2, 4, 3, 3, 3, 1, 0)
        );
        assert!(Outcomes::default().finish(1.0).bce_game_mean.is_none());
    }
}
