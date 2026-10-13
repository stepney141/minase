//! 獅子の捕獲制限と先獅子・付け喰いの判定。
//!
//! 獅子の捕獲制限（RULES.md第13条から第16条、第29条）は合法手の定義の一部
//! （第3条第8項）なので、合法手生成の中で扱う。
//! [`VirtualBoard`]が表す仮想盤面と利きの計算で判定する。

use super::control::piece_control_with_occupancy;
use super::virtual_board::VirtualBoard;
use crate::board::Square;
use crate::mv::Move;
use crate::piece::{PieceCode, PieceKind};
use crate::position::Position;
use crate::rules::{LionRule, MoveRules};

impl MoveRules {
    /// 着手が獅子の捕獲制限(第13条〜第16条)をすべて満たすかどうかを返す。
    /// 獅子を取らない着手は常に満たす。
    ///
    /// 1手で獅子を2枚取る場合、いずれかの捕獲が付け喰いなら着手全体を先獅子の禁止から除く
    /// （RULES.md第15条第8項、第16条第7項）。
    /// 付け喰いでなければ、いずれか1枚が先獅子の禁止に該当するだけで着手全体を禁じる。
    /// L2の例外は、麒麟の成りを記録した升と取る獅子の升が一致し、その獅子が成駒の場合に限る。
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
            && captured_lions.into_iter().flatten().any(|lion_square| {
                // L2採用時、麒麟が成った獅子への直後の取り返しは禁止しない(第29条L2)。
                let l2_exemption = self.l2
                    && trigger.by_kirin_promotion
                    && lion_square == trigger.square
                    && position
                        .piece_at(lion_square)
                        .is_some_and(PieceCode::is_promoted);
                // L1は足の有無にかかわらず非獅子による取り返しを禁じ、
                // 標準規則(L0)は獅子のいた升への利きで足を判定し、足がある場合だけ禁じる(第13条・第15条)。
                !l2_exemption
                    && match self.lion {
                        LionRule::L1 => moving_kind != Some(PieceKind::Lion),
                        LionRule::L0 { l4 } => {
                            // L4は禁止を非獅子の駒による捕獲に限定する(第29条L4)。
                            !(l4 && moving_kind == Some(PieceKind::Lion))
                                && lion_has_foot_after_capture(self, position, mv, lion_square)
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
///
/// 距離は着手前の移動元から取られる獅子までのチェビシェフ距離であり、2を要する
/// （RULES.md第16条第1項）。
/// 経由升は両獅子と直線上に並ぶ必要がない（第16条第11項）。
/// 隣接する獅子を2段階で取る手は付け喰いではなく、第14条第1項により合法である
/// （第16条第12項）。
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
/// 足は獅子を取った段階の直後の仮想盤面で、その獅子のいた升への利きにより判定する（第13条）。
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

/// 獅子を取った段階の直後の仮想盤面で、取られた側の駒が獅子のいた`lion_square`へ疑似利きを持つかを調べる。
/// 居喰いや第1段階での捕獲でも、到達升ではなくこの升で判定する（RULES.md第13条第3項と第8項）。
///
/// [`Position::attackers_to_by`]による疑似利きなので、取り返す手に獅子の捕獲制限を
/// 再帰的に適用せず（RULES.md第13条第7項）、王駒や取られ得る駒も除かない
/// （第13条第5項、第6項）。
/// 取られた獅子を除いた盤面を使うので、裏足も通る（第13条第2項）。
/// 獅子を取った駒を移動元から`lion_square`へ移し、その段階までに取られた駒を除く（第13条第4項と第9項）。
/// 獅子を取った後の移動と捕獲は反映しない。
///
/// L3を採用しない場合、到達升で獅子を取る手では、経由升で取った歩兵または仲人が
/// 取られる前の升から`lion_square`へ利いていたことも足とみなす（第16条第8項から第10項と第13項）。
/// この場合は`mv.to`と`lion_square`が一致する。
/// 占有を戻す方式を採らない理由は`docs/adr/judge-pawn-foot-by-control-before-capture.md`にある。
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
    let board = if mv.mid == Some(lion_square) {
        VirtualBoard::new(position, defending_color.opposite(), mv.from).move_to(lion_square)
    } else {
        VirtualBoard::after_move(position, mv)
    };

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
                        .contains(lion_square)
                })
            })
    };

    debug_assert!(!board.enemy.contains(lion_square));
    // 逆引きは元の駒配置を参照するので、獅子を取った段階までに取られた駒を足の候補から除く。
    captured_pawn_or_go_between_had_foot
        || !(position.attackers_to_by(defending_color, lion_square, board.occupied) & board.enemy)
            .is_empty()
}
