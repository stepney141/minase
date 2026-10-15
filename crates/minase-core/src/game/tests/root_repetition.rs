//! 第31条R1による根の分類、ならびに第21条の終局との関係を検査する。

use super::*;
use crate::RepetitionRule;
use crate::game::root_repetition::TEST_PROBES;

fn rook_cycle_position() -> Position {
    position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(3, 3), piece(Color::Black, PieceKind::Rook)),
            (sq(3, 9), piece(Color::White, PieceKind::Pawn)),
            (sq(8, 8), piece(Color::White, PieceKind::GoldGeneral)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    )
}

fn rook_cycle() -> [Move; 4] {
    [
        step(sq(3, 3), sq(3, 4)),
        step(sq(8, 8), sq(8, 7)),
        step(sq(3, 4), sq(3, 3)),
        step(sq(8, 7), sq(8, 8)),
    ]
}

fn after_plies(mut game: Game, cycle: &[Move], plies: usize) -> Game {
    for (index, mv) in cycle.iter().cycle().take(plies).enumerate() {
        assert_eq!(game.play(*mv), Ok(GameStatus::Ongoing), "ply {}", index + 1);
    }
    game
}

fn direct_loss() -> (Game, Move) {
    let cycle = rook_cycle();
    let mut start = rook_cycle_position();
    start.make_move_unchecked(cycle[0], Rules::ENGINE_DEFAULT.moves);
    let rotated = [cycle[1], cycle[2], cycle[3], cycle[0]];
    (after_plies(game(start), &rotated, 11), cycle[0])
}

fn reply_loss() -> (Game, Move, Move) {
    let cycle = rook_cycle();
    (
        after_plies(game(rook_cycle_position()), &cycle, 10),
        cycle[2],
        cycle[3],
    )
}

fn detour_reply_loss() -> (Game, Move, Move) {
    let cycle = rook_cycle();
    // 2周目だけ白金を(7,8)へ動かす。通常の白金(8,7)の局面Dは1回だけ通る。
    let plies = [
        cycle[0],
        cycle[1],
        cycle[2],
        cycle[3],
        cycle[0],
        step(sq(8, 8), sq(7, 8)),
        cycle[2],
        step(sq(7, 8), sq(8, 8)),
        cycle[0],
        cycle[1],
    ];
    (
        after_plies(game(rook_cycle_position()), &plies, plies.len()),
        cycle[2],
        cycle[3],
    )
}

fn direct_win() -> (Game, Move) {
    let cycle = rook_cycle();
    (
        after_plies(game(rook_cycle_position()), &cycle, 11),
        cycle[3],
    )
}

fn drawn_cycles() -> [Game; 2] {
    let mutual = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(3, 3), piece(Color::Black, PieceKind::Rook)),
            (sq(8, 2), piece(Color::Black, PieceKind::Pawn)),
            (sq(3, 9), piece(Color::White, PieceKind::Pawn)),
            (sq(8, 8), piece(Color::White, PieceKind::Rook)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    );
    let quiet = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(3, 3), piece(Color::Black, PieceKind::GoldGeneral)),
            (sq(8, 8), piece(Color::White, PieceKind::GoldGeneral)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    );
    [mutual, quiet].map(|start| after_plies(game(start), &rook_cycle(), 11))
}

fn lion_passes(plies: usize) -> (Game, [Move; 2]) {
    let start = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(3, 3), piece(Color::Black, PieceKind::Lion)),
            (
                sq(5, 3),
                PieceCode::new_promoted(Color::Black, PieceKind::Lion).unwrap(),
            ),
            (sq(3, 5), piece(Color::White, PieceKind::Pawn)),
            (
                sq(8, 6),
                PieceCode::new_promoted(Color::White, PieceKind::Lion).unwrap(),
            ),
            (sq(8, 8), piece(Color::White, PieceKind::Lion)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    );
    // 第3条第13項。空升を経由するじっとの正準形はmid: Noneである。
    // mid: Someでは経由升の捕獲を伴う居喰いとなり、可逆手ではなくなる。
    let cycle = [step(sq(3, 3), sq(3, 3)), step(sq(8, 8), sq(8, 8))];
    (after_plies(game(start), &cycle, plies), cycle)
}

fn royal_capture_or_reply_loss() -> (Game, Move) {
    let start = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(3, 3), piece(Color::Black, PieceKind::Rook)),
            (sq(3, 10), piece(Color::White, PieceKind::King)),
            (sq(8, 8), piece(Color::White, PieceKind::GoldGeneral)),
        ],
    );
    (
        after_plies(game(start), &rook_cycle(), 10),
        step(sq(3, 4), sq(3, 10)),
    )
}

fn royal_capture_or_repetition_win() -> (Game, Move, Move) {
    let start = position(
        Color::Black,
        &[
            (sq(7, 6), piece(Color::Black, PieceKind::King)),
            (sq(3, 3), piece(Color::Black, PieceKind::Rook)),
            (sq(3, 9), piece(Color::White, PieceKind::Pawn)),
            (sq(8, 8), piece(Color::White, PieceKind::GoldGeneral)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    );
    (
        after_plies(game(start), &rook_cycle(), 11),
        step(sq(8, 7), sq(7, 6)),
        rook_cycle()[3],
    )
}

fn before_mate_rescuing_check() -> (Game, Move, Move) {
    // 第21条第3項cの既存の試験と同じ局面。白飛は全手で攻撃し、最後の黒飛の
    // 戻りだけが非攻撃的になるため、反復勝ちで王駒の捕獲を免れる。
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
    let plies = [
        w_out,
        z_out,
        w_back,
        z_back,
        w_out,
        z_out,
        w_back,
        z_back,
        w_out,
        step(sq(3, 9), sq(3, 10)),
    ];
    (
        after_plies(game(start), &plies, plies.len()),
        w_back,
        step(sq(3, 10), sq(3, 9)),
    )
}

// キーによる絞り込みを使わず、公開された合法手と終局結果だけで判定する。
fn exhaustive(game: &Game) -> (Vec<Move>, Vec<Move>) {
    let mut winning = Vec::new();
    let mut losing = Vec::new();
    if game.rules().repetition != RepetitionRule::R1 || game.status() != GameStatus::Ongoing {
        return (winning, losing);
    }
    let mover = game.position().side_to_move();
    for mv in game.legal_moves() {
        let mut child = game.clone();
        match child.play(mv).unwrap() {
            GameStatus::Finished(GameResult::Win {
                winner,
                reason: WinReason::Repetition,
            }) if winner == mover => winning.push(mv),
            GameStatus::Finished(GameResult::Win {
                reason: WinReason::Repetition,
                ..
            }) => losing.push(mv),
            GameStatus::Finished(_) => {}
            GameStatus::Ongoing => {
                let mut has_losing_reply = false;
                for reply in child.legal_moves() {
                    let outcome = child.clone().play(reply).unwrap();
                    has_losing_reply |= outcome
                        == GameStatus::Finished(GameResult::Win {
                            winner: mover.opposite(),
                            reason: WinReason::Repetition,
                        });
                }
                if has_losing_reply {
                    losing.push(mv);
                }
            }
        }
    }
    (winning, losing)
}

fn occurrences_after(game: &Game, mv: Move) -> usize {
    let mut after = game.position().clone();
    after.make_move_unchecked(mv, game.rules().moves);
    let key = after.zobrist() ^ after.rights_zobrist();
    game.search_key_history()
        .iter()
        .filter(|&&old| old == key)
        .count()
        + 1
}

fn assert_unclassified(game: &Game, mv: Move) {
    assert!(game.legal_moves().contains(&mv));
    let classified = game.repetition_root_moves();
    assert!(!classified.winning().contains(&mv));
    assert!(!classified.losing().contains(&mv));
}

#[test]
fn article_31_r1_attacker_completing_fourth_occurrence_is_losing() {
    // 第31条R1。黒飛が白歩への攻撃を続け、自分の着手で4回目を完成させる。
    let (game, mv) = direct_loss();
    assert_eq!(game.position().side_to_move(), Color::Black);
    assert_eq!(occurrences_after(&game, mv), 4);
    assert_eq!(
        game.clone().play(mv),
        win(Color::White, WinReason::Repetition)
    );
    let classified = game.repetition_root_moves();
    assert!(classified.winning().is_empty());
    assert!(classified.losing().contains(&mv));
}

#[test]
fn article_31_r1_opponents_repetition_winning_reply_makes_root_move_losing() {
    // 第31条R1。黒飛の戻りは3回目だが、直後の白金の戻りで黒が負ける。
    let (game, mv, reply) = reply_loss();
    assert_eq!(occurrences_after(&game, mv), 3);
    let mut child = game.clone();
    assert_eq!(child.play(mv), Ok(GameStatus::Ongoing));
    assert_eq!(occurrences_after(&child, reply), 4);
    assert_eq!(child.play(reply), win(Color::White, WinReason::Repetition));
    let classified = game.repetition_root_moves();
    assert!(classified.winning().is_empty());
    assert!(classified.losing().contains(&mv));
}

#[test]
fn article_31_r1_second_occurrence_still_allows_a_losing_reply_after_detour() {
    // 第31条R1。別経路を挟むと根の着手後は2回目にすぎず、応手だけが4回目になる。
    let (game, mv, reply) = detour_reply_loss();
    assert_eq!(occurrences_after(&game, mv), 2);
    let mut child = game.clone();
    assert_eq!(child.play(mv), Ok(GameStatus::Ongoing));
    assert_eq!(occurrences_after(&child, reply), 4);
    assert_eq!(child.play(reply), win(Color::White, WinReason::Repetition));
    assert!(game.repetition_root_moves().losing().contains(&mv));
}

#[test]
fn article_31_r1_nonattacker_completing_fourth_occurrence_is_winning() {
    // 第31条R1。白金の戻りが攻撃側である黒の敗北を確定する。
    let (game, mv) = direct_win();
    assert_eq!(occurrences_after(&game, mv), 4);
    assert_eq!(
        game.clone().play(mv),
        win(Color::White, WinReason::Repetition)
    );
    let classified = game.repetition_root_moves();
    assert_eq!(classified.winning(), &[mv]);
    assert!(classified.losing().is_empty());
}

#[test]
fn article_31_r1_mutual_and_absent_continuous_attacks_are_unclassified_draws() {
    // 第31条R1。双方攻撃と双方非攻撃の引き分けを、一方だけ攻撃する対照と分ける。
    let mv = rook_cycle()[3];
    for game in drawn_cycles() {
        assert_eq!(occurrences_after(&game, mv), 4);
        assert_eq!(game.clone().play(mv), draw(DrawReason::Repetition));
        let classified = game.repetition_root_moves();
        assert!(classified.winning().is_empty());
        assert!(classified.losing().is_empty());
    }
    let (one_attacker, _) = direct_win();
    assert_eq!(one_attacker.repetition_root_moves().winning(), &[mv]);
}

#[test]
fn articles_31_r1_and_3_13_lion_passes_require_twelve_reversible_plies() {
    // 第31条R1、第3条第13項。4回目でも可逆手6手なら続行し、12手目で初めて裁定する。
    let (early, cycle) = lion_passes(5);
    assert_eq!(occurrences_after(&early, cycle[1]), 4);
    assert_eq!(early.clone().play(cycle[1]), Ok(GameStatus::Ongoing));
    let classified = early.repetition_root_moves();
    assert!(classified.winning().is_empty());
    assert!(classified.losing().is_empty());

    let (before_reply, _) = lion_passes(10);
    assert_eq!(before_reply.clone().play(cycle[0]), Ok(GameStatus::Ongoing));
    assert!(
        before_reply
            .repetition_root_moves()
            .losing()
            .contains(&cycle[0])
    );
    let (late, _) = lion_passes(11);
    assert_eq!(occurrences_after(&late, cycle[1]), 7);
    assert_eq!(
        late.clone().play(cycle[1]),
        win(Color::White, WinReason::Repetition)
    );
    // 獅子と麒麟成獅子のどちらのじっとも同じ裁定を成立させる。
    let classified = late.repetition_root_moves();
    assert_eq!(classified.winning().len(), 2);
    assert!(classified.winning().contains(&cycle[1]));
    assert!(classified.winning().contains(&step(sq(8, 6), sq(8, 6))));
}

#[test]
fn article_31_r1_royal_capture_ends_before_any_reply_is_considered() {
    // 第31条R1、第21条第1項。周回を続ければ応手で反復負けするが、王駒捕獲は終局する。
    // 捕獲後の応手が既出局面へ戻ることは、駒数が減るため不可能である。
    let (game, capture) = royal_capture_or_reply_loss();
    let cycle = rook_cycle();
    let mut continued = game.clone();
    assert_eq!(continued.play(cycle[2]), Ok(GameStatus::Ongoing));
    assert_eq!(
        continued.play(cycle[3]),
        win(Color::White, WinReason::Repetition)
    );
    assert!(game.repetition_root_moves().losing().contains(&cycle[2]));
    assert_unclassified(&game, capture);
    let mut finished = game.clone();
    assert_eq!(
        finished.play(capture),
        win(Color::Black, WinReason::RoyalCapture)
    );
    assert!(finished.legal_moves().is_empty());
    TEST_PROBES.set((0, 0));
    let classified = finished.repetition_root_moves();
    assert!(classified.winning().is_empty());
    assert!(classified.losing().is_empty());
    assert_eq!(TEST_PROBES.get(), (0, 0));
}

#[test]
fn article_31_r1_only_repetition_win_is_winning_when_royal_capture_also_exists() {
    // 第31条R1、第21条第1項。勝てる手が2つあっても、反復以外の勝利は一覧へ混ぜない。
    let (game, capture, repetition) = royal_capture_or_repetition_win();
    assert_eq!(
        game.clone().play(capture),
        win(Color::White, WinReason::RoyalCapture)
    );
    assert_eq!(
        game.clone().play(repetition),
        win(Color::White, WinReason::Repetition)
    );
    let classified = game.repetition_root_moves();
    assert_eq!(classified.winning(), &[repetition]);
    assert!(!classified.losing().contains(&capture));
}

#[test]
fn articles_31_r1_and_21_3_c_repetition_winning_move_rescues_mate() {
    // 第31条R1、第21条第3項c。履歴なしなら詰みとなる王手にも反復勝ちで応じられる。
    let (mut game, check, escape) = before_mate_rescuing_check();
    let mut fresh = Game::from_position(game.rules(), game.position().clone());
    assert_eq!(fresh.play(check), win(Color::White, WinReason::Mate));
    assert_eq!(game.play(check), Ok(GameStatus::Ongoing));
    assert_eq!(
        game.clone().play(escape),
        win(Color::Black, WinReason::Repetition)
    );
    assert!(game.repetition_root_moves().winning().contains(&escape));
}

fn classified_states() -> Vec<Game> {
    let (mut rescued, check, _) = before_mate_rescuing_check();
    assert_eq!(rescued.play(check), Ok(GameStatus::Ongoing));
    let mut states = vec![
        direct_loss().0,
        reply_loss().0,
        detour_reply_loss().0,
        direct_win().0,
        lion_passes(5).0,
        lion_passes(10).0,
        lion_passes(11).0,
        royal_capture_or_reply_loss().0,
        royal_capture_or_repetition_win().0,
        rescued,
        game(rook_cycle_position()),
        after_plies(game(rook_cycle_position()), &rook_cycle(), 7),
    ];
    states.extend(drawn_cycles());
    for code in [RuleCode::R2, RuleCode::R3] {
        let plies = if code == RuleCode::R2 { 3 } else { 11 };
        states.push(after_plies(
            game_with_codes(rook_cycle_position(), &[code]),
            &rook_cycle(),
            plies,
        ));
    }
    let (mut finished, mv) = direct_win();
    assert_eq!(finished.play(mv), win(Color::White, WinReason::Repetition));
    states.push(finished);
    let (mut finished, capture) = royal_capture_or_reply_loss();
    assert_eq!(
        finished.play(capture),
        win(Color::Black, WinReason::RoyalCapture)
    );
    states.push(finished);
    states
}

#[test]
fn article_31_r1_key_filter_matches_exhaustive_classification_in_all_states() {
    // 第31条R1。上記の全状態で総当たりと比較し、順序と重複の有無も検査する。
    let mut observed_win = false;
    let mut observed_loss = false;
    for (index, game) in classified_states().into_iter().enumerate() {
        let (winning, losing) = exhaustive(&game);
        observed_win |= !winning.is_empty();
        observed_loss |= !losing.is_empty();
        let classified = game.repetition_root_moves();
        assert_eq!(classified.winning(), winning, "winning, state {index}");
        assert_eq!(classified.losing(), losing, "losing, state {index}");
        for moves in [classified.winning(), classified.losing()] {
            for (offset, mv) in moves.iter().enumerate() {
                assert!(!moves[..offset].contains(mv), "duplicate in state {index}");
            }
        }
    }
    assert!(observed_win && observed_loss);
}

#[test]
fn article_31_r1_classification_preserves_game_and_skips_unrepeated_history() {
    // 第31条R1。観測可能な状態を保存し、3回以上のキーがなければ局面の調査も省く。
    for game in classified_states() {
        let before = (
            game.position().clone(),
            game.search_key_history().to_vec(),
            game.ply_count(),
            game.status(),
            game.legal_moves(),
        );
        game.repetition_root_moves();
        assert_eq!(game.position(), &before.0);
        assert_eq!(game.search_key_history(), before.1);
        assert_eq!(game.ply_count(), before.2);
        assert_eq!(game.status(), before.3);
        assert_eq!(game.legal_moves(), before.4);
    }
    for plies in [0, 7] {
        let game = after_plies(game(rook_cycle_position()), &rook_cycle(), plies);
        TEST_PROBES.set((0, 0));
        let classified = game.repetition_root_moves();
        assert!(classified.winning().is_empty());
        assert!(classified.losing().is_empty());
        assert_eq!(TEST_PROBES.get(), (0, 0));
    }
    let (game, mv) = direct_loss();
    TEST_PROBES.set((0, 0));
    assert!(game.repetition_root_moves().losing().contains(&mv));
    let (keys, plays) = TEST_PROBES.get();
    assert!(keys > 0 && plays > 0);
}

#[test]
fn article_31_r1_classification_is_empty_under_r2_and_r3() {
    // 第31条R1。R2では最初の再現、R3では4回目が禁じられる直前まで同じ手順を進める。
    for code in [RuleCode::R2, RuleCode::R3] {
        let plies = if code == RuleCode::R2 { 3 } else { 11 };
        let game = after_plies(
            game_with_codes(rook_cycle_position(), &[code]),
            &rook_cycle(),
            plies,
        );
        assert!(!game.legal_moves().is_empty());
        assert!(!game.legal_moves().contains(&rook_cycle()[3]));
        TEST_PROBES.set((0, 0));
        let classified = game.repetition_root_moves();
        assert!(classified.winning().is_empty());
        assert!(classified.losing().is_empty());
        assert_eq!(TEST_PROBES.get(), (0, 0));
    }
    let (r1, mv) = direct_win();
    assert_eq!(r1.repetition_root_moves().winning(), &[mv]);
}
