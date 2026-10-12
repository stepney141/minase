//! NNUEの入力特徴、観測マスク、固定幅バッチと逆索引。

mod batch;
mod error;
mod features;
mod observations;

pub(crate) use batch::max_segment_count;
pub use batch::{
    BatchSlot, COLUMN_BITS, COLUMN_SHIFT, MAX_BATCH_SIZE, PERSPECTIVE_BITS, PERSPECTIVE_SHIFT,
    POSITION_BITS, POSITION_SHIFT, PackedOccurrence, REVERSE_INDEX_SEGMENT_LEN, ROTATION_BITS,
    ROTATION_SHIFT, ReverseIndex, build_batches, build_targets,
};
pub use error::Error;
pub use features::{CANONICAL_ROWS, FEATURE_COLUMNS, InputFeatureSet, SENTINEL, decode_p_feature};
pub use observations::Observations;

/// NNUEの特徴、観測回数またはバッチの構築結果。
pub type Result<T> = std::result::Result<T, Error>;
