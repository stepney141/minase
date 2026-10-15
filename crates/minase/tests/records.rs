//! MNSD版2の配置と公開関数の契約を検証する。

use minase::training::records::*;
use minase_core::test_util::{position, position_from_codes, sq};
use minase_core::{
    Color, DrawReason, Game, GameResult, Move, PieceCode, PieceKind, Position, WinReason,
};
use std::io::Cursor;

const COMMIT: &str = "0123456789abcdef0123456789ABCDEF01234567";
const H: usize = 152;
const G: usize = 161;
const E: usize = H + G;

fn header(kind: TeacherKind) -> Header {
    named_header(kind, "L0,P0,R1,E0")
}
fn named_header(kind: TeacherKind, rules: &str) -> Header {
    Header::new(
        rules.into(),
        COMMIT.into(),
        [0x35; 32],
        kind,
        1_u64 << 40,
        u64::MAX - 1,
    )
    .unwrap()
}
fn step(from: (u8, u8), to: (u8, u8)) -> Move {
    Move {
        from: sq(from.0, from.1),
        mid: None,
        to: sq(to.0, to.1),
        promote: false,
    }
}
fn entry(mv: Option<Move>, policy: Policy) -> Entry {
    Entry::new(mv, Value::None, Exclusion::None, policy, None).unwrap()
}
fn draw() -> Outcome {
    Outcome::Game(GameResult::Draw {
        reason: DrawReason::Agreement,
    })
}
fn record(number: u32, start: Position, entries: Vec<Entry>) -> GameRecord {
    GameRecord::new(number, Origin::HumanGame, draw(), 0, start, entries).unwrap()
}
fn bytes(header: Header, records: &[GameRecord]) -> Vec<u8> {
    let mut writer = Writer::new(Cursor::new(Vec::new()), header).unwrap();
    for record in records {
        writer.write_game(record).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
fn decode(bytes: &[u8]) -> Result<Vec<DecodedGame>, Error> {
    // Seekを実装しない入力でも読めることを併せて検査する。
    let mut reader = Reader::new(bytes)?;
    let mut games = Vec::new();
    while let Some(game) = reader.read_game()? {
        games.push(game);
    }
    Ok(games)
}
fn assert_error(bytes: &[u8], expected: Error) {
    let actual = decode(bytes).unwrap_err();
    assert_eq!(
        std::mem::discriminant(&actual),
        std::mem::discriminant(&expected),
        "{actual:?}, expected {expected:?}"
    );
}
fn simple_file() -> Vec<u8> {
    let h = header(TeacherKind::AlphaBeta);
    let game = Game::new(h.rules());
    let moves = sorted_legal_moves(&game).unwrap();
    let entries = vec![
        entry(Some(moves[0]), Policy::None),
        entry(None, Policy::None),
    ];
    bytes(h, &[record(7, Position::initial(), entries)])
}
fn candidate_file() -> Vec<u8> {
    let h = header(TeacherKind::AlphaBeta);
    let moves = sorted_legal_moves(&Game::new(h.rules())).unwrap();
    bytes(
        h,
        &[record(
            7,
            Position::initial(),
            vec![entry(
                None,
                Policy::Candidates(vec![(moves[0], 20_000), (moves[1], 45_535)]),
            )],
        )],
    )
}
fn key(mv: Move) -> (usize, usize, usize, bool) {
    (
        mv.from.dense_index(),
        mv.mid.map_or(255, |s| s.dense_index()),
        mv.to.dense_index(),
        mv.promote,
    )
}

#[test]
fn all_header_game_and_entry_fields_round_trip_for_both_teachers() {
    for kind in [TeacherKind::AlphaBeta, TeacherKind::Mcts] {
        let h = header(kind);
        let mut records = Vec::new();
        let mut expected_positions = Vec::new();
        for (number, origin) in [
            (1, Origin::RandomSelfplay),
            (4, Origin::HumanOpeningSelfplay),
            (9, Origin::HumanGame),
        ] {
            let mut game = Game::new(h.rules());
            let mut entries = Vec::new();
            let mut positions = Vec::new();
            for (k, exclusion) in [
                Exclusion::None,
                Exclusion::MateBand,
                Exclusion::Tactical,
                Exclusion::Repetition,
            ]
            .into_iter()
            .enumerate()
            {
                positions.push(game.position().clone());
                let moves = sorted_legal_moves(&game).unwrap();
                let played = if k == 3 { None } else { Some(moves[2]) };
                let value = match (kind, k) {
                    (_, 0) => Value::None,
                    (TeacherKind::AlphaBeta, 1) => Value::AlphaBeta(i16::MIN),
                    (TeacherKind::AlphaBeta, _) => Value::AlphaBeta(i16::MAX),
                    (TeacherKind::Mcts, 1) => Value::Mcts(0),
                    (TeacherKind::Mcts, _) => Value::Mcts(u16::MAX),
                };
                let visits = match kind {
                    TeacherKind::AlphaBeta => None,
                    TeacherKind::Mcts => Some(if k == 0 { 0 } else { u32::MAX }),
                };
                let policy = match k {
                    0 => Policy::None,
                    1 => Policy::PlayedMove,
                    _ => Policy::Candidates(vec![(moves[0], 123), (moves[1], 65_412)]),
                };
                entries.push(Entry::new(played, value, exclusion, policy, visits).unwrap());
                if let Some(mv) = played {
                    game.play(mv).unwrap();
                }
            }
            records.push(
                GameRecord::new(number, origin, draw(), 42, Position::initial(), entries).unwrap(),
            );
            expected_positions.push(positions);
        }
        let data = bytes(h.clone(), &records);
        assert_eq!(&data[0..4], b"MNSD");
        assert_eq!(&data[4..8], &2_u32.to_le_bytes());
        assert_eq!(&data[8..19], b"L0,P0,R1,E0");
        assert!(data[19..40].iter().all(|b| *b == 0));
        assert_eq!(&data[40..80], COMMIT.as_bytes());
        assert_eq!(&data[80..112], &[0x35; 32]);
        assert_eq!(data[112], kind as u8);
        assert_eq!(&data[113..120], &[0; 7]);
        assert_eq!(&data[120..128], &(1_u64 << 40).to_le_bytes());
        assert_eq!(&data[128..136], &(u64::MAX - 1).to_le_bytes());
        assert_eq!(&data[136..144], &3_u64.to_le_bytes());
        assert_eq!(&data[144..152], &12_u64.to_le_bytes());
        assert_eq!(&data[H..H + 4], &1_u32.to_le_bytes());
        assert_eq!(&data[H + 4..H + 8], &[0, 2, 11, 0]);
        assert_eq!(&data[H + 8..H + 10], &42_u16.to_le_bytes());
        assert_eq!(&data[H + 10..H + 14], &4_u32.to_le_bytes());
        assert_eq!(&data[H + 158..E], &[0, 255, 0]);
        assert_eq!(&data[E..E + 7], &[2, 0, 0, 0, 4, 0, 0]);
        let mut reader = Reader::new(data.as_slice()).unwrap();
        let got = reader.header();
        assert_eq!(got.rule_set(), h.rule_set());
        assert_eq!(got.rules(), h.rules());
        assert_eq!(got.generation_commit(), h.generation_commit());
        assert_eq!(got.network_checksum(), h.network_checksum());
        assert_eq!(got.teacher_kind(), h.teacher_kind());
        assert_eq!(got.teacher_search_budget(), h.teacher_search_budget());
        assert_eq!(got.seed(), h.seed());
        assert_eq!(got.game_count(), 3);
        assert_eq!(got.entry_count(), 12);
        for (record, positions) in records.iter().zip(expected_positions) {
            let actual = reader.read_game().unwrap().unwrap();
            assert_eq!(actual.record(), record);
            assert_eq!(actual.positions(), positions);
        }
        assert!(reader.read_game().unwrap().is_none());
    }
}

#[test]
fn every_end_reason_has_the_prescribed_byte_code() {
    let wins = [
        WinReason::RoyalCapture,
        WinReason::Repetition,
        WinReason::PieceExhaustion,
        WinReason::BareKing,
        WinReason::Stalemate,
        WinReason::Mate,
        WinReason::Resignation,
    ];
    let draws = [
        DrawReason::Repetition,
        DrawReason::PieceExhaustion,
        DrawReason::BareKing,
        DrawReason::Agreement,
    ];
    let mut outcomes = vec![(Outcome::NotFromGame, 2, 0)];
    for (i, reason) in wins.into_iter().enumerate() {
        for winner in [Color::Black, Color::White] {
            outcomes.push((
                Outcome::Game(GameResult::Win { winner, reason }),
                winner as u8,
                i as u8 + 1,
            ));
        }
    }
    for (i, reason) in draws.into_iter().enumerate() {
        outcomes.push((Outcome::Game(GameResult::Draw { reason }), 2, i as u8 + 8));
    }
    for (outcome, result_code, reason_code) in outcomes {
        let origin = if outcome == Outcome::NotFromGame {
            Origin::Synthetic
        } else {
            Origin::HumanGame
        };
        let record = GameRecord::new(
            0,
            origin,
            outcome,
            u16::MAX,
            Position::initial(),
            vec![entry(None, Policy::None)],
        )
        .unwrap();
        let data = bytes(
            header(TeacherKind::AlphaBeta),
            std::slice::from_ref(&record),
        );
        assert_eq!(data[H + 5], result_code);
        assert_eq!(data[H + 6], reason_code);
        assert_eq!(decode(&data).unwrap()[0].record(), &record);
    }
}

#[test]
fn corrupt_fields_are_classified_by_their_specific_error() {
    for (offset, value, error) in [
        (0, 0, Error::InvalidMagic([0; 4])),
        (4, 9, Error::UnsupportedVersion(9)),
        (112, 2, Error::InvalidTeacherKind(2)),
        (113, 1, Error::NonzeroReservedBytes),
        (119, 1, Error::NonzeroReservedBytes),
        (40, b'g', Error::InvalidGenerationCommit),
        (H + 4, 4, Error::InvalidOrigin(4)),
        (H + 5, 3, Error::InvalidOutcome(3)),
        (H + 6, 12, Error::InvalidEndReason(12)),
        (H + 6, 1, Error::OutcomeReasonMismatch),
        (H + 6, 0, Error::OriginOutcomeMismatch),
        (H + 4, 3, Error::OriginOutcomeMismatch),
        (H + 7, 1, Error::NonzeroReservedBytes),
        (H + 158, 2, Error::InvalidSideToMove(2)),
        (H + 159, 144, Error::InvalidLionSquare(144)),
        (H + 160, 2, Error::InvalidLionPromotionFlag(2)),
        (H + 160, 1, Error::LionPromotionWithoutSquare),
        (H + 14, 59, Error::InvalidPieceCode(59)),
        (E + 4, 0x14, Error::InvalidEntryFlags(0x14)),
        (E + 4, 0x84, Error::InvalidEntryFlags(0x84)),
        (E + 2, 1, Error::NonzeroAbsentValue),
        (E + 4, 12, Error::BestMoveWithCandidates),
    ] {
        let mut data = simple_file();
        data[offset] = value;
        if matches!(error, Error::BestMoveWithCandidates) {
            data[E + 5] = 1;
        }
        assert_error(&data, error);
    }
    let mut data = simple_file();
    data[E..E + 2].copy_from_slice(&u16::MAX.to_le_bytes());
    assert_error(&data, Error::MissingPlayedMove);
    let mut data = simple_file();
    data[E + 7..E + 9].copy_from_slice(&0_u16.to_le_bytes());
    assert_error(&data, Error::MoveOnLastEntry);
    let mut data = simple_file();
    data[E + 11] = 12;
    assert_error(&data, Error::BestMoveWithoutPlayedMove);
    let mut data = simple_file();
    data[H + 10..H + 14].copy_from_slice(&0_u32.to_le_bytes());
    assert_error(&data, Error::EmptyGame);
    let mut data = simple_file();
    data[H + 8..H + 10].copy_from_slice(&u16::MAX.to_le_bytes());
    assert_error(&data, Error::PlyOverflow);
    let mut data = simple_file();
    data[H + 10..H + 14].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_error(&data, Error::PlyOverflow);
}

#[test]
fn header_names_and_commits_are_checked_on_creation_and_reading() {
    let make = |rules: &str, commit: &str| {
        Header::new(
            rules.into(),
            commit.into(),
            [0; 32],
            TeacherKind::AlphaBeta,
            0,
            0,
        )
    };
    assert!(matches!(
        make(&"x".repeat(33), COMMIT),
        Err(Error::InvalidRuleSetName)
    ));
    assert!(matches!(
        make("engine-default\0", COMMIT),
        Err(Error::InvalidRuleSetName)
    ));
    assert!(matches!(make("", COMMIT), Err(Error::RuleSetParse(_))));
    assert!(matches!(
        make("unknown", COMMIT),
        Err(Error::RuleSetParse(_))
    ));
    assert!(matches!(make("L0", COMMIT), Err(Error::Rules(_))));
    assert!(matches!(
        make("L0,P0,R1,E0,L0", COMMIT),
        Err(Error::Rules(_))
    ));
    assert!(matches!(
        make("engine-default", "123"),
        Err(Error::InvalidGenerationCommit)
    ));
    assert!(matches!(
        make("engine-default", &"g".repeat(40)),
        Err(Error::InvalidGenerationCommit)
    ));
    assert!(make("ENGINE-DEFAULT", COMMIT).is_ok());
    for name in [
        vec![255],
        b"L0\0x".to_vec(),
        b"unknown".to_vec(),
        b"L0".to_vec(),
    ] {
        let mut data = simple_file();
        data[8..40].fill(0);
        data[8..8 + name.len()].copy_from_slice(&name);
        let e = decode(&data).unwrap_err();
        match name.as_slice() {
            [255] | [b'L', b'0', 0, b'x'] => assert!(matches!(e, Error::InvalidEncodedRuleSetName)),
            [b'L', b'0'] => assert!(matches!(e, Error::Rules(_))),
            _ => assert!(matches!(e, Error::RuleSetParse(_))),
        }
    }
}

#[test]
fn indices_candidate_order_duplicates_and_probabilities_are_checked() {
    let legal_count = Game::new(header(TeacherKind::AlphaBeta).rules())
        .legal_moves()
        .len() as u16;
    let mut data = simple_file();
    data[E..E + 2].copy_from_slice(&legal_count.to_le_bytes());
    assert_error(&data, Error::InvalidMoveIndex(legal_count));
    for (offset, value, error) in [
        (E + 7, legal_count, Error::InvalidMoveIndex(legal_count)),
        (E + 7, 2, Error::CandidatesNotSorted),
        (E + 11, 0, Error::DuplicateCandidate),
        (E + 9, 0, Error::ZeroProbability),
        (E + 9, 1, Error::InvalidProbabilitySum),
    ] {
        let mut data = candidate_file();
        data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        assert_error(&data, error);
    }
    let moves = sorted_legal_moves(&Game::new(header(TeacherKind::AlphaBeta).rules())).unwrap();
    for (policy, expected) in [
        (vec![], Error::EmptyPolicy),
        (
            vec![(moves[1], 1), (moves[0], 65_534)],
            Error::CandidatesNotSorted,
        ),
        (
            vec![(moves[0], 1), (moves[0], 65_534)],
            Error::DuplicateCandidate,
        ),
        (
            vec![(moves[0], 0), (moves[1], 65_535)],
            Error::ZeroProbability,
        ),
        (vec![(moves[0], 65_534)], Error::InvalidProbabilitySum),
    ] {
        let actual = Entry::new(
            None,
            Value::None,
            Exclusion::None,
            Policy::Candidates(policy),
            None,
        )
        .unwrap_err();
        assert_eq!(
            std::mem::discriminant(&actual),
            std::mem::discriminant(&expected)
        );
    }
}

#[test]
fn truncation_excess_bytes_and_header_counts_are_rejected() {
    let data = candidate_file();
    for cut in 0..data.len() {
        let expected = if cut < H {
            Section::Header
        } else if cut < E {
            Section::GameHeader
        } else if cut < E + 7 {
            Section::Entry
        } else {
            Section::Candidate
        };
        assert!(
            matches!(decode(&data[..cut]), Err(Error::Truncated(s)) if s == expected),
            "cut {cut}"
        );
    }
    let mut extra = data.clone();
    extra.push(1);
    assert_error(&extra, Error::TrailingBytes);
    let mut zero_games = data.clone();
    zero_games[136..144].fill(0);
    zero_games[144..152].fill(0);
    assert_error(&zero_games, Error::TrailingBytes);
    let mut more_games = data.clone();
    more_games[136..144].copy_from_slice(&2_u64.to_le_bytes());
    assert_error(&more_games, Error::Truncated(Section::GameHeader));
    for count in [0_u64, 2, u64::MAX] {
        let mut wrong = data.clone();
        wrong[144..152].copy_from_slice(&count.to_le_bytes());
        assert_error(&wrong, Error::EntryCountMismatch);
    }
    let empty = bytes(header(TeacherKind::AlphaBeta), &[]);
    assert_eq!(empty.len(), H);
    assert!(decode(&empty).unwrap().is_empty());
    let mut invalid_empty = empty.clone();
    invalid_empty[144] = 1;
    assert_error(&invalid_empty, Error::EntryCountMismatch);
}

#[test]
fn quantization_matches_hand_calculated_largest_remainders() {
    assert_eq!(
        quantize_policy(&[(5, 1.0), (2, 1.0)]).unwrap(),
        vec![(2, 32_768), (5, 32_767)]
    );
    // 1:3は16,383.75と49,151.25となり、端数の大きい前者に1を配る。
    assert_eq!(
        quantize_policy(&[(8, 1.0), (1, 3.0)]).unwrap(),
        vec![(1, 49_151), (8, 16_384)]
    );
    assert_eq!(
        quantize_policy(&[(7, 1.0), (1, 1.0), (3, 1.0)]).unwrap(),
        vec![(1, 21_845), (3, 21_845), (7, 21_845)]
    );
    assert_eq!(
        quantize_policy(&[(0, 0.0), (1, 1e-20), (2, 1.0)]).unwrap(),
        vec![(2, 65_535)]
    );
    assert_eq!(
        quantize_policy(&[(0, f64::MAX), (1, f64::MAX)]).unwrap(),
        vec![(0, 32_768), (1, 32_767)]
    );
    assert_eq!(
        quantize_policy(&[(0, f64::from_bits(1))]).unwrap(),
        vec![(0, 65_535)]
    );
    for weights in [vec![], vec![(1, 0.0)]] {
        assert!(matches!(
            quantize_policy(&weights),
            Err(Error::ZeroWeightSum)
        ));
    }
    for weight in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(matches!(
            quantize_policy(&[(0, weight)]),
            Err(Error::InvalidWeight)
        ));
    }
    assert!(matches!(
        quantize_policy(&[(1, 1.0), (1, 2.0)]),
        Err(Error::DuplicateWeightIndex)
    ));
    for size in [2_u16, 10, 100, 1000] {
        let weights: Vec<_> = (0..size).map(|i| (i, f64::from(i + 1))).collect();
        let result = quantize_policy(&weights).unwrap();
        assert_eq!(
            result.iter().map(|(_, p)| u64::from(*p)).sum::<u64>(),
            65_535
        );
        assert!(result.iter().all(|(_, p)| *p > 0));
        assert!(result.windows(2).all(|w| w[0].0 < w[1].0));
    }
}

#[test]
fn promotion_deferred_survives_serialization_and_affects_legal_moves() {
    let h = named_header(TeacherKind::AlphaBeta, "L0,P2,R1,E0");
    let start = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (sq(4, 7), Color::Black, PieceKind::SilverGeneral),
            (sq(10, 10), Color::White, PieceKind::GoldGeneral),
        ],
    );
    let mut game = Game::from_position(h.rules(), start);
    game.play(step((4, 7), (4, 8))).unwrap();
    game.play(step((10, 10), (10, 9))).unwrap();
    assert!(game.position().promotion_deferred().contains(sq(4, 8)));
    let start = game.position().clone();
    let expected = sorted_legal_moves(&Game::from_position(h.rules(), start.clone())).unwrap();
    let promoting = Move {
        promote: true,
        ..step((4, 8), (4, 9))
    };
    // RULES.md第30条P2では、保留中の直後の手番は非捕獲で成れない。
    assert!(!expected.contains(&promoting));
    let record = record(
        3,
        start.clone(),
        vec![entry(
            None,
            Policy::Candidates(vec![(step((4, 8), (4, 9)), u16::MAX)]),
        )],
    );
    let data = bytes(h.clone(), &[record]);
    assert_ne!(data[H + 14 + sq(4, 8).dense_index()] & 128, 0);
    let restored = decode(&data).unwrap().remove(0);
    assert_eq!(restored.positions(), std::slice::from_ref(&start));
    assert_eq!(
        restored.record().start_position().promotion_deferred(),
        start.promotion_deferred()
    );
    assert_eq!(
        sorted_legal_moves(&Game::from_position(
            h.rules(),
            restored.positions()[0].clone()
        ))
        .unwrap(),
        expected
    );
    // 権利を消すと合法手集合が変わることを独立に確かめる。
    let mut builder = minase_core::PositionBuilder::new(start.side_to_move());
    for square in minase_core::Square::all() {
        if let Some(piece) = start.piece_at(square) {
            builder.put(square, piece).unwrap();
        }
    }
    let without =
        sorted_legal_moves(&Game::from_position(h.rules(), builder.finish().unwrap())).unwrap();
    assert!(without.contains(&promoting));
}

#[test]
fn repetition_history_controls_move_indices_for_r2_and_r3() {
    for (rules, cycles) in [("L0,P0,R2,E2", 0), ("L0,P0,R3,E2", 2)] {
        let h = named_header(TeacherKind::AlphaBeta, rules);
        let start = position(
            Color::Black,
            &[
                (sq(3, 3), Color::Black, PieceKind::King),
                (sq(8, 8), Color::White, PieceKind::King),
            ],
        );
        let cycle = [
            step((3, 3), (3, 4)),
            step((8, 8), (8, 7)),
            step((3, 4), (3, 3)),
            step((8, 7), (8, 8)),
        ];
        let mut game = Game::from_position(h.rules(), start.clone());
        let mut entries = Vec::new();
        let mut positions = Vec::new();
        for mv in cycle.iter().cycle().take(cycles * 4 + 3) {
            positions.push(game.position().clone());
            entries.push(entry(Some(*mv), Policy::PlayedMove));
            game.play(*mv).unwrap();
        }
        let mut fresh = Game::from_position(h.rules(), game.position().clone()).legal_moves();
        fresh.sort_by_key(|mv| key(*mv));
        let moves = sorted_legal_moves(&game).unwrap();
        assert!(fresh.contains(&cycle[3]));
        assert!(!moves.contains(&cycle[3]));
        let shifted = fresh.iter().position(|mv| *mv == cycle[3]).unwrap();
        let alternative = moves[shifted];
        assert_ne!(fresh[shifted], alternative);
        positions.push(game.position().clone());
        entries.push(entry(
            Some(alternative),
            Policy::Candidates(vec![(alternative, u16::MAX)]),
        ));
        game.play(alternative).unwrap();
        positions.push(game.position().clone());
        entries.push(entry(None, Policy::None));
        let record = record(1, start, entries);
        let data = bytes(h, std::slice::from_ref(&record));
        let offset = E + (cycles * 4 + 3) * 7;
        assert_eq!(
            u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap()),
            shifted as u16
        );
        let decoded = decode(&data).unwrap();
        assert_eq!(decoded[0].record(), &record);
        assert_eq!(decoded[0].positions(), positions);
    }
}

#[test]
fn lion_capture_state_round_trips() {
    let trigger = sq(4, 4);
    for piece in [
        PieceCode::new(Color::White, PieceKind::Bishop).unwrap(),
        PieceCode::new_promoted(Color::White, PieceKind::Lion).unwrap(),
    ] {
        let mut start = position_from_codes(Color::Black, &[(trigger, piece)]);
        start.set_lion_capture(Some(trigger)).unwrap();
        let record = GameRecord::new(
            0,
            Origin::Synthetic,
            Outcome::NotFromGame,
            0,
            start.clone(),
            vec![entry(None, Policy::None)],
        )
        .unwrap();
        let data = bytes(header(TeacherKind::AlphaBeta), &[record]);
        assert_eq!(data[H + 159], trigger.dense_index() as u8);
        assert_eq!(data[H + 160], u8::from(piece.is_promoted()));
        assert_eq!(decode(&data).unwrap()[0].positions(), &[start]);
        let mut corrupt = data;
        corrupt[H + 160] ^= 1;
        assert_error(&corrupt, Error::LionPromotionMismatch);
    }
}

#[test]
fn synthetic_empty_position_and_deferred_on_empty_square() {
    let start = Position::empty(Color::White);
    let r = GameRecord::new(
        0,
        Origin::Synthetic,
        Outcome::NotFromGame,
        0,
        start.clone(),
        vec![entry(None, Policy::None)],
    )
    .unwrap();
    let data = bytes(header(TeacherKind::AlphaBeta), &[r]);
    assert_eq!(data.len(), 152 + 161 + 7);
    assert_eq!(&data[H + 4..H + 8], &[3, 2, 0, 0]);
    assert_eq!(&data[E..], &[255, 255, 0, 0, 4, 0, 0]);
    assert_eq!(decode(&data).unwrap()[0].positions(), &[start]);
    let mut corrupt = data;
    corrupt[H + 14] = 128;
    assert!(matches!(decode(&corrupt), Err(Error::PositionBuild(_))));
}

#[test]
fn constructors_and_writer_reject_inconsistent_records() {
    assert!(matches!(
        GameRecord::new(
            0,
            Origin::Synthetic,
            draw(),
            0,
            Position::initial(),
            vec![entry(None, Policy::None)]
        ),
        Err(Error::OriginOutcomeMismatch)
    ));
    assert!(matches!(
        GameRecord::new(0, Origin::HumanGame, draw(), 0, Position::initial(), vec![]),
        Err(Error::EmptyGame)
    ));
    let mv = sorted_legal_moves(&Game::with_default_rules()).unwrap()[0];
    assert!(matches!(
        GameRecord::new(
            0,
            Origin::HumanGame,
            draw(),
            0,
            Position::initial(),
            vec![entry(Some(mv), Policy::None)]
        ),
        Err(Error::MoveOnLastEntry)
    ));
    assert!(matches!(
        GameRecord::new(
            0,
            Origin::HumanGame,
            draw(),
            0,
            Position::initial(),
            vec![entry(None, Policy::None), entry(None, Policy::None)]
        ),
        Err(Error::MissingPlayedMove)
    ));
    assert!(matches!(
        GameRecord::new(
            0,
            Origin::HumanGame,
            draw(),
            u16::MAX,
            Position::initial(),
            vec![entry(Some(mv), Policy::None), entry(None, Policy::None)]
        ),
        Err(Error::PlyOverflow)
    ));
    assert!(matches!(
        Entry::new(None, Value::None, Exclusion::None, Policy::PlayedMove, None),
        Err(Error::BestMoveWithoutPlayedMove)
    ));
    for (kind, value, visits, expected) in [
        (
            TeacherKind::AlphaBeta,
            Value::Mcts(1),
            None,
            Error::ValueKindMismatch,
        ),
        (
            TeacherKind::Mcts,
            Value::AlphaBeta(-1),
            Some(0),
            Error::ValueKindMismatch,
        ),
        (
            TeacherKind::AlphaBeta,
            Value::None,
            Some(0),
            Error::UnexpectedVisits,
        ),
        (TeacherKind::Mcts, Value::None, None, Error::MissingVisits),
    ] {
        let bad = record(
            0,
            Position::initial(),
            vec![Entry::new(None, value, Exclusion::None, Policy::None, visits).unwrap()],
        );
        let mut writer = Writer::new(Cursor::new(vec![]), header(kind)).unwrap();
        let actual = writer.write_game(&bad).unwrap_err();
        assert_eq!(
            std::mem::discriminant(&actual),
            std::mem::discriminant(&expected)
        );
        assert_eq!(writer.finish().unwrap().into_inner().len(), H);
    }
    let illegal = step((5, 5), (5, 6));
    for (played, policy) in [
        (Some(illegal), Policy::None),
        (None, Policy::Candidates(vec![(illegal, 65_535)])),
    ] {
        let mut entries = vec![entry(played, policy)];
        if played.is_some() {
            entries.push(entry(None, Policy::None));
        }
        let bad = record(0, Position::initial(), entries);
        let mut writer = Writer::new(Cursor::new(vec![]), header(TeacherKind::AlphaBeta)).unwrap();
        assert!(matches!(writer.write_game(&bad), Err(Error::IllegalMove(m)) if m == illegal));
        assert_eq!(writer.finish().unwrap().into_inner().len(), H);
    }
}

#[test]
fn writer_and_reader_require_strictly_increasing_game_numbers() {
    let h = header(TeacherKind::AlphaBeta);
    let first = record(3, Position::initial(), vec![entry(None, Policy::None)]);
    let second = record(4, Position::initial(), vec![entry(None, Policy::None)]);
    let valid = bytes(h.clone(), &[first.clone(), second]);
    for number in [2_u32, 3] {
        let mut corrupt = valid.clone();
        corrupt[E + 7..E + 11].copy_from_slice(&number.to_le_bytes());
        assert_error(&corrupt, Error::GameNumberNotIncreasing);
        let mut writer = Writer::new(Cursor::new(vec![]), h.clone()).unwrap();
        writer.write_game(&first).unwrap();
        let bad = record(number, Position::initial(), vec![entry(None, Policy::None)]);
        assert!(matches!(
            writer.write_game(&bad),
            Err(Error::GameNumberNotIncreasing)
        ));
        assert_eq!(
            decode(&writer.finish().unwrap().into_inner())
                .unwrap()
                .len(),
            1
        );
    }
}

#[test]
fn terminal_capture_is_restored_and_continuation_is_rejected() {
    let start = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 5), Color::Black, PieceKind::Rook),
            (sq(5, 8), Color::White, PieceKind::King),
        ],
    );
    let capture = step((5, 5), (5, 8));
    let r = GameRecord::new(
        0,
        Origin::HumanGame,
        Outcome::Game(GameResult::Win {
            winner: Color::Black,
            reason: WinReason::RoyalCapture,
        }),
        0,
        start.clone(),
        vec![
            entry(Some(capture), Policy::PlayedMove),
            entry(None, Policy::None),
        ],
    )
    .unwrap();
    let h = header(TeacherKind::AlphaBeta);
    let mut game = Game::from_position(h.rules(), start.clone());
    game.play(capture).unwrap();
    assert!(game.result().is_some());
    let valid = bytes(h.clone(), &[r]);
    assert_eq!(decode(&valid).unwrap()[0].positions()[1], *game.position());
    let bad = record(
        1,
        start,
        vec![
            entry(Some(capture), Policy::None),
            entry(Some(step((0, 0), (0, 1))), Policy::None),
            entry(None, Policy::None),
        ],
    );
    let mut writer = Writer::new(Cursor::new(vec![]), h).unwrap();
    assert!(matches!(
        writer.write_game(&bad),
        Err(Error::MoveAfterGameOver)
    ));
    let mut corrupt = valid;
    corrupt[144..152].copy_from_slice(&3_u64.to_le_bytes());
    corrupt[H + 10..H + 14].copy_from_slice(&3_u32.to_le_bytes());
    corrupt[E + 7..E + 9].copy_from_slice(&0_u16.to_le_bytes());
    corrupt.extend_from_slice(&[255, 255, 0, 0, 4, 0, 0]);
    assert_error(&corrupt, Error::MoveAfterGameOver);
}

#[test]
fn sorted_keys_are_unique_across_random_play_and_all_lion_power_moves() {
    let rules = named_header(TeacherKind::AlphaBeta, "L0,P0,R1,E2").rules();
    let mut random = 0x41b3_c261_528d_709f_u64;
    let mut visited = 0;
    for _ in 0..8 {
        let mut game = Game::new(rules);
        for _ in 0..200 {
            let moves = sorted_legal_moves(&game).unwrap();
            assert!(moves.windows(2).all(|w| key(w[0]) < key(w[1])));
            if moves.is_empty() {
                break;
            }
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            game.play(moves[random as usize % moves.len()]).unwrap();
            visited += 1;
        }
    }
    assert!(visited >= 1000, "visited {visited}");
    for kind in [
        PieceKind::Lion,
        PieceKind::HornedFalcon,
        PieceKind::SoaringEagle,
    ] {
        let power_piece = match kind {
            PieceKind::Lion => PieceCode::new(Color::Black, kind).unwrap(),
            _ => PieceCode::new_promoted(Color::Black, kind).unwrap(),
        };
        let target = if kind == PieceKind::SoaringEagle {
            sq(6, 6)
        } else {
            sq(5, 6)
        };
        let start = position_from_codes(
            Color::Black,
            &[
                (
                    sq(0, 0),
                    PieceCode::new(Color::Black, PieceKind::King).unwrap(),
                ),
                (
                    sq(11, 11),
                    PieceCode::new(Color::White, PieceKind::King).unwrap(),
                ),
                (sq(5, 5), power_piece),
                (
                    target,
                    PieceCode::new(Color::White, PieceKind::Pawn).unwrap(),
                ),
            ],
        );
        let mut game = Game::from_position(rules, start);
        let moves = sorted_legal_moves(&game).unwrap();
        assert!(
            moves
                .iter()
                .any(|m| m.from == sq(5, 5) && m.mid.is_some() && m.to != m.from),
            "two stages {kind:?}"
        );
        assert!(
            moves
                .iter()
                .any(|m| m.from == sq(5, 5) && m.mid.is_some() && m.to == m.from),
            "igui {kind:?}"
        );
        // RULES.md第3条第13項のじっとは、経由できる空升を要する。
        // 角鷹・飛鷲は第11条第6項〜第8項、獅子は第12条第9項・第10項に従う。
        // 捕獲用の駒を外した別局面で、3駒種それぞれのじっとを検査する。
        let mut builder = minase_core::PositionBuilder::new(Color::Black);
        for square in minase_core::Square::all().filter(|s| *s != target) {
            if let Some(piece) = game.position().piece_at(square) {
                builder.put(square, piece).unwrap();
            }
        }
        let passes =
            sorted_legal_moves(&Game::from_position(rules, builder.finish().unwrap())).unwrap();
        assert!(passes.windows(2).all(|w| key(w[0]) < key(w[1])));
        assert!(
            passes
                .iter()
                .any(|m| m.from == sq(5, 5) && m.mid.is_none() && m.to == m.from),
            "pass {kind:?}"
        );
        for _ in 0..100 {
            let moves = sorted_legal_moves(&game).unwrap();
            assert!(moves.windows(2).all(|w| key(w[0]) < key(w[1])));
            if moves.is_empty() {
                break;
            }
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            game.play(moves[random as usize % moves.len()]).unwrap();
        }
    }
}

#[test]
fn mcts_fixed_part_and_candidates_use_the_eleven_byte_layout() {
    let h = header(TeacherKind::Mcts);
    let moves = sorted_legal_moves(&Game::new(h.rules())).unwrap();
    let r = record(
        0,
        Position::initial(),
        vec![
            Entry::new(
                None,
                Value::Mcts(0x1234),
                Exclusion::Tactical,
                Policy::Candidates(vec![(moves[1], u16::MAX)]),
                Some(0x1234_5678),
            )
            .unwrap(),
        ],
    );
    let data = bytes(h, &[r]);
    assert_eq!(
        &data[E..],
        &[
            255, 255, 0x34, 0x12, 2, 1, 0, 0x78, 0x56, 0x34, 0x12, 1, 0, 255, 255
        ]
    );
    for cut in E..data.len() {
        let expected = if cut < E + 11 {
            Section::Entry
        } else {
            Section::Candidate
        };
        assert!(matches!(decode(&data[..cut]), Err(Error::Truncated(s)) if s == expected));
    }
}

#[test]
fn malformed_inputs_never_panic_and_readers_stop_after_failure() {
    let valid = simple_file();
    for offset in 0..valid.len() {
        for byte in [0, 1, 127, 128, 255] {
            let mut data = valid.clone();
            data[offset] = byte;
            let result = std::panic::catch_unwind(|| decode(&data));
            assert!(result.is_ok(), "offset {offset}, value {byte}");
        }
    }
    let mut invalid = valid;
    invalid[E + 4] = 128;
    let mut reader = Reader::new(invalid.as_slice()).unwrap();
    assert!(matches!(
        reader.read_game(),
        Err(Error::InvalidEntryFlags(128))
    ));
    assert!(matches!(reader.read_game(), Err(Error::StreamFailed)));
}

#[test]
fn partial_output_failure_cannot_be_finished_as_a_valid_file() {
    use std::io::{self, Seek, SeekFrom, Write};
    struct ShortOutput(Cursor<Vec<u8>>);
    impl Write for ShortOutput {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.0.position() >= 155 {
                return Err(io::Error::other("injected output error"));
            }
            let remaining = (155 - self.0.position()) as usize;
            self.0.write(&buf[..buf.len().min(remaining)])
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Seek for ShortOutput {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            self.0.seek(from)
        }
    }
    let r = record(0, Position::initial(), vec![entry(None, Policy::None)]);
    let mut writer = Writer::new(
        ShortOutput(Cursor::new(vec![])),
        header(TeacherKind::AlphaBeta),
    )
    .unwrap();
    assert!(matches!(writer.write_game(&r), Err(Error::Io(_))));
    assert!(matches!(writer.write_game(&r), Err(Error::StreamFailed)));
    assert!(matches!(writer.finish(), Err(Error::StreamFailed)));
    assert!(matches!(
        Writer::new(Cursor::new(vec![0]), header(TeacherKind::AlphaBeta)),
        Err(Error::NonemptyOutput)
    ));
}

#[test]
fn board_codes_round_trip_all_piece_states_colors_and_deferred_promotions() {
    use minase_core::PositionBuilder;

    // MNSDの駒コード式と、成り権保留を含む局面の保存契約を検証する。
    for color in Color::ALL {
        let square = sq(5, if color == Color::Black { 11 } else { 0 });
        for kind in PieceKind::ALL {
            for promoted in [false, true] {
                let piece = if promoted {
                    PieceCode::new_promoted(color, kind)
                } else {
                    PieceCode::new(color, kind)
                };
                let Some(piece) = piece else { continue };
                for deferred in [false, true] {
                    if deferred && (promoted || !kind.can_promote()) {
                        continue;
                    }
                    for side in Color::ALL {
                        let mut builder = PositionBuilder::new(side);
                        builder.put(square, piece).unwrap();
                        if deferred {
                            builder.mark_promotion_deferred(square).unwrap();
                        }
                        let position = builder.finish().unwrap();
                        let codes = encode_board(&position).unwrap();
                        assert_eq!(
                            codes[square.dense_index()],
                            1 + kind.index() as u8
                                + 29 * u8::from(promoted)
                                + 64 * color as u8
                                + 128 * u8::from(deferred)
                        );
                        assert_eq!(codes.iter().filter(|&&code| code != 0).count(), 1);
                        assert_eq!(
                            decode_position(&codes, side, None, false).unwrap(),
                            position
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn board_decoder_checks_deferred_and_lion_flags() {
    use minase_core::PositionBuilder;

    let mut codes = [0; 144];
    codes[0] = 63;
    assert!(matches!(
        decode_position(&codes, Color::Black, None, false),
        Err(Error::InvalidPieceCode(63))
    ));
    codes[0] = 128;
    assert!(matches!(
        decode_position(&codes, Color::Black, None, false),
        Err(Error::PositionBuild(_))
    ));
    codes[0] = 0;
    assert!(matches!(
        decode_position(&codes, Color::Black, None, true),
        Err(Error::LionPromotionWithoutSquare)
    ));
    for side in Color::ALL {
        for promoted in [false, true] {
            let square = sq(6, 6);
            let mut builder = PositionBuilder::new(side);
            let piece = if promoted {
                PieceCode::new_promoted(side.opposite(), PieceKind::Lion)
            } else {
                PieceCode::new(side.opposite(), PieceKind::Pawn)
            }
            .unwrap();
            builder.put(square, piece).unwrap();
            let mut position = builder.finish().unwrap();
            position.set_lion_capture(Some(square)).unwrap();
            let codes = encode_board(&position).unwrap();
            assert_eq!(
                decode_position(&codes, side, Some(square), promoted).unwrap(),
                position
            );
            assert!(matches!(
                decode_position(&codes, side, Some(square), !promoted),
                Err(Error::LionPromotionMismatch)
            ));
            assert!(matches!(
                decode_position(&codes, side.opposite(), Some(square), promoted),
                Err(Error::Position(_))
            ));
        }
    }
}
