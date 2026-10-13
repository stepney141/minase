//! 局面合法手を逆にたどり、直前局面を列挙する。
//!
//! 規則`R`での局面合法手の集合を`Moves_R(p)`、着手適用後の局面を`Next_R(p, m)`とする。
//! 対象局面`q`に対し、返す集合は`{p ∈ A | ∃m ∈ Moves_R(p), Next_R(p, m) = q}`である。
//! 局面の一致は[`Position`]の全フィールドの一致で判定する。
//!
//! 集合Aは、次の3群の条件をすべて満たす局面の集合である。
//!
//! 1. [`Position::validate`]を満たし、両Zobrist値が再計算値と一致する。
//! 2. 各対局者の由来別の駒数が初期配置の枚数を超えない。
//!    由来は、不成駒ではその駒種、成駒では[`PieceKind::unpromoted`]が返す駒種である。
//! 3. 成り権保留と先獅子の一時状態が、以下の局所条件を満たす。
//!
//! 成り権保留ビットの升には、所有者の敵陣内に成れる不成駒がある。
//! P0でP5なしなら保留集合は空、P0とP5なら歩兵の升だけとする。
//! P2では、各対局者についてP5の歩兵を除く待機ビットが高々1個である。
//!
//! 先獅子の記録升がある場合は、次の3条件をすべて満たす。
//!
//! 1. 記録升は空升か、直前の着手側の非獅子駒または麒麟由来の成獅子の升である。
//! 2. 手番側の獅子由来または麒麟由来の駒が、初期在庫から1枚以上欠けている。
//! 3. 記録升が空升なら、直前の着手側に角鷹または飛鷲がある。
//!
//! 完全性は集合Aに対する相対的な保証であり、初期局面からの到達可能性は判定しない。
//! 順方向生成器で候補を検証する理由は
//! `docs/adr/verify-predecessor-candidates-with-forward-movegen.md`にある。

mod material;
mod reverse;
mod transient;

use std::fmt;

use crate::board::Bitboard;
use crate::movegen::MoveGenerator;
use crate::piece::{Color, PieceKind};
use crate::position::{Position, PositionError};
use crate::rules::{MoveRules, PromotionRule};

/// 規則集合に従う直前局面の生成器。
///
/// 呼び出しをまたぐ可変状態を持たず、複数スレッドから共有できる。
/// 受け取る規則は[`MoveRules`]だけである。
/// 反復規則と駒枯れ規則は局面合法手を変えないので受け取らない。
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
    /// 異なる駒のじっとなど、同じ直前局面から複数の着手で対象局面へ至る場合も1回だけ返す。
    /// 正常な入力で直前局面がない場合に限り`Ok(Vec::new())`を返す。
    ///
    /// # エラー
    ///
    /// 対象局面にも[`predecessor`](crate::predecessor)の集合Aの全条件を課し、
    /// 違反の種類を[`PredecessorError`]で返す。
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
    /// 盤面の不変条件や両Zobrist値の一致など、[`Position::validate`]の検査に失敗した。
    InvalidPosition(
        /// 局面の検査が検出した原因。
        PositionError,
    ),
    /// 対局者ごとの由来別の駒数が初期在庫を超えている。
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
    ///
    /// P0でP5なしの非空集合、P0とP5での歩兵以外のビット、または
    /// P2でP5の歩兵を除く待機ビットが2個以上ある対局者を検出した。
    /// 構築経路と拡張SFENが受理した局面にも、この生成器の追加検査を適用する。
    RuleStateMismatch,
    /// 先獅子の記録升が局所条件を満たさない(第15条)。
    ///
    /// [`predecessor`](crate::predecessor)で定める先獅子の3条件のいずれかに違反した。
    /// 構築経路と拡張SFENが受理した局面にも、この生成器の追加検査を適用する。
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

/// [`predecessor`](crate::predecessor)で定める集合Aへの所属を検査する。
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

/// [`predecessor`](crate::predecessor)の局所条件を満たす先獅子の記録升の集合を返す。
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
