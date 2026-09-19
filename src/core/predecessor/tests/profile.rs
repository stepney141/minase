//! 設計書「検証 > 性能測定」の固定コーパス。

use super::*;
use crate::core::predecessor::{membership, reverse};
use crate::notation::sfen::{SetupPosition, parse_extended_sfen, to_extended_sfen};
use crate::notation::usi;
use crate::test_util::bench_positions;
use crate::{RuleCode, RuleCode::*, Rules};
use std::time::Instant;

struct Fixture {
    name: &'static str,
    codes: &'static [RuleCode],
    sfen: &'static str,
}

const STANDARD: &[RuleCode] = &[L0, P0, R1, E0];
const CORPUS: &[Fixture] = &[
    Fixture {
        name: "initial_two_plies",
        codes: STANDARD,
        sfen: "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/1ppppppppppp/p2i4i3/12/12/P2I4I3/1PPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL b - 3 -",
    },
    Fixture {
        name: "one_capture",
        codes: STANDARD,
        sfen: "l1c1gekgsc1l/a2st2t1b1a/1vr1dq1dhrv1/mf1p1poxp1fm/pppih1ppippp/4pn6/1P1N8/P1PIPPPPbPPP/MF1PX1C1P3/2RHD1Q1HRVM/ACB1TODT1B1A/LV1SGKEGS1FL b - 2 -",
    },
    Fixture {
        name: "two_lion_captures",
        codes: STANDARD,
        sfen: "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/1pppp1Nppppp/3i4i3/12/p11/3I4I3/PPPPPPPPPPPP/MVRHD1QDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL w - 8 -",
    },
    Fixture {
        name: "just_promoted",
        codes: STANDARD,
        sfen: "l1c1gekgsc1l/a1bst2t1b1a/1vr1dq1dhrv1/mf1p1poxp1fm/pppi2ppippp/4pn6/1P1N8/P1PIPPPPIPPP/+hF1PX1C1P3/2RHD1Q1HRVM/ACB1TODT1B1A/LV1SGKEGS1FL b - 2 -",
    },
    Fixture {
        name: "lion_record",
        codes: STANDARD,
        sfen: "l1c1gekgsc1l/a1bst2t1b1a/1vr1dq1dhrv1/mf1p1poxp1fm/pppi2ppippp/4pn6/1Ph9/P1PIPPPPIPPP/MF1PX1C1P3/2RHD1Q1HRVM/ACB1TODT1B1A/LV1SGKEGS1FL b 10g 4 -",
    },
    Fixture {
        name: "p1_deferred",
        codes: &[L0, P1, R1, E0],
        sfen: "l1c1gekgsc1l/a1bst2t1b1a/1vr1dq1dhrv1/mf1p1poxp1fm/pppi2ppippp/4pn6/1P1N8/P1PIPPPPIPPP/hF1PX1C1P3/2RHD1Q1HRVM/ACB1TODT1B1A/LV1SGKEGS1FL b - 2 12i",
    },
    Fixture {
        name: "p2_waiting",
        codes: &[L0, P2, R1, E0],
        sfen: "l1c1gekgsc1l/a1bst2t1b1a/1vr1dq1dhrv1/mf1p1poxp1fm/pppi2ppippp/4pn6/1P1N8/P1PIPPPPIPPP/hF1PX1C1P3/2RHD1Q1HRVM/ACB1TODT1B1A/LV1SGKEGS1FL b - 2 12i",
    },
    Fixture {
        name: "p5_pawn_deferred",
        codes: &[L0, P0, P5, R1, E0],
        sfen: "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/Pppppp1ppppp/3i2p1i3/12/12/3I4I3/1PPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL w - 10 12d",
    },
    Fixture {
        name: "sparse",
        codes: STANDARD,
        sfen: "11k/12/12/12/12/5G6/2p9/12/12/12/12/K11 w - 2 -",
    },
    Fixture {
        name: "sparse_two_lion_captures",
        codes: STANDARD,
        sfen: "11k/12/12/12/12/12/7N4/12/12/12/12/K11 w - 2 -",
    },
];

fn parse(fixture: &Fixture) -> (MoveRules, Position) {
    let rules = Rules::from_codes(fixture.codes).unwrap().moves;
    let (mut position, record, _) = parse_extended_sfen(fixture.sfen, rules)
        .unwrap()
        .into_parts();
    position.set_lion_capture(record).unwrap();
    (rules, position)
}

// 着手列は拡張USIで固定し、各局面のMoveGeneratorが生成した合法手から選ぶ。
fn documented_move(p: &Position, text: &str, forward: &MoveGenerator) -> Move {
    let requested = usi::parse(p, text).unwrap();
    let mut moves = Vec::new();
    forward.generate_moves(p, &mut moves);
    moves
        .into_iter()
        .find(|&mv| mv == requested)
        .unwrap_or_else(|| panic!("documented move {text} must be legal"))
}

#[test]
fn predecessor_profile_corpus_has_documented_transitions() {
    // 設計書「性能測定」: 捕獲・成り・一時状態の分類と、実戦相当の在庫を保つ。
    let bench = bench_positions();
    for fixture in CORPUS {
        let (rules, expected) = parse(fixture);
        let forward = MoveGenerator::new(rules);
        // bench[9]はBENCH_SFENSの10番目。92枚が残る局面を起点とし、手数は1から数える。
        let (mut p, line): (Position, &[&str]) = match fixture.name {
            "initial_two_plies" => (Position::initial(), &["12i12h", "12d12e"]),
            "one_capture" => (bench[9].clone(), &["10b4h"]),
            "two_lion_captures" => (
                Position::initial(),
                &[
                    "7j7h", "12d12e", "7h7f", "12e12f", "7f7e", "12f12g", "7e7d6d",
                ],
            ),
            "just_promoted" => (bench[9].clone(), &["8e12i+"]),
            "lion_record" => (bench[9].clone(), &["8e9f", "9g10g", "9f10g"]),
            "p1_deferred" | "p2_waiting" => (bench[9].clone(), &["8e12i"]),
            "p5_pawn_deferred" => (
                Position::initial(),
                // 後手は6筋の歩兵を進めて6dを空け、以後は獅子のじっとを繰り返す。
                &[
                    "12i12h", "6d6e", "12h12g", "6c6d6c", "12g12f", "6c6d6c", "12f12e", "6c6d6c",
                    "12e12d",
                ],
            ),
            "sparse" => (
                position(
                    Color::Black,
                    &[
                        (sq(0, 0), Color::Black, PieceKind::King),
                        (sq(11, 11), Color::White, PieceKind::King),
                        (sq(5, 5), Color::Black, PieceKind::GoldGeneral),
                        (sq(2, 5), Color::White, PieceKind::Pawn),
                    ],
                ),
                &["7g7f"],
            ),
            "sparse_two_lion_captures" => (
                position(
                    Color::Black,
                    &[
                        (sq(0, 0), Color::Black, PieceKind::King),
                        (sq(11, 11), Color::White, PieceKind::King),
                        (sq(5, 5), Color::Black, PieceKind::Lion),
                        (sq(6, 5), Color::White, PieceKind::Pawn),
                        (sq(7, 5), Color::White, PieceKind::Pawn),
                    ],
                ),
                &["7g6g5g"],
            ),
            _ => unreachable!("all corpus entries have a forward witness"),
        };
        let (last, prefix) = line.split_last().unwrap();
        for text in prefix {
            let mv = documented_move(&p, text, &forward);
            p.make_move_unchecked(mv, rules);
        }
        let mv = documented_move(&p, last, &forward);
        let before = p.clone();
        p.make_move_unchecked(mv, rules);
        let captured = before.occupied().popcount() - p.occupied().popcount();
        let mover = before.piece_at(mv.from).unwrap();
        match fixture.name {
            "one_capture" => {
                assert_ne!(mover.kind(), Some(PieceKind::Lion));
                assert_eq!(captured, 1);
            }
            "two_lion_captures" | "sparse_two_lion_captures" => {
                assert_eq!(mover.kind(), Some(PieceKind::Lion));
                assert!(mv.mid.is_some());
                assert_eq!(captured, 2);
            }
            "just_promoted" => {
                assert!(!mover.is_promoted() && mv.promote);
                assert!(p.piece_at(mv.to).unwrap().is_promoted());
            }
            "lion_record" => {
                assert_ne!(mover.kind(), Some(PieceKind::Lion));
                assert_eq!(
                    before.piece_at(mv.to).unwrap().kind(),
                    Some(PieceKind::Lion)
                );
                assert_eq!(p.lion_capture_square(), Some(mv.to));
                assert_eq!(captured, 1);
            }
            "p1_deferred" | "p2_waiting" | "p5_pawn_deferred" => {
                assert!(!mv.promote);
                let in_enemy_zone = |s: Square| match before.side_to_move() {
                    Color::Black => s.rank() >= 8,
                    Color::White => s.rank() <= 3,
                };
                assert!(!in_enemy_zone(mv.from) && in_enemy_zone(mv.to));
                assert_eq!(p.promotion_deferred().popcount(), 1);
                assert!(p.promotion_deferred().contains(mv.to));
                if fixture.name == "p5_pawn_deferred" {
                    assert_eq!(mover.kind(), Some(PieceKind::Pawn));
                }
            }
            "initial_two_plies" | "sparse" => assert_eq!(captured, 0),
            _ => unreachable!("all corpus entries have a classification"),
        }
        if !fixture.name.starts_with("sparse") {
            // 初期在庫92枚との差は最大2枚。由来ごとの在庫上限も別途確認する。
            assert!(p.occupied().popcount() >= 90, "{}", fixture.name);
            assert_admissible(rules, &p);
        }
        assert_eq!(p, expected, "{}", fixture.name);
        let setup =
            SetupPosition::new(p.clone(), p.lion_capture_square(), line.len() as u32 + 1).unwrap();
        assert_eq!(to_extended_sfen(&setup), fixture.sfen, "{}", fixture.name);
    }
}

#[test]
#[ignore = "releaseビルドで固定コーパスを21回測定する"]
fn predecessor_profile() {
    // 設計書「性能測定」: 候補数・再適用数・採用数と21回の中央値・p95を記録する。
    println!("position\trules\tcandidates\treplays\taccepted\tratio\tmedian_ms\tp95_ms");
    for fixture in CORPUS {
        let (rules, target) = parse(fixture);
        let forward = MoveGenerator::new(rules);
        let generator = PredecessorGenerator::new(rules);
        let mut constructed = 0;
        let mut replays = 0_usize;
        let accepted = reverse::verify_candidates(&forward, &target, |visit| {
            constructed = reverse::enumerate_candidates(&forward, &target, |mv, candidate| {
                // 列挙中に集合Aを検査済みなので、各組がちょうど1回再適用される。
                assert!(membership(rules, &candidate).is_ok());
                replays += 1;
                visit(mv, candidate);
            });
        });
        assert!(constructed >= replays);
        assert!(!accepted.is_empty());
        let expected: HashSet<_> = accepted.into_iter().collect();
        let mut times = Vec::new();
        for _ in 0..21 {
            let start = Instant::now();
            let result = generator.generate_predecessors(&target).unwrap();
            times.push(start.elapsed());
            assert_eq!(result.into_iter().collect::<HashSet<_>>(), expected);
        }
        times.sort_unstable();
        let codes = fixture
            .codes
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        println!(
            "{}\t{}\t{}\t{}\t{}\t{:.3}\t{:.3}\t{:.3}",
            fixture.name,
            codes,
            constructed,
            replays,
            expected.len(),
            constructed as f64 / expected.len() as f64,
            times[10].as_secs_f64() * 1000.0,
            times[19].as_secs_f64() * 1000.0
        );
    }
}
