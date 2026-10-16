use std::{collections::BTreeMap, path::PathBuf};

use clap::Args;
use minase::eval::features::piece_state;
use minase_core::{Position, PositionBuilder, Square};
use serde::Serialize;

use super::{
    Result,
    network::IntegerNetwork,
    sample::{FileIdentity, SampleArgs, SampleIdentity, file_identity, sample_identity},
    statistics::{Reversals, reversal_statistics},
};
use crate::nnue::{
    data::{BatchSlot, FEATURE_COLUMNS},
    model::{Checkpoint, Model},
    reference::{self, RemovalReference, linear_row, row_value},
};

/// 検証集合の駒除去診断の引数。
#[derive(Args)]
pub struct RemovalArgs {
    /// 学習器が保存したbest.npzチェックポイント。
    #[arg(long = "float")]
    float_path: PathBuf,
    /// 比較するMNUE版5。
    #[arg(long)]
    net: PathBuf,
    #[command(flatten)]
    sample: SampleArgs,
    /// 新規作成するJSON報告。
    #[arg(long)]
    pub(super) output: PathBuf,
}

#[derive(Serialize)]
struct BatchCheck {
    max_absolute_cp: f64,
    limit_cp: f64,
    passed: bool,
}

#[derive(Serialize)]
struct IntegerCheck {
    mean_absolute_cp: f64,
    max_absolute_cp: f64,
}

/// `diag nnue-removal`が保存するJSON報告。
/// [`SampleIdentity`]を最上位へ平坦化し、`split = "validation"`、`population_count`に検証局面数を保存する。
/// `float`と`net`は各入力の[`FileIdentity`]、`removal_count`は測った除去の総数である。
/// `reference = "v0 initial_weights at both endpoints"`は、参照の両端点にv0の駒価値を使うことを示す。
///
/// `float_batch_vs_recomputed`は、一括計算と除去後の局面を作り直したf64再計算の除去差分を比較する。
/// その絶対差の最大`max_absolute_cp`、`limit_cp = 1e-6`、最大差が上限以下なら真の`passed`を持つ。
/// `float_vs_integer_recomputed`は、整数ネットと実数再計算の除去差分の絶対差について、平均`mean_absolute_cp`と最大`max_absolute_cp`を持つ。
/// `reversals`は全除去の整数差分dとv0参照差分rの[`Reversals`]である。
/// `by_piece_state`は除いた駒の[`piece_state`]ごとの同じ集計で、JSONのキーは番号の文字列である。
/// 一括計算と再計算の最大差が1e-6を超えた場合だけ、`error`に
/// `"float batched removal differs from recomputation by {最大差} cp (limit 1e-6)"`を持ち、それ以外は欄ごと省略する。
#[derive(Serialize)]
pub(super) struct Report {
    #[serde(flatten)]
    identity: SampleIdentity,
    split: &'static str,
    population_count: usize,
    float: FileIdentity,
    net: FileIdentity,
    removal_count: usize,
    reference: &'static str,
    float_batch_vs_recomputed: BatchCheck,
    float_vs_integer_recomputed: IntegerCheck,
    reversals: Reversals,
    by_piece_state: BTreeMap<usize, Reversals>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<String>,
}

// 除いた駒が先獅子の升にいても、升そのものは変えない。
fn remove_piece(position: &Position, removed: Square) -> Result<Position> {
    let mut builder = PositionBuilder::new(position.side_to_move());
    for square in Square::all() {
        if square != removed
            && let Some(piece) = position.piece_at(square)
        {
            builder.put(square, piece)?;
        }
    }
    let mut after = builder.finish()?;
    after.set_lion_capture(position.lion_capture_square())?;
    Ok(after)
}

/// 1局面の全駒の除去差分を、局面を作り直さずに累算器からの行の減算で一括して求める。
/// 渡されたf32の重みをf64へ広げて計算し、学習用のf32計算は使わない。
///
/// 線形の全特徴の和を端点ごとに`S_mg`、`S_eg`、盤上駒数をnとする。
/// 除去前は`q·S_mg + (1−q)·S_eg`で、`q = min(max(n−2, 0), 90)/90`である。
/// 駒の行rを除いた後は`q′·(S_mg−r_mg) + (1−q′)·(S_eg−r_eg)`で、
/// `q′ = min(max(n−3, 0), 90)/90`とし、除去後から除去前を引く。
///
/// 非線形では2つの視点vごとに`h_j = Σ 行の値 + b_j`を作る。
/// `cp_per_logit`には整数ネットの勝率尺度（1ロジットあたりのcp）を使う。
/// 活性化を`a(x) = clamp(x, 0, 1)²`、出力重みを`w_{v,j}`とし、各視点の
/// `cp_per_logit·(Σ_j a(h_j − 除いた駒の行_j)·w_{v,j} − Σ_j a(h_j)·w_{v,j})`を線形差分へ加える。
fn float_removal_deltas(
    model: &Model,
    weights: &[f32],
    batch: &BatchSlot,
    cp_per_logit: f64,
) -> Vec<f64> {
    let h = model.shape().width();
    let rows: Vec<_> = batch.linear_features()[..FEATURE_COLUMNS]
        .iter()
        .map(|&feature| linear_row(model, feature).map(f64::from))
        .collect();
    let sums: [f64; 2] = std::array::from_fn(|e| rows.iter().map(|row| row[e]).sum());
    let count = batch.piece_counts()[0];
    let q = f64::from(count.saturating_sub(2).min(90)) / 90.0;
    let after_q = f64::from(count.saturating_sub(3).min(90)) / 90.0;
    let before = q * sums[0] + (1.0 - q) * sums[1];
    let mut delta: Vec<_> = rows[..144]
        .iter()
        .map(|row| after_q * (sums[0] - row[0]) + (1.0 - after_q) * (sums[1] - row[1]) - before)
        .collect();
    let biases = model.expanded_biases();
    let output = model.expanded_output_weights();
    for v in 0..2 {
        let features = &batch.features()[v * FEATURE_COLUMNS..(v + 1) * FEATURE_COLUMNS];
        let hidden: Vec<_> = (0..h)
            .map(|j| {
                features
                    .iter()
                    .map(|&feature| f64::from(row_value(model, weights, feature, j)))
                    .sum::<f64>()
                    + f64::from(biases[j])
            })
            .collect();
        let activation = |value: f64| value.clamp(0.0, 1.0).powi(2);
        let before: f64 = (0..h)
            .map(|j| activation(hidden[j]) * f64::from(output[v * h + j]))
            .sum();
        for (col, value) in delta.iter_mut().enumerate() {
            let after: f64 = (0..h)
                .map(|j| {
                    activation(hidden[j] - f64::from(row_value(model, weights, features[col], j)))
                        * f64::from(output[v * h + j])
                })
                .sum();
            *value += cp_per_logit * (after - before);
        }
    }
    delta
}

/// 検証集合の標本について、王駒以外の各1枚の除去差分を比較する。
/// `--float`のチェックポイントと`--net`のMNUE版5は入力特徴の型と幅が一致しなければならない。
/// 各記録を[`Sample::to_position`](crate::data::Sample::to_position)で復元し、記録された手番側を視点とする。
/// 王将と玉将の駒状態11、および太子の駒状態21を除く盤上の全駒を1枚ずつ除く。
/// 除いた駒が先獅子の対象升にあっても、その対象升は残す。
/// 除ける駒がない局面は飛ばし、標本全体で除去が0件なら報告を作らずにエラーにする。
///
/// 実数の差分は[`reference::evaluate_f64_with_weights`]による除去後の評価から除去前の評価を引く。
/// 整数の差分も整数ネットの除去後から除去前を引き、実数計算にも整数ネットの勝率尺度を使う。
/// 参照差分は[`RemovalReference::v0`]から得る。
/// 除去差分の比較に非有限値が現れた場合は、報告を作らずにエラーにする。
pub(super) fn run(args: &RemovalArgs) -> Result<Report> {
    let dataset = args.sample.open()?;
    let indices = args.sample.select(&dataset, true)?;
    let net = IntegerNetwork::read(&args.net)?;
    let model = Checkpoint::read(&args.float_path)?.model;
    if model.shape().feature_set() != net.feature_set() || model.shape().width() != net.width() {
        return Err("float checkpoint and MNUE must have the same feature set and width".into());
    }
    let weights = reference::fold(&model)?;
    let v0 = RemovalReference::v0();
    let mut by_state: BTreeMap<usize, Vec<(i32, i32)>> = BTreeMap::new();
    let mut all_pairs = Vec::new();
    let mut maximum = 0.0_f64;
    let mut integer_max = 0.0_f64;
    let mut integer_sum = 0.0;
    for &index in &indices {
        let sample = dataset.record(index)?;
        let before = sample.to_position()?;
        let perspective = sample.side_to_move();
        let squares: Vec<_> = Square::all()
            .filter_map(|square| {
                before.piece_at(square).and_then(|piece| {
                    let state = piece_state(piece);
                    (!matches!(state, 11 | 21)).then_some((square, state))
                })
            })
            .collect();
        if squares.is_empty() {
            continue;
        }
        let mut positions = Vec::with_capacity(squares.len() + 1);
        positions.push(before);
        for &(square, _) in &squares {
            positions.push(remove_piece(&positions[0], square)?);
        }
        let batch = BatchSlot::from_evaluation_positions(
            model.shape().feature_set(),
            &positions,
            perspective,
        )?;
        let batched = float_removal_deltas(&model, &weights, &batch, net.cp_per_logit());
        let floating =
            reference::evaluate_f64_with_weights(&model, &batch, &weights, net.cp_per_logit())?;
        let integer_before = net.evaluate(&positions[0], perspective);
        let (references, _) = v0.deltas(&batch, 0)?;
        for (j, &(square, state)) in squares.iter().enumerate() {
            let after = &positions[j + 1];
            let float_delta = floating[j + 1] - floating[0];
            let integer_delta = net.evaluate(after, perspective) - integer_before;
            let batch_error = batched[square.dense_index()] - float_delta;
            let integer_error = f64::from(integer_delta) - float_delta;
            if !batch_error.is_finite() || !integer_error.is_finite() {
                return Err("non-finite removal difference".into());
            }
            maximum = maximum.max(batch_error.abs());
            integer_max = integer_max.max(integer_error.abs());
            integer_sum += integer_error.abs();
            let pair = (integer_delta, references[square.dense_index()]);
            by_state.entry(state).or_default().push(pair);
            all_pairs.push(pair);
        }
    }
    if all_pairs.is_empty() {
        return Err("sample contains no non-royal removals".into());
    }
    Ok(Report {
        identity: sample_identity(&dataset, &indices, args.sample.seed)?,
        split: "validation",
        population_count: dataset.validation_indices().len(),
        float: file_identity(&args.float_path)?,
        net: file_identity(&args.net)?,
        removal_count: all_pairs.len(),
        reference: "v0 initial_weights at both endpoints",
        float_batch_vs_recomputed: BatchCheck {
            max_absolute_cp: maximum,
            limit_cp: 1e-6,
            passed: maximum <= 1e-6,
        },
        float_vs_integer_recomputed: IntegerCheck {
            mean_absolute_cp: integer_sum / all_pairs.len() as f64,
            max_absolute_cp: integer_max,
        },
        reversals: reversal_statistics(all_pairs.into_iter()),
        by_piece_state: by_state
            .into_iter()
            .map(|(state, pairs)| (state, reversal_statistics(pairs.into_iter())))
            .collect(),
        error: (maximum > 1e-6).then(|| {
            format!("float batched removal differs from recomputation by {maximum} cp (limit 1e-6)")
        }),
    })
}
