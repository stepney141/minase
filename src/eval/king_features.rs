//! 王の遮蔽と開いた筋の24特徴。静的評価のたびに局面から計算する。
//!
//! 定義と列順の典拠は `docs/plans/strength-stage9.md` の項目1。

use crate::core::attacks::{attack_tables, movement_profile};
use crate::core::bitboard::FILE_MASKS;
use crate::core::direction::Direction;
use crate::{Bitboard, Color, PieceKind, Position, Square};

/// 王の安全度特徴の定義の識別値。
pub const DEFINITION_ID: u32 = 1;
/// 王の遮蔽と開いた筋の列数。
pub const COLUMN_COUNT: usize = 24;

/// 指定視点の王、相手の王の順に、王の遮蔽と開いた筋の特徴を返す。
pub fn extract(position: &Position, perspective: Color) -> [u8; COLUMN_COUNT] {
    let mut values = [0; COLUMN_COUNT];
    for (side, king_color) in [perspective, perspective.opposite()]
        .into_iter()
        .enumerate()
    {
        let royals = position.royal_pieces(king_color);
        if royals.popcount() != 1 {
            continue;
        }
        let king = royals.lsb().expect("one royal must exist");
        shelter(
            position,
            king_color,
            king,
            &mut values[side * 12..side * 12 + 12],
        );
    }
    values
}

/// 項目1の筋数を数える。走り駒の判定は現在の駒種の走り方向から導く。
fn shelter(position: &Position, king_color: Color, king: Square, values: &mut [u8]) {
    let attacker = king_color.opposite();
    let pawns = |color| {
        position.pieces_of_kind(color, PieceKind::Pawn)
            | position.pieces_of_kind(color, PieceKind::GoBetween)
    };
    // 歩兵と仲人は成ると別の駒種になるため、この集合には未成駒だけが入る。
    let defending_pawns = pawns(king_color);
    let attacking_pawns = pawns(attacker);
    let forward = shelter_tables().forward[king_color.index()][king.rank() as usize];
    let mut sliders = Bitboard::EMPTY;
    for &kind in &shelter_tables().vertical_sliders[attacker.index()] {
        sliders |= position.pieces_of_kind(attacker, kind);
    }
    for df in -2_i8..=2 {
        let Some(square) = king.offset(df, 0) else {
            continue;
        };
        let file = FILE_MASKS[square.file() as usize] & forward;
        if file.intersects(defending_pawns) {
            continue;
        }
        let distance = df.unsigned_abs() as usize;
        let state = if file.intersects(attacking_pawns) {
            0
        } else {
            3
        };
        increment(&mut values[state + distance]);
        match (file & sliders).popcount() {
            0 => {}
            1 => increment(&mut values[6 + distance]),
            _ => increment(&mut values[9 + distance]),
        }
    }
}

/// 項目1が評価のたびに引く、段ごとの前方の升の集合と縦の走り駒の駒種。
struct ShelterTables {
    /// 守る側と王駒の段ごとの、王駒より厳密に前方の升の集合。
    forward: [[Bitboard; 12]; 2],
    /// 攻める側ごとの、前方へ任意の升数を進める駒種。
    vertical_sliders: [Vec<PieceKind>; 2],
}

/// 項目1の表を初回だけ作って返す。
fn shelter_tables() -> &'static ShelterTables {
    static TABLES: std::sync::OnceLock<ShelterTables> = std::sync::OnceLock::new();
    TABLES.get_or_init(|| {
        let mut forward = [[Bitboard::EMPTY; 12]; 2];
        let mut vertical_sliders = [Vec::new(), Vec::new()];
        for color in [Color::Black, Color::White] {
            for rank in 0..12_u8 {
                forward[color.index()][rank as usize] =
                    Bitboard::from_squares(Square::all().filter(|square| match color {
                        Color::Black => square.rank() > rank,
                        Color::White => square.rank() < rank,
                    }));
            }
            let direction = match color {
                Color::Black => Direction::North,
                Color::White => Direction::South,
            };
            for kind in PieceKind::ALL {
                if attack_tables().slide_directions(color, movement_profile(kind))
                    & (1 << direction.index())
                    != 0
                {
                    vertical_sliders[color.index()].push(kind);
                }
            }
        }
        ShelterTables {
            forward,
            vertical_sliders,
        }
    })
}

/// 定義違反による桁あふれは、リリースビルドでも検出する。
fn increment(value: &mut u8) {
    *value = value.checked_add(1).expect("king feature exceeds u8");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{position, position_from_codes, sampled_random_positions, sq};
    use crate::{Game, MoveRules, PieceCode, Rules};
    use Color::{Black, White};
    use PieceKind::{King, Pawn, Rook};

    #[test]
    fn loss_after_173_moves_matches_the_design_example() {
        // 項目1の例: 玉3aに対し、歩は敵4筋・自5筋、縦走りは3筋1枚、2筋1枚、1筋2枚。
        let document = include_str!("../../docs/research/lishogi-game-yO464dzl-loss-analysis.md");
        let moves = document
            .split("## 再現用の着手列")
            .nth(1)
            .unwrap()
            .split("```text")
            .nth(1)
            .unwrap()
            .split("```")
            .next()
            .unwrap();
        let mut game = Game::from_position(Rules::LISHOGI, Position::initial());
        for text in moves.split_whitespace().take(173) {
            let mv = crate::notation::usi::parse(game.position(), text).unwrap();
            game.play(mv).unwrap();
        }
        assert_eq!(game.position().side_to_move(), White);
        let values = extract(game.position(), White);
        assert_eq!(&values[..12], &[0, 1, 0, 1, 1, 1, 1, 1, 0, 0, 0, 1]);
        // 図の先手王7lの対象は5〜9筋。全筋のh段またはi段に自歩・仲人がある。
        // 項目1「守る側の歩がある」には特徴を割り当てず、走りの特徴も対象外。
        assert_eq!(&values[12..24], &[0; 12]);
    }

    #[test]
    fn edges_and_corners_count_only_on_board_files() {
        // 項目1「盤外の筋を数えない」: 端では距離0,1,2に各1筋だけ。
        for king in [sq(0, 0), sq(0, 5), sq(11, 11), sq(11, 5)] {
            let p = position(Black, &[(king, Black, King)]);
            assert_eq!(
                &extract(&p, Black)[..12],
                &[0, 0, 0, 1, 1, 1, 0, 0, 0, 0, 0, 0]
            );
        }
    }

    #[test]
    fn pawns_on_the_royal_rank_are_not_shelter() {
        // 項目1「段より厳密に前方」。同段の自歩も敵仲人も、後方の自歩も含めない。
        let p = position(
            Black,
            &[
                (sq(5, 5), Black, King),
                (sq(4, 5), Black, Pawn),
                (sq(6, 5), White, PieceKind::GoBetween),
                (sq(5, 4), Black, Pawn),
            ],
        );
        assert_eq!(
            &extract(&p, Black)[..12],
            &[0, 0, 0, 1, 2, 2, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn each_side_file_counts_one_slider_regardless_of_blockers() {
        // 項目1は筋数: 左右各1枚なら距離1の「1枚」が2。遮蔽は見ない。
        // 歩でなく横行を遮蔽に使う。
        let pieces = [
            (sq(5, 2), Black, King),
            (sq(4, 9), White, Rook),
            (sq(6, 9), White, Rook),
        ];
        let open = position(Black, &pieces);
        let mut blocked_pieces = pieces.to_vec();
        blocked_pieces.extend([
            (sq(4, 6), Black, PieceKind::SideMover),
            (sq(6, 6), Black, PieceKind::SideMover),
        ]);
        let blocked = position(Black, &blocked_pieces);
        let a = extract(&open, Black);
        let b = extract(&blocked, Black);
        assert_eq!(&a[..12], &[0, 0, 0, 1, 2, 2, 0, 2, 0, 0, 0, 0]);
        assert_eq!(&a[..24], &b[..24]);
    }

    #[test]
    fn promoted_movement_and_unpromoted_pawns_follow_the_current_kind() {
        // 第10・11条と項目1: 成歩の金将は歩の遮蔽でなく、成竪行の飛牛は縦に走る。
        // 角鷹は前方が2升移動なので「任意の升数」の前方走りには入らない。
        let p = position_from_codes(
            Black,
            &[
                (sq(5, 2), PieceCode::new(Black, King).unwrap()),
                (
                    sq(5, 4),
                    PieceCode::new_promoted(Black, PieceKind::GoldGeneral).unwrap(),
                ),
                (
                    sq(4, 9),
                    PieceCode::new_promoted(White, PieceKind::FlyingOx).unwrap(),
                ),
                (
                    sq(6, 9),
                    PieceCode::new_promoted(White, PieceKind::HornedFalcon).unwrap(),
                ),
            ],
        );
        assert_eq!(
            &extract(&p, Black)[..12],
            &[0, 0, 0, 1, 2, 2, 0, 1, 0, 0, 0, 0]
        );
    }

    #[test]
    fn only_sides_with_exactly_one_royal_have_features() {
        // 共通定義「ちょうど1枚の側だけ」。王のない側、片側2枚、双方2枚も対象外。
        let kings = [
            (sq(2, 2), PieceCode::new(Black, King).unwrap()),
            (sq(9, 9), PieceCode::new(White, King).unwrap()),
        ];
        let base = extract(&position_from_codes(Black, &kings), Black);
        for (black_count, white_count) in [(0, 1), (1, 0), (2, 1), (1, 2), (2, 2), (0, 0)] {
            let mut pieces = Vec::new();
            for (side, count) in [black_count, white_count].into_iter().enumerate() {
                if count > 0 {
                    pieces.push(kings[side]);
                }
                if count == 2 {
                    pieces.push((
                        sq(5, if side == 0 { 0 } else { 11 }),
                        PieceCode::new_promoted(Color::ALL[side], PieceKind::CrownPrince).unwrap(),
                    ));
                }
            }
            let v = extract(&position_from_codes(Black, &pieces), Black);
            for (side, count) in [black_count, white_count].into_iter().enumerate() {
                let columns = side * 12..(side + 1) * 12;
                if count == 1 {
                    assert_eq!(v[columns.clone()], base[columns]);
                } else {
                    assert!(v[columns].iter().all(|&x| x == 0));
                }
            }
        }
    }

    #[test]
    fn seeded_positions_obey_perspective_and_board_symmetries() {
        // 設計書「検証」: 手番だけの交換は2区画を交換する。
        // 陣営・段・手番の同時反転と左右鏡映は列を変えない。
        for p in sampled_random_positions(MoveRules::standard()) {
            let v = extract(&p, p.side_to_move());
            let switched = p.clone_with_side_to_move(p.side_to_move().opposite());
            let swapped = extract(&switched, switched.side_to_move());
            assert_eq!(v[..12], swapped[12..]);
            assert_eq!(v[12..], swapped[..12]);
            for reflect_rank in [false, true] {
                let pieces: Vec<_> = p
                    .occupied()
                    .into_iter()
                    .map(|s| {
                        let piece = p.piece_at(s).unwrap();
                        let color = if reflect_rank {
                            piece.color().unwrap().opposite()
                        } else {
                            piece.color().unwrap()
                        };
                        let mapped = if piece.is_promoted() {
                            PieceCode::new_promoted(color, piece.kind().unwrap())
                        } else {
                            PieceCode::new(color, piece.kind().unwrap())
                        }
                        .unwrap();
                        let square = if reflect_rank {
                            sq(s.file(), 11 - s.rank())
                        } else {
                            sq(11 - s.file(), s.rank())
                        };
                        (square, mapped)
                    })
                    .collect();
                let side = if reflect_rank {
                    p.side_to_move().opposite()
                } else {
                    p.side_to_move()
                };
                assert_eq!(v, extract(&position_from_codes(side, &pieces), side));
            }
        }
    }
}
