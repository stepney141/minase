//! 初期配置から導く所有者別・由来別の駒在庫。

use std::sync::OnceLock;

use crate::core::piece::{COLOR_COUNT, Color, PIECE_KIND_COUNT, PieceCode, PieceKind};
use crate::core::position::Position;

/// 所有者と初期配置の駒種を添字にする在庫。
pub(super) type Inventory = [[u8; PIECE_KIND_COUNT]; COLOR_COUNT];

/// 初期配置の在庫は規則と無関係なので、プロセス内で1回だけ導出する。
pub(super) fn initial() -> &'static Inventory {
    static INITIAL: OnceLock<Inventory> = OnceLock::new();
    INITIAL.get_or_init(|| count(&Position::initial()))
}

/// 成駒を成る前の駒種へ戻して、盤上の在庫を数える。
pub(super) fn count(position: &Position) -> Inventory {
    let mut found = [[0; PIECE_KIND_COUNT]; COLOR_COUNT];
    for square in position.occupied().iter() {
        let piece = position
            .piece_at(square)
            .expect("occupied square has a piece");
        let kind = piece.kind().expect("board piece has a kind");
        let origin = if piece.is_promoted() {
            kind.unpromoted().expect("promoted piece has an origin")
        } else {
            kind
        };
        found[piece.color().expect("board piece has an owner").index()][origin.index()] += 1;
    }
    found
}

/// 在庫超過がない検査済み局面について、不足在庫を返す。
pub(super) fn missing(position: &Position) -> Inventory {
    let found = count(position);
    let mut missing = *initial();
    for color in Color::ALL {
        for origin in PieceKind::ALL {
            missing[color.index()][origin.index()] -= found[color.index()][origin.index()];
        }
    }
    missing
}

/// 不足在庫から1枚を復元し、その駒と減算後の在庫を処理へ渡す。
///
/// 同じ由来の複数枚捕獲でも、処理内で残数を引き継ぐことで上限を守る。
pub(super) fn for_each_restoration(
    missing: &mut Inventory,
    color: Color,
    mut visit: impl FnMut(PieceCode, &mut Inventory),
) {
    for origin in PieceKind::ALL {
        if missing[color.index()][origin.index()] == 0 {
            continue;
        }
        missing[color.index()][origin.index()] -= 1;
        visit(
            PieceCode::new(color, origin).expect("initial stock has unpromoted kinds"),
            missing,
        );
        if let Some(promoted) = origin.promoted() {
            visit(
                PieceCode::new_promoted(color, promoted).expect("promotion pair is valid"),
                missing,
            );
        }
        missing[color.index()][origin.index()] += 1;
    }
}
