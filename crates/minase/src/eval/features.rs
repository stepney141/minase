//! 学習と推論で共有する駒状態と視点ごとの特徴番号。

use minase_core::piece::PIECE_KIND_COUNT;
use minase_core::{Color, PieceCode, PieceKind, Position, Square};

/// 学習と推論で共有する基底特徴の総数13,680。
/// 自駒0と相手駒1の2陣営×47駒状態×144升の盤面特徴13,536個に、先獅子の対象升144個を加える。
/// 1局面では各駒がちょうど1個、先獅子があればさらに1個発火する。
/// 通常の対局は最大92枚であるが、入力局面では144升すべてに駒を置けるため、発火数の上限は145個である。
pub const FEATURE_COUNT: usize = 13_680;
/// 駒種と現在の成り可否の組の総数47。
/// 番号の割り当ては[`piece_state`]に従う。
pub const PIECE_STATE_COUNT: usize = 47;
/// [`FEATURE_COUNT`]のうち、相対陣営と駒状態と升の組からなる13,536個の盤面特徴。
pub const BOARD_FEATURE_COUNT: usize = 2 * PIECE_STATE_COUNT * 144;
/// 成りビットのない駒コードを[`piece_state`]の状態番号へ写す表。
/// 成れる18種は29〜46、それ以外は駒種番号そのものへ写す。
const UNPROMOTED_STATES: [u8; PIECE_KIND_COUNT] = build_unpromoted_states();

/// 成っていない駒の状態表を作る。成れる駒種には駒種番号順に29から番号を振る。
const fn build_unpromoted_states() -> [u8; PIECE_KIND_COUNT] {
    let mut table = [0_u8; PIECE_KIND_COUNT];
    let mut next_promotable_state = 29_u8;
    let mut index = 0;
    while index < PIECE_KIND_COUNT {
        if PieceKind::ALL[index].can_promote() {
            table[index] = next_promotable_state;
            next_promotable_state += 1;
        } else {
            table[index] = index as u8;
        }
        index += 1;
    }
    table
}

/// 駒コードを、駒種と現在の成り可否からなる状態番号へ写す。
/// 0〜28は駒種番号と同じ「成れない」状態であり、29〜46は王将、獅子、奔王を除く初期配置の18種の「成れる」状態を駒種番号順に割り当てる。
/// 初期配置の獅子と麒麟が成った獅子はともに20、生の金将は44、歩兵が成った金将は17になる。
/// 評価に必要なのは今後の成りによる動きの変化なので、成りビットそのものを区別しない。
/// 成りビットのない成駒専用種も同じ成り可否の状態へ写り、定義域は[`Position`]が盤上に受理する全駒コードである。
/// 歩兵、仲人、香車、反車、猛豹、盲虎、銅将、銀将、麒麟、鳳凰の「成れない」状態に写る駒コードは盤上に現れず、その特徴の重みは学習で更新されない。
///
/// # Panics
/// 空升または盤外の番兵ではパニックする。
pub fn piece_state(piece: PieceCode) -> usize {
    let kind = piece.kind().expect("feature piece must have a valid kind");
    if piece.is_promoted() {
        kind.index()
    } else {
        UNPROMOTED_STATES[kind.index()] as usize
    }
}

/// 指定視点の盤面特徴番号を`(相対陣営×47 + 駒状態)×144 + 相対段×12 + 筋`で返す。
/// 相対陣営は自駒0、相手駒1であり、駒状態は[`piece_state`]で求める。
/// 後手視点では段だけを`11 − 段`へ反転し、筋は反転しない。
/// 前後非対称の動きを視点でそろえ、左右対称性は線形重みの鏡映対の共有で扱う。
/// 線形評価は手番側視点の重みの和なので、後手番でも和の符号反転を要しない。
///
/// # Panics
/// `piece`が空升または盤外の番兵ではパニックする。
pub fn feature_index(perspective: Color, piece: PieceCode, square: Square) -> usize {
    let relative_color = usize::from(
        piece
            .color()
            .expect("feature piece must have a valid color")
            != perspective,
    );
    let relative_rank = match perspective {
        Color::Black => square.rank(),
        Color::White => 11 - square.rank(),
    };
    let relative_square = relative_rank as usize * 12 + square.file() as usize;
    (relative_color * PIECE_STATE_COUNT + piece_state(piece)) * 144 + relative_square
}

/// 指定視点の先獅子対象升を、盤面特徴に続く144個の番号へ写す。
/// 段の変換は[`feature_index`]と同じである。
/// 入力に含める一時状態の範囲は[`active_features_for`]に従う。
pub fn lion_feature_index(perspective: Color, square: Square) -> usize {
    let relative_rank = match perspective {
        Color::Black => square.rank(),
        Color::White => 11 - square.rank(),
    };
    BOARD_FEATURE_COUNT + relative_rank as usize * 12 + square.file() as usize
}

/// 局面で有効な手番側視点の特徴番号を列挙する。
pub fn active_features(position: &Position, mut f: impl FnMut(usize)) {
    active_features_for(position.side_to_move(), position, &mut f);
}

/// 指定視点で発火する盤面特徴と、先獅子の対象升を列挙する。
/// 盤面以外の一時状態は先獅子の対象升だけを含める。
/// RULES.md第33条第5項の学習用規則セット`engine-default`（L0,P0,R1,E0）では、履歴による反復を除き、盤面以外に合法手を変える一時状態が先獅子だけだからである。
/// 第24条第1項dに当たる成り権保留の升集合は第30条のP1、P2、P5でだけ空でなくなり、この教師データでは常に空なので含めない。
/// 先獅子の麒麟成りフラグは第29条のL2でだけ合法手に影響するので含めない。
/// 別の規則で評価するときは、成り権保留を見ないことやL1などの先獅子の意味の差を静的評価の近似として受け入れる。
/// 重みファイルの規則セット名は生成時の来歴であり、実行時の規則との一致を要求しない。
pub fn active_features_for(perspective: Color, position: &Position, mut f: impl FnMut(usize)) {
    for square in position.occupied() {
        let piece = position
            .piece_at(square)
            .expect("occupied square must contain a piece");
        f(feature_index(perspective, piece, square));
    }
    if let Some(trigger) = position.lion_taken_by_non_lion() {
        f(lion_feature_index(perspective, trigger.square));
    }
}

/// 特徴を盤面の左右鏡映で対応する番号へ写す。
/// `feature`には[`FEATURE_COUNT`]未満の基底特徴番号を渡す。
pub const fn mirror_feature(feature: usize) -> usize {
    feature / 12 * 12 + 11 - feature % 12
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase_core::Position;
    use minase_core::test_util::{position_from_codes, sq};

    /// 駒コードの全定義域が47状態へ仕様どおり写ることを検査する。
    #[test]
    fn every_piece_code_maps_to_the_specified_state() {
        // `piece_state`のdocコメントが定める状態番号を固定する。
        // 右2列は未成コードと成駒コードの状態番号である。
        let states = [
            (PieceKind::Pawn, Some(29), None),
            (PieceKind::GoBetween, Some(30), None),
            (PieceKind::Lance, Some(31), None),
            (PieceKind::ReverseChariot, Some(32), None),
            (PieceKind::SideMover, Some(33), Some(4)),
            (PieceKind::VerticalMover, Some(34), Some(5)),
            (PieceKind::Bishop, Some(35), Some(6)),
            (PieceKind::Rook, Some(36), Some(7)),
            (PieceKind::DragonHorse, Some(37), Some(8)),
            (PieceKind::DragonKing, Some(38), Some(9)),
            (PieceKind::FreeKing, Some(10), Some(10)),
            (PieceKind::King, Some(11), None),
            (PieceKind::DrunkElephant, Some(39), Some(12)),
            (PieceKind::FerociousLeopard, Some(40), None),
            (PieceKind::BlindTiger, Some(41), None),
            (PieceKind::CopperGeneral, Some(42), None),
            (PieceKind::SilverGeneral, Some(43), None),
            (PieceKind::GoldGeneral, Some(44), Some(17)),
            (PieceKind::Kirin, Some(45), None),
            (PieceKind::Phoenix, Some(46), None),
            (PieceKind::Lion, Some(20), Some(20)),
            (PieceKind::CrownPrince, None, Some(21)),
            (PieceKind::WhiteHorse, None, Some(22)),
            (PieceKind::Whale, None, Some(23)),
            (PieceKind::FlyingOx, None, Some(24)),
            (PieceKind::FreeBoar, None, Some(25)),
            (PieceKind::FlyingStag, None, Some(26)),
            (PieceKind::HornedFalcon, None, Some(27)),
            (PieceKind::SoaringEagle, None, Some(28)),
        ];
        for color in Color::ALL {
            for (kind, unpromoted, promoted) in states {
                if let Some(expected) = unpromoted {
                    assert_eq!(piece_state(PieceCode::new(color, kind).unwrap()), expected);
                }
                if let Some(expected) = promoted {
                    assert_eq!(
                        piece_state(PieceCode::new_promoted(color, kind).unwrap()),
                        expected
                    );
                }
            }
        }
    }

    /// 段反転と陣営交換が後手視点を先手視点へ写すことを検査する。
    #[test]
    fn white_features_match_rank_reflection_with_colors_swapped() {
        let promoted_gold = PieceCode::new_promoted(Color::Black, PieceKind::GoldGeneral).unwrap();
        let promoted_lion = PieceCode::new_promoted(Color::White, PieceKind::Lion).unwrap();
        let fixture = position_from_codes(
            Color::White,
            &[
                (sq(1, 2), promoted_gold),
                (sq(7, 8), promoted_lion),
                (
                    sq(4, 6),
                    PieceCode::new(Color::White, PieceKind::King).unwrap(),
                ),
            ],
        );

        for (mut position, expected_count) in [(Position::initial(), 92), (fixture, 4)] {
            if position.side_to_move() == Color::Black {
                let pieces: Vec<_> = Square::all()
                    .filter_map(|square| position.piece_at(square).map(|piece| (square, piece)))
                    .collect();
                position = position_from_codes(Color::White, &pieces);
            } else {
                position.set_lion_capture(Some(sq(3, 5))).unwrap();
            }
            let mut white_features = Vec::new();
            active_features(&position, |feature| white_features.push(feature));
            assert_eq!(white_features.len(), expected_count);
            white_features.sort_unstable();

            let reflected = minase_core::test_util::reflect_ranks_and_swap_colors(&position);
            let mut black_features = Vec::new();
            active_features(&reflected, |feature| black_features.push(feature));
            black_features.sort_unstable();
            assert_eq!(white_features, black_features);
        }
    }
}
