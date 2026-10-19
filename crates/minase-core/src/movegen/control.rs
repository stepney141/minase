//! 駒の利きと到達升、および利きの逆引き。

use crate::attacks::{
    AttackTables, SpecialMovement, attack_tables, movement_profile, movement_profile_data,
};
use crate::board::{Bitboard, Direction, Square, step_square};
use crate::piece::{Color, PieceKind};
use crate::position::Position;

/// 指定した占有状態での駒の利きを返す。
/// 獅子を取った段階の直後の仮想盤面で足を判定する際に使う（RULES.md第13条第4項と第9項）。
pub(crate) fn piece_control_with_occupancy(
    occupied: Bitboard,
    color: Color,
    kind: PieceKind,
    from: Square,
) -> crate::Bitboard {
    piece_control_with_tables(attack_tables(), occupied, color, kind, from)
}

impl Position {
    /// 指定した升にある駒の、現在の占有状態での疑似利きを返す。
    ///
    /// 規則を考慮しない疑似利きであり、固定利き、走り、および獅子・角鷹・飛鷲の
    /// 2段階の到達範囲を含む。獅子の捕獲制限の合法性は調べない。
    ///
    /// # Panics
    ///
    /// `square`に駒がない場合にパニックする。
    pub fn piece_control(&self, square: Square) -> Bitboard {
        let piece = self
            .piece_at(square)
            .expect("piece control square must contain a piece");
        piece_control_with_occupancy(
            self.occupied(),
            piece.color().expect("board piece must have a color"),
            piece.kind().expect("board piece must have a kind"),
            square,
        )
    }

    /// 指定した占有状態で`square`へ疑似利きが届く`color`側の駒を返す。
    ///
    /// 駒の升が結果に含まれることと、その駒が`color`側であり、その升が`occupied`に含まれ、
    /// `square`が同じ占有状態の`piece_control_with_occupancy`に含まれることは同値である。
    /// この疑似利きには固定利き、走り、および特殊移動の到達範囲を含む。
    /// 走りの遮蔽にも`occupied`を使う。
    /// 結果と`occupied`の積を取ることで、交換列の途中で除いた駒が
    /// 固定利きや特殊移動の到達範囲から再び攻撃駒として現れることを防ぐ。
    /// 静的交換評価と王駒への利き判定は片側の色しか使わないため、反対側は計算しない。
    /// 規則を考慮しない疑似利きなので、審判層の判定の代わりにはならない。
    pub fn attackers_to_by(&self, color: Color, square: Square, occupied: Bitboard) -> Bitboard {
        let tables = attack_tables();
        (ordinary_attackers_to(tables, self, color, square, occupied)
            | special_attackers_to(tables, self, color, square))
            & occupied
    }
}

/// 固定利きと走りを逆引きし、`square`へ届く`color`側の駒を返す。
///
/// 全固定移動が5×5近傍に収まるので、その範囲の指定側の駒について固定利き表が`square`を含むか調べる。
/// 走りは`square`から8方向へ占有を考慮した利き線を引き、最初の遮蔽駒が指定側の駒であり、
/// 逆方向へ走る動きの定義を持つ場合に加える。全プロファイルの
/// 走りは距離無制限なので、この判定は駒種ごとの走り計算と同値である。
fn ordinary_attackers_to(
    tables: &AttackTables,
    position: &Position,
    color: Color,
    square: Square,
    occupied: Bitboard,
) -> Bitboard {
    let mut attackers = Bitboard::EMPTY;
    for from in tables.neighbourhood(square) & position.pieces_of(color) {
        let kind = position
            .piece_at(from)
            .and_then(|piece| piece.kind())
            .expect("own square must contain a piece");
        if tables
            .fixed(color, movement_profile(kind), from)
            .contains(square)
        {
            attackers.set(from);
        }
    }
    let own = position.pieces_of(color) & occupied;
    for direction in Direction::ALL {
        // 利き線は最初の遮蔽駒までしか含まないので、`ray & own`は空か1升である。
        let ray = tables.sliding_control(square, direction, occupied);
        let Some(blocker) = (ray & own).lsb() else {
            continue;
        };
        let kind = position
            .piece_at(blocker)
            .and_then(|piece| piece.kind())
            .expect("blocker must be a piece");
        let mask = tables.slide_directions(color, movement_profile(kind));
        let reverse = direction.opposite();
        if (mask >> reverse.index()) & 1 != 0 {
            attackers.set(blocker);
        }
    }
    attackers
}

/// 特殊移動の到達範囲を逆引きし、`square`へ届く`color`側の駒を返す。
/// 特殊移動を持つ駒種は獅子、角鷹および飛鷲だけなので、その3種だけを調べる。
fn special_attackers_to(
    tables: &AttackTables,
    position: &Position,
    color: Color,
    square: Square,
) -> Bitboard {
    let mut attackers = Bitboard::EMPTY;
    for kind in [
        PieceKind::Lion,
        PieceKind::HornedFalcon,
        PieceKind::SoaringEagle,
    ] {
        let pieces = position.pieces_of_kind(color, kind);
        if pieces.is_empty() {
            continue;
        }
        match movement_profile_data(movement_profile(kind)).special {
            SpecialMovement::None => {}
            SpecialMovement::Lion => {
                attackers |= (tables.king_steps(square) | tables.lion_jumps(square)) & pieces;
            }
            SpecialMovement::LionLike(profile) => {
                for relative in profile.directions {
                    let reverse = relative.for_color(color).opposite();
                    if let Some(first) = step_square(square, reverse) {
                        if pieces.contains(first) {
                            attackers.set(first);
                        }
                        if let Some(second) = step_square(first, reverse)
                            && pieces.contains(second)
                        {
                            attackers.set(second);
                        }
                    }
                }
            }
        }
    }
    attackers
}

/// 駒の利き(固定利き・走り・特殊移動の到達範囲)を計算して返す。
fn piece_control_with_tables(
    tables: &AttackTables,
    occupied: Bitboard,
    color: Color,
    kind: PieceKind,
    from: Square,
) -> Bitboard {
    let profile_id = movement_profile(kind);
    let profile = movement_profile_data(profile_id);
    let mut result = piece_control_without_special(tables, occupied, color, kind, from);

    match profile.special {
        SpecialMovement::None => {}
        SpecialMovement::Lion => {
            result |= tables.king_steps(from) | tables.lion_jumps(from);
        }
        SpecialMovement::LionLike(lion_like) => {
            for direction in lion_like.directions {
                let direction = direction.for_color(color);
                if let Some(first) = crate::board::step_square(from, direction) {
                    result.set(first);
                    if let Some(second) = crate::board::step_square(first, direction) {
                        result.set(second);
                    }
                }
            }
        }
    }
    result
}

/// 特殊移動の第1段階だけで停止する場合の到達升を返す。
pub(super) fn special_step_destinations(
    tables: &AttackTables,
    color: Color,
    from: Square,
    special: SpecialMovement,
) -> Bitboard {
    match special {
        SpecialMovement::None => Bitboard::EMPTY,
        SpecialMovement::Lion => tables.king_steps(from),
        SpecialMovement::LionLike(profile) => {
            let mut destinations = Bitboard::EMPTY;
            for relative in profile.directions {
                if let Some(first) = crate::board::step_square(from, relative.for_color(color)) {
                    destinations.set(first);
                }
            }
            destinations
        }
    }
}

/// 特殊移動を除いた駒の利き(固定利きと走りのみ)を返す。
pub(super) fn piece_control_without_special(
    tables: &AttackTables,
    occupied: Bitboard,
    color: Color,
    kind: PieceKind,
    from: Square,
) -> Bitboard {
    let profile_id = movement_profile(kind);
    let profile = movement_profile_data(profile_id);
    let mut result = tables.fixed(color, profile_id, from);
    for slide in profile.slides {
        result |= tables.sliding_control(from, slide.direction.for_color(color), occupied);
    }
    result
}
