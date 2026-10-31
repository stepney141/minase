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
fn control_planes_match_inference_for_every_retained_record() {
    // フェーズ3: MNSDの再生で得た局面を推論で符号化し、盤の配列から得た64面と照合する。
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/zero-selfplay-small.mnsd");
    let records = read_file(&path).unwrap();
    let actual = control_planes(&records.boards, &records.sides).unwrap();
    assert_eq!(actual.len(), records.sides.len() * 64 * 144);
    let mut reader = Reader::new(BufReader::new(File::open(&path).unwrap())).unwrap();
    let mut input = vec![0.0; INPUT_SIZE];
    let mut index = 0;
    while let Some(game) = reader.read_game().unwrap() {
        for (entry, position) in game.record().entries().iter().zip(game.positions()) {
            if entry.exclusion() != Exclusion::None || entry.value() == Value::None {
                continue;
            }
            encode(position, &mut input).unwrap();
            assert_eq!(
                actual[index * CONTROL_SIZE..(index + 1) * CONTROL_SIZE],
                input[95 * 144..],
                "record {index}"
            );
            index += 1;
        }
    }
    assert_eq!(index, 1009);
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
        games.push(
            GameRecord::new(
                side as u32,
                Origin::RandomSelfplay,
                draw(),
                0,
                builder.finish().unwrap(),
                vec![
                    Entry::new(
                        None,
                        Value::AlphaBeta(0),
                        Exclusion::None,
                        Policy::None,
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
        let mut input = vec![0.0; INPUT_SIZE];
        encode(&game.positions()[0], &mut input).unwrap();
        assert_eq!(planes, &input[95 * 144..]);
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
        let header = Header::new(
            "engine-default".into(),
            "a1".repeat(20),
            [37; 32],
            kind,
            1_234,
            11,
        )
        .unwrap();
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
    let mut game = Game::from_position(Rules::ENGINE_DEFAULT, start.clone());
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
fn alpha_beta_matches_every_trainer_record_in_original_order() {
    // 設計書「検証」: 訓練・検証集合の区別によらず、全通し番号を順に比較する。
    let fixture = Fixture::new();
    let mut black = Position::initial();
    black.set_lion_capture(Some(square(1, 8))).unwrap();
    let mut white_game = Game::with_default_rules();
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
    assert_eq!(actual.games, [1, 1, 1, 4, 4, 4]);
    assert_eq!(actual.plies, [13, 14, 19, 30, 31, 36]);
    assert_eq!(actual.values, [100, -200, -32768, 100, -200, -32768]);
    assert_eq!(actual.policy_offsets, [0, 0, 1, 3, 3, 4, 6]);
    assert_eq!(
        actual.policy_probabilities,
        [65535, 10000, 55535, 65535, 10000, 55535]
    );
    assert_eq!(actual.visits, [0; 6]);
    assert_eq!(trainer.record_count(), 6);
    assert_eq!(trainer.training_indices(), [0, 1, 2]);
    assert_eq!(trainer.validation_indices(), [3, 4, 5]);
    for (index, board) in actual.boards.as_chunks::<144>().0.iter().enumerate() {
        let sample = trainer.record(index as u64).unwrap();
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
    assert_eq!(actual.lions[0], 97);
    assert_eq!(actual.sides, [0, 1, 0, 1, 0, 1]);
    assert_eq!(actual.info.origin, Some(Origin::RandomSelfplay));
    assert_eq!(actual.info.header.rule_set(), "engine-default");
    assert_eq!(actual.info.header.rules(), Rules::ENGINE_DEFAULT);
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
    let mut game = Game::with_default_rules();
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
    let game = Game::from_position(Rules::ENGINE_DEFAULT, position.clone());
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
    let game = GameRecord::new(
        1,
        Origin::RandomSelfplay,
        draw(),
        u16::MAX,
        builder.finish().unwrap(),
        vec![
            Entry::new(
                None,
                Value::AlphaBeta(0),
                Exclusion::None,
                Policy::None,
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
            if entry.exclusion() != Exclusion::None || entry.value() == Value::None {
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
    // 仕様: seed=11の対局1は訓練、対局4は検証。各対局の採用項目は0,1,6。
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
    assert_eq!((info.training_count, info.validation_count), (3, 3));
    let selected = read_selected(&path, &[1], true).unwrap();
    assert_eq!(selected.games, [1, 4, 4, 4]);
    assert_eq!(selected.plies, [14, 30, 31, 36]);
    assert_eq!(selected.boards.len(), 4 * 144);
    assert_eq!(
        (selected.info.training_count, selected.info.validation_count),
        (3, 3)
    );
    let empty = read_selected(&path, &[], false).unwrap();
    assert!(empty.boards.is_empty());
    assert_eq!(empty.policy_offsets, [0]);
    assert_eq!(empty.legal_offsets, [0]);
    assert_eq!(empty.info.game_numbers, [1, 4]);
    for indices in [&[1, 1][..], &[2, 0], &[3]] {
        assert!(matches!(
            read_selected(&path, indices, false),
            Err(Error::InvalidSelection)
        ));
    }

    // 指した手を欠かしても計数できるが、抽出が空でも再生による検査は省かない。
    let mut bytes = fs::read(&path).unwrap();
    let entry = records::HEADER_LEN + records::GAME_HEADER_LEN;
    bytes[entry..entry + 2].copy_from_slice(&u16::MAX.to_le_bytes());
    fs::write(&path, bytes).unwrap();
    let info = scan_file(&path).unwrap();
    assert_eq!((info.training_count, info.validation_count), (3, 3));
    assert!(matches!(
        read_selected(&path, &[], false),
        Err(Error::Format(records::Error::MissingPlayedMove))
    ));
}

#[test]
fn game_selection_uses_full_keys_without_the_training_validation_split() {
    // 対局1は訓練、4は検証。選択した対局の採用項目0,1,6をすべて保持する。
    let fixture = Fixture::new();
    let path = fixture.write(
        TeacherKind::AlphaBeta,
        &[
            alpha_game(1, 13, Position::initial()),
            alpha_game(4, 30, Position::initial()),
        ],
    );
    for (keys, games, plies) in [
        (vec![(11, 1), (12, 4)], vec![1, 1, 1], vec![13, 14, 19]),
        (vec![(12, 1), (11, 4)], vec![4, 4, 4], vec![30, 31, 36]),
        (
            vec![(11, 4), (11, 1), (11, 99)],
            vec![1, 1, 1, 4, 4, 4],
            vec![13, 14, 19, 30, 31, 36],
        ),
    ] {
        let selected = read_games(&path, &keys.into_iter().collect()).unwrap();
        assert_eq!(selected.games, games);
        assert_eq!(selected.plies, plies);
        assert_eq!(selected.boards.len(), games.len() * 144);
        assert_eq!(selected.info.game_numbers, [1, 4]);
        assert_eq!(selected.info.header.game_count(), 2);
        assert_eq!(selected.info.header.entry_count(), 14);
        assert_eq!(
            (selected.info.training_count, selected.info.validation_count),
            (3, 3)
        );
        assert_eq!(selected.policy_offsets.len(), games.len() + 1);
        assert_eq!(selected.legal_offsets.len(), games.len() + 1);
        for &values in selected.values.as_chunks::<3>().0 {
            assert_eq!(values, [100, -200, -32768]);
        }
        for &probabilities in selected.policy_probabilities.as_chunks::<3>().0 {
            assert_eq!(probabilities, [65535, 10000, 55535]);
        }
    }
}

#[test]
fn empty_game_selection_preserves_metadata_without_position_arrays() {
    let fixture = Fixture::new();
    let path = fixture.write(
        TeacherKind::AlphaBeta,
        &[alpha_game(1, 0, Position::initial())],
    );
    for keys in [HashSet::new(), HashSet::from([(12, 1), (11, 99)])] {
        let selected = read_games(&path, &keys).unwrap();
        assert_eq!(selected.info.game_numbers, [1]);
        assert_eq!(
            (selected.info.training_count, selected.info.validation_count),
            (3, 0)
        );
        assert!(selected.games.is_empty());
        assert!(selected.boards.is_empty());
        assert!(selected.policy_from.is_empty());
        assert!(selected.legal_from.is_empty());
        assert_eq!(selected.policy_offsets, [0]);
        assert_eq!(selected.legal_offsets, [0]);
    }
    let path = fixture.write(TeacherKind::AlphaBeta, &[]);
    let empty = read_games(&path, &HashSet::from([(11, 1)])).unwrap();
    assert!(empty.info.game_numbers.is_empty());
    assert!(empty.boards.is_empty());
    assert_eq!(empty.policy_offsets, [0]);
    assert_eq!(empty.legal_offsets, [0]);
}

#[test]
fn game_selection_still_replays_and_rejects_invalid_unselected_games() {
    let fixture = Fixture::new();
    let path = fixture.write(
        TeacherKind::AlphaBeta,
        &[
            alpha_game(1, 0, Position::initial()),
            alpha_game(4, 0, Position::initial()),
        ],
    );
    // 選択しない対局1の最初の着手を欠かす。対局4だけを選んでも形式検査は必要である。
    let mut bytes = fs::read(&path).unwrap();
    let entry = records::HEADER_LEN + records::GAME_HEADER_LEN;
    bytes[entry..entry + 2].copy_from_slice(&u16::MAX.to_le_bytes());
    fs::write(&path, bytes).unwrap();
    for keys in [HashSet::new(), HashSet::from([(11, 4)])] {
        assert!(matches!(
            read_games(&path, &keys),
            Err(Error::Format(records::Error::MissingPlayedMove))
        ));
    }
}

#[test]
fn game_selection_accepts_mcts_games_without_retained_positions() {
    let fixture = Fixture::new();
    let path = fixture.write(
        TeacherKind::Mcts,
        &[unselected_game(4, Origin::RandomSelfplay)],
    );
    let selected = read_games(&path, &HashSet::from([(11, 4)])).unwrap();
    assert_eq!(selected.info.header.teacher_kind(), TeacherKind::Mcts);
    assert_eq!(selected.info.game_numbers, [4]);
    assert_eq!(
        (selected.info.training_count, selected.info.validation_count),
        (0, 0)
    );
    assert!(selected.boards.is_empty());
    assert_eq!(selected.policy_offsets, [0]);
    assert_eq!(selected.legal_offsets, [0]);
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
    assert_eq!(info.validation_count as usize, 1009 - training.len());
    assert_eq!(info.game_numbers, all.info.game_numbers);
    let ordinals = [0, 50, training.len() as u64 - 1];
    let selected = read_selected(&path, &ordinals, false).unwrap();
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
    flags[records::HEADER_LEN + records::GAME_HEADER_LEN + 4] = 16;
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
