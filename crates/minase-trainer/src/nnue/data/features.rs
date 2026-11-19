use minase::eval::nnue::input::{
    AnchorError, FeatureSet, HalfKpVm, Indexer, P, for_each_active_row,
};
use minase_core::{Color, Position};

/// 両入力の仕様が定める1視点の発火数上限。
pub const FEATURE_COLUMNS: usize = FeatureSet::P.spec().max_active();
/// 空列。逆索引には含めない。
pub const SENTINEL: u32 = u32::MAX;

pub(crate) fn first_layer_rows(set: FeatureSet) -> usize {
    match set {
        FeatureSet::P => crate::nnue::layerstack::input::CANONICAL_ROWS,
        FeatureSet::HalfKpVm => set.spec().dimensions(),
    }
}

fn rows<I: Indexer>(position: &Position, side: Color) -> Result<Vec<usize>, AnchorError> {
    let context = I::context(position, side)?;
    let mut rows = Vec::with_capacity(I::FEATURE_SET.spec().max_active());
    for_each_active_row::<I>(context, side, position, |r| rows.push(r));
    Ok(rows)
}

pub(crate) fn active_rows(
    set: FeatureSet,
    position: &Position,
    side: Color,
) -> Result<Vec<usize>, AnchorError> {
    match set {
        FeatureSet::P => rows::<P>(position, side),
        FeatureSet::HalfKpVm => rows::<HalfKpVm>(position, side),
    }
}

pub(super) fn input_features(
    set: FeatureSet,
    position: &Position,
    side: Color,
) -> Result<[[u32; FEATURE_COLUMNS]; 2], AnchorError> {
    let mut result = [[SENTINEL; FEATURE_COLUMNS]; 2];
    if set == FeatureSet::P {
        for (v, view) in [side, side.opposite()].into_iter().enumerate() {
            P::context(position, view)?;
            for square in minase_core::Square::all() {
                if let Some(piece) = position.piece_at(square) {
                    let base = minase::eval::features::feature_index(view, piece, square);
                    result[v][square.dense_index()] =
                        crate::nnue::layerstack::input::encode_p_feature(
                            P::row((), base).expect("P includes every base") as u32,
                        );
                }
            }
            if let Some(trigger) = position.lion_taken_by_non_lion() {
                result[v][FEATURE_COLUMNS - 1] = crate::nnue::layerstack::input::encode_p_feature(
                    minase::eval::features::lion_feature_index(view, trigger.square) as u32,
                );
            }
        }
        return Ok(result);
    }
    for (v, side) in [side, side.opposite()].into_iter().enumerate() {
        for (dst, row) in result[v].iter_mut().zip(active_rows(set, position, side)?) {
            *dst = match set {
                FeatureSet::P => crate::nnue::layerstack::input::encode_p_feature(row as u32),
                FeatureSet::HalfKpVm => row as u32,
            };
        }
    }
    Ok(result)
}
