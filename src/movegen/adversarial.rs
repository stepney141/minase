//! 敵対的レビュー用の条文対応テスト。
//! RULES.md 第10〜16条と PLAN.md 第2版4〜6節・10節の確定解釈を検証する。

use super::MoveGenerator;
use crate::mv::Move;
use crate::piece::{Color, PieceCode, PieceKind};
use crate::position::{Position, PositionBuilder};
use crate::square::Square;

fn sq(file: u8, rank: u8) -> Square {
    Square::new(file, rank).unwrap()
}

fn position(side_to_move: Color, pieces: &[(Square, Color, PieceKind)]) -> Position {
    let mut builder = PositionBuilder::new(side_to_move);
    for &(square, color, kind) in pieces {
        builder.put(square, PieceCode::new(color, kind)).unwrap();
    }
    builder.finish().unwrap()
}

fn generated(position: &Position) -> Vec<Move> {
    let mut moves = Vec::new();
    MoveGenerator::standard().generate_moves(position, &mut moves);
    moves
}

/// じっと = Double で to == from かつ mid が空升。
fn jitto_moves(position: &Position, moves: &[Move], origin: Square) -> Vec<Move> {
    moves
        .iter()
        .copied()
        .filter(|&mv| {
            matches!(mv, Move::Double { from, mid, to, .. }
                if from == origin && to == origin && position.piece_at(mid).is_none())
        })
        .collect()
}

// ---------------------------------------------------------------------------
// ケース1: 第11条10項 じっとの成立条件（獅子）
// ---------------------------------------------------------------------------

#[test]
fn case1_article_11_10_lion_surrounded_by_own_and_edge_has_no_jitto() {
    // 隅の獅子。隣接3升をすべて味方歩で塞ぐ（残り5方向は盤端）。
    let position = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::Lion),
            (sq(0, 1), Color::Black, PieceKind::Pawn),
            (sq(1, 0), Color::Black, PieceKind::Pawn),
            (sq(1, 1), Color::Black, PieceKind::Pawn),
        ],
    );
    let moves = generated(&position);
    let lion_moves: Vec<_> = moves
        .iter()
        .copied()
        .filter(|mv| mv.origin() == sq(0, 0))
        .collect();

    // じっとが1手も生成されない。
    assert!(jitto_moves(&position, &moves, sq(0, 0)).is_empty());
    // 第1段階が成立しないので Double 自体が存在しない。
    assert!(
        lion_moves
            .iter()
            .all(|mv| !matches!(mv, Move::Double { .. }))
    );
    // 跳び（第11条5〜7項）は塞がれていても生成される（サニティ）。
    assert!(lion_moves.iter().any(|mv| matches!(mv, Move::Jump { .. })));
}

#[test]
fn case1_article_11_10_lion_surrounded_by_mixed_pieces_has_igui_but_no_jitto() {
    // 隣接升が味方駒・敵駒・盤端で全て塞がれた獅子。居喰いは可、じっとは不可。
    let position = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::Lion),
            (sq(0, 1), Color::Black, PieceKind::Pawn),
            (sq(1, 0), Color::White, PieceKind::Pawn),
            (sq(1, 1), Color::White, PieceKind::SilverGeneral),
        ],
    );
    let moves = generated(&position);

    assert!(jitto_moves(&position, &moves, sq(0, 0)).is_empty());
    // 居喰い（mid が敵駒で to == from）は生成される。
    assert!(moves.contains(&Move::Double {
        from: sq(0, 0),
        mid: sq(1, 0),
        to: sq(0, 0),
        promote: false,
    }));
}

#[test]
fn case1_article_11_9_jitto_is_generated_when_a_neighbor_is_empty() {
    // 対照: 隣接升 (1,1) が空なら、じっとが生成される。
    let position = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::Lion),
            (sq(0, 1), Color::Black, PieceKind::Pawn),
            (sq(1, 0), Color::Black, PieceKind::Pawn),
        ],
    );
    let moves = generated(&position);

    assert!(moves.contains(&Move::Double {
        from: sq(0, 0),
        mid: sq(1, 1),
        to: sq(0, 0),
        promote: false,
    }));
}

// ---------------------------------------------------------------------------
// ケース2: 第10条7〜8項 角鷹・飛鷲のじっと成立条件
// ---------------------------------------------------------------------------

#[test]
fn case2_article_10_7_falcon_blocked_forward_cannot_jitto() {
    // 角鷹の前方隣接升が味方駒: じっとを含む Double が1手もない。
    let position = position(
        Color::Black,
        &[
            (sq(5, 5), Color::Black, PieceKind::HornedFalcon),
            (sq(5, 6), Color::Black, PieceKind::Pawn),
        ],
    );
    let moves = generated(&position);
    let falcon_moves: Vec<_> = moves
        .iter()
        .copied()
        .filter(|mv| mv.origin() == sq(5, 5))
        .collect();

    assert!(jitto_moves(&position, &moves, sq(5, 5)).is_empty());
    assert!(
        falcon_moves
            .iter()
            .all(|mv| !matches!(mv, Move::Double { .. }))
    );
    // 直接跳び（第10条1項d）は味方駒を跳び越して生成される（サニティ）。
    assert!(falcon_moves.contains(&Move::Jump {
        from: sq(5, 5),
        to: sq(5, 7),
        promote: false,
    }));
}

#[test]
fn case2_article_10_7_falcon_with_empty_forward_can_jitto() {
    let position = position(
        Color::Black,
        &[(sq(5, 5), Color::Black, PieceKind::HornedFalcon)],
    );
    let moves = generated(&position);

    assert!(moves.contains(&Move::Double {
        from: sq(5, 5),
        mid: sq(5, 6),
        to: sq(5, 5),
        promote: false,
    }));
}

#[test]
fn case2_article_10_8_eagle_cannot_jitto_only_when_both_diagonals_are_blocked() {
    // (a) 両前斜めが味方駒: じっと不可。
    let both_blocked = position(
        Color::Black,
        &[
            (sq(5, 5), Color::Black, PieceKind::SoaringEagle),
            (sq(4, 6), Color::Black, PieceKind::Pawn),
            (sq(6, 6), Color::Black, PieceKind::Pawn),
        ],
    );
    let moves = generated(&both_blocked);
    assert!(jitto_moves(&both_blocked, &moves, sq(5, 5)).is_empty());

    // (b) 片側だけ塞がれた場合: 空いている側からのみじっとを生成する。
    let one_blocked = position(
        Color::Black,
        &[
            (sq(5, 5), Color::Black, PieceKind::SoaringEagle),
            (sq(6, 6), Color::Black, PieceKind::Pawn),
        ],
    );
    let moves = generated(&one_blocked);
    let jitto = jitto_moves(&one_blocked, &moves, sq(5, 5));
    assert_eq!(
        jitto,
        vec![Move::Double {
            from: sq(5, 5),
            mid: sq(4, 6),
            to: sq(5, 5),
            promote: false,
        }]
    );

    // (c) 片側味方・片側敵駒: じっとは不可だが、敵駒側の居喰いは生成される。
    let enemy_side = position(
        Color::Black,
        &[
            (sq(5, 5), Color::Black, PieceKind::SoaringEagle),
            (sq(6, 6), Color::Black, PieceKind::Pawn),
            (sq(4, 6), Color::White, PieceKind::Pawn),
        ],
    );
    let moves = generated(&enemy_side);
    assert!(jitto_moves(&enemy_side, &moves, sq(5, 5)).is_empty());
    assert!(moves.contains(&Move::Double {
        from: sq(5, 5),
        mid: sq(4, 6),
        to: sq(5, 5),
        promote: false,
    }));
}

// ---------------------------------------------------------------------------
// ケース3: 第12条・第13条・第14条 王駒が唯一の足である場合
// ---------------------------------------------------------------------------

#[test]
fn case3_articles_12_13_king_as_only_foot_blocks_distance_two_lion_capture() {
    // 白獅子(4,2)の唯一の足は白王将(5,2)。黒飛車(4,0)により、王で取り返すと
    // 王が当たりに入るが、深さ1打ち切りの確定解釈では足として数える。
    let with_king = position(
        Color::Black,
        &[
            (sq(2, 2), Color::Black, PieceKind::Lion),
            (sq(4, 2), Color::White, PieceKind::Lion),
            (sq(5, 2), Color::White, PieceKind::King),
            (sq(4, 0), Color::Black, PieceKind::Rook),
        ],
    );
    let moves = generated(&with_king);
    // 黒獅子からの着手で白獅子(4,2)を取るものは、経路を問わず1手もない。
    assert!(
        moves
            .iter()
            .filter(|mv| mv.origin() == sq(2, 2))
            .all(|&mv| {
                !with_king
                    .captured_squares(mv)
                    .into_iter()
                    .flatten()
                    .any(|square| square == sq(4, 2))
            })
    );

    // 対照: 王将を外すと足が消え、跳びによる捕獲が生成される。
    let without_king = position(
        Color::Black,
        &[
            (sq(2, 2), Color::Black, PieceKind::Lion),
            (sq(4, 2), Color::White, PieceKind::Lion),
            (sq(4, 0), Color::Black, PieceKind::Rook),
        ],
    );
    assert!(generated(&without_king).contains(&Move::Jump {
        from: sq(2, 2),
        to: sq(4, 2),
        promote: false,
    }));
}

#[test]
fn case3_article_14_king_as_only_foot_enforces_senjishi() {
    // 黒角(0,0)が白獅子(1,1)を取る。黒獅子(4,5)の唯一の足は黒王将(3,5)。
    // 白金(3,6)が(4,5)に利いており、王の取り返しは自殺的だが、足と数える。
    let mut with_king = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::Bishop),
            (sq(1, 1), Color::White, PieceKind::Lion),
            (sq(4, 5), Color::Black, PieceKind::Lion),
            (sq(3, 5), Color::Black, PieceKind::King),
            (sq(3, 6), Color::White, PieceKind::GoldGeneral),
            (sq(4, 7), Color::White, PieceKind::Rook),
        ],
    );
    with_king.make_move_unchecked(Move::Step {
        from: sq(0, 0),
        to: sq(1, 1),
        promote: false,
    });
    assert!(with_king.lion_taken_by_non_lion().is_some());
    let moves = generated(&with_king);
    let recapture = Move::Step {
        from: sq(4, 7),
        to: sq(4, 5),
        promote: false,
    };
    assert!(!moves.contains(&recapture));
    // サニティ: 先獅子は他の白の着手を妨げない。
    assert!(moves.contains(&Move::Step {
        from: sq(4, 7),
        to: sq(4, 6),
        promote: false,
    }));

    // 対照: 王将を外すと足がなく、直後の捕獲が許される（第14条2項）。
    let mut without_king = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::Bishop),
            (sq(1, 1), Color::White, PieceKind::Lion),
            (sq(4, 5), Color::Black, PieceKind::Lion),
            (sq(3, 6), Color::White, PieceKind::GoldGeneral),
            (sq(4, 7), Color::White, PieceKind::Rook),
        ],
    );
    without_king.make_move_unchecked(Move::Step {
        from: sq(0, 0),
        to: sq(1, 1),
        promote: false,
    });
    assert!(generated(&without_king).contains(&recapture));
}

// ---------------------------------------------------------------------------
// ケース4: 第14条4項 先獅子の解除タイミング
// ---------------------------------------------------------------------------

#[test]
fn case4_article_14_4_senjishi_applies_for_one_move_and_then_expires() {
    let mut position = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::Bishop),
            (sq(1, 1), Color::White, PieceKind::Lion),
            (sq(4, 4), Color::Black, PieceKind::Lion),
            (sq(4, 3), Color::Black, PieceKind::GoldGeneral),
            (sq(0, 5), Color::Black, PieceKind::Pawn),
            (sq(4, 6), Color::White, PieceKind::Rook),
            (sq(10, 10), Color::White, PieceKind::Pawn),
        ],
    );
    let recapture = Move::Step {
        from: sq(4, 6),
        to: sq(4, 4),
        promote: false,
    };

    // 1手目: 黒角が白獅子を取る。トリガーが立つ。
    position.make_move_unchecked(Move::Step {
        from: sq(0, 0),
        to: sq(1, 1),
        promote: false,
    });
    assert_eq!(position.lion_taken_by_non_lion(), Some(sq(1, 1)));
    // 直後の白の1手では、足のある黒獅子(4,4)を取れない。
    assert!(!generated(&position).contains(&recapture));

    // 2手目: 白が別の手を指すとトリガーが消える。
    position.make_move_unchecked(Move::Step {
        from: sq(10, 10),
        to: sq(10, 9),
        promote: false,
    });
    assert_eq!(position.lion_taken_by_non_lion(), None);

    // 3手目: 黒の手（捕獲なし）でもトリガーは立たない。
    position.make_move_unchecked(Move::Step {
        from: sq(0, 5),
        to: sq(0, 6),
        promote: false,
    });
    assert_eq!(position.lion_taken_by_non_lion(), None);

    // 4手目: 同じ捕獲が、足が残っていても合法になっている。
    assert!(generated(&position).contains(&recapture));
}

// ---------------------------------------------------------------------------
// ケース5: 飛鷲の1手2枚獅子取りと先獅子トリガー
// ---------------------------------------------------------------------------

#[test]
fn case5_eagle_double_lion_capture_records_second_square_and_restricts_reply() {
    let mut position = position(
        Color::Black,
        &[
            (sq(4, 4), Color::Black, PieceKind::SoaringEagle),
            (sq(5, 5), Color::White, PieceKind::Lion),
            (sq(6, 6), Color::White, PieceKind::Lion),
            (sq(9, 9), Color::Black, PieceKind::Lion),
            (sq(9, 8), Color::Black, PieceKind::Pawn),
            (sq(0, 9), Color::White, PieceKind::Rook),
        ],
    );
    let double_capture = Move::Double {
        from: sq(4, 4),
        mid: sq(5, 5),
        to: sq(6, 6),
        promote: false,
    };
    // 非獅子は足の有無にかかわらず獅子を取れる（第13条5項）ので生成される。
    assert!(generated(&position).contains(&double_capture));

    position.make_move_unchecked(double_capture);
    // 第2捕獲の升(6,6)がトリガーとして記録される。
    assert_eq!(position.lion_taken_by_non_lion(), Some(sq(6, 6)));

    // 直後の白の1手では、足（黒歩(9,8)）のある黒獅子(9,9)を取れない。
    let recapture = Move::Step {
        from: sq(0, 9),
        to: sq(9, 9),
        promote: false,
    };
    let moves = generated(&position);
    assert!(!moves.contains(&recapture));
    // サニティ: 同じ飛車の他の着手は生成される。
    assert!(moves.contains(&Move::Step {
        from: sq(0, 9),
        to: sq(8, 9),
        promote: false,
    }));
}

#[test]
fn case5_after_double_lion_capture_an_undefended_lion_may_be_taken() {
    // 対照: 黒獅子(9,9)に足がなければ、直後でも取り返せる（第14条2項）。
    let mut position = position(
        Color::Black,
        &[
            (sq(4, 4), Color::Black, PieceKind::SoaringEagle),
            (sq(5, 5), Color::White, PieceKind::Lion),
            (sq(6, 6), Color::White, PieceKind::Lion),
            (sq(9, 9), Color::Black, PieceKind::Lion),
            (sq(0, 9), Color::White, PieceKind::Rook),
        ],
    );
    position.make_move_unchecked(Move::Double {
        from: sq(4, 4),
        mid: sq(5, 5),
        to: sq(6, 6),
        promote: false,
    });
    assert_eq!(position.lion_taken_by_non_lion(), Some(sq(6, 6)));
    assert!(generated(&position).contains(&Move::Step {
        from: sq(0, 9),
        to: sq(9, 9),
        promote: false,
    }));
}

// ---------------------------------------------------------------------------
// ケース6: 第16条6〜7項 最奥段の不成歩兵・香車は死に駒
// ---------------------------------------------------------------------------

#[test]
fn case6_article_16_6_7_unpromoted_pawn_and_lance_on_last_rank_are_dead() {
    let black = position(
        Color::Black,
        &[
            (sq(4, 11), Color::Black, PieceKind::Pawn),
            (sq(0, 11), Color::Black, PieceKind::Lance),
            (sq(6, 6), Color::Black, PieceKind::King),
        ],
    );
    let moves = generated(&black);
    assert!(moves.iter().all(|mv| mv.origin() != sq(4, 11)));
    assert!(moves.iter().all(|mv| mv.origin() != sq(0, 11)));
    assert!(!moves.is_empty());

    let white = position(
        Color::White,
        &[
            (sq(7, 0), Color::White, PieceKind::Pawn),
            (sq(11, 0), Color::White, PieceKind::Lance),
            (sq(6, 6), Color::White, PieceKind::King),
        ],
    );
    let moves = generated(&white);
    assert!(moves.iter().all(|mv| mv.origin() != sq(7, 0)));
    assert!(moves.iter().all(|mv| mv.origin() != sq(11, 0)));
    assert!(!moves.is_empty());
}

// ---------------------------------------------------------------------------
// ケース7: 第16条 成りの境界3ケース
// ---------------------------------------------------------------------------

#[test]
fn case7_article_16_pawn_capturing_into_last_rank_may_promote() {
    let position = position(
        Color::Black,
        &[
            (sq(4, 10), Color::Black, PieceKind::Pawn),
            (sq(4, 11), Color::White, PieceKind::SilverGeneral),
        ],
    );
    let moves = generated(&position);
    assert!(moves.contains(&Move::Step {
        from: sq(4, 10),
        to: sq(4, 11),
        promote: true,
    }));
    // 不成も別手として生成される（第16条12項）。
    assert!(moves.contains(&Move::Step {
        from: sq(4, 10),
        to: sq(4, 11),
        promote: false,
    }));
}

#[test]
fn case7_article_16_4_quiet_move_inside_zone_cannot_promote() {
    let position = position(Color::Black, &[(sq(4, 8), Color::Black, PieceKind::Pawn)]);
    let moves = generated(&position);
    assert!(moves.contains(&Move::Step {
        from: sq(4, 8),
        to: sq(4, 9),
        promote: false,
    }));
    assert!(!moves.contains(&Move::Step {
        from: sq(4, 8),
        to: sq(4, 9),
        promote: true,
    }));
}

#[test]
fn case7_article_16_quiet_move_leaving_zone_cannot_promote() {
    let position = position(Color::Black, &[(sq(4, 8), Color::Black, PieceKind::Rook)]);
    let moves = generated(&position);
    // 敵陣(8段目)から敵陣外(7段目)への非捕獲移動。
    assert!(moves.contains(&Move::Step {
        from: sq(4, 8),
        to: sq(4, 7),
        promote: false,
    }));
    assert!(!moves.contains(&Move::Step {
        from: sq(4, 8),
        to: sq(4, 7),
        promote: true,
    }));
    // 敵陣内の非捕獲移動も成れない（第16条4項）。
    assert!(!moves.contains(&Move::Step {
        from: sq(4, 8),
        to: sq(4, 9),
        promote: true,
    }));
}

// ---------------------------------------------------------------------------
// ケース8: 第13条1項 隣接獅子の居喰い捕獲は足があっても合法
// ---------------------------------------------------------------------------

#[test]
fn case8_article_13_1_igui_capture_of_adjacent_defended_lion_is_legal() {
    // 白獅子(3,2)は白飛車(3,5)に守られている（足あり）が、隣接なので取れる。
    let position = position(
        Color::Black,
        &[
            (sq(2, 2), Color::Black, PieceKind::Lion),
            (sq(3, 2), Color::White, PieceKind::Lion),
            (sq(3, 5), Color::White, PieceKind::Rook),
        ],
    );
    let moves = generated(&position);

    // 居喰い（mid の獅子を取って元の升へ戻る）。
    assert!(moves.contains(&Move::Double {
        from: sq(2, 2),
        mid: sq(3, 2),
        to: sq(2, 2),
        promote: false,
    }));
    // 通常の1段階捕獲も無条件で合法。
    assert!(moves.contains(&Move::Step {
        from: sq(2, 2),
        to: sq(3, 2),
        promote: false,
    }));
    // 隣接捕獲後にさらに進む2段階移動も、捕獲時点の隣接性で合法。
    assert!(moves.contains(&Move::Double {
        from: sq(2, 2),
        mid: sq(3, 2),
        to: sq(4, 2),
        promote: false,
    }));
}

// ---------------------------------------------------------------------------
// ケース9: 麒麟の獅子成り捕獲と先獅子トリガー（第14条6項）
// ---------------------------------------------------------------------------

#[test]
fn case9_article_14_6_kirin_promoting_on_lion_capture_triggers_senjishi() {
    let mut position = position(
        Color::Black,
        &[
            (sq(5, 7), Color::Black, PieceKind::Kirin),
            (sq(5, 9), Color::White, PieceKind::Lion),
            (sq(5, 8), Color::Black, PieceKind::GoldGeneral),
            (sq(5, 11), Color::White, PieceKind::Rook),
        ],
    );
    let promote_capture = Move::Step {
        from: sq(5, 7),
        to: sq(5, 9),
        promote: true,
    };
    let moves = generated(&position);
    // 成り捕獲と不成捕獲の双方が生成される（第16条1項・12項）。
    assert!(moves.contains(&promote_capture));
    assert!(moves.contains(&Move::Step {
        from: sq(5, 7),
        to: sq(5, 9),
        promote: false,
    }));

    position.make_move_unchecked(promote_capture);
    // 麒麟（非獅子）による捕獲なので先獅子トリガーが立つ（第14条6項）。
    assert_eq!(position.lion_taken_by_non_lion(), Some(sq(5, 9)));
    assert_eq!(
        position.piece_at(sq(5, 9)),
        PieceCode::new_promoted(Color::Black, PieceKind::Lion),
    );

    // 成獅子(5,9)には足（黒金(5,8)）があるため、白は直後に取れない。
    let recapture = Move::Step {
        from: sq(5, 11),
        to: sq(5, 9),
        promote: false,
    };
    let white_moves = generated(&position);
    assert!(!white_moves.contains(&recapture));
    assert!(white_moves.contains(&Move::Step {
        from: sq(5, 11),
        to: sq(5, 10),
        promote: false,
    }));
}

#[test]
fn case9_kirin_promotion_capture_of_undefended_new_lion_allows_reply() {
    // 対照: 成獅子に足がなければ直後に取れる（第14条2項）。
    let mut position = position(
        Color::Black,
        &[
            (sq(5, 7), Color::Black, PieceKind::Kirin),
            (sq(5, 9), Color::White, PieceKind::Lion),
            (sq(5, 11), Color::White, PieceKind::Rook),
        ],
    );
    position.make_move_unchecked(Move::Step {
        from: sq(5, 7),
        to: sq(5, 9),
        promote: true,
    });
    assert_eq!(position.lion_taken_by_non_lion(), Some(sq(5, 9)));
    assert!(generated(&position).contains(&Move::Step {
        from: sq(5, 11),
        to: sq(5, 9),
        promote: false,
    }));
}

// ---------------------------------------------------------------------------
// 追加検証A: 第12条4項×第15条8項 歩復元が走り駒の足を遮蔽してはならない
// ---------------------------------------------------------------------------

#[test]
fn extra_articles_12_4_13_2_pawn_restore_must_not_shield_a_sliding_foot() {
    // 黒獅子(3,5)、白歩(4,5)、白獅子(5,5)、白飛車(0,5)。
    // 白歩は(4,4)にしか利かず、白獅子(5,5)の足ではない。
    // 2段階移動で歩を取ると(3,5)と(4,5)が空き、白飛車の利きが(5,5)に通る。
    // 第12条4項の仮想盤面では飛車が足になるため、第13条2項によりこの
    // 2段階捕獲は不成立のはず（付け喰いは歩が介在するため不成立、第15条3項）。
    let position = position(
        Color::Black,
        &[
            (sq(3, 5), Color::Black, PieceKind::Lion),
            (sq(4, 5), Color::White, PieceKind::Pawn),
            (sq(5, 5), Color::White, PieceKind::Lion),
            (sq(0, 5), Color::White, PieceKind::Rook),
        ],
    );
    let moves = generated(&position);

    // 跳び越し捕獲では歩が盤上に残り飛車の利きを遮るため、足がなく合法（準拠確認）。
    assert!(moves.contains(&Move::Jump {
        from: sq(3, 5),
        to: sq(5, 5),
        promote: false,
    }));

    // 歩を取りながらの2段階捕獲は、捕獲後の仮想盤面で飛車の足が通るので
    // 不成立のはず。実装が歩を無条件に復元すると、この足が遮蔽されて
    // 不正な捕獲手が生成される。
    assert!(
        !moves.contains(&Move::Double {
            from: sq(3, 5),
            mid: sq(4, 5),
            to: sq(5, 5),
            promote: false,
        }),
        "第12条4項: 歩兵は白獅子の足ではないので第15条8項の復元対象外。\
         2段階移動後の仮想盤面では白飛車(0,5)の利きが(5,5)へ通り足が生じるため、\
         第13条2項によりこの捕獲は生成されてはならない",
    );
}

// ---------------------------------------------------------------------------
// 追加検証B: 第13条1項と第14条1項の競合（解釈問題の挙動固定）
// ---------------------------------------------------------------------------

#[test]
fn note_articles_13_1_vs_14_1_adjacent_lion_recapture_during_senjishi() {
    // 先獅子中、制限側の獅子が隣接する相手獅子（足あり）を取れるか。
    // RULES.md 第14条1項の文理では取れない（捕獲側の駒種を限定しない）。
    // 第13条1項（隣接は無条件）との優先関係は条文に明記がない。
    let mut position = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::Bishop),
            (sq(1, 1), Color::White, PieceKind::Lion),
            (sq(4, 5), Color::Black, PieceKind::Lion),
            (sq(4, 4), Color::Black, PieceKind::GoldGeneral),
            (sq(5, 6), Color::White, PieceKind::Lion),
        ],
    );
    position.make_move_unchecked(Move::Step {
        from: sq(0, 0),
        to: sq(1, 1),
        promote: false,
    });
    let adjacent_recapture = Move::Step {
        from: sq(5, 6),
        to: sq(4, 5),
        promote: false,
    };
    // 現実装は第14条1項を優先し、獅子による隣接取り返しも禁止する。
    assert!(!generated(&position).contains(&adjacent_recapture));

    // 対照: 先獅子トリガーがなければ隣接捕獲は無条件（第13条1項）。
    let no_trigger = super::super::position::PositionBuilder::new(Color::White);
    let mut builder = no_trigger;
    for (square, color, kind) in [
        (sq(4, 5), Color::Black, PieceKind::Lion),
        (sq(4, 4), Color::Black, PieceKind::GoldGeneral),
        (sq(5, 6), Color::White, PieceKind::Lion),
    ] {
        builder.put(square, PieceCode::new(color, kind)).unwrap();
    }
    let fresh = builder.finish().unwrap();
    assert!(generated(&fresh).contains(&adjacent_recapture));
}
