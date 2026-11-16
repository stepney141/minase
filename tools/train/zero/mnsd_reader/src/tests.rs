//! Writerと既存のRust学習器を独立した参照元にして、選別と復号を照合する。

use super::*;
use minase::nn::control::CONTROL_SIZE;
use minase::nn::{INPUT_SIZE, encode};
use minase::training::records::{Entry, GameRecord, TeacherKind, Writer, sorted_legal_moves};
use minase_core::{
    Color, DrawReason, Game, PieceCode, PieceKind, Position, PositionBuilder, Rules, Square,
    WinReason,
};
use minase_trainer::data::Dataset;
use std::{
    fs,
    io::Cursor,
    num::NonZeroUsize,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[test]
fn validation_keeps_played_moves_without_values_in_input_order() {
    let fixture = Fixture::new();
    let header = Header::new(
        "lishogi".into(),
        "a1".repeat(20),
        [0; 32],
        TeacherKind::NoSearch,
        0,
        11,
    )
    .unwrap();
    let mut writer = Writer::new(Cursor::new(Vec::new()), header).unwrap();
    let mut expected_boards = Vec::new();
    let mut expected_moves = Vec::new();
    let mut expected_legal = Vec::new();
    // 対局1は訓練側、4は検証側だが、指した手は両方ともすべて診断に使う。
    for (number, count, start_ply, winner) in [
        (1, 1, 0, Color::Black),
        (4, 2, 39, Color::White),
        (9, 0, 200, Color::Black),
    ] {
        let start = Position::initial();
        let mut game = Game::from_position(Rules::LISHOGI, start.clone());
        let mut entries = Vec::new();
        for _ in 0..count {
            let legal = sorted_legal_moves(&game).unwrap();
            let mv = legal[0];
            expected_boards.extend(records::encode_board(game.position()).unwrap());
            expected_moves.push(mv);
            expected_legal.push(legal);
            entries.push(
                Entry::new(
                    Some(mv),
                    Value::None,
                    Exclusion::None,
                    Policy::PlayedMove,
                    None,
                )
                .unwrap(),
            );
            game.play(mv).unwrap();
        }
        entries.push(Entry::new(None, Value::None, Exclusion::None, Policy::None, None).unwrap());
        writer
            .write_game(
                &GameRecord::new(
                    number,
                    Origin::HumanGame,
                    Outcome::Game(GameResult::Win {
                        winner,
                        reason: WinReason::Resignation,
                    }),
                    start_ply,
                    start,
                    entries,
                )
                .unwrap(),
            )
            .unwrap();
    }
    let path = fixture.0.join("validation.mnsd");
    fs::write(&path, writer.finish().unwrap().into_inner()).unwrap();
    let actual = read_validation(&path).unwrap();
    assert_eq!(actual.info.header.teacher_kind(), TeacherKind::NoSearch);
    assert_eq!(actual.info.header.rules(), Rules::LISHOGI);
    assert_eq!(actual.info.header.game_count(), 3);
    assert_eq!(actual.info.header.entry_count(), 6);
    assert_eq!(actual.info.origin, Some(Origin::HumanGame));
    assert_eq!(actual.info.game_numbers, [1, 4, 9]);
    assert_eq!(
        (actual.info.training_count, actual.info.validation_count),
        (1, 2)
    );
    assert_eq!(actual.training_indices, [0]);
    assert_eq!(actual.games, [1, 4, 4]);
    assert_eq!(actual.plies, [0, 39, 40]);
    assert_eq!(actual.boards, expected_boards);
    assert_eq!(actual.sides, [0, 0, 1]);
    assert_eq!(actual.lions, [255; 3]);
    assert_eq!(actual.values, [0; 3]);
    assert_eq!(actual.visits, [0; 3]);
    assert_eq!(actual.outcomes, [0, 1, 1]);
    assert_eq!(actual.policy_offsets, [0, 1, 2, 3]);
    assert_eq!(actual.policy_probabilities, [65_535; 3]);
    for (index, mv) in expected_moves.iter().enumerate() {
        assert_eq!(actual.policy_from[index], mv.from.dense_index() as u8);
        assert_eq!(actual.policy_mid[index], 255);
        assert_eq!(actual.policy_to[index], mv.to.dense_index() as u8);
        assert_eq!(actual.policy_promote[index], u8::from(mv.promote));
    }
    for (index, moves) in expected_legal.iter().enumerate() {
        let range = actual.legal_offsets[index] as usize..actual.legal_offsets[index + 1] as usize;
        assert_eq!(
            actual.legal_from[range.clone()],
            moves
                .iter()
                .map(|mv| mv.from.dense_index() as u8)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            actual.legal_mid[range.clone()],
            moves
                .iter()
                .map(|mv| mv.mid.map_or(255, |sq| sq.dense_index() as u8))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            actual.legal_to[range.clone()],
            moves
                .iter()
                .map(|mv| mv.to.dense_index() as u8)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            actual.legal_promote[range],
            moves
                .iter()
                .map(|mv| u8::from(mv.promote))
                .collect::<Vec<_>>()
        );
    }

    let training = read_file(&path).unwrap();
    assert_eq!(training.info.game_numbers, [1, 4, 9]);
    assert_eq!(
        (training.info.training_count, training.info.validation_count),
        (0, 0)
    );
    assert!(training.games.is_empty());
    assert!(training.boards.is_empty());
    assert_eq!(training.policy_offsets, [0]);
}

#[test]
fn validation_rejects_search_teachers() {
    let fixture = Fixture::new();
    for kind in [TeacherKind::AlphaBeta, TeacherKind::Mcts] {
        let path = fixture.write(kind, &[]);
        assert!(matches!(
            read_validation(&path),
            Err(Error::NotValidationData { kind: actual }) if actual == kind
        ));
    }
}

#[test]
fn shuffled_start_replays_boards_plies_legal_moves_and_control_planes() {
    use minase_core::rng::{XorShift64, derive_seed};
    let fixture = Fixture::new();
    let start = Position::shuffled_initial(&mut XorShift64::new(derive_seed(20261110, 1)));
    let mut game = Game::from_position(Rules::JCSA, start.clone());
    let mut moves = Vec::new();
    let mut entries = Vec::new();
    for _ in 0..6 {
        let mv = sorted_legal_moves(&game).unwrap()[0];
        entries.push(
            Entry::new(
                Some(mv),
                Value::Mcts(32768),
                Exclusion::None,
                Policy::Candidates(vec![(mv, 65535)]),
                Some(4),
            )
            .unwrap(),
        );
        game.play(mv).unwrap();
        moves.push(mv);
    }
    entries.push(Entry::new(None, Value::None, Exclusion::None, Policy::None, Some(0)).unwrap());
    let record =
        GameRecord::new(1, Origin::RandomSelfplay, draw(), 0, start.clone(), entries).unwrap();
    let path = fixture.write(TeacherKind::Mcts, &[record]);
    let actual = read_file(&path).unwrap();
    assert_eq!(actual.plies.len(), moves.len());
    let planes = control_planes(&actual.boards, &actual.sides).unwrap();
    let mut expected = Game::from_position(Rules::JCSA, start);
    for (index, mv) in moves.into_iter().enumerate() {
        assert_eq!(u32::from(actual.plies[index]), expected.ply_count());
        assert_eq!(
            actual.sides[index],
            expected.position().side_to_move() as u8
        );
        assert_eq!(
            actual.boards[index * 144..(index + 1) * 144],
            records::encode_board(expected.position()).unwrap()
        );
        let mut input = vec![0; INPUT_SIZE];
        encode(expected.position(), &mut input);
        assert_eq!(
            planes[index * CONTROL_SIZE..(index + 1) * CONTROL_SIZE],
            input[95 * 144..]
                .iter()
                .copied()
                .map(f32::from)
                .collect::<Vec<_>>()
        );
        let begin = actual.legal_offsets[index] as usize;
        let end = actual.legal_offsets[index + 1] as usize;
        let legal: Vec<_> = (begin..end)
            .map(|i| Move {
                from: Square::from_dense(actual.legal_from[i] as usize).unwrap(),
                mid: (actual.legal_mid[i] != 255)
                    .then(|| Square::from_dense(actual.legal_mid[i] as usize).unwrap()),
                to: Square::from_dense(actual.legal_to[i] as usize).unwrap(),
                promote: actual.legal_promote[i] != 0,
            })
            .collect();
        assert_eq!(legal, sorted_legal_moves(&expected).unwrap());
        expected.play(mv).unwrap();
    }
}

#[test]
fn control_planes_match_inference_for_every_retained_record() {
    // MNSDの再生で得た局面を推論で符号化し、盤の配列から得た64面と照合する。
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/zero-selfplay-small.mnsd");
    let records = read_file(&path).unwrap();
    let actual = control_planes(&records.boards, &records.sides).unwrap();
    assert_eq!(actual.len(), records.sides.len() * 64 * 144);
    let mut reader = Reader::new(BufReader::new(File::open(&path).unwrap())).unwrap();
    let mut input = vec![0; INPUT_SIZE];
    let mut index = 0;
    while let Some(game) = reader.read_game().unwrap() {
        for (entry, position) in game.record().entries().iter().zip(game.positions()) {
            if entry.exclusion() != Exclusion::None
                || entry.value() == Value::None
                || entry.policy() == &Policy::None
            {
                continue;
            }
            encode(position, &mut input);
            assert_eq!(
                actual[index * CONTROL_SIZE..(index + 1) * CONTROL_SIZE],
                input[95 * 144..]
                    .iter()
                    .copied()
                    .map(f32::from)
                    .collect::<Vec<_>>(),
                "record {index}"
            );
            index += 1;
        }
    }
    assert_eq!(index, 741);
}

#[test]
fn control_planes_decode_promoted_pieces_and_both_sides_from_records() {
    let fixture = Fixture::new();
    let mut games = Vec::new();
    for side in Color::ALL {
        let mut builder = PositionBuilder::new(side);
        for (file, rank, color, kind) in [
            (2, 3, Color::Black, PieceKind::GoldGeneral),
            (5, 5, Color::Black, PieceKind::HornedFalcon),
            (8, 8, Color::White, PieceKind::Lion),
            (10, 8, Color::White, PieceKind::SoaringEagle),
        ] {
            builder
                .put(
                    square(file, rank),
                    PieceCode::new_promoted(color, kind).unwrap(),
                )
                .unwrap();
        }
        let start = builder.finish().unwrap();
        let candidate =
            sorted_legal_moves(&Game::from_position(Rules::JCSA, start.clone())).unwrap()[0];
        games.push(
            GameRecord::new(
                side as u32,
                Origin::RandomSelfplay,
                draw(),
                0,
                start,
                vec![
                    Entry::new(
                        None,
                        Value::AlphaBeta(0),
                        Exclusion::None,
                        Policy::Candidates(vec![(candidate, 65535)]),
                        None,
                    )
                    .unwrap(),
                ],
            )
            .unwrap(),
        );
    }
    let path = fixture.write(TeacherKind::AlphaBeta, &games);
    let records = read_file(&path).unwrap();
    let actual = control_planes(&records.boards, &records.sides).unwrap();
    let mut reader = Reader::new(BufReader::new(File::open(path).unwrap())).unwrap();
    for planes in actual.as_chunks::<CONTROL_SIZE>().0 {
        let game = reader.read_game().unwrap().unwrap();
        let mut input = vec![0; INPUT_SIZE];
        encode(&game.positions()[0], &mut input);
        assert_eq!(
            planes.as_slice(),
            input[95 * 144..]
                .iter()
                .copied()
                .map(f32::from)
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(records.sides, [0, 1]);
    assert!(reader.read_game().unwrap().is_none());
}

#[test]
fn control_planes_reject_invalid_codes_deferred_promotion_shapes_and_sides() {
    // recordsの駒コードの仕様: 未成は1〜21、成駒は30+成った後の駒種、後手は64を足す。
    let promoted = [
        34, 35, 36, 37, 38, 39, 40, 42, 47, 50, 51, 52, 53, 54, 55, 56, 57, 58,
    ];
    let mut board = [0; 144];
    for code in 1..=255 {
        board[37] = code;
        let valid =
            code < 128 && ((1..=21).contains(&(code % 64)) || promoted.contains(&(code % 64)));
        if valid {
            assert!(control_planes(&board, &[0]).is_ok(), "valid code {code}");
        } else if code & 128 != 0 {
            assert!(matches!(
                control_planes(&board, &[0]),
                Err(Error::PromotionDeferred {
                    position: 0,
                    square: 37
                })
            ));
        } else {
            assert!(
                matches!(control_planes(&board, &[0]), Err(Error::Format(records::Error::InvalidPieceCode(actual))) if actual == code)
            );
        }
    }
    for (boards, sides) in [
        (vec![0; 143], vec![0]),
        (vec![0; 145], vec![0]),
        (vec![0; 288], vec![0]),
        (vec![0; 144], vec![]),
    ] {
        assert!(matches!(
            control_planes(&boards, &sides),
            Err(Error::InvalidControlShape)
        ));
    }
    for side in [2, 255] {
        assert!(
            matches!(control_planes(&[0; 144], &[side]), Err(Error::Format(records::Error::InvalidSideToMove(actual))) if actual == side)
        );
    }
    assert!(control_planes(&[], &[]).unwrap().is_empty());
    assert!(
        control_planes(&[0; 144], &[0])
            .unwrap()
            .iter()
            .all(|&value| value == 0.0)
    );
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "minase-mnsd-reader-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn write(&self, kind: TeacherKind, games: &[GameRecord]) -> PathBuf {
        let header =
            Header::new("JCSA".into(), "a1".repeat(20), [37; 32], kind, 1_234, 11).unwrap();
        let mut writer = Writer::new(Cursor::new(Vec::new()), header).unwrap();
        for game in games {
            writer.write_game(game).unwrap();
        }
        let path = self.0.join("games.mnsd");
        fs::write(&path, writer.finish().unwrap().into_inner()).unwrap();
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn draw() -> Outcome {
    Outcome::Game(GameResult::Draw {
        reason: DrawReason::Agreement,
    })
}

fn square(file: u8, rank: u8) -> Square {
    Square::new(file, rank).unwrap()
}

fn alpha_game(number: u32, start_ply: u16, start: Position) -> GameRecord {
    let mut game = Game::from_position(Rules::JCSA, start.clone());
    let specifications = [
        (Value::AlphaBeta(100), Exclusion::None),
        (Value::AlphaBeta(-200), Exclusion::None),
        (Value::AlphaBeta(300), Exclusion::MateBand),
        (Value::None, Exclusion::None),
        (Value::AlphaBeta(400), Exclusion::Tactical),
        (Value::AlphaBeta(500), Exclusion::Repetition),
        (Value::AlphaBeta(i16::MIN), Exclusion::None),
    ];
    let mut entries = Vec::new();
    for (index, (value, exclusion)) in specifications.into_iter().enumerate() {
        let legal = sorted_legal_moves(&game).unwrap();
        let played = (index + 1 < specifications.len()).then_some(legal[index]);
        let policy = match index {
            1 => Policy::PlayedMove,
            6 => Policy::Candidates(vec![(legal[0], 10_000), (legal[1], 55_535)]),
            _ => Policy::None,
        };
        entries.push(Entry::new(played, value, exclusion, policy, None).unwrap());
        if let Some(mv) = played {
            game.play(mv).unwrap();
        }
    }
    GameRecord::new(
        number,
        Origin::RandomSelfplay,
        draw(),
        start_ply,
        start,
        entries,
    )
    .unwrap()
}

#[test]
fn alpha_beta_policy_positions_match_trainer_records_in_original_order() {
    // 訓練・検証集合の区別によらず、価値と方策を持つ項目をαβ学習器の同じ項目と比較する。
    let fixture = Fixture::new();
    let mut black = Position::initial();
    black.set_lion_capture(Some(square(1, 8))).unwrap();
    let mut white_game = Game::new(Rules::JCSA);
    white_game
        .play(sorted_legal_moves(&white_game).unwrap()[0])
        .unwrap();
    let games = [
        alpha_game(1, 13, black),
        alpha_game(4, 30, white_game.position().clone()),
    ];
    let path = fixture.write(TeacherKind::AlphaBeta, &games);
    let actual = read_file(&path).unwrap();
    let trainer = Dataset::open(&[path], None, NonZeroUsize::new(1).unwrap()).unwrap();
    assert_eq!(actual.games, [1, 1, 4, 4]);
    assert_eq!(actual.plies, [14, 19, 31, 36]);
    assert_eq!(actual.values, [-200, -32768, -200, -32768]);
    assert_eq!(actual.policy_offsets, [0, 1, 3, 4, 6]);
    assert_eq!(
        actual.policy_probabilities,
        [65535, 10000, 55535, 65535, 10000, 55535]
    );
    assert_eq!(actual.visits, [0; 4]);
    assert_eq!(trainer.record_count(), 6);
    assert_eq!(trainer.training_indices(), [0, 1, 2]);
    assert_eq!(trainer.validation_indices(), [3, 4, 5]);
    for (index, board) in actual.boards.as_chunks::<144>().0.iter().enumerate() {
        // αβの学習器は価値だけの項目も使う。方策を持つ項目だけを対応させる。
        let sample = trainer.record([1, 2, 4, 5][index]).unwrap();
        assert_eq!(board, sample.board_codes());
        assert_eq!(actual.sides[index], sample.side_to_move() as u8);
        assert_eq!(
            actual.lions[index],
            sample.lion_square().map_or(255, |s| s.dense_index() as u8)
        );
        assert_eq!(actual.values[index], i32::from(sample.score()));
        assert_eq!(actual.games[index], sample.game_number());
        assert_eq!(actual.plies[index], sample.ply());
    }
    assert_eq!(actual.sides, [1, 0, 0, 1]);
    assert_eq!(actual.info.origin, Some(Origin::RandomSelfplay));
    assert_eq!(actual.info.header.rule_set(), "JCSA");
    assert_eq!(actual.info.header.rules(), Rules::JCSA);
    assert_eq!(actual.info.header.generation_commit(), "a1".repeat(20));
    assert_eq!(actual.info.header.network_checksum(), &[37; 32]);
    assert_eq!(actual.info.header.teacher_kind(), TeacherKind::AlphaBeta);
    assert_eq!(actual.info.header.teacher_search_budget(), 1234);
    assert_eq!(actual.info.header.seed(), 11);
    assert_eq!(actual.info.header.game_count(), 2);
    assert_eq!(actual.info.header.entry_count(), 14);
}

#[test]
fn mcts_preserves_resignation_search_and_expands_played_policy() {
    // 生成器の契約: 序盤と合意終局の最終項目には価値がなく、投了時には探索結果がある。
    let fixture = Fixture::new();
    let mut game = Game::new(Rules::JCSA);
    let opening = sorted_legal_moves(&game).unwrap()[0];
    game.play(opening).unwrap();
    let searched = sorted_legal_moves(&game).unwrap()[1];
    let ordinary = GameRecord::new(
        2,
        Origin::RandomSelfplay,
        draw(),
        0,
        Position::initial(),
        vec![
            Entry::new(
                Some(opening),
                Value::None,
                Exclusion::None,
                Policy::None,
                Some(0),
            )
            .unwrap(),
            Entry::new(
                Some(searched),
                Value::Mcts(65535),
                Exclusion::None,
                Policy::PlayedMove,
                Some(31),
            )
            .unwrap(),
            Entry::new(None, Value::None, Exclusion::None, Policy::None, Some(0)).unwrap(),
        ],
    )
    .unwrap();

    let mut builder = PositionBuilder::new(Color::Black);
    for (file, rank, color, kind) in [
        (0, 0, Color::Black, PieceKind::King),
        (11, 11, Color::White, PieceKind::King),
        (5, 5, Color::Black, PieceKind::Lion),
        (5, 6, Color::White, PieceKind::Pawn),
        (6, 6, Color::White, PieceKind::Pawn),
    ] {
        builder
            .put(square(file, rank), PieceCode::new(color, kind).unwrap())
            .unwrap();
    }
    let position = builder.finish().unwrap();
    let game = Game::from_position(Rules::JCSA, position.clone());
    let two_step: Vec<_> = sorted_legal_moves(&game)
        .unwrap()
        .into_iter()
        .filter(|mv| mv.mid.is_some())
        .collect();
    let candidates = vec![(two_step[0], 12345), (two_step[1], 53190)];
    let resignation = GameRecord::new(
        7,
        Origin::RandomSelfplay,
        Outcome::Game(GameResult::Win {
            winner: Color::White,
            reason: WinReason::Resignation,
        }),
        42,
        position,
        vec![
            Entry::new(
                None,
                Value::Mcts(37),
                Exclusion::None,
                Policy::Candidates(candidates.clone()),
                Some(u32::MAX),
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let path = fixture.write(TeacherKind::Mcts, &[ordinary, resignation]);
    let actual = read_file(&path).unwrap();
    assert_eq!(actual.info.header.teacher_kind(), TeacherKind::Mcts);
    assert_eq!(actual.info.game_numbers, [2, 7]);
    assert_eq!(actual.games, [2, 7]);
    assert_eq!(actual.plies, [1, 42]);
    assert_eq!(actual.sides, [1, 0]);
    assert_eq!(actual.values, [65535, 37]);
    assert_eq!(actual.outcomes, [2, 1]);
    assert_eq!(actual.visits, [31, u32::MAX]);
    assert_eq!(actual.policy_offsets, [0, 1, 3]);
    assert!(actual.legal_offsets[1] > 1);
    assert!(actual.legal_offsets[2] - actual.legal_offsets[1] > 2);
    let expected_legal = sorted_legal_moves(&game).unwrap();
    let start = actual.legal_offsets[1] as usize;
    let end = actual.legal_offsets[2] as usize;
    let legal_columns: Vec<_> = (start..end)
        .map(|index| {
            [
                actual.legal_from[index],
                actual.legal_mid[index],
                actual.legal_to[index],
                actual.legal_promote[index],
            ]
        })
        .collect();
    let expected_columns: Vec<_> = expected_legal
        .iter()
        .map(|mv| {
            [
                mv.from.dense_index() as u8,
                mv.mid.map_or(255, |s| s.dense_index() as u8),
                mv.to.dense_index() as u8,
                u8::from(mv.promote),
            ]
        })
        .collect();
    assert_eq!(legal_columns, expected_columns);
    let expected = [(searched, 65535), candidates[0], candidates[1]];
    for (index, (mv, probability)) in expected.into_iter().enumerate() {
        assert_eq!(actual.policy_from[index], mv.from.dense_index() as u8);
        assert_eq!(
            actual.policy_mid[index],
            mv.mid.map_or(255, |s| s.dense_index() as u8)
        );
        assert_eq!(actual.policy_to[index], mv.to.dense_index() as u8);
        assert_eq!(actual.policy_promote[index], u8::from(mv.promote));
        assert_eq!(actual.policy_probabilities[index], probability);
    }
}

#[test]
fn mcts_scan_and_replay_skip_value_only_entries_and_read_mate_in_three_wins() {
    let fixture = Fixture::new();
    let mut games = Vec::new();
    // seed=11では対局1が訓練、対局4が検証。両方に同じ選別規則を適用する。
    for (number, winner) in [(1, Color::Black), (4, Color::White)] {
        let mut game = Game::new(Rules::JCSA);
        let mut entries = Vec::new();
        for index in 0..6 {
            let mv = sorted_legal_moves(&game).unwrap()[0];
            let (value, policy, exclusion, visits) = match index {
                0 => (Value::None, Policy::None, Exclusion::None, 0),
                1 => (Value::Mcts(111), Policy::None, Exclusion::None, 2),
                2 => (
                    Value::Mcts(222),
                    Policy::Candidates(vec![(mv, 65535)]),
                    Exclusion::None,
                    8,
                ),
                3 => (Value::Mcts(333), Policy::PlayedMove, Exclusion::None, 8),
                4 => (Value::Mcts(444), Policy::None, Exclusion::None, 0),
                5 => (
                    Value::Mcts(444),
                    Policy::Candidates(vec![(mv, 65535)]),
                    Exclusion::Repetition,
                    8,
                ),
                _ => unreachable!(),
            };
            entries.push(Entry::new(Some(mv), value, exclusion, policy, Some(visits)).unwrap());
            game.play(mv).unwrap();
        }
        entries
            .push(Entry::new(None, Value::None, Exclusion::None, Policy::None, Some(0)).unwrap());
        games.push(
            GameRecord::new(
                number,
                Origin::RandomSelfplay,
                Outcome::MateInThree { winner },
                0,
                Position::initial(),
                entries,
            )
            .unwrap(),
        );
    }
    let path = fixture.write(TeacherKind::Mcts, &games);
    assert_eq!(fs::read(&path).unwrap()[records::HEADER_LEN + 6], 12);
    let info = scan_file(&path).unwrap();
    let all = read_file(&path).unwrap();
    assert_eq!((info.training_count, info.validation_count), (2, 2));
    assert_eq!((all.info.training_count, all.info.validation_count), (2, 2));
    assert_eq!(all.games, [1, 1, 4, 4]);
    assert_eq!(all.plies, [2, 3, 2, 3]);
    assert_eq!(all.values, [222, 333, 222, 333]);
    assert_eq!(all.visits, [8; 4]);
    assert_eq!(all.outcomes, [0, 0, 1, 1]);
    assert_eq!(all.policy_offsets, [0, 1, 2, 3, 4]);
    assert_eq!(all.policy_probabilities, [65535; 4]);
    let selected = read_selected(&path, &[1], &[], true).unwrap();
    assert_eq!(selected.games, [1, 4, 4]);
    assert_eq!(selected.plies, [3, 2, 3]);
    assert_eq!(selected.outcomes, [0, 1, 1]);
}

fn mcts_game(side: Color, values: &[Option<u16>], full: &[bool], outcome: Outcome) -> GameRecord {
    let mut builder = PositionBuilder::new(side);
    for (file, rank, color) in [(0, 0, Color::Black), (11, 11, Color::White)] {
        builder
            .put(
                square(file, rank),
                PieceCode::new(color, PieceKind::King).unwrap(),
            )
            .unwrap();
    }
    for (file, rank, color) in [(2, 1, Color::Black), (9, 10, Color::White)] {
        builder
            .put(
                square(file, rank),
                PieceCode::new(color, PieceKind::Rook).unwrap(),
            )
            .unwrap();
    }
    let start = builder.finish().unwrap();
    let mut game = Game::from_position(Rules::JCSA, start.clone());
    let mut entries = Vec::new();
    let mut kings = [square(0, 0), square(11, 11)];
    for (index, (&value, &full)) in values.iter().zip(full).enumerate() {
        let side = game.position().side_to_move() as usize;
        let (corner, adjacent) = [
            (square(0, 0), square(1, 0)),
            (square(11, 11), square(10, 11)),
        ][side];
        let mv = Move {
            from: kings[side],
            mid: None,
            to: if kings[side] == corner {
                adjacent
            } else {
                corner
            },
            promote: false,
        };
        let played = (index + 1 < values.len()).then_some(mv);
        entries.push(
            Entry::new(
                played,
                value.map_or(Value::None, Value::Mcts),
                Exclusion::None,
                if full {
                    Policy::Candidates(vec![(mv, 65535)])
                } else {
                    Policy::None
                },
                Some(if value.is_some() { 8 } else { 0 }),
            )
            .unwrap(),
        );
        if let Some(mv) = played {
            game.play(mv).unwrap();
            kings[side] = mv.to;
        }
    }
    GameRecord::new(1, Origin::RandomSelfplay, outcome, 0, start, entries).unwrap()
}

fn unselected_game(number: u32, origin: Origin) -> GameRecord {
    let outcome = if origin == Origin::Synthetic {
        Outcome::NotFromGame
    } else {
        draw()
    };
    GameRecord::new(
        number,
        origin,
        outcome,
        0,
        Position::initial(),
        vec![Entry::new(None, Value::None, Exclusion::None, Policy::None, Some(0)).unwrap()],
    )
    .unwrap()
}

#[test]
fn synthetic_origin_is_rejected_even_without_selected_positions() {
    let fixture = Fixture::new();
    let path = fixture.write(TeacherKind::Mcts, &[unselected_game(9, Origin::Synthetic)]);
    assert!(matches!(
        read_file(&path),
        Err(Error::SyntheticOrigin { game: 9 })
    ));
}

#[test]
fn mixed_origins_are_rejected_even_without_selected_positions() {
    let fixture = Fixture::new();
    let path = fixture.write(
        TeacherKind::Mcts,
        &[
            unselected_game(1, Origin::RandomSelfplay),
            unselected_game(4, Origin::HumanGame),
        ],
    );
    assert!(matches!(
        read_file(&path),
        Err(Error::MixedOrigins {
            game: 4,
            expected: Origin::RandomSelfplay,
            actual: Origin::HumanGame,
        })
    ));
}

#[test]
fn empty_file_and_unselected_games_keep_metadata_and_empty_offsets() {
    let fixture = Fixture::new();
    let path = fixture.write(TeacherKind::Mcts, &[]);
    let empty = read_file(&path).unwrap();
    assert_eq!(empty.info.origin, None);
    assert_eq!(empty.info.header.game_count(), 0);
    assert_eq!(empty.info.header.entry_count(), 0);
    assert!(empty.info.game_numbers.is_empty());
    assert!(empty.boards.is_empty());
    assert_eq!(empty.policy_offsets, [0]);
    assert_eq!(empty.legal_offsets, [0]);
    assert!(empty.legal_from.is_empty());
    let path = fixture.write(
        TeacherKind::Mcts,
        &[unselected_game(99, Origin::HumanOpeningSelfplay)],
    );
    let unselected = read_file(&path).unwrap();
    assert_eq!(unselected.info.origin, Some(Origin::HumanOpeningSelfplay));
    assert_eq!(unselected.info.game_numbers, [99]);
    assert!(unselected.games.is_empty());
    assert_eq!(unselected.policy_offsets, [0]);
    assert_eq!(unselected.legal_offsets, [0]);
    assert!(unselected.legal_from.is_empty());
}

#[test]
fn deferred_promotion_bit_and_maximum_ply_are_preserved() {
    let fixture = Fixture::new();
    let mut builder = PositionBuilder::new(Color::Black);
    builder
        .put(
            square(4, 8),
            PieceCode::new(Color::Black, PieceKind::Pawn).unwrap(),
        )
        .unwrap();
    builder.mark_promotion_deferred(square(4, 8)).unwrap();
    let start = builder.finish().unwrap();
    let candidate =
        sorted_legal_moves(&Game::from_position(Rules::JCSA, start.clone())).unwrap()[0];
    let game = GameRecord::new(
        1,
        Origin::RandomSelfplay,
        draw(),
        u16::MAX,
        start,
        vec![
            Entry::new(
                None,
                Value::AlphaBeta(0),
                Exclusion::None,
                Policy::Candidates(vec![(candidate, 65535)]),
                None,
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let path = fixture.write(TeacherKind::AlphaBeta, &[game]);
    let actual = read_file(&path).unwrap();
    assert_eq!(actual.boards[100], 129);
    assert_eq!(actual.plies, [u16::MAX]);
    assert_eq!(actual.values, [0]);
}

#[test]
fn truncated_and_overflowing_input_return_typed_format_errors() {
    let fixture = Fixture::new();
    let path = fixture.write(
        TeacherKind::AlphaBeta,
        &[alpha_game(1, 0, Position::initial())],
    );
    let original = fs::read(&path).unwrap();
    let mut overflow = original.clone();
    // recordsの形式仕様: 対局ヘッダの8..10が開始手数。7項目では最大手数を超える。
    overflow[records::HEADER_LEN + 8..records::HEADER_LEN + 10]
        .copy_from_slice(&u16::MAX.to_le_bytes());
    fs::write(&path, overflow).unwrap();
    let error = read_file(&path).unwrap_err();
    assert!(matches!(error, Error::Format(records::Error::PlyOverflow)));
    assert!(std::error::Error::source(&error).is_some());
    fs::write(&path, &original[..records::HEADER_LEN - 1]).unwrap();
    assert!(matches!(
        read_file(&path),
        Err(Error::Format(records::Error::Truncated(
            records::Section::Header
        )))
    ));
    fs::write(&path, &original[..original.len() - 1]).unwrap();
    assert!(matches!(
        read_file(&path),
        Err(Error::Format(records::Error::Truncated(_)))
    ));
}

#[test]
fn missing_file_preserves_io_error_cause() {
    let fixture = Fixture::new();
    let error = read_file(&fixture.0.join("missing.mnsd")).unwrap_err();
    assert!(matches!(error, Error::Io(ref source) if source.kind() == io::ErrorKind::NotFound));
    assert!(std::error::Error::source(&error).is_some());
}

#[test]
fn legal_columns_match_generated_moves_in_record_order() {
    // フェーズ2: 固定自己対局の全採用局面で、合法手の内容と順序を照合する。
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/zero-selfplay-small.mnsd");
    let actual = read_file(&path).unwrap();
    let mut reader = Reader::new(BufReader::new(File::open(&path).unwrap())).unwrap();
    let generator = MoveGenerator::new(reader.header().rules().moves);
    let mut index = 0;
    assert_eq!(actual.legal_offsets.len(), actual.games.len() + 1);
    assert_eq!(actual.legal_offsets[0], 0);
    assert_eq!(
        *actual.legal_offsets.last().unwrap() as usize,
        actual.legal_from.len()
    );
    for column in [&actual.legal_mid, &actual.legal_to, &actual.legal_promote] {
        assert_eq!(column.len(), actual.legal_from.len());
    }
    while let Some(game) = reader.read_game().unwrap() {
        for (entry, position) in game.record().entries().iter().zip(game.positions()) {
            if entry.exclusion() != Exclusion::None
                || entry.value() == Value::None
                || entry.policy() == &Policy::None
            {
                continue;
            }
            let mut generated = Vec::new();
            generator.generate_moves(position, &mut generated);
            let start = actual.legal_offsets[index] as usize;
            let end = actual.legal_offsets[index + 1] as usize;
            let decoded: Vec<_> = (start..end)
                .map(|i| Move {
                    from: Square::from_dense(actual.legal_from[i] as usize).unwrap(),
                    mid: (actual.legal_mid[i] != 255)
                        .then(|| Square::from_dense(actual.legal_mid[i] as usize).unwrap()),
                    to: Square::from_dense(actual.legal_to[i] as usize).unwrap(),
                    promote: actual.legal_promote[i] != 0,
                })
                .collect();
            assert_eq!(decoded.len(), generated.len());
            assert!(generated.iter().all(|mv| decoded.contains(mv)));
            // 履歴を持たないGameで、recordsが規定する順序を独立に照合する。
            let without_history = Game::from_position(reader.header().rules(), position.clone());
            assert_eq!(decoded, sorted_legal_moves(&without_history).unwrap());
            index += 1;
        }
    }
    assert_eq!(index, actual.games.len());
}

#[test]
fn scan_counts_flags_without_replay_and_selection_uses_training_ordinals() {
    // 仕様: seed=11の対局1は訓練、対局4は検証。価値と方策を持つ採用項目は1,6。
    let fixture = Fixture::new();
    let path = fixture.write(
        TeacherKind::AlphaBeta,
        &[
            alpha_game(1, 13, Position::initial()),
            alpha_game(4, 30, Position::initial()),
        ],
    );
    let info = scan_file(&path).unwrap();
    assert_eq!(info.game_numbers, [1, 4]);
    assert_eq!((info.training_count, info.validation_count), (2, 2));
    let selected = read_selected(&path, &[1], &[], true).unwrap();
    assert_eq!(selected.games, [1, 4, 4]);
    assert_eq!(selected.plies, [19, 31, 36]);
    assert_eq!(selected.boards.len(), 3 * 144);
    assert_eq!(
        (selected.info.training_count, selected.info.validation_count),
        (2, 2)
    );
    let empty = read_selected(&path, &[], &[], false).unwrap();
    assert!(empty.boards.is_empty());
    assert_eq!(empty.policy_offsets, [0]);
    assert_eq!(empty.legal_offsets, [0]);
    assert_eq!(empty.info.game_numbers, [1, 4]);
    for indices in [&[1, 1][..], &[1, 0], &[2]] {
        assert!(matches!(
            read_selected(&path, indices, &[], false),
            Err(Error::InvalidSelection)
        ));
    }

    // 指した手を欠かしても計数できるが、抽出が空でも再生による検査は省かない。
    let mut bytes = fs::read(&path).unwrap();
    let entry = records::HEADER_LEN + records::GAME_HEADER_LEN;
    bytes[entry..entry + 2].copy_from_slice(&u16::MAX.to_le_bytes());
    fs::write(&path, bytes).unwrap();
    let info = scan_file(&path).unwrap();
    assert_eq!((info.training_count, info.validation_count), (2, 2));
    assert!(matches!(
        read_selected(&path, &[], &[], false),
        Err(Error::Format(records::Error::MissingPlayedMove))
    ));
}

#[test]
fn mcts_scan_and_selected_columns_match_the_generated_fixture() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/zero-selfplay-small.mnsd");
    let all = read_file(&path).unwrap();
    let info = scan_file(&path).unwrap();
    let training: Vec<_> = all
        .games
        .iter()
        .enumerate()
        .filter(|&(_, &game)| !minase_trainer::data::is_validation(info.header.seed(), game))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(info.training_count as usize, training.len());
    assert_eq!(info.validation_count as usize, 741 - training.len());
    assert_eq!(info.game_numbers, all.info.game_numbers);
    let ordinals = [0, 50, training.len() as u64 - 1];
    let selected = read_selected(&path, &ordinals, &[], false).unwrap();
    assert_eq!(selected.games.len(), 3);
    assert_eq!(selected.boards.len(), 3 * 144);
    for (row, ordinal) in ordinals.into_iter().enumerate() {
        let source = training[ordinal as usize];
        assert_eq!(selected.games[row], all.games[source]);
        assert_eq!(selected.plies[row], all.plies[source]);
        assert_eq!(selected.values[row], all.values[source]);
        assert_eq!(
            &selected.boards[row * 144..(row + 1) * 144],
            &all.boards[source * 144..(source + 1) * 144]
        );
        let policy =
            selected.policy_offsets[row] as usize..selected.policy_offsets[row + 1] as usize;
        let expected = all.policy_offsets[source] as usize..all.policy_offsets[source + 1] as usize;
        assert_eq!(
            selected.policy_probabilities[policy.clone()],
            all.policy_probabilities[expected.clone()]
        );
        assert_eq!(selected.policy_from[policy], all.policy_from[expected]);
        let legal = selected.legal_offsets[row] as usize..selected.legal_offsets[row + 1] as usize;
        let expected = all.legal_offsets[source] as usize..all.legal_offsets[source + 1] as usize;
        assert_eq!(selected.legal_from[legal], all.legal_from[expected]);
    }
}

#[test]
fn scan_rejects_truncation_counts_flags_and_trailing_bytes() {
    let fixture = Fixture::new();
    let path = fixture.write(
        TeacherKind::AlphaBeta,
        &[alpha_game(1, 0, Position::initial())],
    );
    let original = fs::read(&path).unwrap();
    let mut flags = original.clone();
    flags[records::HEADER_LEN + records::GAME_HEADER_LEN + 4] = 32;
    for invalid in [5, 6, 7, 32, 64, 128] {
        let mut malformed = original.clone();
        malformed[records::HEADER_LEN + records::GAME_HEADER_LEN + 4] = invalid;
        fs::write(&path, malformed).unwrap();
        assert!(
            matches!(scan_file(&path), Err(Error::Format(records::Error::InvalidEntryFlags(value))) if value == invalid)
        );
    }
    let mut counts = original.clone();
    counts[144..152].copy_from_slice(&8_u64.to_le_bytes());
    let mut trailing = original.clone();
    trailing.push(0);
    for bytes in [
        original[..151].to_vec(),
        original[..original.len() - 1].to_vec(),
        flags,
        counts,
        trailing,
    ] {
        fs::write(&path, bytes).unwrap();
        assert!(matches!(scan_file(&path), Err(Error::Format(_))));
    }
}

#[test]
fn diagnostic_selection_keeps_only_sample_games_and_merges_training_ordinals() {
    // seed=11の対局13は剰余1、対局1は剰余8、対局4は検証である。
    let fixture = Fixture::new();
    let path = fixture.write(
        TeacherKind::AlphaBeta,
        &[
            alpha_game(1, 20, Position::initial()),
            alpha_game(4, 30, Position::initial()),
            alpha_game(13, 10, Position::initial()),
        ],
    );
    let all = read_file(&path).unwrap();
    let diagnostic = read_selected(&path, &[], &[0, 1, 2, 3], false).unwrap();
    assert_eq!(diagnostic.training_indices, [2, 3]);
    assert_eq!(diagnostic.games, [13, 13]);
    assert_eq!(diagnostic.plies, [11, 16]);
    let merged = read_selected(&path, &[0, 3], &[0, 1, 2, 3], true).unwrap();
    assert_eq!(merged.training_indices, [0, 2, 3]);
    assert_eq!(merged.games, [1, 4, 4, 13, 13]);
    assert_eq!(merged.plies, [21, 31, 36, 11, 16]);
    for (row, &ordinal) in merged.training_indices.iter().enumerate() {
        let row = [0, 3, 4][row];
        let source = [0, 1, 4, 5][ordinal as usize];
        assert_eq!(merged.games[row], all.games[source]);
        assert_eq!(merged.plies[row], all.plies[source]);
        assert_eq!(
            merged.boards[row * 144..(row + 1) * 144],
            all.boards[source * 144..(source + 1) * 144]
        );
    }
    for invalid in [&[1, 1][..], &[1, 0], &[4], &[u64::MAX]] {
        for (training, diagnostic) in [(invalid, &[][..]), (&[][..], invalid)] {
            assert!(matches!(
                read_selected(&path, training, diagnostic, false),
                Err(Error::InvalidSelection)
            ));
        }
    }
}

#[test]
fn scan_and_replay_exclude_check_and_accept_evaluation_adjudication() {
    let fixture = Fixture::new();
    for winner in Color::ALL {
        let original = mcts_game(
            Color::Black,
            &[Some(40000), Some(30000), None],
            &[true, true, false],
            Outcome::EvaluationAdjudication { winner },
        );
        let entries = original
            .entries()
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                Entry::new(
                    entry.played_move(),
                    entry.value(),
                    if index == 0 {
                        Exclusion::Check
                    } else {
                        entry.exclusion()
                    },
                    entry.policy().clone(),
                    entry.visits(),
                )
                .unwrap()
            })
            .collect();
        let game = GameRecord::new(
            original.game_number(),
            original.origin(),
            original.outcome(),
            original.start_ply(),
            original.start_position().clone(),
            entries,
        )
        .unwrap();
        let path = fixture.write(TeacherKind::Mcts, &[game]);
        let info = scan_file(&path).unwrap();
        assert_eq!(info.training_count + info.validation_count, 1);
        let records = read_file(&path).unwrap();
        assert_eq!(records.plies, [1]);
        assert_eq!(records.outcomes, [winner as u8]);
        let mut bytes = fs::read(&path).unwrap();
        bytes[4..8].copy_from_slice(&2_u32.to_le_bytes());
        fs::write(&path, bytes).unwrap();
        assert!(matches!(
            scan_file(&path),
            Err(Error::Format(records::Error::UnsupportedVersion(2)))
        ));
        assert!(matches!(
            read_file(&path),
            Err(Error::Format(records::Error::UnsupportedVersion(2)))
        ));
    }
}

#[test]
fn max_ply_draw_keeps_all_policy_positions_with_draw_outcomes() {
    let fixture = Fixture::new();
    let game = mcts_game(
        Color::Black,
        &[Some(40000), Some(30000), None],
        &[true, true, false],
        Outcome::MaxPlyDraw,
    );
    let path = fixture.write(TeacherKind::Mcts, &[game]);
    let info = scan_file(&path).unwrap();
    assert_eq!(info.training_count + info.validation_count, 2);
    let records = read_file(&path).unwrap();
    assert_eq!(records.plies, [0, 1]);
    assert_eq!(records.outcomes, [2, 2]);
}
