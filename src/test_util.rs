use core::num::NonZeroU64;

use crate::MoveGenerator;
use crate::core::piece::{Color, PieceCode, PieceKind};
use crate::core::position::{Position, PositionBuilder};
use crate::core::rules::MoveRules;
use crate::core::square::Square;
use crate::rng::XorShift64;

/// `bench`の15局面の2欄SFEN。局面別の照合標本として複数のテストが共有する。
pub(crate) const BENCH_SFENS: &[&str] = &[
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

/// `bench`の15局面を解析して返す。
pub(crate) fn bench_positions() -> Vec<Position> {
    BENCH_SFENS
        .iter()
        .map(|sfen| crate::parse_sfen(sfen).expect("bench SFEN is valid"))
        .collect()
}

/// 固定シードの一様ランダム対局2局から4手ごとに採った32局面を返す。
pub(crate) fn sampled_random_positions(rules: MoveRules) -> Vec<Position> {
    let generator = MoveGenerator::new(rules);
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
            position.make_move_unchecked(mv, rules);
        }
    }
    positions
}

pub(crate) fn sq(file: u8, rank: u8) -> Square {
    Square::new(file, rank).unwrap()
}

pub(crate) fn position(side_to_move: Color, pieces: &[(Square, Color, PieceKind)]) -> Position {
    let mut builder = PositionBuilder::new(side_to_move);
    for &(square, color, kind) in pieces {
        builder
            .put(
                square,
                PieceCode::new(color, kind).expect("test fixture uses an unpromoted-capable kind"),
            )
            .unwrap();
    }
    builder.finish().unwrap()
}

pub(crate) fn position_from_codes(side_to_move: Color, pieces: &[(Square, PieceCode)]) -> Position {
    let mut builder = PositionBuilder::new(side_to_move);
    for &(square, piece) in pieces {
        builder.put(square, piece).unwrap();
    }
    builder.finish().unwrap()
}
