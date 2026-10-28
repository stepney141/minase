//! 王駒の捕獲と詰み、および合法手の有無の判定。

use super::super::repetition::repetition_is_forbidden;
use super::super::result::{GameResult, WinReason};
use super::AdjudicationContext;
use crate::board::Bitboard;
use crate::movegen::MoveGenerator;
use crate::mv::Move;
use crate::piece::{Color, PieceKind};
use crate::position::Position;

/// 着手後に相手の王駒がなくなった場合の勝利を返す。
pub(super) fn royal_capture_result(position: &Position, mover: Color) -> Option<GameResult> {
    position
        .royal_pieces(mover.opposite())
        .is_empty()
        .then_some(GameResult::Win {
            winner: mover,
            reason: WinReason::RoyalCapture,
        })
}

/// 着手が相手の残存王駒をすべて取るかを返す。
pub(crate) fn captures_last_royal(position: &Position, mv: Move) -> bool {
    let opponent = position.side_to_move().opposite();
    let opponent_royals = position.royal_pieces(opponent);
    let royal_count = opponent_royals.popcount();
    let captured_royal_count = mv
        .capture_candidates()
        .into_iter()
        .flatten()
        .filter(|&square| opponent_royals.contains(square))
        .count();

    royal_count > 0 && captured_royal_count == royal_count as usize
}

/// 手番側が相手の残存王駒をすべて取れるかを返す。
///
/// 相手の獅子でない駒の升では、疑似利きが届くことと合法に取れることが一致する
/// （RULES.md第8条）。王駒が1枚ならこの補題で判定し、2枚なら1手で両方を取る
/// 必要があるため全合法手を生成する。0枚なら対象がなく、3枚以上なら1手の最大捕獲数
/// である2枚を超えるので、いずれも偽を返す。
/// 捕獲手は既出局面を再現しないので、[`stalemate_or_mate`]の補題により反復のフィルタを掛けない。
pub(crate) fn can_capture_last_royal(position: &Position, generator: &MoveGenerator) -> bool {
    let mover = position.side_to_move();
    let royals = position.royal_pieces(mover.opposite());
    match royals.popcount() {
        1 => {
            let royal = royals.lsb().expect("one royal must be present");
            !position
                .attackers_to_by(mover, royal, position.occupied())
                .is_empty()
        }
        2 => {
            let mut moves = Vec::new();
            generator.generate_moves(position, &mut moves);
            moves
                .into_iter()
                .any(|mv| captures_last_royal(position, mv))
        }
        _ => false,
    }
}

/// 手番側の合法手なしまたは詰みによる敗北理由を返し、どちらでもなければ`None`を返す。
///
/// 王駒、王駒への攻撃駒に疑似利きが届く残りの駒、それ以外の駒の3群を順に生成する。
/// 各群は[`PieceKind::ALL`]順、同じ駒種は升の生値の昇順とする。
/// 王駒が攻撃されている局面で、逃れる手が見つかるまでに試す候補手を減らすための順序である。
/// 各駒を1回ずつ生成するので候補手の集合は全件生成と同じであり、合法手と逃れる手の
/// 有無だけを判定するため、生成と走査の順序は結果に影響しない。
///
/// RULES.md第21条第3項では、相手の最後の王駒を先に取る手と、即時の勝ちまたは
/// 引き分けを生じる手も逃れる手に数える。そのため、候補を王駒の移動や攻撃駒の捕獲に
/// 限らない。着手側の即時敗北は逃れる手としない。
/// R2とR3の禁止手を除いた最初の逃れる手で生成を打ち切る。
/// E1では詰みを判定せず、禁止されない最初の合法手で打ち切る。
/// すべての仮想着手後に局面を復元し、合法手がなければ詰みと区別して`Stalemate`を返す。
///
/// 捕獲を含む手は駒数、成りを含む手は駒種が不可逆に変わるので既出局面を再現せず、
/// R2とR3では禁じられない。
/// このため、受けの側の着手にはR2とR3のフィルタを掛けるが、
/// 相手が最後の王駒を取る応手には掛けない。
pub(crate) fn stalemate_or_mate(
    position: &mut Position,
    context: &AdjudicationContext<'_>,
) -> Option<WinReason> {
    let mover = position.side_to_move();
    let occupied = position.occupied();
    let royals = position.royal_pieces(mover);
    let mut attackers = Bitboard::EMPTY;
    for royal in royals {
        attackers |= position.attackers_to_by(mover.opposite(), royal, occupied);
    }
    let mut capturers = Bitboard::EMPTY;
    for attacker in attackers {
        capturers |= position.attackers_to_by(mover, attacker, occupied);
    }
    capturers &= !royals;
    let remaining = position.pieces_of(mover) & !(royals | capturers);

    let mut base_moves = Vec::new();
    let mut moves = Vec::new();
    let mut has_legal_move = false;
    for group in [royals, capturers, remaining] {
        for kind in PieceKind::ALL {
            for from in position.pieces_of_kind(mover, kind) & group {
                moves.clear();
                context
                    .generator
                    .generate_moves_from(position, from, &mut base_moves, &mut moves);
                for &mv in &moves {
                    let captures_royals = captures_last_royal(position, mv);
                    let undo = position.make_move_unchecked(mv, context.rules.moves);
                    if repetition_is_forbidden(context.state.repetition(), position) {
                        position.unmake_move(undo);
                        continue;
                    }
                    has_legal_move = true;
                    if context.rules.e1 {
                        position.unmake_move(undo);
                        return None;
                    }

                    let escapes_mate = captures_royals
                        || match context.candidate_immediate_result(position, mv, &undo) {
                            Some(GameResult::Win { winner, .. }) => winner == mover,
                            Some(GameResult::Draw { .. }) => true,
                            None => !can_capture_last_royal(position, context.generator),
                        };
                    position.unmake_move(undo);
                    if escapes_mate {
                        return None;
                    }
                }
            }
        }
    }

    Some(if has_legal_move {
        WinReason::Mate
    } else {
        WinReason::Stalemate
    })
}
