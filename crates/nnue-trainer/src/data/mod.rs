//! MNSDの前処理と、CPUワーカーによる固定幅バッチの構築。
//!
//! 教師探索値は手番側視点のセンチポーンで保持する。教師用Kでの勝率への
//! 変換は利用側の責務であり、終局結果との混合比はλ=1である。

mod batch;
mod error;
mod features;
mod mnsd;
mod observations;
mod shuffle;
mod teacher;

pub use batch::{
    BatchSlot, COLUMN_BITS, COLUMN_SHIFT, MAX_BATCH_SIZE, PERSPECTIVE_BITS, PERSPECTIVE_SHIFT,
    POSITION_BITS, POSITION_SHIFT, PackedOccurrence, ROTATION_BITS, ROTATION_SHIFT, ReverseIndex,
    build_batches,
};
pub use error::Error;
pub use features::{CANONICAL_ROWS, FEATURE_COLUMNS, InputFeatureSet, SENTINEL, decode_p_feature};
pub use mnsd::{Dataset, RecordLocation, TeacherClass};
pub use observations::Observations;
pub use shuffle::{epoch_permutation, hash64, is_validation};
pub use teacher::{Lookahead, compute_lookahead, estimate_k};

/// データの読み込み、検証またはバッチ構築の結果。
pub type Result<T> = std::result::Result<T, Error>;
