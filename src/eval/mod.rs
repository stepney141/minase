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
    // 領域D7-EVAL（評価関数v0）のspec-firstテスト。期待値の根拠は挙動
    // マトリクスd7-search-eval.md、docs/plans/search.md「評価関数v0」節の
    // 凍結駒価値表（2026年8月14日確定）、およびRULES.md第5条・第20条に
    // 限る。PSTの個別値は固定しない（SPEC_UNCLEAR-01の方針）。

    use super::*;
    use crate::core::piece::PieceCode;
    use crate::test_util::{position, position_from_codes, sq};

    // D7-EVAL-01。RULES.md第5条の初期配置は王将⇄玉将の対応を除き点対称で
    // あり、negamax（search.md「設計判断」）が要求する手番対称性の下で
    // 自己対称局面の評価は0でなければならない。
    #[test]
    fn initial_position_evaluates_to_zero_for_either_side_to_move() {
        let initial = Position::initial();
        assert_eq!(evaluate(&initial), 0);

        // 同一配置を後手番として評価しても0になる。
        let pieces: Vec<(Square, PieceCode)> = Square::all()
            .filter_map(|square| initial.piece_at(square).map(|piece| (square, piece)))
            .collect();
        let white_to_move = position_from_codes(Color::White, &pieces);
        assert_eq!(evaluate(&white_to_move), 0);
    }

    // D7-EVAL-02。search.md「評価関数v0」節の凍結駒価値表（HaChu
    // chuPieces[]の2.5倍・切り上げ換算、獅子と王駒は例外規定）と29駒種
    // すべてが一致する。表の変更は設計書改訂とSPRTを伴う規範値である。
    #[test]
    fn piece_values_match_the_frozen_table_for_all_29_kinds() {
        let frozen_table = [
            (PieceKind::Pawn, 100),             // 歩兵
            (PieceKind::GoBetween, 125),        // 仲人
            (PieceKind::Lance, 375),            // 香車
            (PieceKind::ReverseChariot, 375),   // 反車
            (PieceKind::SideMover, 500),        // 横行
            (PieceKind::VerticalMover, 500),    // 竪行
            (PieceKind::Bishop, 625),           // 角行
            (PieceKind::Rook, 750),             // 飛車
            (PieceKind::DragonHorse, 875),      // 龍馬
            (PieceKind::DragonKing, 1_000),     // 龍王
            (PieceKind::FreeKing, 1_500),       // 奔王
            (PieceKind::King, 2_600),           // 王将・玉将
            (PieceKind::DrunkElephant, 503),    // 醉象
            (PieceKind::BlindTiger, 380),       // 盲虎
            (PieceKind::SilverGeneral, 250),    // 銀将
            (PieceKind::GoldGeneral, 378),      // 金将
            (PieceKind::Kirin, 385),            // 麒麟
            (PieceKind::Phoenix, 383),          // 鳳凰
            (PieceKind::Lion, 2_500),           // 獅子
            (PieceKind::CrownPrince, 2_600),    // 太子
            (PieceKind::WhiteHorse, 875),       // 白駒
            (PieceKind::Whale, 625),            // 鯨鯢
            (PieceKind::FlyingOx, 1_000),       // 飛牛
            (PieceKind::FreeBoar, 1_000),       // 奔猪
            (PieceKind::FlyingStag, 750),       // 飛鹿
            (PieceKind::HornedFalcon, 1_250),   // 角鷹
            (PieceKind::SoaringEagle, 1_375),   // 飛鷲
            (PieceKind::FerociousLeopard, 375), // 猛豹
            (PieceKind::CopperGeneral, 250),    // 銅将
        ];

        // 29駒種を重複なく網羅していることの確認。
        assert_eq!(frozen_table.len(), PIECE_KIND_COUNT);
        let mut covered = [false; PIECE_KIND_COUNT];
        for (kind, value) in frozen_table {
            assert!(!covered[kind.index()]);
            covered[kind.index()] = true;
            assert_eq!(piece_value(kind), value);
        }
        assert!(covered.iter().all(|&seen| seen));
    }

    // D7-EVAL-03。search.md「評価関数v0」節: 王将・玉将・太子はHaChuの
    // 280・270を採用せず、獅子を上回る2600とする。王駒2枚側の1枚目の喪失
    // （RULES.md第20条第3〜5項）を評価へ反映する順序の意図を独立に固定する。
    #[test]
    fn royal_piece_value_exceeds_the_lion_value() {
        assert_eq!(
            piece_value(PieceKind::King),
            piece_value(PieceKind::CrownPrince)
        );
        assert!(piece_value(PieceKind::King) > piece_value(PieceKind::Lion));
    }

    // D7-EVAL-04[実装契約]。search.md「設計判断」のfail-soft negamaxは静的
    // 評価の手番対称性を要件とする。局面Pの先手視点評価と、Pを180度回転して
    // 所有者を入れ替えた局面P'の後手視点評価は一致しなければならない。
    // PSTの個別値を固定せず、先後で異なるテーブル参照などの対称性の破れを
    // 検出する。
    #[test]
    fn evaluation_is_symmetric_under_180_degree_rotation() {
        let fixtures: [&[(Square, Color, PieceKind)]; 2] = [
            &[
                (sq(5, 0), Color::Black, PieceKind::King),
                (sq(7, 5), Color::Black, PieceKind::Lion),
                (sq(2, 3), Color::Black, PieceKind::Rook),
                (sq(6, 4), Color::Black, PieceKind::Pawn),
                (sq(6, 11), Color::White, PieceKind::King),
                (sq(5, 10), Color::White, PieceKind::GoldGeneral),
                (sq(9, 8), Color::White, PieceKind::Bishop),
            ],
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(4, 6), Color::Black, PieceKind::DragonHorse),
                (sq(11, 2), Color::Black, PieceKind::Lance),
                (sq(11, 11), Color::White, PieceKind::King),
                (sq(10, 10), Color::White, PieceKind::CrownPrince),
                (sq(3, 9), Color::White, PieceKind::FreeKing),
                (sq(8, 7), Color::White, PieceKind::Pawn),
            ],
        ];

        for pieces in fixtures {
            let original = position(Color::Black, pieces);
            // 180度回転（升(f, r)→(11−f, 11−r)）と所有者の入れ替え。王将⇄
            // 玉将は駒種として同一（RULES.md第5条: 性能同一）に読み替える。
            let rotated_pieces: Vec<(Square, Color, PieceKind)> = pieces
                .iter()
                .map(|&(square, color, kind)| {
                    (
                        sq(11 - square.file(), 11 - square.rank()),
                        color.opposite(),
                        kind,
                    )
                })
                .collect();
            let rotated = position(Color::White, &rotated_pieces);
            assert_eq!(evaluate(&original), evaluate(&rotated));
        }
    }
}
