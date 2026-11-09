//! 空局面と初期配置の作成。

use core::num::NonZeroUsize;

use super::zobrist::zobrist_keys;
use super::{Position, PositionBuilder};
use crate::board::{BOARD_FILES, BOARD_RANKS, Bitboard, RAW_SQUARE_COUNT, Square};
use crate::piece::{COLOR_COUNT, Color, PIECE_KIND_COUNT, PieceCode, PieceKind};
use crate::rng::XorShift64;

impl Position {
    /// 指定手番で駒のない局面を作る。
    pub fn empty(side_to_move: Color) -> Self {
        let mut board = [PieceCode::WALL; RAW_SQUARE_COUNT];
        for square in Square::all() {
            board[square.raw_index()] = PieceCode::EMPTY;
        }

        Self {
            board,
            occupied: Bitboard::EMPTY,
            by_color: [Bitboard::EMPTY; COLOR_COUNT],
            by_kind: [[Bitboard::EMPTY; PIECE_KIND_COUNT]; COLOR_COUNT],
            side_to_move,
            lion_taken_by_non_lion: None,
            zobrist: zobrist_keys().side(side_to_move),
            promotion_deferred: Bitboard::EMPTY,
            rights_zobrist: 0,
        }
    }

    /// 初期配置(第5条)の局面を作る。
    pub fn initial() -> Self {
        Self::from_setup_ranks(initial_ranks())
    }

    /// 自己対局の開始局面に使う組み替えた配置を作る。第5条の初期配置ではない。
    ///
    /// 奥の3段を段ごとにFisher–Yates法で並べ替え、2枚の角行は筋の偶奇を異ならせる。
    /// 歩兵と仲人は動かさず、後手を先手の180度回転として置く。
    pub fn shuffled_initial(rng: &mut XorShift64) -> Self {
        let mut ranks = initial_ranks();
        for (rank_index, rank) in ranks.iter_mut().enumerate() {
            loop {
                for i in (1..rank.len()).rev() {
                    let j = rng.index(NonZeroUsize::new(i + 1).expect("i + 1 is positive"));
                    rank.swap(i, j);
                }
                if rank_index != 1
                    || rank
                        .iter()
                        .enumerate()
                        .filter(|(_, kind)| **kind == Some(PieceKind::Bishop))
                        .map(|(file, _)| file % 2)
                        .sum::<usize>()
                        == 1
                {
                    break;
                }
            }
        }
        Self::from_setup_ranks(ranks)
    }

    fn from_setup_ranks(ranks: [[Option<PieceKind>; 12]; 3]) -> Self {
        let mut builder = PositionBuilder::new(Color::Black);
        for color in Color::ALL {
            let rotate = |file: u8, rank: u8| match color {
                Color::Black => (file, rank),
                Color::White => (BOARD_FILES - 1 - file, BOARD_RANKS - 1 - rank),
            };
            let mut put = |file, rank, kind| {
                let (file, rank) = rotate(file, rank);
                builder
                    .put(
                        Square::new(file, rank).unwrap(),
                        PieceCode::new(color, kind)
                            .expect("the initial position uses unpromoted-capable kinds"),
                    )
                    .unwrap();
            };

            for (rank, kinds) in ranks.iter().enumerate() {
                for (file, kind) in kinds.iter().enumerate() {
                    if let Some(kind) = kind {
                        put(file as u8, rank as u8, *kind);
                    }
                }
            }
            for file in 0..BOARD_FILES {
                put(file, 3, PieceKind::Pawn);
            }
            for file in [3, 8] {
                put(file, 4, PieceKind::GoBetween);
            }
        }

        builder.finish().unwrap()
    }
}

fn initial_ranks() -> [[Option<PieceKind>; 12]; 3] {
    let back_rank = [
        PieceKind::Lance,
        PieceKind::FerociousLeopard,
        PieceKind::CopperGeneral,
        PieceKind::SilverGeneral,
        PieceKind::GoldGeneral,
        PieceKind::King,
        PieceKind::DrunkElephant,
        PieceKind::GoldGeneral,
        PieceKind::SilverGeneral,
        PieceKind::CopperGeneral,
        PieceKind::FerociousLeopard,
        PieceKind::Lance,
    ];
    let second_rank = [
        Some(PieceKind::ReverseChariot),
        None,
        Some(PieceKind::Bishop),
        None,
        Some(PieceKind::BlindTiger),
        Some(PieceKind::Kirin),
        Some(PieceKind::Phoenix),
        Some(PieceKind::BlindTiger),
        None,
        Some(PieceKind::Bishop),
        None,
        Some(PieceKind::ReverseChariot),
    ];
    let third_rank = [
        PieceKind::SideMover,
        PieceKind::VerticalMover,
        PieceKind::Rook,
        PieceKind::DragonHorse,
        PieceKind::DragonKing,
        PieceKind::Lion,
        PieceKind::FreeKing,
        PieceKind::DragonKing,
        PieceKind::DragonHorse,
        PieceKind::Rook,
        PieceKind::VerticalMover,
        PieceKind::SideMover,
    ];
    [back_rank.map(Some), second_rank, third_rank.map(Some)]
}
