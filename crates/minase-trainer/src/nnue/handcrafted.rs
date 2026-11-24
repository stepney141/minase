//! 手書き評価v0を幅256のA型LayerStackへ書き出す。PSQT以外はすべて0とする。

use crate::nn::hex;
use minase::eval::{
    features::{FEATURE_COUNT, PIECE_STATE_COUNT, piece_state},
    handcrafted,
    nnue::{FormatError, LayerStackNetwork, LayerStackParameters, Network},
};
use minase_core::{Color, PieceCode, PieceKind};
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::PathBuf};

/// v0書き出しの失敗。
#[derive(Debug)]
pub enum Error {
    /// tatara形式の符号化または復号の失敗。
    Format(FormatError),
    /// ファイル操作の失敗。
    Io(std::io::Error),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Format(e) => e.fmt(f),
            Self::Io(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Format(e) => Some(e),
            Self::Io(e) => Some(e),
        }
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
/// v0書き出しの処理結果。
pub type Result<T> = std::result::Result<T, Error>;

/// v0を書き出す新規ファイルのパス。
#[derive(clap::Args)]
pub struct Args {
    /// 新規作成するtatara形式のネットのパスを指定する。既存のパスは拒否する。
    #[arg(long)]
    pub output: PathBuf,
}

fn parameters() -> LayerStackParameters {
    const WIDTH: usize = 256;
    let mut psqt = vec![0; FEATURE_COUNT];
    for kind in PieceKind::ALL {
        let states = std::iter::once(kind.index())
            .chain(PieceCode::new(Color::Black, kind).map(piece_state));
        for state in states {
            for side in 0..2 {
                for rank in 0..12_u8 {
                    for file in 0..12_u8 {
                        let owner_rank = if side == 0 { rank } else { 11 - rank };
                        let sign = if side == 0 { 1 } else { -1 };
                        let feature = (side * PIECE_STATE_COUNT + state) * 144
                            + usize::from(rank) * 12
                            + usize::from(file);
                        psqt[feature] = sign
                            * (handcrafted::piece_value(kind)
                                + handcrafted::square_value(kind, owner_rank, file));
                    }
                }
            }
        }
    }
    LayerStackParameters {
        width: WIDTH,
        fv_scale: 1,
        ft_biases: vec![0; WIDTH],
        ft_weights: vec![0; FEATURE_COUNT * WIDTH],
        psqt_bias: 0,
        psqt_weights: psqt,
        l1_biases: vec![0; 16],
        l1_weights: vec![0; 16 * WIDTH],
        l2_biases: vec![0; 32],
        l2_weights: vec![0; 32 * 30],
        output_bias: 0,
        output_weights: vec![0; 32],
    }
}

/// v0の駒割と位置評価をPSQTへ置き、残りの重みとバイアスを0として新規作成する。
/// 後段の寄与がないので、PSQTを除数に合わせればv0を正確に再現でき、`fv_scale`は1に固定する。
/// 第1世代の教師はこのネットから生成し、教師の勝率尺度K_tはその教師から較正する。
/// 書き出しにK_tを要求すると依存関係が循環するため、勝率尺度の引数は設けない。
/// 書き出し後は読み直して検証し、パス、`fv_scale`、ファイル全体のSHA-256を各1行で出力する。
///
/// # Errors
/// 重みが形式の制約に違反する場合、出力先が存在する場合、入出力に失敗した場合に返す。
pub fn run(args: Args) -> Result<()> {
    let bytes = LayerStackNetwork::new(parameters())
        .map_err(Error::Format)?
        .encode();
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args.output)?;
    output.write_all(&bytes)?;
    output.sync_all()?;
    let bytes = fs::read(&args.output)?;
    let net = Network::decode(&bytes).map_err(Error::Format)?;
    writeln!(
        std::io::stdout().lock(),
        "{}\n{}\n{}",
        args.output.display(),
        net.fv_scale(),
        hex(&Sha256::digest(&bytes))
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase::eval::features::BOARD_FEATURE_COUNT;
    use minase_core::{
        MoveGenerator, Position, Rules,
        rng::{XorShift64, derive_seed},
        test_util::{
            bench_positions, capture_test_positions, position_from_codes, sampled_random_positions,
            sq,
        },
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "nnue-handcrafted-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn export(&self) -> Network {
            let output = self.0.join("v0.bin");
            run(Args {
                output: output.clone(),
            })
            .unwrap();
            Network::decode(&fs::read(output).unwrap()).unwrap()
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn assert_evaluation(net: &Network, position: &Position) {
        assert_eq!(
            net.evaluate(position).unwrap(),
            handcrafted::evaluate(position),
            "{}",
            minase_core::to_sfen(position)
        );
    }

    fn assert_both_sides(net: &Network, original: &Position) {
        let pieces: Vec<_> = original
            .occupied()
            .into_iter()
            .map(|square| (square, original.piece_at(square).unwrap()))
            .collect();
        for side in Color::ALL {
            let position = position_from_codes(side, &pieces);
            assert_evaluation(net, &position);
            // 先獅子の捕獲者は手番側の相手なので、手番だけ反転した局面へ移せない。
            // 元の局面を両視点で評価し、v0が先獅子の有無に依存しないことも検査する。
            assert_eq!(
                net.evaluate_for(original, side).unwrap(),
                handcrafted::evaluate(&position),
                "{}: {side:?}",
                minase_core::to_sfen(original)
            );
        }
    }

    #[test]
    fn exported_evaluation_matches_v0_on_fixed_positions_and_promotions() {
        let directory = Directory::new();
        let net = directory.export();
        // bench標本は初期局面から終盤までを含む。配置を保ち、両方の手番を検査する。
        for original in bench_positions()
            .into_iter()
            .chain(sampled_random_positions(Rules::LISHOGI.moves))
            .chain(capture_test_positions())
        {
            assert_both_sides(&net, &original);
        }
        // 歩兵成りの金、角行成りの龍馬、龍馬成りの角鷹、麒麟成りの獅子、太子を明示する。
        let pieces = [
            (
                sq(4, 0),
                PieceCode::new(Color::Black, PieceKind::King).unwrap(),
            ),
            (
                sq(7, 11),
                PieceCode::new(Color::White, PieceKind::King).unwrap(),
            ),
            (
                sq(2, 8),
                PieceCode::new_promoted(Color::Black, PieceKind::GoldGeneral).unwrap(),
            ),
            (
                sq(9, 2),
                PieceCode::new_promoted(Color::White, PieceKind::DragonHorse).unwrap(),
            ),
            (
                sq(3, 7),
                PieceCode::new_promoted(Color::Black, PieceKind::HornedFalcon).unwrap(),
            ),
            (
                sq(5, 4),
                PieceCode::new_promoted(Color::White, PieceKind::Lion).unwrap(),
            ),
            (
                sq(5, 1),
                PieceCode::new_promoted(Color::Black, PieceKind::CrownPrince).unwrap(),
            ),
        ];
        for side in Color::ALL {
            let mut position = position_from_codes(side, &pieces);
            assert_evaluation(&net, &position);
            let target = if side == Color::Black {
                sq(9, 2)
            } else {
                sq(2, 8)
            };
            position.set_lion_capture(Some(target)).unwrap();
            assert_evaluation(&net, &position);
            // 後手の王将を取り去った終局直後も、手書き評価と一致する。
            let without_white_king: Vec<_> = pieces
                .iter()
                .copied()
                .filter(|(_, piece)| {
                    !(piece.color() == Some(Color::White) && piece.kind() == Some(PieceKind::King))
                })
                .collect();
            assert_evaluation(&net, &position_from_codes(side, &without_white_king));
        }
        // 盤上に置ける全駒コードの位置項を、前進と中央性が異なる段・筋で照合する。
        for owner in Color::ALL {
            for kind in PieceKind::ALL {
                for piece in [
                    PieceCode::new(owner, kind),
                    PieceCode::new_promoted(owner, kind),
                ]
                .into_iter()
                .flatten()
                {
                    for rank in 0..12 {
                        for file in [0, 5, 6, 11] {
                            for side in Color::ALL {
                                assert_evaluation(
                                    &net,
                                    &position_from_codes(side, &[(sq(file, rank), piece)]),
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn exported_evaluation_matches_v0_on_4096_random_positions() {
        let directory = Directory::new();
        let net = directory.export();
        let rules = Rules::LISHOGI.moves;
        let generator = MoveGenerator::new(rules);
        let mut visited = 0;
        for game in 0..16 {
            let mut rng = XorShift64::new(derive_seed(0x7630_4d4e_5545, game));
            let mut position = Position::initial();
            for _ in 0..256 {
                assert_both_sides(&net, &position);
                visited += 1;
                let mut moves = Vec::new();
                generator.generate_moves(&position, &mut moves);
                // 長い対局列を保つため、王駒を取る手だけを標本から除く。
                moves.retain(|&mv| {
                    position
                        .captured_squares(mv)
                        .into_iter()
                        .flatten()
                        .all(|square| {
                            !matches!(
                                position.piece_at(square).unwrap().kind(),
                                Some(PieceKind::King | PieceKind::CrownPrince)
                            )
                        })
                });
                assert!(!moves.is_empty());
                position.make_move_unchecked(moves[rng.next() as usize % moves.len()], rules);
            }
        }
        assert_eq!(visited, 4096);
    }

    #[test]
    fn export_preserves_width_scale_and_zero_parameters() {
        let directory = Directory::new();
        let net = directory.export();
        assert_eq!(net.width(), 256);
        assert_eq!(net.fv_scale(), 1);
        assert!(matches!(net, Network::LayerStack(_)));
        let p = parameters();
        assert!(p.ft_biases.iter().all(|&x| x == 0));
        assert!(p.ft_weights.iter().all(|&x| x == 0));
        assert!(p.l1_biases.iter().all(|&x| x == 0));
        assert!(p.l1_weights.iter().all(|&x| x == 0));
        assert!(p.l2_biases.iter().all(|&x| x == 0));
        assert!(p.l2_weights.iter().all(|&x| x == 0));
        assert_eq!(p.output_bias, 0);
        assert!(p.output_weights.iter().all(|&x| x == 0));
        assert_eq!(p.psqt_bias, 0);
        assert!(
            p.psqt_weights[BOARD_FEATURE_COUNT..]
                .iter()
                .all(|&x| x == 0)
        );
    }

    #[test]
    fn export_rejects_existing_paths_without_changing_contents() {
        let directory = Directory::new();
        let output = directory.0.join("existing.bin");
        fs::write(&output, b"keep this file").unwrap();
        let error = run(Args {
            output: output.clone(),
        })
        .unwrap_err();
        assert!(matches!(error, Error::Io(ref e) if e.kind() == std::io::ErrorKind::AlreadyExists));
        assert_eq!(fs::read(&output).unwrap(), b"keep this file");
        assert!(
            run(Args {
                output: directory.0.clone()
            })
            .is_err()
        );
        assert!(directory.0.is_dir());
    }

    #[test]
    fn export_requires_only_output() {
        use clap::Parser;
        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            args: Args,
        }
        assert_eq!(
            Cli::try_parse_from(["export"]).err().unwrap().kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
        let cli = Cli::try_parse_from(["export", "--output", "v0.bin"]).unwrap();
        assert_eq!(cli.args.output, PathBuf::from("v0.bin"));
    }
}
