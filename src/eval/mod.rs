//! 中将棋の静的評価関数。

use crate::core::piece::{Color, PIECE_KIND_COUNT, PieceKind};
use crate::core::position::Position;
use crate::core::square::{BOARD_FILES, BOARD_RANKS, BOARD_SQUARE_COUNT, Square};

/// 静的評価値の絶対値上限。
const EVALUATION_LIMIT: i32 = 28_999;

// 出典: H. G. Muller, HaChu (https://github.com/ddugovic/hachu) の
// variant.cにあるchuPieces[]。歩兵=40の値を2.5倍し、0.5は切り上げて
// 歩兵=100のセンチポーンへ換算した。
//
// HaChuの王将・玉将=280、太子=270は、王駒捕獲を終局とする前提の値なので
// 採用しない。王駒を2枚持つ側の1枚目の喪失も評価へ反映するため、王将・
// 玉将と太子は獅子を上回る2600とする(第20条第3項から第5項まで)。
const PIECE_VALUES: [i32; PIECE_KIND_COUNT] = [
    100,   // 歩兵
    125,   // 仲人
    375,   // 香車
    375,   // 反車
    500,   // 横行
    500,   // 竪行
    625,   // 角行
    750,   // 飛車
    875,   // 龍馬
    1_000, // 龍王
    1_500, // 奔王
    2_600, // 王将・玉将
    503,   // 醉象
    375,   // 猛豹
    380,   // 盲虎
    250,   // 銅将
    250,   // 銀将
    378,   // 金将
    385,   // 麒麟
    383,   // 鳳凰
    2_500, // 獅子
    2_600, // 太子
    875,   // 白駒
    625,   // 鯨鯢
    1_000, // 飛牛
    1_000, // 奔猪
    750,   // 飛鹿
    1_250, // 角鷹
    1_375, // 飛鷲
];

/// 成れる駒の前進を評価する先手用PST。
const FORWARD_PST: [i32; BOARD_SQUARE_COUNT] = build_forward_pst();
/// 王駒の自陣側待機を評価する先手用PST。
const ROYAL_PST: [i32; BOARD_SQUARE_COUNT] = build_royal_pst();
/// 獅子と奔王の中央性を評価する先手用PST。
const CENTRAL_PST: [i32; BOARD_SQUARE_COUNT] = build_central_pst();

/// 前進PSTを作る。段が進むごとに1升あたり2の等差で加点する。
const fn build_forward_pst() -> [i32; BOARD_SQUARE_COUNT] {
    let mut table = [0; BOARD_SQUARE_COUNT];
    let mut index = 0;
    while index < BOARD_SQUARE_COUNT {
        table[index] = (index / BOARD_FILES as usize) as i32 * 2;
        index += 1;
    }
    table
}

/// 王駒PSTを作る。最下段16、その1段前10、以遠は0を与える。
const fn build_royal_pst() -> [i32; BOARD_SQUARE_COUNT] {
    let mut table = [0; BOARD_SQUARE_COUNT];
    let mut index = 0;
    while index < BOARD_SQUARE_COUNT {
        table[index] = match index / BOARD_FILES as usize {
            0 => 16,
            1 => 10,
            _ => 0,
        };
        index += 1;
    }
    table
}

/// 中央性PSTを作る。盤中心へのチェビシェフ距離に応じて最大12を加点する。
const fn build_central_pst() -> [i32; BOARD_SQUARE_COUNT] {
    let mut table = [0; BOARD_SQUARE_COUNT];
    let mut index = 0;
    while index < BOARD_SQUARE_COUNT {
        let file = index % BOARD_FILES as usize;
        let rank = index / BOARD_FILES as usize;
        // 盤中心は升間にあるため、座標を2倍して中心11との距離を整数で測る。
        let file_distance = (file * 2).abs_diff(11);
        let rank_distance = (rank * 2).abs_diff(11);
        let distance = if file_distance > rank_distance {
            file_distance
        } else {
            rank_distance
        };
        table[index] = if distance < 6 {
            12 - distance as i32 * 2
        } else {
            0
        };
        index += 1;
    }
    table
}

/// 駒種の駒価値をセンチポーンで返す。
#[inline]
pub const fn piece_value(kind: PieceKind) -> i32 {
    PIECE_VALUES[kind.index()]
}

/// 現局面を手番側の視点からセンチポーンで評価する。
///
/// 駒割と最小限のPSTを先手視点で合計し、後手番では符号を反転する。
/// 詰み値の帯域と衝突しないよう、結果の絶対値は28999以下に制限する。
pub fn evaluate(position: &Position) -> i32 {
    let mut score = 0;
    for color in Color::ALL {
        let sign = match color {
            Color::Black => 1,
            Color::White => -1,
        };
        for square in position.pieces_of(color) {
            let kind = position
                .piece_at(square)
                .and_then(|piece| piece.kind())
                .expect("piece set must contain a valid piece");
            score += sign * (piece_value(kind) + positional_value(kind, color, square));
        }
    }

    let side_score = match position.side_to_move() {
        Color::Black => score,
        Color::White => -score,
    };
    side_score.clamp(-EVALUATION_LIMIT, EVALUATION_LIMIT)
}

/// 駒種に応じたPSTの加点を、後手は盤を上下反転した先手視点で返す。
fn positional_value(kind: PieceKind, color: Color, square: Square) -> i32 {
    let black_rank = match color {
        Color::Black => square.rank(),
        Color::White => BOARD_RANKS - 1 - square.rank(),
    };
    let black_index = black_rank as usize * BOARD_FILES as usize + square.file() as usize;
    let mut value = 0;
    if kind.can_promote() {
        value += FORWARD_PST[black_index];
    }
    if matches!(kind, PieceKind::King | PieceKind::CrownPrince) {
        value += ROYAL_PST[black_index];
    }
    if matches!(kind, PieceKind::Lion | PieceKind::FreeKing) {
        value += CENTRAL_PST[black_index];
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piece_value_table_covers_all_29_piece_kinds() {
        let expected = [
            (PieceKind::Pawn, 100),
            (PieceKind::GoBetween, 125),
            (PieceKind::Lance, 375),
            (PieceKind::ReverseChariot, 375),
            (PieceKind::SideMover, 500),
            (PieceKind::VerticalMover, 500),
            (PieceKind::Bishop, 625),
            (PieceKind::Rook, 750),
            (PieceKind::DragonHorse, 875),
            (PieceKind::DragonKing, 1_000),
            (PieceKind::FreeKing, 1_500),
            (PieceKind::King, 2_600),
            (PieceKind::DrunkElephant, 503),
            (PieceKind::FerociousLeopard, 375),
            (PieceKind::BlindTiger, 380),
            (PieceKind::CopperGeneral, 250),
            (PieceKind::SilverGeneral, 250),
            (PieceKind::GoldGeneral, 378),
            (PieceKind::Kirin, 385),
            (PieceKind::Phoenix, 383),
            (PieceKind::Lion, 2_500),
            (PieceKind::CrownPrince, 2_600),
            (PieceKind::WhiteHorse, 875),
            (PieceKind::Whale, 625),
            (PieceKind::FlyingOx, 1_000),
            (PieceKind::FreeBoar, 1_000),
            (PieceKind::FlyingStag, 750),
            (PieceKind::HornedFalcon, 1_250),
            (PieceKind::SoaringEagle, 1_375),
        ];

        assert_eq!(expected.len(), PIECE_KIND_COUNT);
        assert_eq!(PieceKind::ALL.len(), PIECE_KIND_COUNT);
        for (index, (kind, value)) in expected.into_iter().enumerate() {
            assert_eq!(kind.index(), index);
            assert_eq!(piece_value(kind), value);
        }
    }

    #[test]
    fn initial_position_evaluates_to_zero() {
        assert_eq!(evaluate(&Position::initial()), 0);
    }
}
