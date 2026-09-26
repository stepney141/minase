//! 駒種・所有者・駒コードの表現。

mod code;
mod color;
mod kind;

#[cfg(test)]
mod tests;

pub use code::PieceCode;
pub use color::{COLOR_COUNT, Color};
pub use kind::{PIECE_KIND_COUNT, PieceKind};
