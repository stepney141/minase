//! 鏡映共有PSTのCPU学習、再開、MNPT版2への書き出し。

mod args;
mod base;
pub mod batch;
mod checkpoint;
mod compute;
mod error;
pub mod export;
mod linear;
mod model;
mod npz;
mod record;
mod removal;
mod replay;
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
