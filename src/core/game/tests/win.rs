//! 第21条の王駒による勝敗の試験。

use super::*;

#[test]
fn article_21_2_mate_ends_the_game_at_move_completion() {
    // D3-021-02: 3項a・b・cのいずれの合法手もない局面を作る着手の完了時に、
    // 詰みを理由として終局する。
    let (position, mv) = mate_predecessor();
    let mut game = game(position);

    assert_eq!(game.play(mv), win(Color::White, WinReason::Mate));
}

#[test]
fn article_21_3_a_any_escape_prevents_mate() {
    // D3-021-03: 捕獲を回避する着手が1つでもあれば詰みは成立しない。
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
    // D3-021-04: 王手放置は合法(第8条)であり、相手の最後の王駒を先に取る
    // 着手が3項bの回避手段になるため詰みではない。
    // D3-021-06: 王手を受けている側が先に取れば、その側の勝ちとなる。
    let mut game = game(position(
        Color::White,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(9, 9), piece(Color::Black, PieceKind::Pawn)),
            (sq(1, 1), piece(Color::White, PieceKind::King)),
            (sq(5, 5), piece(Color::White, PieceKind::Rook)),
        ],
    ));

    // 飛車が(5,0)から段0の王手をかける。黒歩の手は捕獲を回避しないため、
    // 黒の受けは白王将の捕獲だけである。
    assert_eq!(game.play(step(sq(5, 5), sq(5, 0))), Ok(GameStatus::Ongoing));
    assert_eq!(
        game.play(step(sq(0, 0), sq(1, 1))),
        win(Color::Black, WinReason::RoyalCapture)
    );
}

#[test]
fn article_21_3_c_and_31_r1_repetition_win_rescues_mate() {
    // D3-021-05/D3-031-05: 白飛の照準往復(全手が攻撃的着手)で反復履歴を
    // 蓄積すると、黒の非攻撃的な候補手(3,10)→(3,9)だけが4回目の同一局面を
    // 生じさせ、R1裁定(片側継続攻撃側=白の負け)を直ちに成立させる。この
    // 候補手は3項cの回避手段に数えられるため詰みと判定されず、実際に指すと
    // R1裁定で黒の勝ちになる。黒の攻撃連続数(1〜10手目のうち黒の5手は全て
    // 攻撃的)は候補手の評価で0へ戻る必要があり、実着手と仮想着手の遷移の
    // 一致(adjudication-refactor.md「R1の攻撃連続数」)を回帰として固定する。
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

#[test]
fn article_21_5_lion_double_capture_of_both_royals_wins() {
    // D3-021-07: 獅子が1手の2段階移動で王将(玉将)と太子の両方を取ると、
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
    // D3-021-08: 投了した側が敗者となる。盤面や履歴は裁定に関与しない。
    let mut game = Game::with_default_rules();
    assert_eq!(
        game.resign(Color::Black),
        win(Color::White, WinReason::Resignation)
    );
    assert_eq!(game.position(), &Position::initial());
    assert_eq!(game.ply_count(), 0);
}

#[test]
fn article_21_7_draw_agreement_ends_the_game() {
    // D3-021-09: 双方の合意で対局は引き分けとして終局する。
    let mut game = Game::with_default_rules();
    assert_eq!(game.agree_draw(), draw(DrawReason::Agreement));
    assert_eq!(game.position(), &Position::initial());
}

#[test]
fn article_21_10_r2_mate_judgement_uses_the_filtered_move_set() {
    // D3-021-10: 詰み判定の1手目(手番側の受け)にはR2禁止フィルタ後の
    // 対局合法手を用いる。唯一の非捕獲の受けが既出局面の再現となる局面は、
    // R1では継続だがR2では詰みになる(game-referee.md 6節)。
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
