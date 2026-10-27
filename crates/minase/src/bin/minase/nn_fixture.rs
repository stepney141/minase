//! 学習器との符号化の照合と推論の比較に使う固定局面集を書き出す。

use std::fs;
use std::num::{NonZeroU64, NonZeroUsize};
use std::path::PathBuf;
use std::process;

use clap::Parser;
use minase::nn::{INPUT_SIZE, encode, policy_label};
use minase_core::notation::sfen::{SetupPosition, to_extended_sfen};
use minase_core::rng::XorShift64;
use minase_core::{
    Color, MoveGenerator, PieceCode, PieceKind, Position, PositionBuilder, Rules, Square,
};
use serde::Serialize;

/// 固定局面集の書き出し先。
#[derive(Parser)]
pub(super) struct Arguments {
    /// JSON Lines形式で書き出すファイル。既存のファイルは上書きする。
    #[arg(long)]
    out: PathBuf,
}

/// 初期局面とランダム対局から採る局面数。
const GAME_POSITION_COUNT: usize = 284;

/// 初期局面、ランダム対局の283局面、および手で組んだ4局面を、この順に返す。
///
/// 規則は`engine-default`、乱数は固定シード2026の`XorShift64`を対局間で継続する。
/// 各対局は最大1,024手とし、合法手がなくなるか、どちらかの王駒がなくなれば打ち切る。
/// 序盤から終盤まで一定の17手間隔で採り、奇数間隔により先手番と後手番を交互に含める。
/// 手で組む4局面は、獅子の2歩と長い走りを持つ疎な盤面、および両陣営の成駒と先獅子を持つ盤面を、両手番で用意する。
pub(crate) fn fixture_positions() -> Vec<Position> {
    let rules = Rules::ENGINE_DEFAULT.moves;
    let generator = MoveGenerator::new(rules);
    let mut rng = XorShift64::new(NonZeroU64::new(2026).expect("fixture seed is nonzero"));
    let mut positions = vec![Position::initial()];
    let mut moves = Vec::new();
    while positions.len() < GAME_POSITION_COUNT {
        let mut position = Position::initial();
        for ply in 0..1024 {
            if Color::ALL
                .iter()
                .any(|&side| position.royal_pieces(side).is_empty())
            {
                break;
            }
            moves.clear();
            generator.generate_moves(&position, &mut moves);
            let Some(count) = NonZeroUsize::new(moves.len()) else {
                break;
            };
            if ply > 0 && ply % 17 == 0 {
                positions.push(position.clone());
                if positions.len() == GAME_POSITION_COUNT {
                    break;
                }
            }
            position.make_move_unchecked(moves[rng.index(count)], rules);
        }
    }

    for side in Color::ALL {
        let opponent = side.opposite();
        let mut builder = PositionBuilder::new(side);
        for (file, rank, color, kind) in [
            (0, 0, side, PieceKind::King),
            (11, 11, opponent, PieceKind::King),
            (5, 5, side, PieceKind::Lion),
            (5, 6, opponent, PieceKind::Pawn),
            (6, 6, opponent, PieceKind::Pawn),
            (1, 3, side, PieceKind::Rook),
            (8, 8, side, PieceKind::Phoenix),
            (3, 8, side, PieceKind::Kirin),
        ] {
            builder
                .put(
                    fixture_square(file, rank, side),
                    PieceCode::new(color, kind).expect("fixture kind has an unpromoted code"),
                )
                .expect("fixture squares are distinct");
        }
        positions.push(builder.finish().expect("fixture position is valid"));

        let mut builder = PositionBuilder::new(side);
        for (file, rank, color, kind, promoted) in [
            (0, 0, side, PieceKind::King, false),
            (11, 11, opponent, PieceKind::King, false),
            (2, 3, side, PieceKind::GoldGeneral, true),
            (5, 5, side, PieceKind::HornedFalcon, true),
            (8, 8, opponent, PieceKind::Lion, true),
            (10, 8, opponent, PieceKind::FreeKing, true),
        ] {
            let piece = if promoted {
                PieceCode::new_promoted(color, kind)
            } else {
                PieceCode::new(color, kind)
            }
            .expect("fixture kind has the specified promotion state");
            builder
                .put(fixture_square(file, rank, side), piece)
                .expect("fixture squares are distinct");
        }
        let mut position = builder.finish().expect("fixture position is valid");
        position
            .set_lion_capture(Some(fixture_square(8, 8, side)))
            .expect("fixture lion capture square belongs to the opponent");
        positions.push(position);
    }
    positions
}

/// 手で組む局面の座標を手番側から見た座標へそろえる。
fn fixture_square(file: u8, rank: u8, side: Color) -> Square {
    let (file, rank) = match side {
        Color::Black => (file, rank),
        Color::White => (11 - file, 11 - rank),
    };
    Square::new(file, rank).expect("fixture square is on the board")
}

/// 1行の交換形式。構造体のフィールド順をJSONのキー順として保持する。
#[derive(Serialize)]
struct Fixture {
    sfen: String,
    side_to_move: &'static str,
    /// 密番号順の144升。駒は現在の陣営・駒種番号・成りビットで表す。
    board: Vec<Option<(usize, usize, bool)>>,
    lion_capture: Option<usize>,
    /// 入力平面で1.0になる添字の昇順列。
    input: Vec<usize>,
    /// 合法手の生成順に、移動元・中継升・移動先・成り・ラベルを並べる。
    moves: Vec<(usize, Option<usize>, usize, bool, usize)>,
}

/// 固定局面集をUTF-8のJSON Linesへ直列化する。最終行にも改行を付ける。
fn fixture_bytes() -> Result<Vec<u8>, serde_json::Error> {
    let generator = MoveGenerator::new(Rules::ENGINE_DEFAULT.moves);
    let mut bytes = Vec::new();
    let mut input = vec![0.0; INPUT_SIZE];
    let mut moves = Vec::new();
    for position in fixture_positions() {
        let setup = SetupPosition::new(position.clone(), position.lion_capture_square(), 1)
            .expect("fixed positions and move number are valid");
        encode(&position, &mut input).expect("engine-default fixtures have no deferred promotion");
        moves.clear();
        generator.generate_moves(&position, &mut moves);
        let fixture = Fixture {
            sfen: to_extended_sfen(&setup),
            side_to_move: match position.side_to_move() {
                Color::Black => "black",
                Color::White => "white",
            },
            board: Square::all()
                .map(|square| {
                    position.piece_at(square).map(|piece| {
                        (
                            piece.color().expect("board piece has a color").index(),
                            piece.kind().expect("board piece has a kind").index(),
                            piece.is_promoted(),
                        )
                    })
                })
                .collect(),
            lion_capture: position.lion_capture_square().map(Square::dense_index),
            input: input
                .iter()
                .enumerate()
                .filter_map(|(index, &value)| (value == 1.0).then_some(index))
                .collect(),
            moves: moves
                .iter()
                .map(|&mv| {
                    (
                        mv.from.dense_index(),
                        mv.mid.map(Square::dense_index),
                        mv.to.dense_index(),
                        mv.promote,
                        policy_label(mv, position.side_to_move()),
                    )
                })
                .collect(),
        };
        serde_json::to_writer(&mut bytes, &fixture)?;
        bytes.push(b'\n');
    }
    Ok(bytes)
}

/// 指定されたファイルへ固定局面集を書き出し、失敗は標準エラーへ報告する。
pub(super) fn main(arguments: Arguments) {
    let bytes = match fixture_bytes() {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("failed to serialize NN fixtures: {error}");
            process::exit(1);
        }
    };
    if let Err(error) = fs::write(&arguments.out, bytes) {
        eprintln!(
            "failed to write NN fixtures to {}: {error}",
            arguments.out.display()
        );
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase::nn::POLICY_SIZE;
    use minase_core::notation::sfen::parse_extended_sfen;

    const FIXTURE_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/train/zero/fixtures/nn-positions.jsonl"
    );
    const REGENERATE: &str = "再生成: cargo run --release --offline --bin minase -- dev nn-fixture --out tools/train/zero/fixtures/nn-positions.jsonl";

    // [`fixture_positions`]の契約: 288局面が両手番・先獅子・両軍の成駒・全ラベル族・長い移動を含む。
    #[test]
    fn fixture_positions_cover_both_sides_special_states_and_move_families() {
        let positions = fixture_positions();
        assert_eq!(positions.len(), 288);
        assert_eq!(positions[0], Position::initial());
        let generator = MoveGenerator::new(Rules::ENGINE_DEFAULT.moves);
        let mut sides = [0; 2];
        let mut lion_captures = [0; 2];
        let mut families = [0; 6];
        let mut both_sides_promoted = false;
        let mut long_move = false;
        let mut moves = Vec::new();
        for position in positions {
            let side = position.side_to_move();
            sides[side.index()] += 1;
            lion_captures[side.index()] += usize::from(position.lion_capture_square().is_some());
            both_sides_promoted |= Color::ALL.iter().all(|&color| {
                position
                    .pieces_of(color)
                    .into_iter()
                    .any(|square| position.piece_at(square).unwrap().is_promoted())
            });
            assert!(position.promotion_deferred().is_empty());
            moves.clear();
            generator.generate_moves(&position, &mut moves);
            for &mv in &moves {
                let label = policy_label(mv, side);
                assert!(label < POLICY_SIZE);
                let family = match label / 144 {
                    0..=87 => 0,
                    88..=175 => 1,
                    176..=183 => 2,
                    184..=239 => 3,
                    240..=247 => 4,
                    248 => 5,
                    _ => unreachable!(),
                };
                families[family] += 1;
                long_move |= mv.from.file().abs_diff(mv.to.file()) >= 6
                    || mv.from.rank().abs_diff(mv.to.rank()) >= 6;
            }
        }
        assert!(sides.iter().all(|&count| count >= 100), "{sides:?}");
        assert!(lion_captures.iter().all(|&count| count >= 1));
        assert!(families.iter().all(|&count| count >= 1), "{families:?}");
        assert!(both_sides_promoted);
        assert!(long_move);
    }

    // [`fixture_bytes`]の契約: 学習器の参照ファイルとバイト単位で一致し、SFENから元の局面を復元できる。
    #[test]
    fn generated_jsonl_matches_checked_in_fixture() {
        let bytes = fixture_bytes().unwrap();
        let expected = fs::read(FIXTURE_PATH).expect(REGENERATE);
        assert!(bytes == expected, "固定局面集が一致しない。{REGENERATE}");
        assert_eq!(bytes.last(), Some(&b'\n'));
        let lines: Vec<_> = std::str::from_utf8(&bytes).unwrap().lines().collect();
        assert_eq!(lines.len(), 288);
        for (line, position) in lines.into_iter().zip(fixture_positions()) {
            let keys = [
                "sfen",
                "side_to_move",
                "board",
                "lion_capture",
                "input",
                "moves",
            ]
            .map(|key| line.find(&format!("\"{key}\":")).unwrap());
            assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["board"].as_array().unwrap().len(), 144);
            let restored =
                parse_extended_sfen(value["sfen"].as_str().unwrap(), Rules::ENGINE_DEFAULT.moves)
                    .unwrap()
                    .restore_position()
                    .unwrap();
            assert_eq!(restored, position);
        }
    }
}
