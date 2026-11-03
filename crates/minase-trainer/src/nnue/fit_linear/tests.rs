//! 指示書の固定例、合成結果および実ファイルから結果回帰の契約を検証する。

use super::*;
use minase::{
    eval::{
        features::{BOARD_FEATURE_COUNT, PIECE_STATE_COUNT},
        nnue::{EncodeInput, Nnue, feature_set::P, head::DirectScrelu},
    },
    training::records::{
        Entry, Exclusion, GameRecord, Header, Outcome as GameOutcome, Policy, TeacherKind, Value,
        Writer, sorted_legal_moves,
    },
};
use minase_core::{Game, GameResult, PieceCode, PieceKind, PositionBuilder, WinReason};
use sha2::{Digest, Sha256};
use std::{
    io::Cursor,
    sync::atomic::{AtomicU64, Ordering},
};

fn sq(file: u8, rank: u8) -> Square {
    Square::new(file, rank).unwrap()
}

fn piece(side: Color, kind: PieceKind) -> PieceCode {
    PieceCode::new(side, kind).unwrap()
}

fn position_from_codes(side: Color, pieces: &[(Square, PieceCode)]) -> Position {
    let mut builder = PositionBuilder::new(side);
    for &(square, piece) in pieces {
        builder.put(square, piece).unwrap();
    }
    builder.finish().unwrap()
}

fn with_side(position: &Position, side: Color) -> Position {
    position_from_codes(
        side,
        &Square::all()
            .filter_map(|s| position.piece_at(s).map(|p| (s, p)))
            .collect::<Vec<_>>(),
    )
}

fn sample(position: &Position, game: u32, ply: u16, outcome: Outcome) -> Sample {
    Sample::from_position(position, 0, outcome, game, ply, Origin::RandomSelfplay).unwrap()
}

fn encode(linear: &[[i16; 2]]) -> Vec<u8> {
    // 盤上に現れる非王駒を100、王と太子を最大非王駒+歩=200とする。
    let mut values = [100; PIECE_STATE_COUNT];
    values[11] = 200;
    values[21] = 200;
    let head = DirectScrelu::new(2, &[5, 5, -5, -5], 0, true).unwrap();
    nnue::encode::<P, _>(&EncodeInput {
        width: 2,
        cp_per_logit: 200.0,
        rule_set: "engine-default",
        piece_values: &values,
        linear,
        weights: &vec![1; FEATURE_COUNT * 2],
        biases: &[2, 2],
        head: &head,
    })
    .unwrap()
}

fn base_bytes() -> Vec<u8> {
    let linear: Vec<_> = (0..FEATURE_COUNT)
        .map(|f| {
            let coordinate = (f / 12) * 6 + (f % 12).min(11 - f % 12);
            [(coordinate % 23) as i16 - 11, (coordinate % 17) as i16 - 8]
        })
        .collect();
    encode(&linear)
}

fn nonzero_coefficients(exact: bool) -> Vec<f64> {
    (0..PARAMETERS)
        .map(|i| ((i * 37 % 61) as f64 - 30.0) * if exact { 0.125 } else { 0.137 })
        .collect()
}

#[test]
fn feature_counts_match_fixed_owner_rank_and_file_example() {
    use PieceKind::*;
    let pieces = [
        (sq(0, 0), piece(Color::Black, FreeKing)),
        (sq(5, 8), piece(Color::Black, DragonHorse)),
        (sq(6, 0), piece(Color::Black, King)),
        (
            sq(1, 1),
            PieceCode::new_promoted(Color::Black, GoldGeneral).unwrap(),
        ),
        (sq(3, 11), piece(Color::White, FreeKing)),
        (
            sq(6, 6),
            PieceCode::new_promoted(Color::White, Lion).unwrap(),
        ),
        (sq(6, 11), piece(Color::White, King)),
    ];
    // 指示書のxは段番号1始まり、内部配列だけ0始まりとする。
    let expected = [
        (2 * 7 + 5, -1.0),
        (11 * 7 + 6, 1.0),
        (147 + 2, -1.0),
        (147 + 5, 1.0),
        (147 + 2 * 6, -1.0),
        (147 + 11 * 6, 1.0),
    ];
    for side in Color::ALL {
        let features = Features::from_position(&position_from_codes(side, &pieces));
        assert_eq!(features.q, 5.0);
        let sign = if side == Color::Black { 1.0 } else { -1.0 };
        let mut expected: Vec<_> = expected.iter().map(|&(i, x)| (i, sign * x)).collect();
        expected.sort_by_key(|&(i, _)| i);
        assert_eq!(features.counts, expected);
    }
}

fn fixed_calibration(k: f64) -> Calibration {
    Calibration {
        cp_per_logit: [k; 5],
        side_intercept: SideIntercept {
            black: 0.0,
            white: 0.0,
        },
    }
}

fn synthetic_games(game_count: u32) -> (Vec<SelectedGame>, Vec<f64>) {
    let mut true_deltas = Vec::new();
    let position = Position::initial();
    let games = (0..game_count)
        .map(|game| {
            let mut counts = Vec::new();
            let mut delta = 0.0;
            for kind in 0..2 {
                let rank = (hash64(101 + kind as u64, game) % 7) as usize;
                let file = (hash64(201 + kind as u64, game) % 6) as usize;
                let difference = (hash64(301 + kind as u64, game) % 3) as i32 - 1;
                if difference != 0 {
                    counts.push((kind * 7 + rank, f64::from(difference)));
                    counts.push((147 + kind * 6 + file, f64::from(difference)));
                }
                // 真の信号はAの2段とc=1で各+30、Bの5段とc=3で各−30 cp。
                delta += f64::from(difference)
                    * if kind == 0 {
                        30.0 * (usize::from(rank == 1) + usize::from(file == 1)) as f64
                    } else {
                        -30.0 * (usize::from(rank == 4) + usize::from(file == 3)) as f64
                    };
            }
            counts.sort_by_key(|&(i, _)| i);
            let probability = 1.0 / (1.0 + (-delta / 200.0).exp());
            true_deltas.push(delta);
            let uniform = (hash64(401, game) >> 11) as f64 / (1_u64 << 53) as f64;
            let outcome = if uniform < probability {
                Outcome::Win
            } else {
                Outcome::Loss
            };
            SelectedGame {
                file: 0,
                game,
                positions: vec![SelectedPosition {
                    sample: sample(&position, game, 60, outcome),
                    evaluation: 0,
                    features: Features { counts, q: 90.0 },
                }],
            }
        })
        .collect();
    (games, true_deltas)
}

#[test]
fn known_rank_plus_file_effects_are_recovered() {
    // 指示書2: 200,000局のベルヌーイ結果からa+bとδを回復する。
    let (games, true_deltas) = synthetic_games(200_000);
    let calibration = fixed_calibration(200.0);
    let theta = fit(&games, &calibration, 1000.0).unwrap();
    let a = theta[1] + theta[148];
    let b = theta[11] + theta[156];
    let error = games
        .iter()
        .zip(true_deltas)
        .map(|(g, delta)| {
            let f = &g.positions[0].features;
            (f.correction(&theta) - delta).abs()
        })
        .sum::<f64>()
        / games.len() as f64;
    println!("A: a+b={a} cp, B: a+b={b} cp, mean absolute delta error={error} cp");
    assert!(error <= 15.0, "mean absolute delta error: {error}");
    assert!(a > 0.0 && (a - 60.0).abs() <= 15.0, "A: {a}");
    assert!(b < 0.0 && (b + 60.0).abs() <= 15.0, "B: {b}");
}

#[test]
fn regression_accounts_for_evaluation_side_intercept_phase_and_draws() {
    let calibration = Calibration {
        cp_per_logit: [200.0; 5],
        side_intercept: SideIntercept {
            black: 0.2,
            white: -0.4,
        },
    };
    let initial = Position::initial();
    let mut games = Vec::new();
    for (kind, side, q, evaluation, wins, draws) in [
        (0, Color::Black, 45.0, 100, 7_311, 0),
        (1, Color::White, 90.0, -100, 5_000, 0),
        (2, Color::Black, 90.0, 0, 4_000, 5_000),
    ] {
        let position = with_side(&initial, side);
        for i in 0..10_000 {
            let outcome = if i < wins {
                Outcome::Win
            } else if i < wins + draws {
                Outcome::Draw
            } else {
                Outcome::Loss
            };
            let game = games.len() as u32;
            games.push(SelectedGame {
                file: 0,
                game,
                positions: vec![SelectedPosition {
                    sample: sample(&position, game, 60, outcome),
                    evaluation,
                    features: Features {
                        counts: vec![(kind * 7, 1.0), (147 + kind * 6, 1.0)],
                        q,
                    },
                }],
            });
        }
    }
    let theta = fit(&games, &calibration, 1e6).unwrap();
    // 目的値の平均のロジットから、基点の評価値と手番の切片を引いてa+bを求める。
    let expected = [
        400.0 * ((7311.0_f64 / 2689.0).ln() - 0.5 - 0.2),
        200.0 * (0.0 + 0.5 + 0.4),
        200.0 * ((0.65_f64 / 0.35).ln() - 0.2),
    ];
    for (kind, expected) in expected.into_iter().enumerate() {
        let actual = theta[kind * 7] + theta[147 + kind * 6];
        assert!(
            (actual - expected).abs() <= 0.5,
            "kind={kind}: a+b={actual} cp, expected={expected} cp"
        );
    }
}

#[test]
fn tau_strictly_increases_total_squared_norm() {
    // 指示書3: 同じ合成データで正則化の単調性を確かめる。
    let (games, _) = synthetic_games(20_000);
    let calibration = fixed_calibration(200.0);
    let mut previous = 0.0;
    for tau in [5.0, 10.0, 20.0, 40.0, 80.0] {
        let norm: f64 = fit(&games, &calibration, tau)
            .unwrap_or_else(|error| panic!("tau={tau}: {error}"))
            .iter()
            .map(|x| x * x)
            .sum();
        assert!(norm > previous, "tau={tau}: {norm} <= {previous}");
        previous = norm;
    }
}

#[test]
fn symmetric_positions_have_no_features_or_integer_correction() {
    use PieceKind::*;
    let mut pieces = Vec::new();
    for (file, rank, kind) in [
        (1, 0, King),
        (2, 2, FreeKing),
        (3, 3, DragonHorse),
        (5, 4, GoldGeneral),
        (0, 1, Lion),
    ] {
        pieces.push((sq(file, rank), piece(Color::Black, kind)));
        pieces.push((sq(11 - file, 11 - rank), piece(Color::White, kind)));
    }
    let base = base_bytes();
    let original = IntegerNetwork::decode(&base).unwrap();
    let corrected =
        nnue::add_to_linear_midgame(&base, &corrections(&nonzero_coefficients(false)).unwrap())
            .unwrap();
    let corrected = IntegerNetwork::decode(&corrected).unwrap();
    for position in [
        Position::initial(),
        position_from_codes(Color::Black, &pieces),
    ] {
        for side in Color::ALL {
            let position = with_side(&position, side);
            assert!(Features::from_position(&position).counts.is_empty());
            assert_eq!(
                corrected.evaluate(&position, side),
                original.evaluate(&position, side)
            );
        }
    }
}

#[test]
fn export_changes_only_specified_midgame_rows_and_checksum() {
    let base = base_bytes();
    let mut theta = nonzero_coefficients(false);
    // +0.5と+1.5の偶数丸め、および相手行の符号も固定する。
    theta[0] = 0.0625;
    theta[1] = 0.1875;
    theta[147] = 0.0;
    let written = nnue::add_to_linear_midgame(&base, &corrections(&theta).unwrap()).unwrap();
    let states = [
        10, 11, 20, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46,
    ];
    for feature in 0..FEATURE_COUNT {
        let offset = 192 + 4 * feature;
        let old = i16::from_le_bytes(base[offset..offset + 2].try_into().unwrap());
        let new = i16::from_le_bytes(written[offset..offset + 2].try_into().unwrap());
        let expected = if feature < 2 * 47 * 144 {
            let state = feature / 144 % 47;
            match states.iter().position(|&s| s == state) {
                Some(kind) => {
                    let opponent = feature >= 47 * 144;
                    let rank = feature / 12 % 12;
                    let rank_bin = if opponent {
                        (11 - rank).min(6)
                    } else {
                        rank.min(6)
                    };
                    let file = feature % 12;
                    let c = [5, 4, 3, 2, 1, 0, 0, 1, 2, 3, 4, 5][file];
                    let rounded = (8.0 * (theta[kind * 7 + rank_bin] + theta[147 + kind * 6 + c]))
                        .round_ties_even() as i32;
                    if opponent { -rounded } else { rounded }
                }
                None => 0,
            }
        } else {
            0
        };
        assert_eq!(
            i32::from(new) - i32::from(old),
            expected,
            "feature {feature}"
        );
        assert_eq!(
            &written[offset + 2..offset + 4],
            &base[offset + 2..offset + 4]
        );
    }
    assert_eq!(&written[..144], &base[..144]);
    assert_eq!(&written[176..192], &base[176..192]);
    assert_eq!(
        &written[192 + FEATURE_COUNT * 4..],
        &base[192 + FEATURE_COUNT * 4..]
    );
    assert_eq!(
        &written[192 + BOARD_FEATURE_COUNT * 4..192 + FEATURE_COUNT * 4],
        &base[192 + BOARD_FEATURE_COUNT * 4..192 + FEATURE_COUNT * 4]
    );
    assert_eq!(
        &written[144..176],
        Sha256::digest(&written[192..]).as_slice()
    );
    Nnue::decode(&written).unwrap();
    assert!(corrections(&vec![f64::MAX; PARAMETERS]).is_err());
}

fn check_integer_evaluation_change(exact: bool) {
    let base = base_bytes();
    let original = IntegerNetwork::decode(&base).unwrap();
    let mut positions = Vec::new();
    let mut game = Game::with_default_rules();
    for ply in 0..40 {
        positions.push(game.position().clone());
        let moves = sorted_legal_moves(&game).unwrap();
        game.play(moves[(ply * 17 + 9) % moves.len()]).unwrap();
    }
    assert!(
        Color::ALL
            .iter()
            .all(|&side| positions.iter().any(|p| p.side_to_move() == side))
    );
    {
        let theta = nonzero_coefficients(exact);
        let bytes = nnue::add_to_linear_midgame(&base, &corrections(&theta).unwrap()).unwrap();
        let corrected = IntegerNetwork::decode(&bytes).unwrap();
        let mut nonzero = false;
        for position in &positions {
            let features = Features::from_position(position);
            let side = position.side_to_move();
            let difference =
                f64::from(corrected.evaluate(position, side) - original.evaluate(position, side));
            let delta = features.correction(&theta);
            nonzero |= delta.abs() > 0.1;
            let rounding_bound = if exact {
                0.0
            } else {
                features.q * f64::from(position.occupied().popcount()) * 0.5 / 720.0
            };
            let bound = rounding_bound + 2.0;
            let linear_value = |bytes: &[u8]| {
                let mut numerator = 0_i64;
                minase::eval::features::active_features(position, |feature| {
                    let offset = 192 + feature * 4;
                    let mg = i16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap());
                    let eg = i16::from_le_bytes(bytes[offset + 2..offset + 4].try_into().unwrap());
                    numerator += features.q as i64 * i64::from(mg)
                        + (90 - features.q as i64) * i64::from(eg);
                });
                numerator as f64 / 720.0
            };
            let base_linear = linear_value(&base);
            let corrected_linear = linear_value(&bytes);
            let linear_error = (corrected_linear - base_linear - delta).abs();
            if exact {
                assert!(
                    linear_error < 1e-9,
                    "linear_error={linear_error}, base_linear={base_linear}, corrected_linear={corrected_linear}, delta={delta}"
                );
            } else {
                assert!(
                    linear_error <= rounding_bound + 1e-9,
                    "linear_error={linear_error}, rounding_bound={rounding_bound}, base_linear={base_linear}, corrected_linear={corrected_linear}, delta={delta}"
                );
            }
            assert!(
                (difference - delta).abs() < bound,
                "exact={exact}, difference={difference}, delta={delta}, bound={bound}, base_linear={base_linear}, corrected_linear={corrected_linear}"
            );
        }
        assert!(nonzero);
    }
}

#[test]
fn exact_eighth_cp_correction_has_less_than_two_cp_integer_error() {
    check_integer_evaluation_change(true);
}

#[test]
fn rounded_correction_has_bounded_integer_error() {
    check_integer_evaluation_change(false);
}

fn calibration_games(score: i32, wins: usize, trials: usize) -> Vec<SelectedGame> {
    let initial = Position::initial();
    let mut games = Vec::new();
    for band in 0..5 {
        for side in Color::ALL {
            let position = with_side(&initial, side);
            for sign in [-1, 1] {
                for i in 0..trials {
                    let win = if sign > 0 { i < wins } else { i >= wins };
                    let game = games.len() as u32;
                    games.push(SelectedGame {
                        file: 0,
                        game,
                        positions: vec![SelectedPosition {
                            sample: sample(
                                &position,
                                game,
                                60 + band * 20,
                                if win { Outcome::Win } else { Outcome::Loss },
                            ),
                            evaluation: sign * score,
                            features: Features {
                                counts: Vec::new(),
                                q: 90.0,
                            },
                        }],
                    });
                }
            }
        }
    }
    games
}

#[test]
fn calibration_rejects_unrelated_and_too_sharp_scores_but_recovers_valid_scale() {
    // K = e / log(p/(1-p)): 無関係なら無限大、10/log(9)なら約4.6、200/log(.731/.269)なら約200。
    for (score, wins, trials) in [(200, 5, 10), (10, 9, 10)] {
        assert!(matches!(
            calibrate(&calibration_games(score, wins, trials)),
            Err(Error::CalibrationOutOfRange { band: 0, .. })
        ));
    }
    let calibration = calibrate(&calibration_games(200, 731, 1000)).unwrap();
    for k in calibration.cp_per_logit {
        assert!((k - 200.0).abs() <= 10.0, "K={k}");
    }
    assert!(calibration.side_intercept.black.abs() < 1e-6);
    assert!(calibration.side_intercept.white.abs() < 1e-6);
}

#[test]
fn calibration_assigns_intercepts_to_the_side_to_move() {
    let initial = Position::initial();
    let mut games = Vec::new();
    // K=200、先手の切片+0.5、後手の切片−0.5での勝率を小数点以下4桁に丸める。
    for band in 0..5 {
        for (side, positive_wins, negative_wins) in
            [(Color::Black, 8_176, 3_775), (Color::White, 6_225, 1_824)]
        {
            let position = with_side(&initial, side);
            for (evaluation, wins) in [(200, positive_wins), (-200, negative_wins)] {
                for i in 0..10_000 {
                    let game = games.len() as u32;
                    games.push(SelectedGame {
                        file: 0,
                        game,
                        positions: vec![SelectedPosition {
                            sample: sample(
                                &position,
                                game,
                                60 + band * 20,
                                if i < wins {
                                    Outcome::Win
                                } else {
                                    Outcome::Loss
                                },
                            ),
                            evaluation,
                            features: Features {
                                counts: Vec::new(),
                                q: 90.0,
                            },
                        }],
                    });
                }
            }
        }
    }
    let calibration = calibrate(&games).unwrap();
    for (band, k) in calibration.cp_per_logit.into_iter().enumerate() {
        assert!((k - 200.0).abs() <= 5.0, "band={band}: K={k}");
    }
    let black = calibration.side_intercept.black;
    let white = calibration.side_intercept.white;
    assert!((black - 0.5).abs() <= 0.02, "black intercept={black}");
    assert!((white + 0.5).abs() <= 0.02, "white intercept={white}");
}

#[test]
fn selection_obeys_ply_score_hash_limits_and_equal_game_weight() {
    let position = Position::initial();
    let hash = hash64(123, 0);
    let candidates = [59, 60, 159, 160].map(|ply| sample(&position, 0, ply, Outcome::Win));
    let selected = select_game(candidates.to_vec(), hash, |_, _| 0).unwrap();
    assert_eq!(
        selected.iter().map(|p| p.sample.ply()).collect::<Vec<_>>(),
        [60, 159]
    );
    let candidates: Vec<_> = (60..64)
        .map(|ply| sample(&position, 0, ply, Outcome::Win))
        .collect();
    let selected = select_game(candidates, hash, |s, _| {
        [3000, -3000, 2999, -2999][usize::from(s.ply() - 60)]
    })
    .unwrap();
    assert_eq!(
        selected.iter().map(|p| p.sample.ply()).collect::<Vec<_>>(),
        [62, 63]
    );
    let candidates: Vec<_> = (60..72)
        .map(|ply| sample(&position, 0, ply, Outcome::Win))
        .collect();
    let mut ranked: Vec<_> = (60_u16..72).collect();
    ranked.sort_by_key(|&ply| (hash64(hash, u32::from(ply)), ply));
    let mut expected = ranked[..5].to_vec();
    expected.sort();
    let mut evaluated = 0;
    let five = select_game(candidates, hash, |_, _| {
        evaluated += 1;
        100
    })
    .unwrap();
    assert_eq!(evaluated, 5);
    assert_eq!(
        five.iter().map(|p| p.sample.ply()).collect::<Vec<_>>(),
        expected
    );
    let one = select_game(
        vec![sample(&position, 1, 60, Outcome::Win)],
        hash,
        |_, _| 100,
    )
    .unwrap();
    let one = vec![SelectedGame {
        file: 0,
        game: 1,
        positions: one,
    }];
    let five = vec![SelectedGame {
        file: 0,
        game: 0,
        positions: five,
    }];
    let expected_loss = -(1.0 / (1.0 + (-0.5_f64).exp())).ln();
    for games in [&one, &five] {
        assert!(
            (mean_loss(games, &fixed_calibration(200.0), &[0.0; PARAMETERS]) - expected_loss).abs()
                < 1e-15
        );
        assert!(
            (games[0]
                .positions
                .iter()
                .map(|_| 1.0 / games[0].positions.len() as f64)
                .sum::<f64>()
                - 1.0)
                .abs()
                < 1e-15
        );
    }
}

#[test]
fn duplicating_positions_within_a_game_preserves_both_fits() {
    // 対局の重み1を、実際の推定で検査する。勝った対局だけ局面を5個に増やしても最適値は変わらない。
    let mut games = calibration_games(200, 7, 10);
    let before = calibrate(&games).unwrap();
    for game in &mut games {
        if game.positions[0].sample.outcome() == Outcome::Win {
            let first = &game.positions[0];
            let copies: Vec<_> = (1..5)
                .map(|_| SelectedPosition {
                    sample: first.sample.clone(),
                    evaluation: first.evaluation,
                    features: Features {
                        counts: Vec::new(),
                        q: 90.0,
                    },
                })
                .collect();
            game.positions.extend(copies);
        }
    }
    let after = calibrate(&games).unwrap();
    for (left, right) in before.cp_per_logit.into_iter().zip(after.cp_per_logit) {
        assert!((left - right).abs() < 1e-6);
    }
    assert!((before.side_intercept.black - after.side_intercept.black).abs() < 1e-6);
    assert!((before.side_intercept.white - after.side_intercept.white).abs() < 1e-6);

    let position = Position::initial();
    let games: Vec<_> = [(Outcome::Win, 5), (Outcome::Loss, 1)]
        .into_iter()
        .enumerate()
        .map(|(game, (outcome, count))| SelectedGame {
            file: 0,
            game: game as u32,
            positions: (0..count)
                .map(|i| SelectedPosition {
                    sample: sample(&position, game as u32, 60 + i, outcome),
                    evaluation: 0,
                    features: Features {
                        counts: vec![(0, 1.0), (147, 1.0)],
                        q: 90.0,
                    },
                })
                .collect(),
        })
        .collect();
    // 同じ説明変数を持つ勝ち1局・負け1局なので、正則化した最適値は全係数0。
    let theta = fit(&games, &fixed_calibration(200.0), 1000.0).unwrap();
    assert!(theta.iter().all(|x| x.abs() < 1e-6));
}

#[test]
fn command_arguments_require_every_input_except_threads() {
    use clap::Parser;
    #[derive(Parser)]
    struct Command {
        #[command(flatten)]
        args: Args,
    }
    let arguments = [
        "fit-linear",
        "--base",
        "base.bin",
        "--data",
        "one.mnsd",
        "two.mnsd",
        "--tau",
        "10",
        "20",
        "40",
        "80",
        "--seed",
        "17",
        "--output-dir",
        "output",
    ];
    let parsed = Command::try_parse_from(arguments).unwrap().args;
    assert_eq!(parsed.tau, [10.0, 20.0, 40.0, 80.0]);
    assert_eq!(
        parsed.data,
        [PathBuf::from("one.mnsd"), PathBuf::from("two.mnsd")]
    );
    assert_eq!(parsed.seed, 17);
    assert_eq!(parsed.threads, None);
    for (start, end) in [(1, 3), (3, 6), (6, 11), (11, 13), (13, 15)] {
        let mut missing = arguments.to_vec();
        missing.drain(start..end);
        assert!(Command::try_parse_from(missing).is_err());
    }
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "minase-fit-linear-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn write(&self, name: &str, seed: u64, games: &[GameRecord]) -> PathBuf {
        let header = Header::new(
            "L0,P0,R1,E0".into(),
            "1".repeat(40),
            [1; 32],
            TeacherKind::AlphaBeta,
            1000,
            seed,
        )
        .unwrap();
        let mut writer = Writer::new(Cursor::new(Vec::new()), header).unwrap();
        for game in games {
            writer.write_game(game).unwrap();
        }
        let path = self.0.join(name);
        fs::write(&path, writer.finish().unwrap().into_inner()).unwrap();
        path
    }

    fn args(&self, data: Vec<PathBuf>) -> Args {
        let path = self.0.join("base.bin");
        fs::write(&path, material_base()).unwrap();
        Args {
            base: path,
            data,
            tau: vec![10.0],
            seed: 123,
            output_dir: self.0.join("output"),
            threads: Some(1),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn material_base() -> Vec<u8> {
    // N=12、q=10、歩の枚数差±2なので、序中盤7200/8 cpが評価±200 cpを与える。
    let mut linear = vec![[0; 2]; FEATURE_COUNT];
    for square in 0..144 {
        linear[29 * 144 + square][0] = 7200;
        linear[(47 + 29) * 144 + square][0] = -7200;
    }
    encode(&linear)
}

fn material_position(side: Color, black_ahead: bool, black_signal: bool) -> Position {
    let mut pieces = vec![
        (sq(5, 0), piece(Color::Black, PieceKind::King)),
        (sq(6, 11), piece(Color::White, PieceKind::King)),
    ];
    for color in Color::ALL {
        let count = if (color == Color::Black) == black_ahead {
            6
        } else {
            4
        };
        for file in 0..count {
            let rank = if color == Color::Black {
                if file == 0 && black_signal { 3 } else { 2 }
            } else {
                9
            };
            pieces.push((sq(file, rank), piece(color, PieceKind::Pawn)));
        }
    }
    position_from_codes(side, &pieces)
}

fn split_number(seed: u64, game: u32) -> usize {
    if is_validation(seed, game) {
        2
    } else if hash64(123 ^ hash64(seed, game), 0).is_multiple_of(10) {
        1
    } else {
        0
    }
}

fn records(seed: u64, decision_count: usize) -> Vec<GameRecord> {
    let mut counts = [0; 3];
    let limits = [400, 60, decision_count];
    let mut games = Vec::new();
    for game in 0..100_000 {
        let split = split_number(seed, game);
        if counts[split] == limits[split] {
            continue;
        }
        let i = counts[split];
        counts[split] += 1;
        let band = i / 80 % 5;
        let side = if (i / 40).is_multiple_of(2) {
            Color::Black
        } else {
            Color::White
        };
        let positive = (i / 20).is_multiple_of(2);
        let win = if positive { i % 20 < 14 } else { i % 20 >= 14 };
        let winner = if win { side } else { side.opposite() };
        let position = material_position(
            side,
            positive == (side == Color::Black),
            winner == Color::Black,
        );
        games.push(
            GameRecord::new(
                game,
                Origin::RandomSelfplay,
                GameOutcome::Game(GameResult::Win {
                    winner,
                    reason: WinReason::Resignation,
                }),
                60 + band as u16 * 20,
                position,
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
        if counts == limits {
            break;
        }
    }
    assert_eq!(counts, limits);
    games
}

fn read_summary(args: &Args) -> serde_json::Value {
    serde_json::from_slice(&fs::read(args.output_dir.join("summary.json")).unwrap()).unwrap()
}

fn summary_bytes_without_output_path(args: &Args) -> Vec<u8> {
    let bytes = fs::read(args.output_dir.join("summary.json")).unwrap();
    let document = String::from_utf8(bytes).unwrap();
    let path = args.output_dir.canonicalize().unwrap().join("nnue.bin");
    let field = format!("\"path\": {}", serde_json::to_string(&path).unwrap());
    assert_eq!(document.matches(&field).count(), 1);
    // JSONを再整形せず、出力先の欄だけを同じ文字列へ置き換える。
    document
        .replacen(&field, "\"path\": \"OUTPUT\"", 1)
        .into_bytes()
}

fn reversed_results(games: &[GameRecord], seed: u64, changed_split: usize) -> Vec<GameRecord> {
    games
        .iter()
        .map(|g| {
            let outcome = if split_number(seed, g.game_number()) == changed_split {
                let GameOutcome::Game(GameResult::Win { winner, reason }) = g.outcome() else {
                    panic!("win fixture")
                };
                GameOutcome::Game(GameResult::Win {
                    winner: winner.opposite(),
                    reason,
                })
            } else {
                g.outcome()
            };
            GameRecord::new(
                g.game_number(),
                g.origin(),
                outcome,
                g.start_ply(),
                g.start_position().clone(),
                g.entries().to_vec(),
            )
            .unwrap()
        })
        .collect()
}

#[test]
fn real_mnsd_splits_are_independent_and_thread_count_preserves_output_bytes() {
    let fixture = Fixture::new();
    let seeds = [33, 11, 22];
    let records: Vec<_> = seeds.iter().map(|&seed| records(seed, 20)).collect();
    let paths: Vec<_> = seeds
        .iter()
        .zip(&records)
        .enumerate()
        .map(|(i, (&seed, games))| fixture.write(&format!("data-{}.mnsd", 2 - i), seed, games))
        .collect();
    let mut args = fixture.args(paths);
    let dataset = load(&args, &fs::read(&args.base).unwrap(), 3).unwrap();
    let mut keys = HashSet::new();
    for (part, games) in [&dataset.training, &dataset.tau_selection, &dataset.decision]
        .into_iter()
        .enumerate()
    {
        let actual: Vec<_> = games.iter().map(|g| (g.file, g.game)).collect();
        let expected: Vec<_> = records
            .iter()
            .enumerate()
            .flat_map(|(file, games)| {
                games
                    .iter()
                    .filter(move |g| split_number(seeds[file], g.game_number()) == part)
                    .map(move |g| (file, g.game_number()))
            })
            .collect();
        assert_eq!(actual, expected);
        for key in actual {
            assert!(keys.insert(key));
        }
    }
    run(args.clone()).unwrap();
    let bytes = fs::read(args.output_dir.join("nnue.bin")).unwrap();
    assert_ne!(bytes, fs::read(&args.base).unwrap());
    let summary = summary_bytes_without_output_path(&args);
    for (name, threads) in [("repeat", 1), ("parallel", 3)] {
        args.output_dir = fixture.0.join(name);
        args.threads = Some(threads);
        run(args.clone()).unwrap();
        assert_eq!(fs::read(args.output_dir.join("nnue.bin")).unwrap(), bytes);
        assert_eq!(summary_bytes_without_output_path(&args), summary);
    }
    for changed_split in [2, 1] {
        args.data = seeds
            .iter()
            .zip(&records)
            .enumerate()
            .map(|(i, (&seed, games))| {
                fixture.write(
                    &format!("changed-{changed_split}-{i}.mnsd"),
                    seed,
                    &reversed_results(games, seed, changed_split),
                )
            })
            .collect();
        args.output_dir = fixture.0.join(format!("changed-{changed_split}"));
        run(args.clone()).unwrap();
        assert_eq!(fs::read(args.output_dir.join("nnue.bin")).unwrap(), bytes);
    }
}

#[test]
fn summary_decision_losses_match_manual_bce_of_reread_integer_network() {
    let fixture = Fixture::new();
    let records = records(11, 1);
    let path = fixture.write("games.mnsd", 11, &records);
    let mut args = fixture.args(vec![path]);
    args.tau = vec![5.0, 10.0, 40.0];
    run(args.clone()).unwrap();
    let summary = read_summary(&args);
    let game = records
        .iter()
        .find(|g| is_validation(11, g.game_number()))
        .unwrap();
    let position = game.start_position();
    let side = position.side_to_move();
    let band = usize::from((game.start_ply() - 60) / 20);
    let k = summary["calibration"]["cp_per_logit"][band]
        .as_f64()
        .unwrap();
    let intercept = summary["calibration"]["side_intercept"][if side == Color::Black {
        "black"
    } else {
        "white"
    }]
    .as_f64()
    .unwrap();
    let GameOutcome::Game(result) = game.outcome() else {
        panic!("game fixture")
    };
    let target = match Outcome::from_game_result(result, side) {
        Outcome::Win => 1.0,
        Outcome::Draw => 0.5,
        Outcome::Loss => 0.0,
    };
    let loss = |path| {
        let network = IntegerNetwork::decode(&fs::read(path).unwrap()).unwrap();
        let score = network.evaluate(position, side);
        let p = 1.0 / (1.0 + (-(f64::from(score) / k + intercept)).exp());
        -target * p.ln() - (1.0 - target) * (1.0 - p).ln()
    };
    let base = loss(args.base.clone());
    let corrected = loss(args.output_dir.join("nnue.bin"));
    let decision = &summary["decision"];
    for (key, expected) in [
        ("base_loss", base),
        ("corrected_loss", corrected),
        ("d", corrected - base),
    ] {
        assert!((decision[key].as_f64().unwrap() - expected).abs() <= 1e-12);
    }
    assert_eq!(decision["games"].as_array().unwrap().len(), 1);
    assert_eq!(decision["games"][0]["base_loss"], decision["base_loss"]);
    assert_eq!(
        decision["games"][0]["corrected_loss"],
        decision["corrected_loss"]
    );
    let losses = summary["tau_losses"].as_array().unwrap();
    let best = losses
        .iter()
        .min_by(|a, b| {
            a["loss"]
                .as_f64()
                .unwrap()
                .total_cmp(&b["loss"].as_f64().unwrap())
        })
        .unwrap();
    assert_eq!(summary["selected_tau"], best["tau"]);
    assert_eq!(summary["rank_coefficients"].as_array().unwrap().len(), 21);
    assert_eq!(summary["file_coefficients"].as_array().unwrap().len(), 21);
    assert_eq!(summary["kinds"].as_array().unwrap().len(), 21);
}

#[test]
fn invalid_inputs_fail_with_specific_errors() {
    let fixture = Fixture::new();
    let records = records(11, 1);
    let path = fixture.write("games.mnsd", 11, &records);
    let args = fixture.args(vec![path.clone()]);
    let mut invalid = args.clone();
    invalid.data.push(fixture.0.join(".").join("games.mnsd"));
    assert!(matches!(run(invalid), Err(Error::DuplicatePath { .. })));
    let mut invalid = args.clone();
    invalid
        .data
        .push(fixture.write("same-seed.mnsd", 11, &records));
    assert!(matches!(
        run(invalid),
        Err(Error::DuplicateSeed { seed: 11 })
    ));
    for origin in [Origin::HumanGame, Origin::HumanOpeningSelfplay] {
        let g = &records[0];
        let human = GameRecord::new(
            g.game_number(),
            origin,
            g.outcome(),
            g.start_ply(),
            g.start_position().clone(),
            g.entries().to_vec(),
        )
        .unwrap();
        let mut invalid = args.clone();
        invalid.data = vec![fixture.write("human.mnsd", 11, &[human])];
        assert!(matches!(run(invalid), Err(Error::NonRandomSelfplay { .. })));
    }
    for value in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let mut invalid = args.clone();
        invalid.tau = vec![value];
        assert!(matches!(run(invalid), Err(Error::InvalidTau { .. })));
    }
    let mut invalid = args.clone();
    invalid.tau = vec![10.0, 10.0];
    assert!(matches!(
        run(invalid),
        Err(Error::DuplicateTau { value: 10.0 })
    ));
    let mut invalid = args.clone();
    invalid.tau.clear();
    assert!(matches!(run(invalid), Err(Error::MissingTaus)));
    let mut invalid = args.clone();
    invalid.data.clear();
    assert!(matches!(run(invalid), Err(Error::MissingData)));
    let mut invalid = args.clone();
    invalid.threads = Some(0);
    assert!(matches!(run(invalid), Err(Error::Workers(_))));
    fs::create_dir(&args.output_dir).unwrap();
    fs::write(args.output_dir.join("keep"), b"untouched").unwrap();
    assert!(matches!(
        run(args.clone()),
        Err(Error::NonEmptyOutput { .. })
    ));
    assert_eq!(
        fs::read(args.output_dir.join("keep")).unwrap(),
        b"untouched"
    );
    assert!(!args.output_dir.join("nnue.bin").exists());
}
