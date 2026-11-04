//! 着手の順序付けを検査する。

use super::*;

/// 捕獲手と非捕獲手のどちらも、置換表の記録手は正しく分類されて先頭に出る。
#[test]
fn staged_picker_classifies_capture_and_quiet_tt_moves() {
    let position = staged_picker_fixture();
    let nnue = test_nnue();
    let capture_history = empty_capture_history();
    let generator = MoveGenerator::new(engine_rules());
    let history = Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
    let capture = Move {
        from: fs(6, 8),
        mid: None,
        to: fs(6, 5),
        promote: false,
    };
    let quiet = Move {
        to: fs(5, 8),
        ..capture
    };
    let other_quiet = Move {
        to: fs(4, 8),
        ..capture
    };

    for (tt_move, expected_capture) in [(capture, true), (quiet, false)] {
        assert!(generator.is_legal_move(&position, tt_move, &mut Vec::new(), &mut Vec::new()));
        let mut picker = MovePicker::new(Some(tt_move), [Some(quiet), Some(other_quiet)]);
        let picked = picker
            .next(
                &position,
                nnue,
                &generator,
                isolated_move_picker_context(&history, &capture_history),
            )
            .unwrap();
        assert_eq!(picked, (tt_move, expected_capture));
    }
}

// 静かな手はhistory値の降順、線形経路の差分の降順、生成順に並べる。
// 採用理由はdocs/adr/order-quiets-by-the-stockfish-history-sum-and-update-histories-in-stockfish-form.md。
struct ReferenceMovePicker {
    stage: MovePickerStage,
    tt_move: Option<Move>,
    killers: [Option<Move>; KILLER_COUNT],
    captures: Vec<(Move, MoveOrderKey)>,
    capture_index: usize,
    captures_generated: bool,
    quiets: Vec<Move>,
    quiet_index: usize,
    quiets_generated: bool,
}

impl ReferenceMovePicker {
    /// 助言手を保持した空の手選択器を作る。
    fn new(tt_move: Option<Move>, killers: [Option<Move>; KILLER_COUNT]) -> Self {
        Self {
            stage: MovePickerStage::Tt,
            tt_move,
            killers,
            captures: Vec::new(),
            capture_index: 0,
            captures_generated: false,
            quiets: Vec::new(),
            quiet_index: 0,
            quiets_generated: false,
        }
    }

    /// 現在の段階で次に探索する合法手と、捕獲手かどうかの組を返す。
    fn next(
        &mut self,
        position: &Position,
        nnue: &Nnue,
        generator: &MoveGenerator,
        history: &HistoryTable,
    ) -> Option<(Move, bool)> {
        loop {
            match self.stage {
                MovePickerStage::EarlyKiller0 | MovePickerStage::EarlyKiller1 => {
                    unreachable!("reference picker does not move killers before captures")
                }
                MovePickerStage::Tt => {
                    self.stage = MovePickerStage::Captures;
                    if let Some(tt_move) = self.tt_move
                        && {
                            let mut moves = Vec::new();
                            generator.generate_moves(position, &mut moves);
                            moves.contains(&tt_move)
                        }
                    {
                        return Some((tt_move, move_order_key(position, nnue, tt_move).is_some()));
                    }
                }
                MovePickerStage::Captures => {
                    if !self.captures_generated {
                        let mut moves = Vec::new();
                        generator.generate_captures(position, &mut moves);
                        self.captures.extend(
                            moves
                                .into_iter()
                                .filter(|&mv| Some(mv) != self.tt_move)
                                .map(|mv| {
                                    let key = move_order_key(position, nnue, mv)
                                        .expect("capture generator must not return a quiet move");
                                    (mv, key)
                                }),
                        );
                        self.captures.sort_by_key(|&(_, key)| {
                            (Reverse(key.captured_value), key.attacker_value)
                        });
                        self.captures_generated = true;
                    }
                    if let Some(&(mv, _)) = self.captures.get(self.capture_index) {
                        self.capture_index += 1;
                        return Some((mv, true));
                    }
                    self.stage = MovePickerStage::Killer0;
                }
                MovePickerStage::Killer0 | MovePickerStage::Killer1 => {
                    if !self.quiets_generated {
                        generator.generate_moves(position, &mut self.quiets);
                        self.quiets.retain(|&mv| {
                            position
                                .captured_squares(mv)
                                .into_iter()
                                .all(|sq| sq.is_none())
                        });
                        self.quiets.retain(|&mv| Some(mv) != self.tt_move);
                        self.quiets_generated = true;
                    }
                    let killer_index = usize::from(matches!(self.stage, MovePickerStage::Killer1));
                    self.stage = if killer_index == 0 {
                        MovePickerStage::Killer1
                    } else {
                        MovePickerStage::Quiets
                    };
                    if let Some(killer) = self.killers[killer_index]
                        && let Some(index) = self.quiets.iter().position(|&mv| mv == killer)
                    {
                        return Some((self.quiets.remove(index), false));
                    }
                }
                MovePickerStage::Quiets => {
                    let color = position.side_to_move().index();
                    self.quiets.sort_by_cached_key(|&mv| {
                        (
                            Reverse(history[color][mv.from.dense_index()][mv.to.dense_index()]),
                            Reverse(nnue.quiet_move_pst_delta(position, mv)),
                        )
                    });
                    self.stage = MovePickerStage::Done;
                }
                MovePickerStage::Done => {
                    if let Some(&mv) = self.quiets.get(self.quiet_index) {
                        self.quiet_index += 1;
                        return Some((mv, false));
                    }
                    return None;
                }
            }
        }
    }
}

/// 静かな手の整列は同点時の生成順を保ち、履歴を更新しても重複した手を返さない。
#[test]
fn staged_picker_matches_reference_sequence_with_changing_history() {
    let nnue = test_nnue();
    let capture_history = empty_capture_history();
    let mut picker = MovePicker::new(None, [None; KILLER_COUNT]);
    for name in [
        "engine-default",
        "lishogi",
        "L1,L3,P0,P5,P6,R2,E1,E2",
        "L0,P1,R1,E0",
        "L1,L2,P2,P3,P4,R2,E0",
    ] {
        let rules = minase_core::rules::Rules::from_codes(
            &minase_core::rules::parse_rule_set(name).unwrap(),
        )
        .unwrap()
        .moves;
        let generator = MoveGenerator::new(rules);
        let positions = std::iter::once(staged_picker_fixture())
            .chain(minase_core::test_util::sampled_random_positions(rules));
        for position in positions {
            let mut all = Vec::new();
            generator.generate_moves(&position, &mut all);
            if all.is_empty() {
                continue;
            }
            let quiets: Vec<_> = all
                .iter()
                .copied()
                .filter(|&mv| move_order_key(&position, nnue, mv).is_none())
                .collect();
            let first = quiets.first().copied();
            let last = quiets.last().copied();
            let illegal = Move {
                from: sq(0, 0),
                to: sq(11, 11),
                mid: Some(sq(4, 4)),
                promote: true,
            };
            for (tt, killers) in [
                (None, [None, None]),
                (first, [first, last]),
                (Some(all[all.len() / 2]), [first, first]),
                (Some(illegal), [Some(illegal), last]),
            ] {
                let mut history =
                    Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
                let mut reference = ReferenceMovePicker::new(tt, killers);
                picker.reset(tt, killers, false);
                let mut ordinal = 0;
                loop {
                    let expected = reference.next(&position, nnue, &generator, &history);
                    let actual = picker.next(
                        &position,
                        nnue,
                        &generator,
                        isolated_move_picker_context(&history, &capture_history),
                    );
                    assert_eq!(actual, expected, "ordinal={ordinal}, rules={rules:?}");
                    let Some((mv, _)) = expected else {
                        break;
                    };
                    // 子探索から戻るたびに履歴が変わる。少数の値を使い同点も保つ。
                    history[position.side_to_move().index()][mv.from.dense_index()]
                        [mv.to.dense_index()] = ordinal % 3 - 1;
                    ordinal += 1;
                }
            }
        }
    }
}

/// 候補2: 奔王による飛車取りは歩兵に取り返され、別の歩兵取りも残る。
pub(super) fn losing_capture_fixture() -> (Position, Move, [Option<Move>; KILLER_COUNT]) {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(5, 3), Color::Black, PieceKind::FreeKing),
            (sq(5, 4), Color::White, PieceKind::Rook),
            (sq(5, 5), Color::White, PieceKind::Pawn),
            (sq(3, 3), Color::Black, PieceKind::Pawn),
            (sq(3, 4), Color::White, PieceKind::Pawn),
        ],
    );
    let capture = Move {
        from: sq(5, 3),
        mid: None,
        to: sq(5, 4),
        promote: false,
    };
    let killer0 = Move {
        from: sq(11, 0),
        to: sq(10, 0),
        ..capture
    };
    let killer1 = Move {
        to: sq(10, 1),
        ..killer0
    };
    (board, capture, [Some(killer0), Some(killer1)])
}

fn reference_sequence(
    board: &Position,
    tt_move: Option<Move>,
    killers: [Option<Move>; KILLER_COUNT],
) -> Vec<(Move, bool)> {
    let nnue = test_nnue();
    let generator = MoveGenerator::new(engine_rules());
    let history = Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
    let mut reference = ReferenceMovePicker::new(tt_move, killers);
    std::iter::from_fn(|| reference.next(board, nnue, &generator, &history)).collect()
}

fn picker_sequence(picker: &mut MovePicker, board: &Position) -> Vec<(Move, bool)> {
    let nnue = test_nnue();
    let generator = MoveGenerator::new(engine_rules());
    let history = Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
    let capture_history = empty_capture_history();
    std::iter::from_fn(|| {
        picker.next(
            board,
            nnue,
            &generator,
            isolated_move_picker_context(&history, &capture_history),
        )
    })
    .collect()
}

/// 合法なkillerだけを前置し、元の捕獲列を保つ。
/// 採用理由はdocs/adr/try-killers-before-a-losing-first-capture.md。
#[test]
fn early_killers_preserve_capture_order_and_return_every_legal_move_once() {
    let (board, capture, killers) = losing_capture_fixture();
    assert!(see_prunes(&board, engine_rules(), test_nnue(), capture, 0));
    let illegal = Move {
        from: sq(10, 10),
        ..capture
    };
    let mut picker = MovePicker::new(None, [None; KILLER_COUNT]);
    for (hints, early) in [
        (killers, vec![killers[0].unwrap(), killers[1].unwrap()]),
        ([killers[0], killers[0]], vec![killers[0].unwrap()]),
        ([Some(illegal), killers[1]], vec![killers[1].unwrap()]),
        ([killers[0], Some(illegal)], vec![killers[0].unwrap()]),
        ([Some(capture), killers[1]], vec![killers[1].unwrap()]),
        ([Some(illegal), None], vec![]),
        ([None, None], vec![]),
    ] {
        let baseline = reference_sequence(&board, None, hints);
        assert_eq!(baseline[0], (capture, true));
        assert!(baseline.iter().filter(|(_, capture)| *capture).count() >= 2);
        let expected: Vec<_> = early
            .iter()
            .map(|&mv| (mv, false))
            .chain(
                baseline
                    .iter()
                    .copied()
                    .filter(|(mv, _)| !early.contains(mv)),
            )
            .collect();
        // 有効・無効を交互にresetし、前ノードの状態が残らないことも確かめる。
        for enabled in [true, false, true] {
            picker.reset(None, hints, enabled);
            let actual = picker_sequence(&mut picker, &board);
            assert_eq!(&actual, if enabled { &expected } else { &baseline });
            let legal = legal_moves(&board);
            assert_eq!(actual.len(), legal.len());
            for mv in legal {
                assert_eq!(actual.iter().filter(|(picked, _)| *picked == mv).count(), 1);
            }
        }
    }
}

/// 非負の先頭捕獲と捕獲手のないノードでは、前置を要求しても順序を変えない。
#[test]
fn early_killers_leave_non_losing_or_absent_captures_in_reference_order() {
    let nnue = test_nnue();
    for board in [
        staged_picker_fixture(),
        position(
            Color::Black,
            &[
                (sq(11, 0), Color::Black, PieceKind::King),
                (sq(0, 11), Color::White, PieceKind::King),
            ],
        ),
    ] {
        let quiets: Vec<_> = legal_moves(&board)
            .into_iter()
            .filter(|&mv| move_order_key(&board, nnue, mv).is_none())
            .collect();
        let killers = [quiets.first().copied(), quiets.last().copied()];
        let expected = reference_sequence(&board, None, killers);
        if expected[0].1 {
            assert!(!see_prunes(&board, engine_rules(), nnue, expected[0].0, 0));
        }
        let mut picker = MovePicker::new(None, killers);
        picker.reset(None, killers, true);
        assert_eq!(picker_sequence(&mut picker, &board), expected);
    }
}

/// 対象外のノードと、TT手に一致するkillerは従来どおりの列で1回ずつ返す。
#[test]
fn early_killers_leave_ineligible_nodes_and_tt_duplicates_in_reference_order() {
    let (board, capture, killers) = losing_capture_fixture();
    for tt_move in [None, Some(capture), killers[0], killers[1]] {
        let mut picker = MovePicker::new(tt_move, killers);
        picker.reset(tt_move, killers, false);
        assert_eq!(
            picker_sequence(&mut picker, &board),
            reference_sequence(&board, tt_move, killers),
        );
    }
}

/// 最後の王駒の捕獲は後回しにしない。
#[test]
fn early_killers_do_not_precede_last_royal_capture() {
    let board = position(
        Color::Black,
        &[
            (sq(11, 0), Color::Black, PieceKind::King),
            (sq(5, 3), Color::Black, PieceKind::Rook),
            (sq(5, 4), Color::White, PieceKind::King),
        ],
    );
    let killer = Move {
        from: sq(11, 0),
        mid: None,
        to: sq(10, 0),
        promote: false,
    };
    let killers = [Some(killer), None];
    let expected = reference_sequence(&board, None, killers);
    assert!(captures_last_royal(&board, expected[0].0));
    let mut picker = MovePicker::new(None, killers);
    picker.reset(None, killers, true);
    assert_eq!(picker_sequence(&mut picker, &board), expected);
}

/// Killer候補が全件生成の非捕獲列に含まれることと、単独検査の同値を確かめる。
fn assert_killer_hint(
    board: &Position,
    rules: minase_core::rules::MoveRules,
    candidate: Move,
    legal: bool,
    quiet: bool,
) {
    let nnue = test_nnue();
    let generator = MoveGenerator::new(rules);
    let history = Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
    let capture_history = empty_capture_history();
    let mut all = Vec::new();
    generator.generate_moves(board, &mut all);
    assert_eq!(all.contains(&candidate), legal, "candidate={candidate:?}");
    if legal {
        assert_eq!(
            board
                .captured_squares(candidate)
                .iter()
                .all(Option::is_none),
            quiet
        );
    }
    for tt in [None, Some(candidate)] {
        let killers = [Some(candidate); KILLER_COUNT];
        let mut reference = ReferenceMovePicker::new(tt, killers);
        let expected: Vec<_> =
            std::iter::from_fn(|| reference.next(board, nnue, &generator, &history)).collect();
        for early in [false, true] {
            let mut expected = expected.clone();
            if early
                && legal
                && quiet
                && tt != Some(candidate)
                && let Some(&(first, true)) = expected.first()
                && !captures_last_royal(board, first)
                && see_prunes(board, rules, nnue, first, 0)
            {
                expected.retain(|&(mv, _)| mv != candidate);
                expected.insert(0, (candidate, false));
            }
            let mut picker = MovePicker::new(tt, killers);
            picker.reset(tt, killers, early);
            let actual: Vec<_> = std::iter::from_fn(|| {
                picker.next(
                    board,
                    nnue,
                    &generator,
                    isolated_move_picker_context(&history, &capture_history),
                )
            })
            .collect();
            assert_eq!(actual, expected, "candidate={candidate:?}, early={early}");
        }
    }
}

/// 第11・12条のじっとと居喰いは、到達升の占有だけでは分類できない。
#[test]
fn article_12_killers_distinguish_jitto_and_mid_capture() {
    let from = sq(5, 5);
    let board = position(
        Color::Black,
        &[
            (from, Color::Black, PieceKind::Lion),
            (sq(5, 6), Color::White, PieceKind::Pawn),
        ],
    );
    let rules = engine_rules();
    assert_killer_hint(&board, rules, step(from, from), true, true);
    for to in [from, sq(6, 6)] {
        assert_killer_hint(
            &board,
            rules,
            Move {
                from,
                mid: Some(sq(5, 6)),
                to,
                promote: false,
            },
            true,
            false,
        );
    }
    let mut pieces = vec![(from, Color::Black, PieceKind::Lion)];
    for file in 4..=6 {
        for rank in 4..=6 {
            if sq(file, rank) != from {
                pieces.push((sq(file, rank), Color::Black, PieceKind::Pawn));
            }
        }
    }
    let blocked = position(Color::Black, &pieces);
    assert_killer_hint(&blocked, rules, step(from, from), false, false);
}

/// 第14〜16条の捕獲制限、付け喰い、先獅子がある候補も非捕獲のkillerへ混ぜない。
#[test]
fn articles_14_15_16_killers_preserve_lion_capture_exceptions() {
    let from = sq(5, 5);
    let to = sq(5, 7);
    let mut pieces = vec![
        (from, Color::Black, PieceKind::Lion),
        (to, Color::White, PieceKind::Lion),
        (sq(6, 7), Color::White, PieceKind::Rook),
    ];
    let defended = position(Color::Black, &pieces);
    assert!(defended.piece_control(from).contains(to));
    assert_killer_hint(&defended, engine_rules(), step(from, to), false, false);
    for (kind, legal) in [(PieceKind::Pawn, false), (PieceKind::GoldGeneral, true)] {
        pieces.push((sq(5, 6), Color::White, kind));
        let board = position(Color::Black, &pieces);
        assert_killer_hint(
            &board,
            engine_rules(),
            Move {
                from,
                mid: Some(sq(5, 6)),
                to,
                promote: false,
            },
            legal,
            false,
        );
        pieces.pop();
    }
    let mut sente = position(
        Color::Black,
        &[
            (from, Color::Black, PieceKind::Rook),
            (to, Color::White, PieceKind::Lion),
            (sq(6, 7), Color::White, PieceKind::Rook),
        ],
    );
    assert!(sente.piece_control(from).contains(to));
    assert_killer_hint(&sente, engine_rules(), step(from, to), true, false);
    sente.set_lion_capture(Some(sq(1, 1))).unwrap();
    assert_killer_hint(&sente, engine_rules(), step(from, to), false, false);
}

/// 第30条P6の強制成りとP1・P2の保留は、到達できる升への手でも合法性を変える。
#[test]
fn article_30_killers_preserve_forced_and_deferred_promotions() {
    use minase_core::rules::{MoveRules, PromotionRule};
    let forced = MoveRules {
        p6: true,
        ..engine_rules()
    };
    let board = position(Color::Black, &[(sq(5, 7), Color::Black, PieceKind::Lance)]);
    let mv = step(sq(5, 7), sq(5, 11));
    assert!(board.piece_control(mv.from).contains(mv.to));
    assert_killer_hint(&board, forced, mv, false, false);
    assert_killer_hint(
        &board,
        forced,
        Move {
            promote: true,
            ..mv
        },
        true,
        true,
    );
    for promotion in [PromotionRule::P1, PromotionRule::P2] {
        let rules = MoveRules {
            promotion,
            ..engine_rules()
        };
        let mut board = position(
            Color::Black,
            &[
                (sq(5, 7), Color::Black, PieceKind::FerociousLeopard),
                (sq(1, 5), Color::White, PieceKind::GoBetween),
            ],
        );
        board.make_move_unchecked(step(sq(5, 7), sq(5, 8)), rules);
        board.make_move_unchecked(step(sq(1, 5), sq(1, 4)), rules);
        let waiting = Move {
            promote: true,
            ..step(sq(5, 8), sq(5, 9))
        };
        assert!(board.piece_control(waiting.from).contains(waiting.to));
        assert_killer_hint(&board, rules, waiting, false, false);
        board.make_move_unchecked(
            Move {
                promote: false,
                ..waiting
            },
            rules,
        );
        board.make_move_unchecked(step(sq(1, 4), sq(1, 5)), rules);
        assert_killer_hint(
            &board,
            rules,
            Move {
                promote: true,
                ..step(sq(5, 9), sq(5, 10))
            },
            true,
            true,
        );
    }
}
