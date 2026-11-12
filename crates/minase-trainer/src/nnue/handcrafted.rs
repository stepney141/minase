//! 手書き評価v0を幅256のP型MNUEへ書き出す。非線形部分はすべて0とする。

use super::model::{Error, Result};
use crate::{nn::hex, pst::v0_piece_values};
use minase::eval::{
    Nnue,
    features::{FEATURE_COUNT, PIECE_STATE_COUNT, piece_state},
    handcrafted,
    nnue::{
        self, EncodeInput,
        feature_set::{FeatureSet, P},
        head::{Head, PairwiseL16L32},
    },
};
use minase_core::{Color, PieceCode, PieceKind, Rules};
use std::{fs, io::Write, path::PathBuf};

/// v0を書き出す勝率尺度と新規ファイルのパス。どちらも必須である。
#[derive(clap::Args)]
pub struct Args {
    /// 1ロジットあたりのセンチポーンを、正の有限値かつ100,000以下で指定する。
    #[arg(long)]
    pub cp_per_logit: f32,
    /// 新規作成するMNUEのパスを指定する。既存のパスは拒否する。
    #[arg(long)]
    pub output: PathBuf,
}

/// v0の駒割と位置評価を両端点に置き、非線形部分を0として新規作成する。
/// 書き出し後はMNUEを読み直して検証し、パス、勝率尺度、本体のSHA-256を各1行で出力する。
///
/// # Errors
/// 勝率尺度または重みがMNUEの制約に違反する場合、出力先が存在する場合、入出力に失敗した場合に返す。
pub fn run(args: Args) -> Result<()> {
    const WIDTH: usize = 256;
    let mut linear = vec![[0; 2]; FEATURE_COUNT];
    for kind in PieceKind::ALL {
        let states = std::iter::once(kind.index())
            .chain(PieceCode::new(Color::Black, kind).map(piece_state));
        for state in states {
            for side in 0..2 {
                for rank in 0..12_u8 {
                    for file in 0..12_u8 {
                        let owner_rank = if side == 0 { rank } else { 11 - rank };
                        let sign = if side == 0 { 1 } else { -1 };
                        let value = sign
                            * 8
                            * (handcrafted::piece_value(kind)
                                + handcrafted::square_value(kind, owner_rank, file));
                        let feature = (side * PIECE_STATE_COUNT + state) * 144
                            + usize::from(rank) * 12
                            + usize::from(file);
                        let value = i16::try_from(value).map_err(|_| {
                            Error::Mnue(nnue::Error::ParameterOutOfRange {
                                parameter: "linear",
                                index: feature,
                                value,
                            })
                        })?;
                        linear[feature] = [value; 2];
                    }
                }
            }
        }
    }
    let head = PairwiseL16L32::decode(&vec![0; PairwiseL16L32::byte_len(WIDTH)], WIDTH, true)
        .map_err(Error::Mnue)?;
    let bytes = nnue::encode::<P, PairwiseL16L32>(&EncodeInput {
        width: WIDTH as u32,
        cp_per_logit: args.cp_per_logit,
        rule_set: &Rules::JCSA.to_string(),
        piece_values: &v0_piece_values(),
        linear: &linear,
        weights: &vec![0; P::DIMENSIONS * WIDTH],
        biases: &[0; WIDTH],
        head: &head,
    })
    .map_err(Error::Mnue)?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args.output)?;
    output.write_all(&bytes)?;
    output.sync_all()?;
    let net = Nnue::decode(&fs::read(&args.output)?).map_err(Error::Mnue)?;
    writeln!(
        std::io::stdout().lock(),
        "{}\n{}\n{}",
        args.output.display(),
        net.cp_per_logit(),
        hex(net.checksum())
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase::eval::features::BOARD_FEATURE_COUNT;
    use minase_core::{
        MoveGenerator, Position,
        rng::{XorShift64, derive_seed},
        test_util::{bench_positions, position_from_codes, sq},
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

        fn export(&self, cp_per_logit: f32) -> Nnue {
            let output = self.0.join("v0.bin");
            run(Args {
                cp_per_logit,
                output: output.clone(),
            })
            .unwrap();
            Nnue::decode(&fs::read(output).unwrap()).unwrap()
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn assert_evaluation(net: &Nnue, position: &Position) {
        assert_eq!(
            nnue::evaluate(net, position),
            handcrafted::evaluate(position),
            "{}",
            minase_core::to_sfen(position)
        );
    }

    #[test]
    fn exported_evaluation_matches_v0_on_fixed_positions_and_promotions() {
        let directory = Directory::new();
        let net = directory.export(1_117.25);
        // bench標本は初期局面から終盤までを含む。配置を保ち、両方の手番を検査する。
        for original in bench_positions() {
            let pieces: Vec<_> = original
                .occupied()
                .into_iter()
                .map(|square| (square, original.piece_at(square).unwrap()))
                .collect();
            for side in Color::ALL {
                assert_evaluation(&net, &position_from_codes(side, &pieces));
            }
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
        let net = directory.export(731.5);
        let rules = Rules::JCSA.moves;
        let generator = MoveGenerator::new(rules);
        let mut visited = 0;
        for game in 0..16 {
            let mut rng = XorShift64::new(derive_seed(0x7630_4d4e_5545, game));
            let mut position = Position::initial();
            for _ in 0..256 {
                assert_evaluation(&net, &position);
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
    fn export_preserves_v0_search_values_scale_rules_and_zero_nonlinear_parameters() {
        for cp_per_logit in [f32::MIN_POSITIVE, 913.75, 100_000.0] {
            let directory = Directory::new();
            let net = directory.export(cp_per_logit);
            assert_eq!(net.width(), 256);
            assert_eq!(net.cp_per_logit().to_bits(), cp_per_logit.to_bits());
            assert_eq!(net.rule_set(), Rules::JCSA.to_string());
            for kind in PieceKind::ALL {
                assert_eq!(
                    net.piece_value_of_state(kind.index()),
                    handcrafted::piece_value(kind)
                );
                if let Some(piece) = PieceCode::new(Color::Black, kind) {
                    assert_eq!(net.piece_value(piece), handcrafted::piece_value(kind));
                }
            }
            let bytes = fs::read(directory.0.join("v0.bin")).unwrap();
            // MNUE版5の192バイトのヘッダ、線形2列、47個の探索用駒価値に続く全区画は0。
            assert!(
                bytes[192 + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4..]
                    .iter()
                    .all(|&b| b == 0)
            );
            assert!(
                bytes[192 + BOARD_FEATURE_COUNT * 4..192 + FEATURE_COUNT * 4]
                    .iter()
                    .all(|&b| b == 0)
            );
        }
    }

    #[test]
    fn export_rejects_existing_paths_without_changing_contents() {
        let directory = Directory::new();
        let output = directory.0.join("existing.bin");
        fs::write(&output, b"keep this file").unwrap();
        let error = run(Args {
            cp_per_logit: 913.75,
            output: output.clone(),
        })
        .unwrap_err();
        assert!(matches!(error, Error::Io(ref e) if e.kind() == std::io::ErrorKind::AlreadyExists));
        assert_eq!(fs::read(&output).unwrap(), b"keep this file");
        assert!(
            run(Args {
                cp_per_logit: 913.75,
                output: directory.0.clone()
            })
            .is_err()
        );
        assert!(directory.0.is_dir());
    }

    #[test]
    fn invalid_scales_do_not_create_output_files() {
        let directory = Directory::new();
        for cp_per_logit in [
            0.0,
            -0.0,
            -1.0,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            100_000.01,
        ] {
            let output = directory.0.join("invalid.bin");
            let error = run(Args {
                cp_per_logit,
                output: output.clone(),
            })
            .unwrap_err();
            assert!(matches!(
                error,
                Error::Mnue(nnue::Error::InvalidCpPerLogit { .. })
            ));
            assert!(!output.exists());
        }
    }

    #[test]
    fn export_requires_both_arguments() {
        use clap::Parser;

        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            args: Args,
        }
        for args in [
            vec!["export"],
            vec!["export", "--output", "v0.bin"],
            vec!["export", "--cp-per-logit", "913.75"],
        ] {
            assert_eq!(
                Cli::try_parse_from(args).err().unwrap().kind(),
                clap::error::ErrorKind::MissingRequiredArgument
            );
        }
    }
}
