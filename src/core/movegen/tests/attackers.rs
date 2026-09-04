//! `Position::attackers_to`の契約テスト。
//!
//! 期待値は`docs/plans/strength-stage3.md`「升への疑似利き集合」が定める、
//! 既存の駒別利きとの同値関係から導く。

use core::num::NonZeroU64;

use super::MoveGenerator;
use crate::core::bitboard::Bitboard;
use crate::core::movegen::piece_control_with_occupancy;
use crate::core::piece::{Color, PieceCode, PieceKind};
use crate::core::position::Position;
use crate::core::rules::MoveRules;
use crate::core::square::Square;
use crate::parse_sfen;
use crate::rng::XorShift64;
use crate::test_util::{position_from_codes, sq};

const BENCH_SFENS: &[&str] = &[
    "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL b",
    "lfcsgekgsc1l/a1b1txot1bfa/mvrhd1d4r/1p1p1p2+INpp/p1pi2pp4/4p1n5/9P2/3IP2P4/PPPP1PP1POPP/MVR1D2D1RVM/A1B1T1XT1B1A/LFCSGKEGSCFL b",
    "lfcsgekg1c1l/a1b1txo2s1a/mvrh1d6/1p1p1p2+IN+Op/p2i2ppB3/2p1p1n5/9P2/3IP2P4/PPPP1PP1P1PP/MVR1D2D1RVM/A3T1XT1B1A/LFCSGKEGSCFL b",
    "1q1b2ko3l/1d1h1ett1b1a/1dm7r1/1+g1psn1xpmv1/2pisghcip2/3+p5fpp/6N5/2P3PH4/4R2TMV2/3T1DBDG3/2H2OXS3A/Q2SGKE1FC1L b",
    "3+s1ok5/5ett4/12/3ps1x+bpm1+r/3i1g1cip+p1/4+b+pP5/4n2N4/7E4/12/12/4S+VD5/5KS1FC2 w",
    "2q1gekog3/4tddtsb2/fvrbs3h3/2mp1c1xp1v1/1ppip1ppi2m/5p6/3I5p2/1P2P1PPI3/1MPP4PN2/2VRDQD1+c3/2BHTOXR4/1FCSGKEGS1n1 b",
    "4gekog3/5d1ts3/1v1t8/2mp1c2p3/Xf1ipsppmx2/1pp9/3I8/1P2PpPN4/1MPP8/2V1OGHE3+d/R3T7/1FCSGK2S1+v+r b",
    "1hcs1ek3+P1/1f2gxgq4/m1rd1t2ton1/1v1pp3p1s1/2p3N5/1p3p2X3/3I4H3/1PP1PP2IQ2/L2P4P2+L/VC4D3V1/2F1TO1TC3/3SGKEGS1F1 b",
    "2cs1keh4/1f4g3o1/m2dtg4+H1/1v1pp7/11O/1pp9/3I1pQ4+V/1PPPPP2I3/L7P3/VC4+L5/2F1TEGT1F2/3SGK2S3 w",
    "l1c1gekgsc1l/a1bst2t1b1a/1vr1dq1dhrv1/mf1p1poxp1fm/pppih1ppippp/4pn6/1P1N8/P1PIPPPPIPPP/MF1PX1C1P3/2RHD1Q1HRVM/ACB1TODT1B1A/LV1SGKEGS1FL w",
    "l1c1gekgsc1l/3st2t1b1a/1vr1dq1dhrv1/a2p1poxp1fm/1p2h1ppippp/p4n6/1PFbf7/PN3PPPIPPP/MC1P1OC1P3/2RHD1Q1HRVM/A1B1T1DT1B1A/LV1SGKEGS1FL w",
    "l3g1k4l/a2stegt3a/m7d2m/p2+I+D+P+S4p/1F9h/10s1/P8f2/3P1PP2nc1/2R8P/4CTG1TF1A/A5XE1C2/L4K2Q2L w",
    "l5k4l/a2sge1t3a/12/p4+P5p/3+f3m2E1/1+I1+F5T2/P2C2P5/3P8/11P/11A/A3K7/L10L w",
    "+l+d4k4v/4exqhob2/1+rr5mc2/3tt5f1/5ppNp1p1/1m7p2/5P4P1/4O2Pi3/2+a3P3M1/3T2GDR2F/2S2GXEHV2/2Q3K1SC2 w",
    "4+l1k2+b1v/2+d3e5/6+m5/2+r9/8pNp1/5p3p2/6p3P1/2o9/5G6/6E4F/8+R1VC/6QKS3 b",
];

fn assert_attackers_match_piece_controls(position: &Position, occupied: Bitboard) {
    for target in Square::all() {
        let attackers = position.attackers_to(target, occupied);
        for from in position.occupied() {
            let piece = position
                .piece_at(from)
                .expect("occupied square has a piece");
            let expected = occupied.contains(from)
                && piece_control_with_occupancy(
                    occupied,
                    piece.color().expect("board piece has a color"),
                    piece.kind().expect("board piece has a kind"),
                    from,
                )
                .contains(target);
            assert_eq!(
                attackers.contains(from),
                expected,
                "from={from:?}, target={target:?}, occupied={occupied:?}"
            );
        }
    }
}

fn sampled_random_positions() -> Vec<Position> {
    let generator = MoveGenerator::standard();
    let mut positions = Vec::new();
    for seed in [0x5345_452d_4154_4b31_u64, 0x5345_452d_4154_4b32] {
        let mut rng = XorShift64::new(NonZeroU64::new(seed).unwrap());
        let mut position = Position::initial();
        for ply in 0..64 {
            if ply % 4 == 0 {
                positions.push(position.clone());
            }
            let mut moves = Vec::new();
            generator.generate_moves(&position, &mut moves);
            if moves.is_empty() {
                break;
            }
            let mv = moves[rng.next() as usize % moves.len()];
            position.make_move_unchecked(mv, MoveRules::standard());
        }
    }
    positions
}

// strength-stage3.md「升への疑似利き集合」: 初期局面、benchの14局面、および
// 一様ランダム対局の数十局面で、逆引き集合と駒別利きの同値を全升で固定する。
#[test]
fn attackers_to_matches_piece_control_on_reference_positions() {
    let positions = BENCH_SFENS
        .iter()
        .map(|sfen| parse_sfen(sfen).expect("bench SFEN is valid"))
        .chain(sampled_random_positions());

    for position in positions {
        let occupied = position.occupied();
        assert_attackers_match_piece_controls(&position, occupied);

        if let Some(removed) = occupied.lsb() {
            let mut without_one = occupied;
            without_one.clear(removed);
            assert_attackers_match_piece_controls(&position, without_one);
        }

        let mut without_several = occupied;
        for (index, removed) in occupied.into_iter().enumerate() {
            if index % 5 == 0 {
                without_several.clear(removed);
            }
        }
        assert_attackers_match_piece_controls(&position, without_several);
    }
}

// strength-stage3.md「升への疑似利き集合」: 仮想盤面から除いた駒は攻撃駒として
// 再出現せず、走り駒の後ろの利きは遮蔽物の除去によって開通する。
#[test]
fn attackers_to_respects_removed_pieces_and_opens_xrays() {
    let rook = sq(2, 2);
    let first_blocker = sq(2, 4);
    let second_blocker = sq(2, 6);
    let target = sq(2, 8);
    let position = position_from_codes(
        Color::Black,
        &[
            (rook, PieceCode::new(Color::Black, PieceKind::Rook).unwrap()),
            (
                first_blocker,
                PieceCode::new(Color::White, PieceKind::Pawn).unwrap(),
            ),
            (
                second_blocker,
                PieceCode::new(Color::White, PieceKind::Pawn).unwrap(),
            ),
        ],
    );

    let mut without_first = position.occupied();
    without_first.clear(first_blocker);
    assert_attackers_match_piece_controls(&position, without_first);
    assert!(!position.attackers_to(target, without_first).contains(rook));
    assert!(
        !position
            .attackers_to(target, without_first)
            .contains(first_blocker)
    );

    let mut without_both = without_first;
    without_both.clear(second_blocker);
    assert_attackers_match_piece_controls(&position, without_both);
    assert!(position.attackers_to(target, without_both).contains(rook));
    assert!(
        !position
            .attackers_to(target, without_both)
            .contains(second_blocker)
    );

    let lion = sq(7, 7);
    let lion_target = sq(9, 9);
    let special = position_from_codes(
        Color::Black,
        &[(lion, PieceCode::new(Color::Black, PieceKind::Lion).unwrap())],
    );
    let mut without_lion = special.occupied();
    without_lion.clear(lion);
    assert_attackers_match_piece_controls(&special, without_lion);
    assert!(
        !special
            .attackers_to(lion_target, without_lion)
            .contains(lion)
    );
}
