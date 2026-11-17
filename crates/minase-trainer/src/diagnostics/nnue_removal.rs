use std::{collections::BTreeMap, path::PathBuf};

use clap::Args;
use minase::eval::features::piece_state;
use minase_core::Square;
use serde::Serialize;

use super::{
    Result,
    network::{FloatCheckpoint, IntegerNetwork},
    sample::{FileIdentity, SampleArgs, SampleIdentity, file_identity, sample_identity},
    statistics::{Reversals, reversal_statistics},
};
use minase::eval::{
    nnue::input::{AnchorError, FeatureSet, HalfKpVm, Indexer},
    piece_values::piece_value,
};
use minase_core::{Color, Position, PositionBuilder};

#[derive(Debug)]
enum Error {
    WidthMismatch { checkpoint: usize, network: usize },
    ScaleMismatch { checkpoint: i32, network: i32 },
    NonFiniteDifference,
    NoRemovals,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WidthMismatch {
                checkpoint,
                network,
            } => write!(
                f,
                "NNUE width mismatch: checkpoint={checkpoint}, network={network}"
            ),
            Self::ScaleMismatch {
                checkpoint,
                network,
            } => write!(
                f,
                "NNUE fv_scale mismatch: checkpoint={checkpoint}, network={network}"
            ),
            Self::NonFiniteDifference => f.write_str("non-finite removal difference"),
            Self::NoRemovals => f.write_str("sample contains no non-royal removals"),
        }
    }
}
impl std::error::Error for Error {}

/// 駒だけを除き、空升になった場合も先獅子の対象升を保持する。
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

// v0は両端点が同一なので、位相補間は駒価値の符号付き和に等しい。
fn v0_sum(position: &Position, side: Color) -> i32 {
    Square::all()
        .filter_map(|s| position.piece_at(s))
        .map(|p| {
            if p.color() == Some(side) {
                piece_value(p)
            } else {
                -piece_value(p)
            }
        })
        .sum()
}

/// 検証集合の駒除去診断の引数。
#[derive(Args)]
pub struct RemovalArgs {
    /// LayerStackまたはSimpleのNPZチェックポイント。
    #[arg(long = "float")]
    float_path: PathBuf,
    /// 比較するtatara形式のLayerStackまたはSimple。
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
    terminal_removal_count: usize,
    reference: &'static str,
    float_batch_vs_recomputed: BatchCheck,
    float_vs_integer_recomputed: IntegerCheck,
    reversals: Reversals,
    by_piece_state: BTreeMap<usize, Reversals>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<String>,
}

/// Simpleでは王将は入力に含まれないため除去しない。太子は除去対象に含む。
/// 王将のない側の最後の太子を除くと静的評価が定義できないため、終局の除去として
/// terminal_removal_countに数え、差分統計から除く。LayerStackでは王駒を除去しない。
/// 対象駒を1枚ずつ除き、K_o×(Y_after−Y_before)を整数のevaluate_for差分と比較する。
/// 両視点の累算値とPSQTから行を引く方法と、除去後局面からのf64再計算を照合する。
/// K_o=8128/mはチェックポイントから取り、ネットの幅とfv_scale=mの一致を要求する。
/// v0参照はエンジンの固定駒価値を合計し、両局面を±28,999 cpへ制限してから引く。
pub(super) fn run(args: &RemovalArgs) -> Result<Report> {
    let dataset = args.sample.open()?;
    let indices = args.sample.select(&dataset, true)?;
    let net = IntegerNetwork::read(&args.net)?;
    let checkpoint = FloatCheckpoint::read(&args.float_path, net.feature_set())?;
    if net.width() != checkpoint.width() {
        return Err(Error::WidthMismatch {
            checkpoint: checkpoint.width(),
            network: net.width(),
        }
        .into());
    }
    if checkpoint.divisor() != net.fv_scale() {
        return Err(Error::ScaleMismatch {
            checkpoint: checkpoint.divisor(),
            network: net.fv_scale(),
        }
        .into());
    }
    let mut terminal_removal_count = 0;
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
                    (state != 11 && (net.feature_set() == FeatureSet::HalfKpVm || state != 21))
                        .then_some((square, state, piece))
                })
            })
            .collect();
        if squares.is_empty() {
            continue;
        }
        let accumulator = checkpoint.accumulator(&before, perspective)?;
        let floating_before = checkpoint.output(&accumulator)?;
        let integer_before = net.evaluate(&before, perspective)?;
        let reference_before = v0_sum(&before, perspective);
        for &(square, state, piece) in &squares {
            let after = remove_piece(&before, square)?;
            if net.feature_set() == FeatureSet::HalfKpVm {
                let mut terminal = false;
                for side in Color::ALL {
                    match HalfKpVm::context(&after, side) {
                        Ok(_) => {}
                        Err(AnchorError::MissingRoyal { .. }) => terminal = true,
                        Err(error) => return Err(error.into()),
                    }
                }
                if terminal {
                    terminal_removal_count += 1;
                    continue;
                }
            }
            let batched = checkpoint.output_scale()
                * (checkpoint.without(&accumulator, &before, perspective, piece, square)?
                    - floating_before);
            let floating_after =
                checkpoint.output(&checkpoint.accumulator(&after, perspective)?)?;
            let float_delta = checkpoint.output_scale() * (floating_after - floating_before);
            let integer_delta = net.evaluate(&after, perspective)? - integer_before;
            let batch_error = batched - float_delta;
            let integer_error = f64::from(integer_delta) - float_delta;
            if !batch_error.is_finite() || !integer_error.is_finite() {
                return Err(Error::NonFiniteDifference.into());
            }
            maximum = maximum.max(batch_error.abs());
            integer_max = integer_max.max(integer_error.abs());
            integer_sum += integer_error.abs();
            let reference_after = v0_sum(&after, perspective);
            let pair = (
                integer_delta,
                reference_after.clamp(-28_999, 28_999) - reference_before.clamp(-28_999, 28_999),
            );
            by_state.entry(state).or_default().push(pair);
            all_pairs.push(pair);
        }
    }
    if all_pairs.is_empty() {
        return Err(Error::NoRemovals.into());
    }
    Ok(Report {
        identity: sample_identity(&dataset, &indices, args.sample.seed)?,
        split: "validation",
        population_count: dataset.validation_indices().len(),
        float: file_identity(&args.float_path)?,
        net: file_identity(&args.net)?,
        removal_count: all_pairs.len(),
        terminal_removal_count,
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
