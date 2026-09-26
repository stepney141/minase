//! 着手の順序付けを検査する。

use super::*;

/// 段階的手選択が助言手の合法性と重複を処理し、全合法手を1回ずつ返す。
#[test]
fn staged_picker_yields_every_legal_move_exactly_once() {
    let position = staged_picker_fixture();
    let pst = weights().unwrap();
    let legal = legal_moves(&position);
    let capture = legal
        .iter()
        .copied()
        .find(|&mv| move_order_key(&position, &pst, mv).is_some())
        .unwrap();
    let quiets: Vec<_> = legal
        .iter()
        .copied()
        .filter(|&mv| move_order_key(&position, &pst, mv).is_none())
        .collect();
    let illegal = Move {
        from: fs(1, 1),
        mid: None,
        to: fs(1, 2),
        promote: false,
    };
    let history = Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);

    for (tt_move, killers) in [
        (Some(capture), [None, None]),
        (Some(quiets[0]), [Some(quiets[0]), None]),
        (Some(illegal), [Some(illegal), Some(quiets[1])]),
    ] {
        let mut picker = MovePicker::new(tt_move, killers);
        let mut actual = Vec::new();
        while let Some((mv, _)) = picker.next(
            &position,
            &pst,
            &MoveGenerator::new(engine_rules()),
            &history,
        ) {
            actual.push(mv);
        }

        assert_eq!(actual.len(), legal.len());
        assert_eq!(
            actual
                .iter()
                .copied()
                .collect::<std::collections::HashSet<_>>(),
            legal.iter().copied().collect()
        );
    }
}

/// 段階的手選択がTT手、捕獲手、killer手、その他の静かな手の順を守る。
#[test]
fn staged_picker_respects_advisory_precedence() {
    let position = staged_picker_fixture();
    let pst = weights().unwrap();
    let legal = legal_moves(&position);
    let capture = legal
        .iter()
        .copied()
        .find(|&mv| move_order_key(&position, &pst, mv).is_some())
        .unwrap();
    let quiets: Vec<_> = legal
        .iter()
        .copied()
        .filter(|&mv| move_order_key(&position, &pst, mv).is_none())
        .collect();
    let killer = quiets[0];
    let history = Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
    let mut picker = MovePicker::new(Some(quiets[1]), [Some(killer), None]);
    let mut actual = Vec::new();
    while let Some((mv, _)) = picker.next(
        &position,
        &pst,
        &MoveGenerator::new(engine_rules()),
        &history,
    ) {
        actual.push(mv);
    }

    assert_eq!(actual[0], quiets[1]);
    let capture_index = actual.iter().position(|&mv| mv == capture).unwrap();
    let killer_index = actual.iter().position(|&mv| mv == killer).unwrap();
    assert!(capture_index < killer_index);
    assert!(
        actual[killer_index + 1..]
            .iter()
            .all(|&mv| move_order_key(&position, &pst, mv).is_none())
    );
}

/// 段階5「手の分類」: 捕獲・非捕獲のTT手が正しく分類されて先頭に出る。
#[test]
fn staged_picker_classifies_capture_and_quiet_tt_moves() {
    let position = staged_picker_fixture();
    let pst = weights().unwrap();
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
        let picked = picker.next(&position, &pst, &generator, &history).unwrap();
        assert_eq!(picked, (tt_move, expected_capture));
    }
}

/// 段階5「手の分類」: 捕獲専用生成の全手が捕獲手として返り、整列キーと一致する。
#[test]
fn staged_picker_classifies_all_generated_captures() {
    let pst = weights().unwrap();
    let generator = MoveGenerator::new(engine_rules());
    let history = Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
    let lion_position = position(
        Color::Black,
        &[
            (fs(7, 12), Color::Black, PieceKind::King),
            (fs(6, 6), Color::Black, PieceKind::Lion),
            (fs(6, 5), Color::White, PieceKind::Pawn),
            (fs(5, 5), Color::White, PieceKind::Pawn),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    // benchのgame-3-ply-465。後手番で成駒を含む局面。
    let bench_position = crate::parse_sfen(
        "3+s1ok5/5ett4/12/3ps1x+bpm1+r/3i1g1cip+p1/4+b+pP5/4n2N4/7E4/12/12/4S+VD5/5KS1FC2 w",
    )
    .unwrap();

    for position in [staged_picker_fixture(), lion_position, bench_position] {
        let mut captures = Vec::new();
        generator.generate_captures(&position, &mut captures);
        assert!(!captures.is_empty());
        let mut picker = MovePicker::new(None, [None, None]);
        let mut picked_captures = Vec::new();
        while let Some((mv, capture)) = picker.next(&position, &pst, &generator, &history) {
            assert_eq!(capture, captures.contains(&mv));
            assert_eq!(capture, move_order_key(&position, &pst, mv).is_some());
            if capture {
                picked_captures.push(mv);
            }
        }
        assert_eq!(picked_captures.len(), captures.len());
        assert_eq!(
            picked_captures
                .into_iter()
                .collect::<std::collections::HashSet<_>>(),
            captures.into_iter().collect()
        );
    }
}

// movegen-speedup-2.md「段階6」: 変更前の全出力列を契約の参照にする。
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
        pst: &Pst,
        generator: &MoveGenerator,
        history: &HistoryTable,
    ) -> Option<(Move, bool)> {
        loop {
            match self.stage {
                MovePickerStage::Tt => {
                    self.stage = MovePickerStage::Captures;
                    if let Some(tt_move) = self.tt_move
                        && {
                            let mut moves = Vec::new();
                            generator.generate_moves(position, &mut moves);
                            moves.contains(&tt_move)
                        }
                    {
                        return Some((tt_move, move_order_key(position, pst, tt_move).is_some()));
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
                                    let key = move_order_key(position, pst, mv)
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
                        Reverse(history[color][mv.from.dense_index()][mv.to.dense_index()])
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

/// 「段階6」（movegen-speedup-2.md）の全出力列を同点・重複・履歴更新込みで照合する。
#[test]
fn staged_picker_matches_reference_sequence_with_changing_history() {
    let pst = weights().unwrap();
    let mut picker = MovePicker::new(None, [None; KILLER_COUNT]);
    for rules in crate::core::movegen::tests::capture_test_rules() {
        let generator = MoveGenerator::new(rules);
        let positions = std::iter::once(staged_picker_fixture())
            .chain(crate::test_util::sampled_random_positions(rules));
        for position in positions {
            let mut all = Vec::new();
            generator.generate_moves(&position, &mut all);
            if all.is_empty() {
                continue;
            }
            let quiets: Vec<_> = all
                .iter()
                .copied()
                .filter(|&mv| move_order_key(&position, &pst, mv).is_none())
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
                picker.reset(tt, killers);
                let mut ordinal = 0;
                loop {
                    let expected = reference.next(&position, &pst, &generator, &history);
                    let actual = picker.next(&position, &pst, &generator, &history);
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
