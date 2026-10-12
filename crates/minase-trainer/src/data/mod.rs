//! MNSDの前処理と、CPUワーカーによる固定幅バッチの構築。
//!
//! 教師探索値は手番側視点のセンチポーンで保持する。教師用Kで勝率へ変換し、
//! 来歴の分類ごとの混合比λで手番側の終局結果と混ぜて教師値を作る。

mod batch;
mod error;
mod features;
mod mnsd;
mod observations;
mod shuffle;
mod teacher;

pub(crate) use batch::max_segment_count;
pub use batch::{
    BatchSlot, COLUMN_BITS, COLUMN_SHIFT, MAX_BATCH_SIZE, PERSPECTIVE_BITS, PERSPECTIVE_SHIFT,
    POSITION_BITS, POSITION_SHIFT, PackedOccurrence, REVERSE_INDEX_SEGMENT_LEN, ROTATION_BITS,
    ROTATION_SHIFT, ReverseIndex, build_batches,
};
pub use error::Error;
pub(crate) use features::canonical_row;
pub use features::{CANONICAL_ROWS, FEATURE_COLUMNS, InputFeatureSet, SENTINEL, decode_p_feature};
pub use mnsd::{Dataset, RecordLocation, TeacherClass};
pub use observations::Observations;
pub use shuffle::{epoch_permutation, hash64, is_validation};
pub use teacher::{
    Lookahead, build_targets, build_targets_from_arrays, compute_lookahead, estimate_k,
    validate_lambda_override,
};

/// データの読み込み、検証またはバッチ構築の結果。
pub type Result<T> = std::result::Result<T, Error>;
