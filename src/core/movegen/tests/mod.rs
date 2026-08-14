//! 挙動マトリクスD1「駒の動きと成り」に基づく合法手生成のテスト。
//!
//! 期待値の根拠はRULES.md第12版（第6〜12条・第17〜19条・第30条）と
//! docs/plans/move-canonicalization.md だけである。獅子の捕獲制限
//! （第13〜16条・第29条L系）は領域D2（rules.rs側）が検証する。

mod lion_moves;
mod movement;
mod pieces;
mod promotion;
mod properties;

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
