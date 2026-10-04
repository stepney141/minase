//! 第17〜19条（成り）と第30条（成りに関するローカルルールP系）のテスト。
//!
//! 先手（Black）の敵陣はマトリクス段1〜4、相手側の最奥段は段1である（第3条6号）。

use super::{MoveGenerator, PROMOTION_PAIRS, generated, generated_with, moves_from, msq, mv};
use crate::core::mv::Move;
use crate::core::piece::{Color, PieceCode, PieceKind};
use crate::core::rules::{MoveRules, PromotionRule};

fn promotion_rules(rule: PromotionRule) -> MoveRules {
    MoveRules {
        promotion: rule,
        ..MoveRules::standard()
    }
}

fn modifier_rules(p3: bool, p4: bool, p5: bool, p6: bool) -> MoveRules {
    MoveRules {
        p3,
        p4,
        p5,
        p6,
        ..MoveRules::standard()
    }
}
use crate::core::board::Square;
use crate::test_util::{position, position_from_codes};

/// (from, to) が一致する着手を成り選択別に数える。
fn promotion_choices(moves: &[Move], from: Square, to: Square) -> (usize, usize) {
    let unpromoted = moves
        .iter()
        .filter(|m| m.from == from && m.to == to && m.mid.is_none() && !m.promote)
        .count();
    let promoted = moves
        .iter()
        .filter(|m| m.from == from && m.to == to && m.mid.is_none() && m.promote)
        .count();
    (unpromoted, promoted)
}

/// 成り・不成の2手がちょうど1手ずつ含まれることを検査する（第18条5項）。
fn assert_both_choices(moves: &[Move], from: Square, to: Square) {
    assert_eq!(promotion_choices(moves, from, to), (1, 1));
}

/// 不成の1手だけが含まれることを検査する。
fn assert_no_promotion(moves: &[Move], from: Square, to: Square) {
    assert_eq!(promotion_choices(moves, from, to), (1, 0));
}

/// 成りの1手だけが含まれることを検査する（第30条P5・P6）。
fn assert_promotion_forced(moves: &[Move], from: Square, to: Square) {
    assert_eq!(promotion_choices(moves, from, to), (0, 1));
}

// ---------------------------------------------------------------------------
// 第17条　成ることができる駒
// ---------------------------------------------------------------------------

// D1-009-23: 成り先対応18組（第9条の表、第17条2項）。
#[test]
fn article_9_17_promotion_maps_the_eighteen_pairs() {
    for (kind, promoted_kind) in PROMOTION_PAIRS {
        // 敵陣直前 (6,5) から敵陣（段4以内）へ入る成り選択ありの着手を適用する。
        let board = position(Color::Black, &[(msq(6, 5), Color::Black, kind)]);
        let entry = generated(&board)
            .into_iter()
            .find(|m| m.from == msq(6, 5) && m.promote && m.to.rank() >= 8)
            .unwrap_or_else(|| panic!("kind={kind:?} must have a promoting entry"));
        let mut after = board.clone();
        after.make_move_unchecked(entry, MoveRules::standard());
        assert_eq!(
            after.piece_at(entry.to),
            PieceCode::new_promoted(Color::Black, promoted_kind),
            "kind={kind:?}"
        );
    }
}

// D1-017-01: 王将・玉将・獅子・奔王は成らない（第17条1項）。
#[test]
fn article_17_1_king_lion_and_free_king_never_promote() {
    for kind in [PieceKind::King, PieceKind::Lion, PieceKind::FreeKing] {
        // 敵陣直前から入陣する着手を含めて、成り選択ありの変種が存在しない。
        let board = position(Color::Black, &[(msq(6, 5), Color::Black, kind)]);
        let moves = moves_from(&generated(&board), msq(6, 5));
        assert!(
            moves.iter().any(|m| m.to.rank() >= 8),
            "kind={kind:?} should enter the zone"
        );
        assert!(moves.iter().all(|m| !m.promote), "kind={kind:?}");
    }
}

// D1-017-03: 再成の禁止（第17条4項）。
#[test]
fn article_17_4_promoted_pieces_never_promote_again() {
    // 仲人の成駒の醉象は、敵陣内の捕獲でも太子へ成れない。
    let promoted_elephant = position_from_codes(
        Color::Black,
        &[
            (
                msq(6, 3),
                PieceCode::new_promoted(Color::Black, PieceKind::DrunkElephant).unwrap(),
            ),
            (
                msq(6, 2),
                PieceCode::new(Color::White, PieceKind::Pawn).unwrap(),
            ),
        ],
    );
    let moves = generated(&promoted_elephant);
    assert_no_promotion(&moves, msq(6, 3), msq(6, 2));

    // 対照: 初期駒の醉象なら同じ捕獲で成り（太子）を選べる（第18条2項a）。
    let raw_elephant = position(
        Color::Black,
        &[
            (msq(6, 3), Color::Black, PieceKind::DrunkElephant),
            (msq(6, 2), Color::White, PieceKind::Pawn),
        ],
    );
    assert_both_choices(&generated(&raw_elephant), msq(6, 3), msq(6, 2));
}

// ---------------------------------------------------------------------------
// 第18条　成ることができる着手
// ---------------------------------------------------------------------------

// D1-018-01: 敵陣入りの成り（第18条1項・5項）。
#[test]
fn article_18_1_entry_into_the_zone_offers_promotion_choice() {
    let board = position(
        Color::Black,
        &[(msq(7, 5), Color::Black, PieceKind::SilverGeneral)],
    );
    let moves = generated(&board);
    // 直進して敵陣へ入る着手には成り・不成の2手がある。
    assert_both_choices(&moves, msq(7, 5), msq(7, 4));
    // 敵陣外から敵陣外への後斜め移動には成り変種がない。
    assert_no_promotion(&moves, msq(7, 5), msq(6, 6));
}

// D1-018-02: 敵陣内の捕獲による再度の成り（第18条2項a）。
#[test]
fn article_18_2a_capture_inside_the_zone_offers_promotion() {
    let board = position(
        Color::Black,
        &[
            (msq(6, 3), Color::Black, PieceKind::FerociousLeopard),
            (msq(6, 2), Color::White, PieceKind::Pawn),
        ],
    );
    let moves = generated(&board);
    // 捕獲なら成り（角行）と不成の2手。
    assert_both_choices(&moves, msq(6, 3), msq(6, 2));
    // 同じ from からの空升への移動には成り変種がない（第18条3項）。
    assert_no_promotion(&moves, msq(6, 3), msq(5, 2));
}

// D1-018-03: 敵陣から出る捕獲による成り（第18条2項b）。
#[test]
fn article_18_2b_capture_leaving_the_zone_offers_promotion() {
    let board = position(
        Color::Black,
        &[
            (msq(6, 4), Color::Black, PieceKind::FerociousLeopard),
            (msq(7, 5), Color::White, PieceKind::Pawn),
        ],
    );
    let moves = generated(&board);
    assert_both_choices(&moves, msq(6, 4), msq(7, 5));

    // 成りの成立は到達升が敵陣内であることを要しない。敵陣外に成駒が生じる。
    let mut after = board.clone();
    after.make_move_unchecked(mv(msq(6, 4), None, msq(7, 5), true), MoveRules::standard());
    assert_eq!(
        after.piece_at(msq(7, 5)),
        PieceCode::new_promoted(Color::Black, PieceKind::Bishop)
    );
}

// D1-018-04: 敵陣外へ出た後の再入による成り（第18条2項c・1項）。
#[test]
fn article_18_2c_reentry_offers_promotion_again() {
    let mut board = position(
        Color::Black,
        &[
            (msq(6, 5), Color::Black, PieceKind::SilverGeneral),
            (msq(1, 7), Color::White, PieceKind::GoBetween),
        ],
    );
    let rules = MoveRules::standard();
    // 敵陣入りで不成を選ぶ。
    assert_both_choices(&generated(&board), msq(6, 5), msq(6, 4));
    board.make_move_unchecked(mv(msq(6, 5), None, msq(6, 4), false), rules);
    board.make_move_unchecked(mv(msq(1, 7), None, msq(1, 8), false), rules);
    // 敵陣から非捕獲で退出する（後斜め）。
    assert_no_promotion(&generated(&board), msq(6, 4), msq(7, 5));
    board.make_move_unchecked(mv(msq(6, 4), None, msq(7, 5), false), rules);
    board.make_move_unchecked(mv(msq(1, 8), None, msq(1, 7), false), rules);
    // 改めて敵陣へ入る着手には成り・不成の2手が含まれる。
    assert_both_choices(&generated(&board), msq(7, 5), msq(6, 4));
}

// ---------------------------------------------------------------------------
// 第19条　最奥段における成り
// ---------------------------------------------------------------------------

// D1-019-01: 歩兵の最奥段救済（第19条1項）。
#[test]
fn article_19_1_pawn_reaching_last_rank_may_promote() {
    let board = position(Color::Black, &[(msq(6, 2), Color::Black, PieceKind::Pawn)]);
    let moves = generated(&board);
    // 非捕獲の最奥段到達でも成り（金将と同じ動きの成駒）と不成の2手がある。
    assert_both_choices(&moves, msq(6, 2), msq(6, 1));
    let mut after = board.clone();
    after.make_move_unchecked(mv(msq(6, 2), None, msq(6, 1), true), MoveRules::standard());
    assert_eq!(
        after.piece_at(msq(6, 1)),
        PieceCode::new_promoted(Color::Black, PieceKind::GoldGeneral)
    );
}

// D1-019-02: 最奥段で不成の歩兵は移動不能（第19条2項）。
#[test]
fn article_19_2_unpromoted_pawn_on_last_rank_is_immobile() {
    // 不成を選択した直後の局面を作る。
    let mut board = position(
        Color::Black,
        &[
            (msq(6, 2), Color::Black, PieceKind::Pawn),
            (msq(1, 7), Color::White, PieceKind::GoBetween),
        ],
    );
    let rules = MoveRules::standard();
    board.make_move_unchecked(mv(msq(6, 2), None, msq(6, 1), false), rules);
    board.make_move_unchecked(mv(msq(1, 7), None, msq(1, 8), false), rules);
    // 以後、最奥段の未成歩兵を from とする着手は生成されない。
    assert!(moves_from(&generated(&board), msq(6, 1)).is_empty());
}

// D1-019-03: 標準規則では香車の最奥段救済はない（第19条4項）。
#[test]
fn article_19_4_lance_gets_no_last_rank_relief() {
    // 敵陣内からの非捕獲の最奥段到達は不成の1手だけ（歩兵と異なり救済なし）。
    let board = position(Color::Black, &[(msq(6, 3), Color::Black, PieceKind::Lance)]);
    let moves = generated(&board);
    assert_no_promotion(&moves, msq(6, 3), msq(6, 1));

    // 敵陣外から最奥段へ直接走り込む着手は敵陣入りとして成れる（第18条1項）。
    let entry = position(Color::Black, &[(msq(6, 5), Color::Black, PieceKind::Lance)]);
    assert_both_choices(&generated(&entry), msq(6, 5), msq(6, 1));

    // 最奥段での捕獲は第18条2項aとして成れる。
    let capture = position(
        Color::Black,
        &[
            (msq(6, 3), Color::Black, PieceKind::Lance),
            (msq(6, 1), Color::White, PieceKind::Pawn),
        ],
    );
    assert_both_choices(&generated(&capture), msq(6, 3), msq(6, 1));
}

// D1-019-05: 成りの機会のマージ（第19条1項、第18条1項・2項、第30条P3、
// move-canonicalization.md の着手の同一性）。
#[test]
fn article_19_18_overlapping_promotion_chances_yield_two_moves() {
    // 例1: 歩兵の最奥段捕獲（第18条2項aと第19条1項が重なる）でも2手だけ。
    let pawn = position(
        Color::Black,
        &[
            (msq(6, 2), Color::Black, PieceKind::Pawn),
            (msq(6, 1), Color::White, PieceKind::CopperGeneral),
        ],
    );
    assert_both_choices(&generated(&pawn), msq(6, 2), msq(6, 1));

    // 例2: P3採用下で香車が敵陣外から最奥段へ走り込む（第18条1項とP3が重なる）。
    let generator = MoveGenerator::new(modifier_rules(true, false, false, false));
    let lance = position(Color::Black, &[(msq(6, 5), Color::Black, PieceKind::Lance)]);
    assert_both_choices(&generated_with(&generator, &lance), msq(6, 5), msq(6, 1));
}

// ---------------------------------------------------------------------------
// 第30条　成りに関するローカルルール
// ---------------------------------------------------------------------------

// D1-030-01: P1（Hodges式成り権回復）。不成後は敵陣内で1回待機し、次の敵陣内
// 非捕獲移動で成れる。再不成でトグルする。
#[test]
fn article_30_p1_promotion_right_recovers_after_one_wait_and_toggles() {
    let rules = promotion_rules(PromotionRule::P1);
    let generator = MoveGenerator::new(rules);
    let mut board = position(
        Color::Black,
        &[
            (msq(6, 5), Color::Black, PieceKind::FerociousLeopard),
            (msq(1, 7), Color::White, PieceKind::GoBetween),
        ],
    );

    // 敵陣入り。成り・不成を選べる（第18条1項）ので不成を選ぶ。
    assert_both_choices(&generated_with(&generator, &board), msq(6, 5), msq(6, 4));
    board.make_move_unchecked(mv(msq(6, 5), None, msq(6, 4), false), rules);
    board.make_move_unchecked(mv(msq(1, 7), None, msq(1, 8), false), rules);

    // 不成直後の敵陣内非捕獲移動は待機に当たり、成り変種がない。
    assert_no_promotion(&generated_with(&generator, &board), msq(6, 4), msq(6, 3));
    board.make_move_unchecked(mv(msq(6, 4), None, msq(6, 3), false), rules);
    board.make_move_unchecked(mv(msq(1, 8), None, msq(1, 7), false), rules);

    // 待機後の敵陣内非捕獲移動では成り権が回復する。再び不成を選ぶ。
    assert_both_choices(&generated_with(&generator, &board), msq(6, 3), msq(6, 2));
    board.make_move_unchecked(mv(msq(6, 3), None, msq(6, 2), false), rules);
    board.make_move_unchecked(mv(msq(1, 7), None, msq(1, 8), false), rules);

    // 回復後の再不成で本規則を改めて適用し、次の1回は待機になる。
    assert_no_promotion(&generated_with(&generator, &board), msq(6, 2), msq(6, 1));
    board.make_move_unchecked(mv(msq(6, 2), None, msq(6, 1), false), rules);
    board.make_move_unchecked(mv(msq(1, 8), None, msq(1, 7), false), rules);

    // その次の敵陣内非捕獲移動（後斜め (5,2)）で再び成れる。
    assert_both_choices(&generated_with(&generator, &board), msq(6, 1), msq(5, 2));
}

// 第30条P2: 敵陣入りで不成を選ぶと、その側の直後の1手番だけ待機する。
#[test]
fn article_30_p2_entry_defers_quiet_promotion_until_the_following_own_turn() {
    let rules = promotion_rules(PromotionRule::P2);
    let generator = MoveGenerator::new(rules);
    let mut board = position(
        Color::Black,
        &[
            (msq(6, 5), Color::Black, PieceKind::FerociousLeopard),
            (msq(1, 7), Color::White, PieceKind::GoBetween),
        ],
    );

    // 第30条P2: 敵陣入りでは成り・不成を選べるため、不成を選ぶ。
    assert_both_choices(&generated_with(&generator, &board), msq(6, 5), msq(6, 4));
    board.make_move_unchecked(mv(msq(6, 5), None, msq(6, 4), false), rules);
    board.make_move_unchecked(mv(msq(1, 7), None, msq(1, 8), false), rules);

    // 第30条P2: 直後の自手番は待機中なので、敵陣内の非捕獲移動では成れない。
    assert_no_promotion(&generated_with(&generator, &board), msq(6, 4), msq(6, 3));
    board.make_move_unchecked(mv(msq(6, 4), None, msq(6, 3), false), rules);
    board.make_move_unchecked(mv(msq(1, 8), None, msq(1, 7), false), rules);

    // 第30条P2: 待機満了後の敵陣内非捕獲移動では再び選択できる。
    assert_both_choices(&generated_with(&generator, &board), msq(6, 3), msq(6, 2));
}

#[test]
fn article_30_p2_waiting_does_not_block_a_capture_promotion() {
    // 第30条P2: 待機中でも、敵陣内の捕獲による成りの機会は妨げない。
    let rules = promotion_rules(PromotionRule::P2);
    let generator = MoveGenerator::new(rules);
    let mut board = position(
        Color::Black,
        &[
            (msq(6, 5), Color::Black, PieceKind::FerociousLeopard),
            (msq(6, 3), Color::White, PieceKind::Pawn),
            (msq(1, 7), Color::White, PieceKind::GoBetween),
        ],
    );
    board.make_move_unchecked(mv(msq(6, 5), None, msq(6, 4), false), rules);
    board.make_move_unchecked(mv(msq(1, 7), None, msq(1, 8), false), rules);

    assert_both_choices(&generated_with(&generator, &board), msq(6, 4), msq(6, 3));
}

#[test]
fn article_30_p2_quiet_move_started_in_zone_does_not_create_waiting() {
    // 第30条P2: 敵陣内で開始する非捕獲着手の不成は、新たな待機を生じさせない。
    let rules = promotion_rules(PromotionRule::P2);
    let generator = MoveGenerator::new(rules);
    let mut board = position(
        Color::Black,
        &[
            (msq(6, 4), Color::Black, PieceKind::FerociousLeopard),
            (msq(1, 7), Color::White, PieceKind::GoBetween),
        ],
    );
    assert_both_choices(&generated_with(&generator, &board), msq(6, 4), msq(6, 3));
    board.make_move_unchecked(mv(msq(6, 4), None, msq(6, 3), false), rules);
    board.make_move_unchecked(mv(msq(1, 7), None, msq(1, 8), false), rules);

    assert_both_choices(&generated_with(&generator, &board), msq(6, 3), msq(6, 2));
}

#[test]
fn article_30_p2_quiet_exit_from_zone_offers_promotion() {
    // 第30条P2: 敵陣から出る非捕獲着手にも成りの選択肢がある。
    let rules = promotion_rules(PromotionRule::P2);
    let generator = MoveGenerator::new(rules);
    let board = position(
        Color::Black,
        &[(msq(6, 4), Color::Black, PieceKind::FerociousLeopard)],
    );

    assert_both_choices(&generated_with(&generator, &board), msq(6, 4), msq(6, 5));
}

#[test]
fn article_30_p2_waiting_expires_when_another_piece_moves() {
    // 第30条P2: 待機は当該駒ではなく、その側が何かを動かした直後に満了する。
    let rules = promotion_rules(PromotionRule::P2);
    let generator = MoveGenerator::new(rules);
    let mut board = position(
        Color::Black,
        &[
            (msq(6, 5), Color::Black, PieceKind::FerociousLeopard),
            (msq(2, 8), Color::Black, PieceKind::GoldGeneral),
            (msq(1, 7), Color::White, PieceKind::GoBetween),
        ],
    );
    board.make_move_unchecked(mv(msq(6, 5), None, msq(6, 4), false), rules);
    board.make_move_unchecked(mv(msq(1, 7), None, msq(1, 8), false), rules);
    board.make_move_unchecked(mv(msq(2, 8), None, msq(2, 7), false), rules);
    board.make_move_unchecked(mv(msq(1, 8), None, msq(1, 7), false), rules);

    assert_both_choices(&generated_with(&generator, &board), msq(6, 4), msq(6, 3));
}

#[test]
fn article_30_p2_capture_entry_also_creates_waiting() {
    // 第30条P2: 敵陣入りが捕獲を伴っても、不成を選べば直後の自手番は待機中となる。
    let rules = promotion_rules(PromotionRule::P2);
    let generator = MoveGenerator::new(rules);
    let mut board = position(
        Color::Black,
        &[
            (msq(6, 5), Color::Black, PieceKind::FerociousLeopard),
            (msq(6, 4), Color::White, PieceKind::Pawn),
            (msq(1, 7), Color::White, PieceKind::GoBetween),
        ],
    );
    assert_both_choices(&generated_with(&generator, &board), msq(6, 5), msq(6, 4));
    board.make_move_unchecked(mv(msq(6, 5), None, msq(6, 4), false), rules);
    board.make_move_unchecked(mv(msq(1, 7), None, msq(1, 8), false), rules);

    assert_no_promotion(&generated_with(&generator, &board), msq(6, 4), msq(6, 3));
}

// D1-030-03: P3（香車の最奥段救済）。
#[test]
fn article_30_p3_lance_gains_last_rank_relief() {
    let generator = MoveGenerator::new(modifier_rules(true, false, false, false));
    let board = position(Color::Black, &[(msq(6, 3), Color::Black, PieceKind::Lance)]);
    let moves = generated_with(&generator, &board);
    // 非捕獲の最奥段到達に成り（白駒）と不成の2手が含まれる（D1-019-03 との差分）。
    assert_both_choices(&moves, msq(6, 3), msq(6, 1));
    // 最奥段以外への敵陣内非捕獲移動は P3 の下でも成れない。
    assert_no_promotion(&moves, msq(6, 3), msq(6, 2));
}

// D1-030-04: P4（仲人の最奥段救済）。
#[test]
fn article_30_p4_go_between_gains_last_rank_relief() {
    let board = position(
        Color::Black,
        &[(msq(6, 2), Color::Black, PieceKind::GoBetween)],
    );
    // 標準規則では最奥段到達は不成の1手だけ。
    assert_no_promotion(&generated(&board), msq(6, 2), msq(6, 1));

    // P4では成り（醉象）と不成の2手。
    let generator = MoveGenerator::new(modifier_rules(false, true, false, false));
    assert_both_choices(&generated_with(&generator, &board), msq(6, 2), msq(6, 1));
    // 仲人は前後に動けるため、最奥段で不成でも移動不能にならない
    // （第19条2項・3項は歩兵・香車限定）。
    let unpromoted = position(
        Color::Black,
        &[(msq(6, 1), Color::Black, PieceKind::GoBetween)],
    );
    let moves = moves_from(&generated_with(&generator, &unpromoted), msq(6, 1));
    assert!(moves.contains(&mv(msq(6, 1), None, msq(6, 2), false)));
}

#[test]
fn article_30_p5_deferred_pawn_cannot_promote_on_a_later_capture() {
    // 第30条P5: 敵陣入りで不成を選んだ歩兵は、以後の敵陣内捕獲でも成れない。
    let rules = modifier_rules(false, false, true, false);
    let generator = MoveGenerator::new(rules);
    let mut board = position(
        Color::Black,
        &[
            (msq(6, 5), Color::Black, PieceKind::Pawn),
            (msq(6, 3), Color::White, PieceKind::Pawn),
            (msq(1, 7), Color::White, PieceKind::GoBetween),
        ],
    );
    board.make_move_unchecked(mv(msq(6, 5), None, msq(6, 4), false), rules);
    board.make_move_unchecked(mv(msq(1, 7), None, msq(1, 8), false), rules);

    assert_no_promotion(&generated_with(&generator, &board), msq(6, 4), msq(6, 3));
}

#[test]
fn article_30_p5_deferred_pawn_cannot_promote_on_a_quiet_last_rank_move() {
    // 第30条P5: 標準規則の下では非捕獲着手で成ることができないため、保留歩兵が
    // 非捕獲で最奥段へ到達しても成れない。
    let rules = modifier_rules(false, false, true, false);
    let generator = MoveGenerator::new(rules);
    let mut board = position(
        Color::Black,
        &[
            (msq(6, 5), Color::Black, PieceKind::Pawn),
            (msq(1, 7), Color::White, PieceKind::GoBetween),
        ],
    );
    board.make_move_unchecked(mv(msq(6, 5), None, msq(6, 4), false), rules);
    board.make_move_unchecked(mv(msq(1, 7), None, msq(1, 8), false), rules);
    board.make_move_unchecked(mv(msq(6, 4), None, msq(6, 3), false), rules);
    board.make_move_unchecked(mv(msq(1, 8), None, msq(1, 7), false), rules);
    board.make_move_unchecked(mv(msq(6, 3), None, msq(6, 2), false), rules);
    board.make_move_unchecked(mv(msq(1, 7), None, msq(1, 8), false), rules);

    assert_no_promotion(&generated_with(&generator, &board), msq(6, 2), msq(6, 1));
}

#[test]
fn article_30_p2_p5_deferred_pawn_must_promote_on_a_quiet_last_rank_move() {
    // 第30条P2・P5: P2の下では非捕獲着手でも成れるため、保留歩兵が非捕獲で
    // 最奥段へ到達するときは成りを強制する。
    let rules = MoveRules {
        promotion: PromotionRule::P2,
        p5: true,
        ..MoveRules::standard()
    };
    let generator = MoveGenerator::new(rules);
    let mut board = position(
        Color::Black,
        &[
            (msq(6, 5), Color::Black, PieceKind::Pawn),
            (msq(1, 7), Color::White, PieceKind::GoBetween),
        ],
    );
    board.make_move_unchecked(mv(msq(6, 5), None, msq(6, 4), false), rules);
    board.make_move_unchecked(mv(msq(1, 7), None, msq(1, 8), false), rules);
    board.make_move_unchecked(mv(msq(6, 4), None, msq(6, 3), false), rules);
    board.make_move_unchecked(mv(msq(1, 8), None, msq(1, 7), false), rules);
    board.make_move_unchecked(mv(msq(6, 3), None, msq(6, 2), false), rules);
    board.make_move_unchecked(mv(msq(1, 7), None, msq(1, 8), false), rules);

    assert_promotion_forced(&generated_with(&generator, &board), msq(6, 2), msq(6, 1));
}

#[test]
fn article_30_p5_deferred_pawn_must_promote_on_a_capturing_last_rank_move() {
    // 第30条P5: 保留歩兵が捕獲を伴って最奥段へ到達するときも成りを強制する。
    let rules = modifier_rules(false, false, true, false);
    let generator = MoveGenerator::new(rules);
    let mut board = position(
        Color::Black,
        &[
            (msq(6, 5), Color::Black, PieceKind::Pawn),
            (msq(6, 1), Color::White, PieceKind::Lance),
            (msq(1, 7), Color::White, PieceKind::GoBetween),
        ],
    );
    board.make_move_unchecked(mv(msq(6, 5), None, msq(6, 4), false), rules);
    board.make_move_unchecked(mv(msq(1, 7), None, msq(1, 8), false), rules);
    board.make_move_unchecked(mv(msq(6, 4), None, msq(6, 3), false), rules);
    board.make_move_unchecked(mv(msq(1, 8), None, msq(1, 7), false), rules);
    board.make_move_unchecked(mv(msq(6, 3), None, msq(6, 2), false), rules);
    board.make_move_unchecked(mv(msq(1, 7), None, msq(1, 8), false), rules);

    assert_promotion_forced(&generated_with(&generator, &board), msq(6, 2), msq(6, 1));
}

#[test]
fn article_30_p5_unmarked_pawn_has_no_quiet_last_rank_relief() {
    // 第30条P5: 保留状態でない歩兵には第19条1項の最奥段救済を適用しない。
    let rules = modifier_rules(false, false, true, false);
    let generator = MoveGenerator::new(rules);
    let board = position(
        Color::Black,
        &[
            (msq(6, 2), Color::Black, PieceKind::Pawn),
            (msq(1, 7), Color::White, PieceKind::GoBetween),
        ],
    );
    assert_no_promotion(&generated_with(&generator, &board), msq(6, 2), msq(6, 1));
}

#[test]
fn article_30_p2_p5_deferred_pawn_stays_unpromotable_after_waiting_expires() {
    // 第30条P2・P5: P2の待機が別駒の着手で満了しても、P5の歩兵保留は恒久に残る。
    let rules = MoveRules {
        promotion: PromotionRule::P2,
        p5: true,
        ..MoveRules::standard()
    };
    let generator = MoveGenerator::new(rules);
    let mut board = position(
        Color::Black,
        &[
            (msq(6, 5), Color::Black, PieceKind::Pawn),
            (msq(2, 8), Color::Black, PieceKind::GoldGeneral),
            (msq(1, 7), Color::White, PieceKind::GoBetween),
        ],
    );
    board.make_move_unchecked(mv(msq(6, 5), None, msq(6, 4), false), rules);
    board.make_move_unchecked(mv(msq(1, 7), None, msq(1, 8), false), rules);
    board.make_move_unchecked(mv(msq(2, 8), None, msq(2, 7), false), rules);
    board.make_move_unchecked(mv(msq(1, 8), None, msq(1, 7), false), rules);

    assert_no_promotion(&generated_with(&generator, &board), msq(6, 4), msq(6, 3));
}

#[test]
fn article_30_p6_forces_a_lance_only_on_the_last_rank() {
    // 第30条P6: 香車の最奥段到達は強制成りだが、それ以外の入陣は選択制である。
    let rules = modifier_rules(false, false, false, true);
    let generator = MoveGenerator::new(rules);
    let board = position(Color::Black, &[(msq(6, 5), Color::Black, PieceKind::Lance)]);
    let moves = generated_with(&generator, &board);

    assert_promotion_forced(&moves, msq(6, 5), msq(6, 1));
    assert_both_choices(&moves, msq(6, 5), msq(6, 4));
}

#[test]
fn article_30_p6_forces_standard_pawn_relief() {
    // 第30条P6・第19条1項: 歩兵の最奥段への非捕獲到達は成りを強制する。
    let rules = modifier_rules(false, false, false, true);
    let generator = MoveGenerator::new(rules);
    let board = position(Color::Black, &[(msq(6, 2), Color::Black, PieceKind::Pawn)]);

    assert_promotion_forced(&generated_with(&generator, &board), msq(6, 2), msq(6, 1));
}

#[test]
fn article_30_p2_p6_forces_an_in_zone_lance_on_the_last_rank() {
    // 第30条P2・P6: P2で得た敵陣内非捕獲の成り機会にもP6の強制成りを適用する。
    let rules = MoveRules {
        promotion: PromotionRule::P2,
        p6: true,
        ..MoveRules::standard()
    };
    let generator = MoveGenerator::new(rules);
    let board = position(Color::Black, &[(msq(6, 3), Color::Black, PieceKind::Lance)]);

    assert_promotion_forced(&generated_with(&generator, &board), msq(6, 3), msq(6, 1));
}

#[test]
fn article_30_p6_does_not_force_other_piece_kinds() {
    // 第30条P6: 最奥段であっても、歩兵・香車以外の駒種の成りは選択制のままとする。
    let rules = modifier_rules(false, false, false, true);
    let generator = MoveGenerator::new(rules);
    let board = position(
        Color::Black,
        &[
            (msq(6, 2), Color::Black, PieceKind::SilverGeneral),
            (msq(5, 1), Color::White, PieceKind::Pawn),
        ],
    );

    assert_both_choices(&generated_with(&generator, &board), msq(6, 2), msq(5, 1));
}
