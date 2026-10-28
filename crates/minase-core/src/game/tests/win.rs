//! 第21条の王駒による勝敗の試験。

use super::*;

#[test]
fn article_21_2_mate_ends_the_game_at_move_completion() {
    // 3項aからdまでのいずれの合法手もない局面を作る着手の完了時に、
    // 詰みを理由として終局する。
    let (position, mv) = mate_predecessor();
    let mut game = game(position);

    assert_eq!(game.play(mv), win(Color::White, WinReason::Mate));
}

#[test]
fn article_21_3_a_any_escape_prevents_mate() {
    // 捕獲を回避する着手が1つでもあれば詰みは成立しない。
    // mate_predecessorから(0,11)の飛車を除くと(0,1)への逃げが残る。
    let mut game = game(position(
        Color::White,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(11, 0), piece(Color::White, PieceKind::Rook)),
            (sq(11, 11), piece(Color::White, PieceKind::Bishop)),
            (sq(10, 8), piece(Color::White, PieceKind::King)),
        ],
    ));

    assert_eq!(
        game.play(step(sq(10, 8), sq(10, 9))),
        Ok(GameStatus::Ongoing)
    );
}

#[test]
fn article_21_3_b_and_21_4_checked_side_may_capture_the_last_royal_first() {
    // 王手放置は合法(第8条)であり、相手の最後の王駒を先に取る
    // 着手が3項bの回避手段になるため詰みではない。
    // 王手を受けている側が先に取れば、その側の勝ちとなる。
    // (1,0)の飛車は白王将の升(1,1)も守るが、最後の王駒の先取りで勝ちは確定する。
    for (rook_from, checking_square) in [(sq(5, 5), sq(5, 0)), (sq(2, 0), sq(1, 0))] {
        let mut game = game(position(
            Color::White,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(9, 9), piece(Color::Black, PieceKind::Pawn)),
                (sq(1, 1), piece(Color::White, PieceKind::King)),
                (rook_from, piece(Color::White, PieceKind::Rook)),
            ],
        ));

        // 飛車が段0から王手をかける。黒歩の手は捕獲を回避しないため、
        // 黒の受けは白王将の捕獲だけである。
        assert_eq!(
            game.play(step(rook_from, checking_square)),
            Ok(GameStatus::Ongoing)
        );
        assert_eq!(
            game.play(step(sq(0, 0), sq(1, 1))),
            win(Color::Black, WinReason::RoyalCapture)
        );
    }
}

/// 白飛の照準往復(全手が攻撃的着手)で反復履歴を
/// 蓄積すると、黒の非攻撃的な候補手(3,10)→(3,9)だけが4回目の同一局面を
/// 生じさせ、R1裁定(片側継続攻撃側=白の負け)を直ちに成立させる。この
/// 候補手は3項cの回避手段に数えられるため詰みと判定されず、実際に指すと
/// R1裁定で黒の勝ちになる。黒の攻撃連続数(1〜10手目のうち黒の5手は全て
/// 攻撃的)は候補手の評価で0へ戻る必要があり、実着手と仮想着手の遷移の
/// 一致を[`updated_attacking_counters`](crate::game::repetition::r1::updated_attacking_counters)の契約に従って検査する。
#[test]
fn article_21_3_c_and_31_r1_repetition_win_rescues_mate() {
    let start = position(
        Color::White,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(1, 8), piece(Color::Black, PieceKind::Rook)),
            (sq(3, 9), piece(Color::Black, PieceKind::Rook)),
            (sq(2, 9), piece(Color::Black, PieceKind::Pawn)),
            (sq(0, 5), piece(Color::White, PieceKind::Rook)),
            (sq(2, 1), piece(Color::White, PieceKind::GoldGeneral)),
            (sq(1, 11), piece(Color::White, PieceKind::Pawn)),
            (sq(7, 10), piece(Color::White, PieceKind::Pawn)),
            (sq(11, 0), piece(Color::White, PieceKind::King)),
        ],
    );
    let w_out = step(sq(0, 5), sq(2, 5));
    let w_back = step(sq(2, 5), sq(0, 5));
    let z_out = step(sq(1, 8), sq(1, 2));
    let z_back = step(sq(1, 2), sq(1, 8));
    let y_out = step(sq(3, 9), sq(3, 10));
    let candidate = step(sq(3, 10), sq(3, 9));

    let mut game = game_with_codes(start, &[RuleCode::R1]);
    let plies = [
        w_out, z_out, w_back, z_back, w_out, z_out, w_back, z_back, w_out, y_out, w_back,
    ];
    for (index, mv) in plies.into_iter().enumerate() {
        // 11手目(白の王手)の完了時も、反復による即時勝利が受けに残るため
        // 詰みにならない。
        assert_eq!(game.play(mv), Ok(GameStatus::Ongoing), "ply {}", index + 1);
    }
    assert_eq!(
        game.play(candidate),
        win(Color::Black, WinReason::Repetition)
    );

    // 対照: 同じ最終盤面へ履歴なしで到達すると、同じ白の王手が詰みになる。
    let fresh = position(
        Color::White,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(1, 8), piece(Color::Black, PieceKind::Rook)),
            (sq(3, 10), piece(Color::Black, PieceKind::Rook)),
            (sq(2, 9), piece(Color::Black, PieceKind::Pawn)),
            (sq(2, 5), piece(Color::White, PieceKind::Rook)),
            (sq(2, 1), piece(Color::White, PieceKind::GoldGeneral)),
            (sq(1, 11), piece(Color::White, PieceKind::Pawn)),
            (sq(7, 10), piece(Color::White, PieceKind::Pawn)),
            (sq(11, 0), piece(Color::White, PieceKind::King)),
        ],
    );
    let mut fresh_game = game_with_codes(fresh, &[RuleCode::R1]);
    assert_eq!(fresh_game.play(w_back), win(Color::White, WinReason::Mate));
}

/// 王駒の捕獲を回避できなくても、4回目の同一局面で直ちに引き分けとなる手は
/// 詰みの逃れとなる（第21条第3項d、第31条R1）。E1でも同じ着手で引き分ける。
#[test]
fn article_21_3_d_r1_repetition_draw_rescues_mate() {
    let start = position(
        Color::White,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(1, 8), piece(Color::Black, PieceKind::Rook)),
            (sq(3, 9), piece(Color::Black, PieceKind::Rook)),
            (sq(2, 9), piece(Color::Black, PieceKind::Pawn)),
            (sq(0, 5), piece(Color::White, PieceKind::Rook)),
            (sq(2, 1), piece(Color::White, PieceKind::GoldGeneral)),
            (sq(1, 11), piece(Color::White, PieceKind::Pawn)),
            (sq(7, 9), piece(Color::White, PieceKind::Pawn)),
            (sq(7, 10), piece(Color::White, PieceKind::Pawn)),
            (sq(11, 0), piece(Color::White, PieceKind::King)),
        ],
    );
    let w_out = step(sq(0, 5), sq(2, 5));
    let w_back = step(sq(2, 5), sq(0, 5));
    let z_out = step(sq(1, 8), sq(1, 2));
    let z_back = step(sq(1, 2), sq(1, 8));
    let y_out = step(sq(3, 9), sq(3, 10));
    let candidate = step(sq(3, 10), sq(3, 9));

    for e1 in [false, true] {
        let mut rules = Rules::ENGINE_DEFAULT;
        rules.e1 = e1;
        let mut game = Game::from_position(rules, start.clone());
        // 初期局面・4手目・8手目と同じ局面へ12手目で戻る。
        // 双方の全着手が攻撃的なので、R1は一方の負けではなく引き分けとする。
        for mv in [
            w_out, z_out, w_back, z_back, w_out, z_out, w_back, z_back, w_out, y_out,
        ] {
            assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
        }

        // 履歴がなければ同じ王手で詰む。E1は詰みによる終局を行わない。
        let mut fresh = Game::from_position(rules, game.position().clone());
        assert_eq!(
            fresh.play(w_back),
            if e1 {
                Ok(GameStatus::Ongoing)
            } else {
                win(Color::White, WinReason::Mate)
            }
        );
        assert_eq!(game.play(w_back), Ok(GameStatus::Ongoing));
        assert_eq!(game.play(candidate), draw(DrawReason::Repetition));
    }
}

#[test]
fn article_21_5_lion_double_capture_of_both_royals_wins() {
    // 獅子が1手の2段階移動で王将(玉将)と太子の両方を取ると、
    // その着手の終了時に勝利する(第12条4項)。
    let pieces = [
        (sq(0, 0), piece(Color::Black, PieceKind::King)),
        (sq(5, 5), piece(Color::Black, PieceKind::Lion)),
        (sq(5, 6), piece(Color::White, PieceKind::King)),
        (sq(6, 6), prince(Color::White)),
    ];
    let mut full_capture = game(position(Color::Black, &pieces));
    let double_capture = Move {
        from: sq(5, 5),
        mid: Some(sq(5, 6)),
        to: sq(6, 6),
        promote: false,
    };
    assert_eq!(
        full_capture.play(double_capture),
        win(Color::Black, WinReason::RoyalCapture)
    );

    // 境界: 第1段階の玉将捕獲だけで停止すると太子が残り、終局しない。
    let mut partial = game(position(Color::Black, &pieces));
    assert_eq!(
        partial.play(step(sq(5, 5), sq(5, 6))),
        Ok(GameStatus::Ongoing)
    );
}

#[test]
fn article_21_6_resignation_ends_the_game() {
    // 投了した側が敗者となる。盤面や履歴は裁定に関与しない。
    let mut game = Game::with_default_rules();
    assert_eq!(
        game.resign(Color::Black),
        win(Color::White, WinReason::Resignation)
    );
    assert_eq!(game.position(), &Position::initial());
    assert_eq!(game.ply_count(), 0);
}

/// 詰み判定の1手目(手番側の受け)にはR2禁止フィルタ後の
/// 対局合法手を用いる。唯一の非捕獲の受けが既出局面の再現となる局面は、
/// R1では継続だがR2では詰みになる（RULES.md第31条R2、[`stalemate_or_mate`](crate::game::adjudication::mate::stalemate_or_mate)の契約）。
#[test]
fn articles_21_3_and_31_r2_mate_judgement_uses_the_filtered_move_set() {
    let pieces = [
        (sq(0, 1), piece(Color::Black, PieceKind::King)),
        (sq(11, 0), piece(Color::White, PieceKind::Rook)),
        (sq(11, 11), piece(Color::White, PieceKind::Bishop)),
        (sq(10, 9), piece(Color::White, PieceKind::King)),
    ];
    let sequence = [
        step(sq(10, 9), sq(10, 8)),
        step(sq(0, 1), sq(0, 0)),
        step(sq(10, 8), sq(10, 9)),
    ];

    let mut r1 = game_with_codes(
        position(Color::White, &pieces),
        &[RuleCode::R1, RuleCode::E2],
    );
    for mv in sequence {
        assert_eq!(r1.play(mv), Ok(GameStatus::Ongoing));
    }

    let mut r2 = game_with_codes(
        position(Color::White, &pieces),
        &[RuleCode::R2, RuleCode::E2],
    );
    assert_eq!(r2.play(sequence[0]), Ok(GameStatus::Ongoing));
    assert_eq!(r2.play(sequence[1]), Ok(GameStatus::Ongoing));
    assert_eq!(r2.play(sequence[2]), win(Color::White, WinReason::Mate));
}

/// 第21条第3項c、第22条第2項・第5項。王手した飛車を成って取れば駒枯れ勝ちとなり、詰みを免れる。
#[test]
fn article_21_3_c_promotion_exhaustion_win_rescues_mate() {
    let setup = |pawn_square| {
        position(
            Color::White,
            &[
                (sq(0, 11), piece(Color::Black, PieceKind::King)),
                (pawn_square, piece(Color::Black, PieceKind::Pawn)),
                (sq(2, 10), piece(Color::White, PieceKind::King)),
                (sq(1, 8), piece(Color::White, PieceKind::Rook)),
            ],
        )
    };
    let check = step(sq(1, 8), sq(0, 8));
    // 黒王の移動先はすべて白王または白飛の利きに入る。
    // 歩兵が隣の筋にあれば飛車を取れず、同じ王手で詰む。
    let mut trapped = game(setup(sq(1, 7)));
    assert_eq!(trapped.play(check), win(Color::White, WinReason::Mate));

    let mut rescued = game(setup(sq(0, 7)));
    assert_eq!(rescued.play(check), Ok(GameStatus::Ongoing));
    let promotion = promoting(sq(0, 7), sq(0, 8));
    assert!(rescued.legal_moves().contains(&promotion));
    // 白王は成歩も黒王も取れないので、取り返しの猶予なしで終局する。
    assert_eq!(
        rescued.play(promotion),
        win(Color::Black, WinReason::PieceExhaustion)
    );
}
