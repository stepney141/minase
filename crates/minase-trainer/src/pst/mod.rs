//! 鏡映共有PSTの学習と再開、およびFMの次元0のMNPT版3への書き出し。
//! CPUでの精度、加算順、並列度の契約は[`crate`]のdocコメントに従う。

mod args;
mod base;
pub mod batch;
mod checkpoint;
mod compute;
mod error;
pub mod export;
pub(crate) mod linear;
pub(crate) mod model;
mod npz;
mod record;
mod removal;
mod train;

pub use args::{Args, RemovalReferenceKind};
pub use batch::Batch;
pub use checkpoint::{Checkpoint, Progress};
pub use compute::{Gradients, Losses, evaluate, gradients, validation_sum};
pub use error::{Error, Result};
pub use model::{Adam, Model};
pub use npz::{encode_pst_npz, load_initial};
pub use record::{EpochRecord, StopReason, Summary};
pub use removal::{RemovalReference, v0_piece_values};
pub use train::run;
