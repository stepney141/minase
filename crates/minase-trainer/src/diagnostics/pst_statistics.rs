//! PST比較の全検証集合の損失と標本誤差を集計する。
//! 教師分類と補間係数の帯ごとの二値交差エントロピー、標本の平均絶対誤差と相関、対局ごとの平均損失を求める。
//! 各出力欄の定義は、この集計を報告へ載せる[`teacher_ks`]、[`validation`]、[`OutcomeMetrics`]、[`ErrorSummary`]と、`pst_compare.rs`の`Report`および`Band`のdocコメントにある。

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

/// 教師分類ごとの教師用Kを訓練集合だけで推定する。
/// 呼び手は[`Dataset::diagnostic_teacher_scores`]でf64として再計算した探索値を渡し、先読み指定時は先読み値を使う。
/// 終局結果だけを目標に[`estimate_k`]で推定し、実効λが0の分類はNaNを返すため、報告では`null`になる。
/// 訓練集合が空なら失敗し、λが0でない分類に訓練局面がなければ[`estimate_k`]が空標本を拒否する。
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

/// `report.json`の`outcome_metrics.base`と`.candidate`に保存する終局結果との比較。
/// 教師の混合比に依存せず、全検証局面の評価値をそのモデルの出力用Kで確率へ変換する。
/// `records`は検証局面数、`output_k`はモデルの出力用Kである。
/// `games`は[`Dataset::game_key`]で識別する対局数で、人間の棋譜は棋譜ID、乱数開始の自己対局は生成シードと対局番号で区別する。
/// 複数ファイルに現れる同じ棋譜IDは1対局と数える。
///
/// `bce_position_mean`は目標`result/2`への二値交差エントロピーの局面平均で、局面0件なら`null`である。
/// `bce_game_mean`は対局ごとの局面平均を対局間で等しい重みで平均した値で、対局0件なら`null`である。
/// `sign_records`は引き分けでも評価値0でもない局面数、`sign_matches`はその中で「評価値が正」と「勝ち」が一致した局面数である。
/// `sign_agreement = sign_matches / sign_records`で、分母0なら`null`である。
/// `sign_excluded`は引き分けまたは評価値0の局面数を和集合で数える。
/// `draw_records`と`zero_score_records`は理由別の件数で、両方に該当する局面はそれぞれに数える。
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
struct Outcomes {
    games: HashMap<GameKey, usize>,
    totals: Vec<(f64, usize)>,
    loss_sum: f64,
    records: usize,
    draws: usize,
    zeros: usize,
    excluded: usize,
    eligible: usize,
    matches: usize,
}
impl Outcomes {
    fn add(&mut self, key: GameKey, score: i32, outcome: u8, k: f64) {
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

/// 全検証局面の損失和を教師分類と位相帯ごとに集計し、モデルごとの[`OutcomeMetrics`]も求める。
/// 教師確率は実効λが0なら`result/2`、それ以外は`λ·σ(score/K_teacher) + (1−λ)·result/2`である。
/// `score`はf64の教師探索値、`result`は手番側の負け0、引き分け1、勝ち2、`σ(x) = 1/(1+exp(−x))`とする。
/// 混合はf64で計算して一度f32へ丸め、損失の計算時にf64へ戻す。
/// 混合後の丸めは学習と同じだが、この診断の探索値はf32へ丸める前の値である。
/// 各モデルのロジットは独立した整数PST参照の評価値をそのモデルの出力用Kで割った値で、教師確率との二値交差エントロピーを求める。
/// `Band.validation_loss`はこの和を分類と帯ごとの検証局面数で割った値である。
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
    let mut outcomes = Models::<Outcomes>::default();
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

/// `Band.base`と`Band.candidate`に保存する教師探索値との標本比較。
/// `mae_scaled_cp`はモデル評価と`raw × 候補の出力用K / 教師用K`に換算した教師探索値との平均絶対誤差である。
/// `mae_raw_cp`はモデル評価と生の教師探索値との平均絶対誤差で、いずれもセンチポーン単位である。
/// `correlation`はモデル評価と換算後の教師探索値のPearson相関係数である。
/// 中心化した積和を両者の平方和の平方根で割り、[−1, 1]へ切り詰める。
/// 定義できなければ`null`とし、`correlation_reason`へ理由を優先順に1つ記録する。
/// 標本2件未満は`"fewer than 2 samples"`、換算後の教師探索値がすべて等しい場合は
/// `"constant teacher scores"`、モデル評価がすべて等しい場合は`"constant predicted scores"`である。
/// 相関が定義できる場合の`correlation_reason`は`null`である。
#[derive(Debug, Serialize)]
pub(super) struct ErrorSummary {
    mae_scaled_cp: f64,
    mae_raw_cp: f64,
    correlation: Option<f64>,
    correlation_reason: Option<&'static str>,
}

/// 空でなく同じ長さのモデル評価、換算後の教師値、生の教師値から[`ErrorSummary`]を作る。
/// この長さの条件に違反すればパニックする。
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
            outcomes.add(GameKey(0, 1), 0, 1, 100.0);
        }
        outcomes.add(GameKey(0, 2), 100, 0, 100.0);
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
