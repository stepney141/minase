//! 空局面と初期配置の作成の試験。

use crate::core::board::{BOARD_RANKS, Bitboard};
use crate::core::piece::{COLOR_COUNT, Color, PIECE_KIND_COUNT, PieceCode, PieceKind};
use crate::core::position::Position;
use crate::test_util::{position_from_codes, sq};

/// 第5条の配置図の機械可読な写し(D4-005-01の期待表)。
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

// 初期配置は第5条の配置図と全144升で完全に一致し、全駒が未成である(D4-005-01)。
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
// (第5条・第20条1項、D4-005-02・D4-020-01)。王駒の判定は駒種だけで決まる(第3条4項)。
#[test]
fn articles_5_and_20_1_each_side_starts_with_exactly_one_royal_king() {
    let initial = Position::initial();
    // 7十二は内部(5,0)、6一は内部(6,11)にあたる(座標規約はD4マトリクス)。
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
