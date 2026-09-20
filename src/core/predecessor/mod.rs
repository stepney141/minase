//! 局面合法手を逆にたどり、直前局面を列挙する。
//!
//! 設計書predecessor-generator.md「直前局面の定義」の集合Aを入力と候補に適用する。

mod material;
mod reverse;
mod transient;

use std::fmt;

use crate::core::bitboard::Bitboard;
use crate::core::movegen::MoveGenerator;
use crate::core::piece::{Color, PieceKind};
use crate::core::position::{Position, PositionError};
use crate::core::rules::{MoveRules, PromotionRule};

/// 規則集合に従う直前局面の生成器。
///
/// 呼び出しをまたぐ可変状態を持たず、複数スレッドから共有できる。
#[derive(Clone)]
pub struct PredecessorGenerator {
    /// 候補着手の合法性と再適用を検証する順方向生成器。
    forward: MoveGenerator,
}

impl PredecessorGenerator {
    /// 標準の着手規則(L0・P0)を使う生成器を作る。
    pub fn standard() -> Self {
        Self::new(MoveRules::standard())
    }

    /// 指定した着手規則を使う生成器を作る。
    pub fn new(rules: MoveRules) -> Self {
        Self {
            forward: MoveGenerator::new(rules),
        }
    }

    /// 対象局面へ1手で到達する直前局面を、重複なく返す。
    ///
    /// 返却順序は保証しない。入力は変更しない。
    /// 獅子力を含む全移動形式、最大2枚の捕獲、および一時状態を復元する。
    ///
    /// # Errors
    ///
    /// 設計書predecessor-generator.md「直前局面の定義」の集合Aに
    /// 対象局面が属さない場合、違反の種類を返す。
    pub fn generate_predecessors(
        &self,
        target: &Position,
    ) -> Result<Vec<Position>, PredecessorError> {
        membership(self.forward.rules(), target)?;
        Ok(reverse::generate(&self.forward, target))
    }
}

/// 直前局面生成器の入力が満たさない条件。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PredecessorError {
    /// 盤面または派生状態が局面の不変条件を満たさない。
    InvalidPosition(
        /// 局面の検査が検出した原因。
        PositionError,
    ),
    /// 設計書predecessor-generator.md「駒在庫」の初期在庫を超えている。
    MaterialExceedsInitial {
        /// 在庫を超えた対局者。
        color: Color,
        /// 初期配置における駒種。
        origin: PieceKind,
        /// 盤上に存在する由来別の駒数。
        found: u8,
        /// 初期配置における由来別の駒数。
        maximum: u8,
    },
    /// 成り権保留集合が採用規則と整合しない(第30条P0・P1・P2・P5)。
    RuleStateMismatch,
    /// 先獅子の記録升が局所条件を満たさない(第15条)。
    InvalidLionState,
}

impl fmt::Display for PredecessorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPosition(error) => write!(formatter, "invalid position: {error}"),
            Self::MaterialExceedsInitial {
                color,
                origin,
                found,
                maximum,
            } => write!(
                formatter,
                "material exceeds initial stock: {color:?} {origin:?}: {found} > {maximum}"
            ),
            Self::RuleStateMismatch => {
                write!(formatter, "promotion state does not match move rules")
            }
            Self::InvalidLionState => write!(formatter, "invalid lion-capture state"),
        }
    }
}

impl std::error::Error for PredecessorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidPosition(error) => Some(error),
            _ => None,
        }
    }
}

/// 設計書predecessor-generator.md「直前局面の定義」の集合Aへの所属を検査する。
fn membership(rules: MoveRules, position: &Position) -> Result<(), PredecessorError> {
    let found = base_membership(rules, position)?;
    lion_membership(position, lion_missing(&found, position.side_to_move()))
}

/// 記録升に依存しない盤面・在庫・成り権保留の条件を基底候補で検査する。
fn base_membership(
    rules: MoveRules,
    position: &Position,
) -> Result<material::Inventory, PredecessorError> {
    position
        .validate()
        .map_err(PredecessorError::InvalidPosition)?;
    let found = material::count(position);
    let initial = material::initial();
    for color in Color::ALL {
        for origin in PieceKind::ALL {
            let found = found[color.index()][origin.index()];
            let maximum = initial[color.index()][origin.index()];
            if found > maximum {
                return Err(PredecessorError::MaterialExceedsInitial {
                    color,
                    origin,
                    found,
                    maximum,
                });
            }
        }
    }

    let mut waiting = [0; 2];
    for square in position.promotion_deferred().iter() {
        let piece = position.piece_at(square).expect("validated deferred piece");
        let pawn = piece.kind() == Some(PieceKind::Pawn);
        match rules.promotion {
            PromotionRule::P0 if !rules.p5 || !pawn => {
                return Err(PredecessorError::RuleStateMismatch);
            }
            PromotionRule::P2 if !(rules.p5 && pawn) => {
                let count = &mut waiting[piece.color().expect("validated owner").index()];
                *count += 1;
                if *count > 1 {
                    return Err(PredecessorError::RuleStateMismatch);
                }
            }
            _ => {}
        }
    }

    Ok(found)
}

/// 手番側の獅子由来または麒麟由来の駒が初期在庫から欠けているかを返す。
fn lion_missing(found: &material::Inventory, side: Color) -> bool {
    [PieceKind::Lion, PieceKind::Kirin].iter().any(|kind| {
        found[side.index()][kind.index()] < material::initial()[side.index()][kind.index()]
    })
}

/// 設計書predecessor-generator.md「直前局面の定義」の局所条件を満たす
/// 先獅子の記録升の集合を返す。
///
/// 記録升は、直前着手側(`side_to_move().opposite()`)の非獅子駒または麒麟由来の
/// 成獅子がある升か、直前着手側に角鷹または飛鷲がある場合の空升である(第15条)。
/// 手番側の獅子在庫の条件は含めず、呼び出し側が[`lion_missing`]で判定する。
fn lion_record_squares(position: &Position) -> Bitboard {
    let previous = position.side_to_move().opposite();
    let mut squares = Bitboard::EMPTY;
    for square in position.pieces_of(previous).iter() {
        let piece = position.piece_at(square).expect("own square has a piece");
        if piece.kind() != Some(PieceKind::Lion) || piece.is_promoted() {
            squares.set(square);
        }
    }
    let lion_like = position.pieces_of_kind(previous, PieceKind::HornedFalcon)
        | position.pieces_of_kind(previous, PieceKind::SoaringEagle);
    if !lion_like.is_empty() {
        squares |= !position.occupied();
    }
    squares
}

/// 先獅子の記録升だけに依存する集合Aの条件を検査する。
///
/// 盤面・在庫・保留集合を検査済みの局面では、この検査だけを再適用する。
fn lion_membership(position: &Position, missing: bool) -> Result<(), PredecessorError> {
    match position.lion_capture_square() {
        None => Ok(()),
        Some(square) if missing && lion_record_squares(position).contains(square) => Ok(()),
        Some(_) => Err(PredecessorError::InvalidLionState),
    }
}

/// 生成器の共有に必要な自動トレイトを常時検査する。
const fn assert_send_sync<T: Send + Sync>() {}
const _: () = assert_send_sync::<PredecessorGenerator>();

#[cfg(test)]
mod tests;
