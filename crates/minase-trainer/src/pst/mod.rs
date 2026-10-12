//! 鏡映共有PSTのCPU学習、再開、MNPT版2への書き出し。

mod args;
pub mod export;
mod record;
mod train;

pub use args::{Args, RemovalReferenceKind};
pub use record::EpochRecord;
pub use train::run;
