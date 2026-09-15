//! 挙動マトリクスD1「駒の動きと成り」に基づく合法手生成のテスト。
//!
//! 期待値の根拠はRULES.md第12版（第6〜12条・第17〜19条・第30条）と
//! docs/plans/move-canonicalization.md だけである。獅子の捕獲制限
//! （第13〜16条・第29条L系）は領域D2（rules.rs側）が検証する。

mod attackers;
mod lion_moves;
mod movement;
mod pieces;
mod promotion;
mod properties;
mod search_captures;

use std::collections::BTreeSet;

use super::MoveGenerator;
use crate::core::mv::Move;
use crate::core::piece::{Color, PieceKind};
use crate::core::position::Position;
use crate::core::square::Square;

/// マトリクスの升表記 (筋f, 段r)（各1〜12、段1=後手側最奥、段12=先手側最奥）を
/// 盤座標へ写す。先手の「前」は段が減る方向である（第3条3号）。
pub(super) fn msq(file: u8, rank: u8) -> Square {
    assert!((1..=12).contains(&file) && (1..=12).contains(&rank));
    Square::new(file - 1, 12 - rank).unwrap()
}

/// 正準形の着手（move-canonicalization.md の4つ組）を組み立てる。
pub(super) fn mv(from: Square, mid: Option<Square>, to: Square, promote: bool) -> Move {
    Move {
        from,
        mid,
        to,
        promote,
    }
}

/// 標準規則で手番側の合法手集合を生成する。
pub(super) fn generated(position: &Position) -> Vec<Move> {
    let mut moves = Vec::new();
    MoveGenerator::standard().generate_moves(position, &mut moves);
    moves
}

/// 指定した生成器で手番側の合法手集合を生成する。
pub(super) fn generated_with(generator: &MoveGenerator, position: &Position) -> Vec<Move> {
    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    moves
}

/// 指定した生成器で手番側の捕獲手だけを生成する。
pub(super) fn generated_captures_with(generator: &MoveGenerator, position: &Position) -> Vec<Move> {
    let mut moves = Vec::new();
    generator.generate_captures(position, &mut moves);
    moves
}

/// origin を from とする着手だけを取り出す。
pub(super) fn moves_from(moves: &[Move], origin: Square) -> Vec<Move> {
    moves
        .iter()
        .copied()
        .filter(|mv| mv.from == origin)
        .collect()
}

/// origin からの直接到達升集合（mid なし・to≠from）。
pub(super) fn direct_destinations(position: &Position, origin: Square) -> BTreeSet<Square> {
    generated(position)
        .into_iter()
        .filter(|mv| mv.from == origin && mv.mid.is_none() && mv.to != origin)
        .map(|mv| mv.to)
        .collect()
}

/// じっと（mid なしで to == from の正準着手。第3条13号）だけを取り出す。
pub(super) fn jitto_moves(moves: &[Move], origin: Square) -> Vec<Move> {
    moves
        .iter()
        .copied()
        .filter(|mv| mv.from == origin && mv.mid.is_none() && mv.to == origin)
        .collect()
}

/// マトリクス座標の1升移動の期待到達升集合を作る。deltas は先手基準の
/// 相対ベクトル (df, dr)（前=(0,−1)）で与える。盤外は切り詰める（第7条1項）。
pub(super) fn step_squares(origin: (i16, i16), deltas: &[(i16, i16)]) -> BTreeSet<Square> {
    let mut result = BTreeSet::new();
    for &(df, dr) in deltas {
        let (file, rank) = (origin.0 + df, origin.1 + dr);
        if (1..=12).contains(&file) && (1..=12).contains(&rank) {
            result.insert(msq(file as u8, rank as u8));
        }
    }
    result
}

/// マトリクス座標の走りの期待到達升集合を作る（空盤の想定で盤端まで伸ばす）。
pub(super) fn ray_squares(origin: (i16, i16), directions: &[(i16, i16)]) -> BTreeSet<Square> {
    let mut result = BTreeSet::new();
    for &(df, dr) in directions {
        let (mut file, mut rank) = (origin.0 + df, origin.1 + dr);
        while (1..=12).contains(&file) && (1..=12).contains(&rank) {
            result.insert(msq(file as u8, rank as u8));
            file += df;
            rank += dr;
        }
    }
    result
}

/// 2つの升集合の和を返す。
pub(super) fn union(a: BTreeSet<Square>, b: BTreeSet<Square>) -> BTreeSet<Square> {
    a.into_iter().chain(b).collect()
}

/// 盤面成分（全升の駒配置）が一致するかを返す。手番は比較しない。
pub(super) fn same_board(a: &Position, b: &Position) -> bool {
    Square::all().all(|square| a.piece_at(square) == b.piece_at(square))
}

/// 第9条の成り対応18組（未成駒種→成駒種）。歩兵の成駒は「金将と同じ動き」
/// なので金将の駒種で表す。
pub(super) const PROMOTION_PAIRS: [(PieceKind, PieceKind); 18] = [
    (PieceKind::GoldGeneral, PieceKind::Rook),
    (PieceKind::SilverGeneral, PieceKind::VerticalMover),
    (PieceKind::CopperGeneral, PieceKind::SideMover),
    (PieceKind::FerociousLeopard, PieceKind::Bishop),
    (PieceKind::BlindTiger, PieceKind::FlyingStag),
    (PieceKind::DrunkElephant, PieceKind::CrownPrince),
    (PieceKind::Pawn, PieceKind::GoldGeneral),
    (PieceKind::GoBetween, PieceKind::DrunkElephant),
    (PieceKind::Lance, PieceKind::WhiteHorse),
    (PieceKind::ReverseChariot, PieceKind::Whale),
    (PieceKind::SideMover, PieceKind::FreeBoar),
    (PieceKind::VerticalMover, PieceKind::FlyingOx),
    (PieceKind::Bishop, PieceKind::DragonHorse),
    (PieceKind::Rook, PieceKind::DragonKing),
    (PieceKind::DragonHorse, PieceKind::HornedFalcon),
    (PieceKind::DragonKing, PieceKind::SoaringEagle),
    (PieceKind::Kirin, PieceKind::Lion),
    (PieceKind::Phoenix, PieceKind::FreeKing),
];

/// 2段階移動を持つ駒種（獅子・角鷹・飛鷲。第3条7号）。
pub(super) const TWO_STAGE_KINDS: [PieceKind; 3] = [
    PieceKind::Lion,
    PieceKind::HornedFalcon,
    PieceKind::SoaringEagle,
];

/// 先手基準の相対ベクトル（マトリクス座標。前=(0,−1)、後=(0,+1)）。
pub(super) mod dir {
    pub(crate) const F: (i16, i16) = (0, -1);
    pub(crate) const B: (i16, i16) = (0, 1);
    pub(crate) const L: (i16, i16) = (-1, 0);
    pub(crate) const R: (i16, i16) = (1, 0);
    pub(crate) const FL: (i16, i16) = (-1, -1);
    pub(crate) const FR: (i16, i16) = (1, -1);
    pub(crate) const BL: (i16, i16) = (-1, 1);
    pub(crate) const BR: (i16, i16) = (1, 1);
}

/// 手番側から見た「前」の盤座標上の段方向。先手（Black）は+1、後手は−1。
/// 先手側最奥（段12）が盤座標rank 0に当たることから従う（第3条3号、第5条）。
pub(super) fn forward_rank_delta(color: Color) -> i8 {
    match color {
        Color::Black => 1,
        Color::White => -1,
    }
}

// movegen-speedup.md「捕獲対象を生成前に除外する」の共通照合標本。
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

/// attackers.rsと同じbench標本および同じ方法で作る一様ランダム対局標本。
pub(crate) fn capture_test_positions() -> Vec<Position> {
    use crate::core::rules::MoveRules;
    use crate::rng::XorShift64;
    use core::num::NonZeroU64;
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

    BENCH_SFENS
        .iter()
        .chain(CAPTURE_EDGE_SFENS)
        .map(|s| crate::parse_sfen(s).unwrap())
        .chain(sampled_random_positions())
        .collect()
}

/// 標準、lishogi、および成り・獅子の各規則分岐を含む照合集合。
pub(crate) fn capture_test_rules() -> [crate::core::rules::MoveRules; 5] {
    use crate::core::rules::{LionRule, MoveRules, PromotionRule, Rules};
    [
        Rules::ENGINE_DEFAULT.moves,
        Rules::LISHOGI.moves,
        MoveRules {
            promotion: PromotionRule::P1,
            p3: true,
            p4: true,
            ..MoveRules::standard()
        },
        MoveRules {
            promotion: PromotionRule::P2,
            p5: true,
            p6: true,
            ..MoveRules::standard()
        },
        MoveRules {
            lion: LionRule::L0 { l4: true },
            l3: true,
            promotion: PromotionRule::P2,
            p5: true,
            p6: true,
            ..MoveRules::standard()
        },
    ]
}

// 空捕獲、同価値、成りと不成り、特殊捕獲のみ、2枚取り、最後の王駒を固定する。
pub(crate) const CAPTURE_EDGE_SFENS: &[&str] = &[
    "k11/12/12/12/12/12/12/12/12/12/12/11K b",
    "k11/2p1p7/2R1R7/12/12/12/12/12/12/12/12/11K b",
    "k11/12/12/12/5p6/5p6/5N6/12/12/12/12/11K b",
    "k11/12/12/12/5p6/5p6/5+H6/12/12/12/12/11K b",
    "k11/12/12/12/4p1p5/5+D6/12/12/12/12/12/11K b",
    "12/12/12/12/5k6/5p6/5N6/12/12/12/12/11K b",
    "12/12/12/12/5k6/5R6/12/12/12/12/12/11K b",
    "12/12/12/12/5k6/5+e6/5N6/12/12/12/12/11K b",
    "k11/12/12/5r6/12/5n6/5+H6/12/12/12/12/11K b",
    "k11/12/12/5r6/5n6/5p6/5N6/12/12/12/12/11K b",
];
