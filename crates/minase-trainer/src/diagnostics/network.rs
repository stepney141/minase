use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(super) enum Error {
    MissingFmWeights(PathBuf),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingFmWeights(path) => write!(
                f,
                "{}: baseline must contain FM weights (rank 32)",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {}

use minase::eval::{
    nnue::{
        self, Network,
        feature_set::{FeatureSet, HalfKpHmB8, P},
        head::{DirectScrelu, DirectScreluPhase2, Head, ScreluHidden16, ScreluHidden16Phase2},
    },
    pst::Pst,
};
use minase_core::{Color, Position};

use super::Result;
use crate::nnue::data::InputFeatureSet;

/// MNUE版5の2つの入力特徴集合と4つの出力部を扱う診断用整数ネット。
pub(crate) struct IntegerNetwork {
    network: Box<dyn DiagnosticNetwork>,
    feature_set: InputFeatureSet,
}

trait DiagnosticNetwork {
    fn width(&self) -> usize;
    fn cp_per_logit(&self) -> f64;
    fn evaluate(&self, position: &Position, side: Color) -> i32;
}

impl<F: FeatureSet, H: Head> DiagnosticNetwork for Network<F, H> {
    fn width(&self) -> usize {
        self.width()
    }
    fn cp_per_logit(&self) -> f64 {
        f64::from(self.cp_per_logit())
    }
    fn evaluate(&self, position: &Position, side: Color) -> i32 {
        nnue::evaluate_for_diagnostics(self, position, side)
    }
}

impl IntegerNetwork {
    /// 入力特徴集合と出力部を選び、構造記述全体と本体を`Network::decode`で検査する。
    pub(super) fn read(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        Ok(Self::decode(&bytes)?)
    }
    pub(crate) fn decode(bytes: &[u8]) -> std::result::Result<Self, nnue::Error> {
        let architecture = nnue::architecture(bytes)?;
        let feature = architecture.split(';').next();
        let head = architecture.rsplit(';').next();
        let feature_set = match feature {
            Some("features=P") => InputFeatureSet::P,
            Some("features=HalfKP-hm-b8") => InputFeatureSet::HalfKpHmB8,
            _ => {
                return Err(nnue::Error::UnexpectedArchitecture {
                    actual: architecture.into(),
                });
            }
        };
        let network = match feature_set {
            InputFeatureSet::P => Self::decode_head::<P>(bytes, head)?,
            InputFeatureSet::HalfKpHmB8 => Self::decode_head::<HalfKpHmB8>(bytes, head)?,
        };
        Ok(Self {
            network,
            feature_set,
        })
    }

    fn decode_head<F: FeatureSet + 'static>(
        bytes: &[u8],
        head: Option<&str>,
    ) -> std::result::Result<Box<dyn DiagnosticNetwork>, nnue::Error> {
        Ok(match head {
            Some("head=direct-screlu") => Box::new(Network::<F, DirectScrelu>::decode(bytes)?),
            Some("head=direct-screlu-phase2") => {
                Box::new(Network::<F, DirectScreluPhase2>::decode(bytes)?)
            }
            Some("head=screlu-hidden16") => Box::new(Network::<F, ScreluHidden16>::decode(bytes)?),
            Some("head=screlu-hidden16-phase2") => {
                Box::new(Network::<F, ScreluHidden16Phase2>::decode(bytes)?)
            }
            _ => {
                return Err(nnue::Error::UnexpectedArchitecture {
                    actual: nnue::architecture(bytes)?.into(),
                });
            }
        })
    }

    pub(crate) fn feature_set(&self) -> InputFeatureSet {
        self.feature_set
    }
    pub(crate) fn width(&self) -> usize {
        self.network.width()
    }
    /// 重みファイルの勝率尺度（1ロジットあたりのcp）をf64へ広げて返す。
    pub(crate) fn cp_per_logit(&self) -> f64 {
        self.network.cp_per_logit()
    }
    /// 累算器を局面から作り直し、指定した視点の評価をセンチポーンで返す。
    pub(crate) fn evaluate(&self, position: &Position, side: Color) -> i32 {
        self.network.evaluate(position, side)
    }
}

/// MNPTを復号し、PSTとFMの両方を持つ版3だけを受理する。
/// FMの次元が32であることを要求し、次元0を拒否する。
pub(super) fn read_baseline(path: &Path) -> Result<Pst> {
    let bytes = std::fs::read(path)?;
    let pst = Pst::decode(&bytes)?;
    if pst.fm_weights().is_none() {
        return Err(Error::MissingFmWeights(path.to_owned()).into());
    }
    Ok(pst)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 実ファイルを読み、全8種類の型選択と診断APIを検査する。
    #[test]
    fn integer_network_reads_all_feature_and_head_combinations() {
        fn check<F: FeatureSet, H: Head>(head: H, expected_feature: InputFeatureSet) {
            use minase::eval::features::{FEATURE_COUNT, PIECE_STATE_COUNT};
            use sha2::{Digest, Sha256};
            let source = include_bytes!("../../../minase/nets/nnue-epoch0.bin");
            let width = u32::from_le_bytes(source[104..108].try_into().unwrap()) as usize;
            let start = 192 + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4;
            let end = start + FEATURE_COUNT * width * 2;
            let mut bytes = source[..start].to_vec();
            for _ in 0..F::DIMENSIONS / FEATURE_COUNT {
                bytes.extend_from_slice(&source[start..end]);
            }
            bytes.extend_from_slice(&source[end..end + width * 4]);
            bytes.extend(head.to_bytes());
            let text = format!(
                "features={};ft={width};linear=tapered2;head={}",
                F::NAME,
                H::NAME
            );
            bytes[8..104].fill(0);
            bytes[8..8 + text.len()].copy_from_slice(text.as_bytes());
            let checksum = Sha256::digest(&bytes[192..]);
            bytes[144..176].copy_from_slice(&checksum);
            let reference = Network::<F, H>::decode(&bytes).unwrap();
            let path = std::env::temp_dir().join(format!(
                "minase-diag-head-{}-{}-{}.bin",
                std::process::id(),
                F::NAME,
                H::NAME
            ));
            struct File(PathBuf);
            impl Drop for File {
                fn drop(&mut self) {
                    std::fs::remove_file(&self.0).unwrap();
                }
            }
            std::fs::write(&path, &bytes).unwrap();
            let file = File(path);
            let actual = IntegerNetwork::read(&file.0).unwrap();
            assert_eq!(actual.feature_set(), expected_feature);
            assert_eq!(actual.width(), width);
            assert_eq!(actual.cp_per_logit(), f64::from(reference.cp_per_logit()));
            let mut sparse = minase_core::PositionBuilder::new(Color::Black);
            for (file, rank, side) in [(0, 0, Color::Black), (11, 11, Color::White)] {
                sparse
                    .put(
                        minase_core::Square::new(file, rank).unwrap(),
                        minase_core::PieceCode::new(side, minase_core::PieceKind::King).unwrap(),
                    )
                    .unwrap();
            }
            let sparse = sparse.finish().unwrap();
            for position in [Position::initial(), sparse] {
                for side in Color::ALL {
                    assert_eq!(
                        actual.evaluate(&position, side),
                        nnue::evaluate_for_diagnostics(&reference, &position, side)
                    );
                }
            }
            for text in [
                format!(
                    "features=unknown;ft={width};linear=tapered2;head={}",
                    H::NAME
                ),
                format!(
                    "features={};ft={width};linear=tapered2;head=unknown",
                    F::NAME
                ),
                format!(
                    "features={};ft={width};linear=unknown;head={}",
                    F::NAME,
                    H::NAME
                ),
            ] {
                bytes[8..104].fill(0);
                bytes[8..8 + text.len()].copy_from_slice(text.as_bytes());
                std::fs::write(&file.0, &bytes).unwrap();
                let error = match IntegerNetwork::read(&file.0) {
                    Ok(_) => panic!("unsupported architecture accepted"),
                    Err(error) => error,
                };
                assert!(matches!(
                    error.downcast_ref::<nnue::Error>(),
                    Some(nnue::Error::UnexpectedArchitecture { .. })
                ));
            }
        }
        fn heads<F: FeatureSet>(feature: InputFeatureSet) {
            let width = 64;
            check::<F, _>(
                DirectScrelu::new(width, &[128; 128], -65_536, true).unwrap(),
                feature,
            );
            check::<F, _>(
                DirectScreluPhase2::new(
                    width,
                    [&[128; 128], &[-128; 128]],
                    [-65_536, 131_072],
                    true,
                )
                .unwrap(),
                feature,
            );
            check::<F, _>(
                ScreluHidden16::new(
                    width,
                    [&[128; 128]],
                    [-65_536],
                    &[1; 2048],
                    [64; 16],
                    [[4096; 16]],
                    true,
                )
                .unwrap(),
                feature,
            );
            check::<F, _>(
                ScreluHidden16Phase2::new(
                    width,
                    [&[128; 128], &[-128; 128]],
                    [-65_536, 131_072],
                    &[1; 2048],
                    [64; 16],
                    [[4096; 16], [-4096; 16]],
                    true,
                )
                .unwrap(),
                feature,
            );
        }
        heads::<P>(InputFeatureSet::P);
        heads::<HalfKpHmB8>(InputFeatureSet::HalfKpHmB8);
    }

    /// 版3でもFMの次元0なら、FMを要する診断の基準には使えない。
    #[test]
    fn baseline_requires_fm_weights() {
        use minase::eval::{
            features::FEATURE_COUNT,
            pst::{FM_RANK, FmWeights},
        };
        struct Directory(std::path::PathBuf);
        impl Drop for Directory {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "diagnostic-baseline-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let directory = Directory(path);
        let weights = vec![[0; 2]; FEATURE_COUNT];
        let values = crate::pst::v0_piece_values();
        let without = Pst::from_weights(&weights, values, 1000.0, "engine-default").unwrap();
        let embeddings = vec![[1; FM_RANK]; FEATURE_COUNT];
        let with = Pst::from_weights_with_fm(
            &weights,
            values,
            1000.0,
            "engine-default",
            FmWeights {
                embeddings: &embeddings,
                signs: &[1; FM_RANK],
                exponent: 1,
            },
        )
        .unwrap();
        let path = directory.0.join("baseline.bin");
        std::fs::write(&path, without.encode()).unwrap();
        let error = match read_baseline(&path) {
            Ok(_) => panic!("PST-only baseline was accepted"),
            Err(error) => error,
        };
        assert!(
            matches!(error.downcast_ref::<Error>(), Some(Error::MissingFmWeights(rejected)) if rejected == &path)
        );
        std::fs::write(&path, with.encode()).unwrap();
        assert!(read_baseline(&path).unwrap().fm_weights().is_some());
    }
}
