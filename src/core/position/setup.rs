//! 空局面と初期配置の作成。

use super::zobrist::zobrist_keys;
use super::{Position, PositionBuilder};
use crate::core::board::{BOARD_FILES, BOARD_RANKS, Bitboard, RAW_SQUARE_COUNT, Square};
use crate::core::piece::{COLOR_COUNT, Color, PIECE_KIND_COUNT, PieceCode, PieceKind};

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
        let mut builder = PositionBuilder::new(Color::Black);
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
            (0, PieceKind::ReverseChariot),
            (2, PieceKind::Bishop),
            (4, PieceKind::BlindTiger),
            (5, PieceKind::Kirin),
            (6, PieceKind::Phoenix),
            (7, PieceKind::BlindTiger),
            (9, PieceKind::Bishop),
            (11, PieceKind::ReverseChariot),
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

            for (file, kind) in back_rank.into_iter().enumerate() {
                put(file as u8, 0, kind);
            }
            for (file, kind) in second_rank {
                put(file, 1, kind);
            }
            for (file, kind) in third_rank.into_iter().enumerate() {
                put(file as u8, 2, kind);
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
