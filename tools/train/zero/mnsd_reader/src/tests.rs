//! Writerと既存のRust学習器を独立した参照元にして、選別と復号を照合する。

use super::*;
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
    assert_eq!(actual.origin, Some(Origin::RandomSelfplay));
    assert_eq!(actual.header.rule_set(), "engine-default");
    assert_eq!(actual.header.rules(), Rules::ENGINE_DEFAULT);
    assert_eq!(actual.header.generation_commit(), "a1".repeat(20));
    assert_eq!(actual.header.network_checksum(), &[37; 32]);
    assert_eq!(actual.header.teacher_kind(), TeacherKind::AlphaBeta);
    assert_eq!(actual.header.teacher_search_budget(), 1234);
    assert_eq!(actual.header.seed(), 11);
    assert_eq!(actual.header.game_count(), 2);
    assert_eq!(actual.header.entry_count(), 14);
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
    assert_eq!(actual.header.teacher_kind(), TeacherKind::Mcts);
    assert_eq!(actual.game_numbers, [2, 7]);
    assert_eq!(actual.games, [2, 7]);
    assert_eq!(actual.plies, [1, 42]);
    assert_eq!(actual.sides, [1, 0]);
    assert_eq!(actual.values, [65535, 37]);
    assert_eq!(actual.outcomes, [2, 1]);
    assert_eq!(actual.visits, [31, u32::MAX]);
    assert_eq!(actual.policy_offsets, [0, 1, 3]);
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
    assert_eq!(empty.origin, None);
    assert_eq!(empty.header.game_count(), 0);
    assert_eq!(empty.header.entry_count(), 0);
    assert!(empty.game_numbers.is_empty());
    assert!(empty.boards.is_empty());
    assert_eq!(empty.policy_offsets, [0]);
    let path = fixture.write(
        TeacherKind::Mcts,
        &[unselected_game(99, Origin::HumanOpeningSelfplay)],
    );
    let unselected = read_file(&path).unwrap();
    assert_eq!(unselected.origin, Some(Origin::HumanOpeningSelfplay));
    assert_eq!(unselected.game_numbers, [99]);
    assert!(unselected.games.is_empty());
    assert_eq!(unselected.policy_offsets, [0]);
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
