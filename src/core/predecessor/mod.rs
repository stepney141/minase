//! 局面合法手を逆にたどり、直前局面を列挙する。
//!
//! 設計書predecessor-generator.md「直前局面の定義」の集合Aを入力と候補に適用する。

mod material;
mod reverse;

use std::fmt;

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
    /// 現段階では固定移動・走り・固定跳びと0枚または1枚の捕獲を扱う。
    /// 獅子力の特殊移動、直前局面の先獅子記録、およびP2の非移動駒の
    /// 待機状態の復元は未対応である。
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

    if let Some(square) = position.lion_capture_square() {
        let side = position.side_to_move();
        let previous = side.opposite();
        let piece = position.piece_at(square);
        if piece.is_some_and(|piece| {
            piece.color() != Some(previous)
                || (piece.kind() == Some(PieceKind::Lion) && !piece.is_promoted())
        }) {
            return Err(PredecessorError::InvalidLionState);
        }
        let lion_origins = [PieceKind::Lion, PieceKind::Kirin];
        let present: u8 = lion_origins
            .iter()
            .map(|kind| found[side.index()][kind.index()])
            .sum();
        let maximum: u8 = lion_origins
            .iter()
            .map(|kind| initial[side.index()][kind.index()])
            .sum();
        if present >= maximum {
            return Err(PredecessorError::InvalidLionState);
        }
        if piece.is_none()
            && position
                .pieces_of_kind(previous, PieceKind::HornedFalcon)
                .is_empty()
            && position
                .pieces_of_kind(previous, PieceKind::SoaringEagle)
                .is_empty()
        {
            return Err(PredecessorError::InvalidLionState);
        }
    }
    Ok(())
}

/// 生成器の共有に必要な自動トレイトを常時検査する。
const fn assert_send_sync<T: Send + Sync>() {}
const _: () = assert_send_sync::<PredecessorGenerator>();

#[cfg(test)]
mod tests;
