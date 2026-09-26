//! 獅子の捕獲制限と先獅子・付け喰いの判定。

use super::{VirtualBoard, piece_control_with_occupancy};
use crate::core::board::Square;
use crate::core::mv::Move;
use crate::core::piece::{PieceCode, PieceKind};
use crate::core::position::Position;
use crate::core::rules::{LionRule, MoveRules};

impl MoveRules {
    /// 着手が獅子の捕獲制限(第13条〜第16条)をすべて満たすかどうかを返す。
    /// 獅子を取らない着手は常に満たす。
    pub(crate) fn special_move_is_legal(self, position: &Position, mv: Move) -> bool {
        let captured_lions = captured_lions(position, mv);
        if captured_lions.into_iter().all(|lion| lion.is_none()) {
            return true;
        }

        // 獅子による獅子の捕獲制限(第14条・第16条)。
        let moving_kind = position.piece_at(mv.from).and_then(|piece| piece.kind());
        if moving_kind == Some(PieceKind::Lion)
            && captured_lions
                .into_iter()
                .flatten()
                .any(|lion| !lion_capture_is_legal(self, position, mv, lion))
        {
            return false;
        }

        // 先獅子による直後の捕獲禁止(第15条)。付け喰いは先獅子より優先する
        // (第16条第7項)ため、付け喰いが成立する着手には適用しない。
        let move_is_tsukegui = captured_lions
            .into_iter()
            .flatten()
            .any(|lion| is_tsukegui(position, mv, lion));
        if !move_is_tsukegui
            && let Some(trigger) = position.lion_taken_by_non_lion()
            && captured_lions.into_iter().flatten().any(|lion| {
                // L2採用時、麒麟が成った獅子への直後の取り返しは禁止しない(第29条L2)。
                let l2_exemption = self.l2
                    && trigger.by_kirin_promotion
                    && lion == trigger.square
                    && position.piece_at(lion).is_some_and(PieceCode::is_promoted);
                // L1は足の有無にかかわらず非獅子による取り返しを禁じ、
                // 標準規則(L0)は取った側に残る獅子に足がある場合だけ禁じる(第29条)。
                !l2_exemption
                    && match self.lion {
                        LionRule::L1 => moving_kind != Some(PieceKind::Lion),
                        LionRule::L0 { l4 } => {
                            // L4は禁止を非獅子の駒による捕獲に限定する(第29条L4)。
                            !(l4 && moving_kind == Some(PieceKind::Lion))
                                && lion_has_foot_after_capture(self, position, mv, lion)
                        }
                    }
            })
        {
            return false;
        }

        true
    }
}

/// 着手で取られる相手獅子の升を返す。
fn captured_lions(position: &Position, mv: Move) -> [Option<Square>; 2] {
    position.captured_squares(mv).map(|capture| {
        capture.filter(|&square| {
            position
                .piece_at(square)
                .is_some_and(|piece| piece.kind() == Some(PieceKind::Lion))
        })
    })
}

/// 着手が付け喰い(第16条)にあたるかどうかを返す。付け喰いとは、獅子が第1段階で
/// 価値ある駒(歩兵・仲人以外)を取り、第2段階で隣接していない相手獅子を取る着手をいう。
fn is_tsukegui(position: &Position, mv: Move, lion_square: Square) -> bool {
    if position.piece_at(mv.from).and_then(|piece| piece.kind()) != Some(PieceKind::Lion) {
        return false;
    }
    let Some(mid) = mv.mid else {
        return false;
    };
    let distance = mv
        .from
        .file()
        .abs_diff(lion_square.file())
        .max(mv.from.rank().abs_diff(lion_square.rank()));
    if mv.to != lion_square || distance != 2 {
        return false;
    }

    let [mid_capture, destination_capture] = position.captured_squares(mv);
    mid_capture == Some(mid)
        && destination_capture == Some(lion_square)
        && position.piece_at(mid).is_some_and(|piece| {
            !matches!(piece.kind(), Some(PieceKind::Pawn | PieceKind::GoBetween))
        })
}

/// 獅子による相手獅子の捕獲が第14条・第16条を満たすかどうかを返す。隣接していれば
/// 無条件に取れる。距離2では、付け喰いが成立するか、取られる獅子に足がない場合に限る。
fn lion_capture_is_legal(
    rules: MoveRules,
    position: &Position,
    mv: Move,
    lion_square: Square,
) -> bool {
    let distance = mv
        .from
        .file()
        .abs_diff(lion_square.file())
        .max(mv.from.rank().abs_diff(lion_square.rank()));

    match distance {
        1 => true,
        2 if is_tsukegui(position, mv, lion_square) => true,
        2 => !lion_has_foot_after_capture(rules, position, mv, lion_square),
        _ => false,
    }
}

/// 相手獅子を取った直後に取り返される足(第13条)があるかどうかを返す。標準規則では、
/// 歩兵または仲人が唯一の足である場合、第1段階でその駒を取っても足が消滅したとは扱わない
/// (第16条第8項から第10項)。L3採用時は、これらの規定を適用せず、着手適用後の
/// 仮想盤面だけで足を判定する(第29条L3)。
/// 残る足は利きを逆引きして求める(`movegen-speedup-2.md`「段階4」)。
fn lion_has_foot_after_capture(
    rules: MoveRules,
    position: &Position,
    mv: Move,
    lion_square: Square,
) -> bool {
    let defending_color = position
        .piece_at(lion_square)
        .and_then(|piece| piece.color())
        .expect("capture square must contain a lion");
    let board = VirtualBoard::after_move(position, mv);

    let captured_pawn_or_go_between_had_foot = if rules.l3 {
        false
    } else {
        let [mid_capture, destination_capture] = position.captured_squares(mv);
        destination_capture == Some(lion_square)
            && mid_capture.is_some_and(|mid| {
                position.piece_at(mid).is_some_and(|piece| {
                    let Some(kind @ (PieceKind::Pawn | PieceKind::GoBetween)) = piece.kind() else {
                        return false;
                    };
                    piece_control_with_occupancy(board.occupied, defending_color, kind, mid)
                        .contains(mv.to)
                })
            })
    };

    debug_assert!(board.own.contains(mv.to));
    debug_assert!(!board.enemy.contains(mv.to));
    captured_pawn_or_go_between_had_foot
        || !position
            .attackers_to_by(defending_color, mv.to, board.occupied)
            .is_empty()
}
