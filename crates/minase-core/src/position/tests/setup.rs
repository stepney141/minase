//! 空局面と初期配置の作成の試験。

use crate::board::{BOARD_RANKS, Bitboard};
use crate::piece::{COLOR_COUNT, Color, PIECE_KIND_COUNT, PieceCode, PieceKind};
use crate::position::Position;
use crate::rng::{XorShift64, derive_seed};
use crate::test_util::{position_from_codes, sq};
use crate::{Game, Rules, Square};

fn shuffled_sample(number: u64) -> Position {
    Position::shuffled_initial(&mut XorShift64::new(derive_seed(20261110, number)))
}

// 設計書alphazero-rl-2-prepの配置条件。S1〜S5を各2,000標本で検査する。
#[test]
fn shuffled_setup_preserves_fixed_ranks_multisets_bishop_parity_and_rotation() {
    let initial = Position::initial();
    for number in 1..=2000 {
        let position = shuffled_sample(number);
        for rank in 3..=8 {
            for file in 0..12 {
                assert_eq!(
                    position.piece_at(sq(file, rank)),
                    initial.piece_at(sq(file, rank))
                );
            }
        }
        for rank in 0..3 {
            for kind in PieceKind::ALL {
                let count = |position: &Position| {
                    (0..12)
                        .filter(|&file| {
                            position.piece_at(sq(file, rank)).and_then(PieceCode::kind)
                                == Some(kind)
                        })
                        .count()
                };
                assert_eq!(
                    count(&position),
                    count(&initial),
                    "sample {number}, rank {rank}, {kind:?}"
                );
            }
        }
        assert_eq!(
            (0..12)
                .filter(|&file| position.piece_at(sq(file, 1)).is_none())
                .count(),
            4
        );
        let bishops: Vec<_> = (0..12)
            .filter(|&file| {
                position.piece_at(sq(file, 1)).and_then(PieceCode::kind) == Some(PieceKind::Bishop)
            })
            .collect();
        assert_eq!(bishops.len(), 2);
        assert_ne!(bishops[0] % 2, bishops[1] % 2);
        for rank in 0..12 {
            for file in 0..12 {
                let expected = position.piece_at(sq(file, rank)).map(|piece| {
                    PieceCode::new(piece.color().unwrap().opposite(), piece.kind().unwrap())
                        .unwrap()
                });
                assert_eq!(position.piece_at(sq(11 - file, 11 - rank)), expected);
            }
        }
        assert_eq!(position.side_to_move(), Color::Black);
        assert!(position.lion_taken_by_non_lion.is_none());
        assert!(position.promotion_deferred.is_empty());
        position.validate().unwrap();
    }
}

#[test]
fn shuffled_setup_is_reproducible_and_diverse() {
    let mut layouts = std::collections::HashSet::new();
    for number in 1..=2000 {
        let position = shuffled_sample(number);
        assert_eq!(position, shuffled_sample(number));
        layouts.insert(
            Square::all()
                .map(|square| position.piece_at(square))
                .collect::<Vec<_>>(),
        );
    }
    assert!(layouts.len() >= 1990, "distinct layouts: {}", layouts.len());
}

#[test]
fn shuffled_setup_file_frequencies_are_within_five_sigma() {
    let mut kings = [0; 12];
    let mut empty = [0; 12];
    for number in 1..=12000 {
        let position = shuffled_sample(number);
        for file in 0..12 {
            kings[usize::from(file)] += usize::from(
                position.piece_at(sq(file, 0)).and_then(PieceCode::kind) == Some(PieceKind::King),
            );
            empty[usize::from(file)] += usize::from(position.piece_at(sq(file, 1)).is_none());
        }
    }
    // 二項分布の標準偏差は玉将約30.3、空き升約51.6。許容幅は設計の5σ。
    for file in 0..12 {
        assert!(
            (848..=1152).contains(&kings[file]),
            "king file {file}: {}",
            kings[file]
        );
        assert!(
            (3742..=4258).contains(&empty[file]),
            "empty file {file}: {}",
            empty[file]
        );
    }
}

#[test]
fn shuffled_setup_has_legal_moves_and_no_first_move_captures() {
    for number in 1..=2000 {
        let position = shuffled_sample(number);
        let game = Game::from_position(Rules::JCSA, position.clone());
        let moves = game.legal_moves();
        assert!(!moves.is_empty(), "sample {number}");
        for mv in moves {
            for square in mv.mid.into_iter().chain([mv.to]) {
                // 正準形のじっとはto == fromであり、移動中はその升から自駒が離れている。
                if square != mv.from {
                    assert!(
                        position.piece_at(square).is_none(),
                        "sample {number}: {mv:?}"
                    );
                }
            }
            assert_eq!(position.captured_squares(mv), [None, None]);
        }
    }
}

/// 第5条の配置図の機械可読な写し。
/// 行は段一〜十二、列は配置図の印字順(筋12→筋1)で、内部座標では
/// 筋f・段rの升が(file=12−f, rank=12−r)にあたる。
fn article_5_expected_table() -> [[Option<(Color, PieceKind)>; 12]; 12] {
    use PieceKind::{
        Bishop, BlindTiger, CopperGeneral, DragonHorse, DragonKing, DrunkElephant,
        FerociousLeopard, FreeKing, GoBetween, GoldGeneral, King, Kirin, Lance, Lion, Pawn,
        Phoenix, ReverseChariot, Rook, SideMover, SilverGeneral, VerticalMover,
    };
    let b = |kind| Some((Color::Black, kind));
    let w = |kind| Some((Color::White, kind));
    let n = None;
    [
        // 一段目: 香 猛 銅 銀 金 醉 玉 金 銀 銅 猛 香
        [
            w(Lance),
            w(FerociousLeopard),
            w(CopperGeneral),
            w(SilverGeneral),
            w(GoldGeneral),
            w(DrunkElephant),
            w(King),
            w(GoldGeneral),
            w(SilverGeneral),
            w(CopperGeneral),
            w(FerociousLeopard),
            w(Lance),
        ],
        // 二段目: 反 ・ 角 ・ 盲 鳳 麒 盲 ・ 角 ・ 反
        [
            w(ReverseChariot),
            n,
            w(Bishop),
            n,
            w(BlindTiger),
            w(Phoenix),
            w(Kirin),
            w(BlindTiger),
            n,
            w(Bishop),
            n,
            w(ReverseChariot),
        ],
        // 三段目: 横 竪 飛 馬 龍 奔 獅 龍 馬 飛 竪 横
        [
            w(SideMover),
            w(VerticalMover),
            w(Rook),
            w(DragonHorse),
            w(DragonKing),
            w(FreeKing),
            w(Lion),
            w(DragonKing),
            w(DragonHorse),
            w(Rook),
            w(VerticalMover),
            w(SideMover),
        ],
        // 四段目: 歩×12
        [w(Pawn); 12],
        // 五段目: ・ ・ ・ 仲 ・ ・ ・ ・ 仲 ・ ・ ・
        [n, n, n, w(GoBetween), n, n, n, n, w(GoBetween), n, n, n],
        // 六段目・七段目: 空
        [n; 12],
        [n; 12],
        // 八段目: ・ ・ ・ 仲 ・ ・ ・ ・ 仲 ・ ・ ・
        [n, n, n, b(GoBetween), n, n, n, n, b(GoBetween), n, n, n],
        // 九段目: 歩×12
        [b(Pawn); 12],
        // 十段目: 横 竪 飛 馬 龍 獅 奔 龍 馬 飛 竪 横
        [
            b(SideMover),
            b(VerticalMover),
            b(Rook),
            b(DragonHorse),
            b(DragonKing),
            b(Lion),
            b(FreeKing),
            b(DragonKing),
            b(DragonHorse),
            b(Rook),
            b(VerticalMover),
            b(SideMover),
        ],
        // 十一段: 反 ・ 角 ・ 盲 麒 鳳 盲 ・ 角 ・ 反
        [
            b(ReverseChariot),
            n,
            b(Bishop),
            n,
            b(BlindTiger),
            b(Kirin),
            b(Phoenix),
            b(BlindTiger),
            n,
            b(Bishop),
            n,
            b(ReverseChariot),
        ],
        // 十二段: 香 猛 銅 銀 金 王 醉 金 銀 銅 猛 香
        [
            b(Lance),
            b(FerociousLeopard),
            b(CopperGeneral),
            b(SilverGeneral),
            b(GoldGeneral),
            b(King),
            b(DrunkElephant),
            b(GoldGeneral),
            b(SilverGeneral),
            b(CopperGeneral),
            b(FerociousLeopard),
            b(Lance),
        ],
    ]
}

// 初期配置は第5条の配置図と全144升で完全に一致し、全駒が未成である。
#[test]
fn article_5_initial_position_matches_the_full_144_square_table() {
    let initial = Position::initial();
    let table = article_5_expected_table();
    let mut occupied = Bitboard::EMPTY;
    let mut by_color = [Bitboard::EMPTY; COLOR_COUNT];
    let mut by_kind = [[Bitboard::EMPTY; PIECE_KIND_COUNT]; COLOR_COUNT];

    for (row, expected_rank) in table.into_iter().enumerate() {
        // 段d(一=1)は内部rank 12−d、印字列の左からi番目(筋12−i)は内部file iにあたる。
        let rank = BOARD_RANKS - 1 - row as u8;
        for (file, expected) in expected_rank.into_iter().enumerate() {
            let square = sq(file as u8, rank);
            if let Some((color, kind)) = expected {
                occupied.set(square);
                by_color[color.index()].set(square);
                by_kind[color.index()][kind.index()].set(square);
            }
            let actual = initial
                .piece_at(square)
                .map(|piece| (piece.color().unwrap(), piece.kind().unwrap()));
            assert_eq!(actual, expected, "第5条の{}段目・筋{}", row + 1, 12 - file);
            // 初期局面に成駒は存在しない(第4条3項)。
            if let Some(piece) = initial.piece_at(square) {
                assert!(!piece.is_promoted());
            }
        }
    }
    assert_eq!(initial.occupied(), occupied);
    for color in Color::ALL {
        assert_eq!(initial.pieces_of(color), by_color[color.index()]);
        for kind in PieceKind::ALL {
            assert_eq!(
                initial.pieces_of_kind(color, kind),
                by_kind[color.index()][kind.index()],
                "{color:?} {kind:?}"
            );
        }
    }
}

// 初期局面の王駒は先手が王将7十二、後手が玉将6一の各1枚で、太子は存在しない
// (第5条・第20条1項)。王駒の判定は駒種だけで決まる(第3条4項)。
#[test]
fn articles_5_and_20_1_each_side_starts_with_exactly_one_royal_king() {
    let initial = Position::initial();
    // 7十二は内部(5,0)、6一は内部(6,11)にあたる。
    let expected = [(Color::Black, sq(5, 0)), (Color::White, sq(6, 11))];
    for (color, square) in expected {
        assert_eq!(
            initial.royal_pieces(color).iter().collect::<Vec<_>>(),
            vec![square]
        );
        let piece = initial.piece_at(square).unwrap();
        assert_eq!(piece.kind(), Some(PieceKind::King));
        assert_eq!(piece.color(), Some(color));
        assert!(
            initial
                .pieces_of_kind(color, PieceKind::CrownPrince)
                .is_empty()
        );
    }

    // 王駒(王将・玉将・太子)の判定は位置に依存せず、醉象は王駒ではない
    // (第3条4項、第20条2項: 醉象は成って太子となったときに初めて王駒となる)。
    let custom = position_from_codes(
        Color::Black,
        &[
            (
                sq(3, 3),
                PieceCode::new(Color::Black, PieceKind::King).unwrap(),
            ),
            (
                sq(7, 7),
                PieceCode::new_promoted(Color::Black, PieceKind::CrownPrince).unwrap(),
            ),
            (
                sq(9, 9),
                PieceCode::new(Color::Black, PieceKind::DrunkElephant).unwrap(),
            ),
        ],
    );
    assert_eq!(
        custom.royal_pieces(Color::Black),
        Bitboard::from_squares([sq(3, 3), sq(7, 7)])
    );
}
