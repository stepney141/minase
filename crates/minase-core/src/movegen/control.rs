//! 駒の利きと到達升、および利きの逆引き。

use crate::attacks::{
    AttackTables, SpecialMovement, attack_tables, movement_profile, movement_profile_data,
};
use crate::board::{Bitboard, Direction, Square, step_square};
use crate::piece::{Color, PieceKind};
use crate::position::Position;

/// 指定した占有状態での駒の利きを返す。
/// 獅子を取った段階の直後の仮想盤面で足を判定する際に使う（chu-shogi.md第13条第4項と第9項）。
pub(crate) fn piece_control_with_occupancy(
    occupied: Bitboard,
    color: Color,
    kind: PieceKind,
    from: Square,
) -> crate::Bitboard {
    piece_control_with_tables(attack_tables(), occupied, color, kind, from)
}

impl Position {
    /// 指定した占有状態で、`from`に置いた`color`側の`kind`の疑似利きを返す。
    ///
    /// 固定利き、走り、および獅子・角鷹・飛鷲の2段階の到達範囲を含む。
    /// `occupied`は走りの遮蔽に使い、`from`を含まなくてもよい。
    pub fn piece_control_with_occupancy(
        occupied: Bitboard,
        color: Color,
        kind: PieceKind,
        from: Square,
    ) -> Bitboard {
        piece_control_with_occupancy(occupied, color, kind, from)
    }

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
    /// 実際の占有状態で、相手の獅子でない駒の升に疑似利きが届くことと、その駒を取る
    /// 合法手があることは一致する。王手を定めるchu-shogi.md第8条はこの一致を前提とし、
    /// 審判層の王駒の捕獲可能性とR1の攻撃的着手の判定もこれに基づく。
    /// 相手の獅子の升では捕獲制限があるため、疑似利きと合法な捕獲の有無は一致しない。
    pub fn attackers_to_by(&self, color: Color, square: Square, occupied: Bitboard) -> Bitboard {
        let tables = attack_tables();
        (ordinary_attackers_to(tables, self, color, square, occupied)
            | special_attackers_to(tables, self, color, square))
            & occupied
    }
}

/// 固定利きと走りを逆引きし、`square`へ届く`color`側の駒を返す。
///
/// 全固定移動が5×5近傍に収まるので、その範囲で相対升ごとの駒種集合に含まれる駒を調べる。
/// 走りは`square`から8方向の最初の遮蔽駒を求め、それが指定側の駒であり、
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
    let own = position.pieces_of(color) & occupied;
    for from in tables.neighbourhood(square) & own {
        let kinds = tables.fixed_attacker_kinds(color, square, from);
        if kinds == 0 {
            continue;
        }
        let kind = position
            .piece_at(from)
            .and_then(|piece| piece.kind())
            .expect("own square must contain a piece");
        if kinds & (1 << kind.index()) != 0 {
            attackers.set(from);
        }
    }
    attackers
        | sliding_attackers_to::<true>(tables, position, color, square, occupied, own)
        | sliding_attackers_to::<false>(tables, position, color, square, occupied, own)
}

/// 生値が増える方向と減る方向を別々に走査し、最初の遮蔽駒の逆方向の走りを調べる。
/// 全占有から遮蔽を選び、指定側の駒だけを残すので、仮想占有や相手駒も飛び越さない。
fn sliding_attackers_to<const INCREASING: bool>(
    tables: &AttackTables,
    position: &Position,
    color: Color,
    square: Square,
    occupied: Bitboard,
    own: Bitboard,
) -> Bitboard {
    let directions = if INCREASING {
        [
            Direction::East,
            Direction::North,
            Direction::NorthEast,
            Direction::NorthWest,
        ]
    } else {
        [
            Direction::West,
            Direction::South,
            Direction::SouthEast,
            Direction::SouthWest,
        ]
    };
    let mut attackers = Bitboard::EMPTY;
    for direction in directions {
        let blockers = tables.ray(square, direction) & occupied;
        let blocker = if INCREASING {
            blockers.lsb()
        } else {
            blockers.msb()
        };
        let Some(blocker) = blocker else {
            continue;
        };
        if !own.contains(blocker) {
            continue;
        }
        let kind = position
            .piece_at(blocker)
            .and_then(|piece| piece.kind())
            .expect("blocker must be a piece");
        let mask = tables.slide_directions(color, movement_profile(kind));
        if (mask >> direction.opposite().index()) & 1 != 0 {
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

/// 固定利きと走りによって`from`から`to`へ届くかを返す。
/// 固定利きと遮蔽なしの範囲で絞り、到達升を含む1方向だけ遮蔽を調べるので、
/// `piece_control_without_special(...).contains(to)`と同じ判定になる。
pub(super) fn reaches_without_special(
    tables: &AttackTables,
    occupied: Bitboard,
    color: Color,
    kind: PieceKind,
    from: Square,
    to: Square,
) -> bool {
    let id = movement_profile(kind);
    if tables.fixed(color, id, from).contains(to) {
        return true;
    }
    if !tables.reach(color, id, from).contains(to) {
        return false;
    }
    movement_profile_data(id).slides.iter().any(|slide| {
        let direction = slide.direction.for_color(color);
        tables.ray(from, direction).contains(to)
            && tables
                .sliding_control(from, direction, occupied)
                .contains(to)
    })
}

/// 固定利きと走りによって取れる`enemy`の升だけを返す。`enemy`は占有升の部分集合とする。
/// 遮蔽なしの範囲で絞り、各走り線では最初の遮蔽駒だけを調べるので、
/// 結果は`piece_control_without_special(...) & enemy`と一致する。
pub(super) fn capture_targets_without_special(
    tables: &AttackTables,
    occupied: Bitboard,
    color: Color,
    kind: PieceKind,
    from: Square,
    enemy: Bitboard,
) -> Bitboard {
    let id = movement_profile(kind);
    if !tables.reach(color, id, from).intersects(enemy) {
        return Bitboard::EMPTY;
    }
    let mut captures = tables.fixed(color, id, from) & enemy;
    for slide in movement_profile_data(id).slides {
        let direction = slide.direction.for_color(color);
        if tables.ray(from, direction).intersects(enemy)
            && let Some(blocker) = tables.first_blocker(from, direction, occupied)
            && enemy.contains(blocker)
        {
            captures.set(blocker);
        }
    }
    captures
}
