use super::Result;
use minase::eval::{
    nnue::{FormatError, Network},
    pst::Pst,
};
use minase_core::{Color, Position};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(super) enum Error {
    MissingFmWeights(PathBuf),
    Format(FormatError),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingFmWeights(path) => write!(
                f,
                "{}: baseline must contain FM weights (rank 32)",
                path.display()
            ),
            Self::Format(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Format(e) => Some(e),
            _ => None,
        }
    }
}

/// tatara形式を復号し、LayerStackとSimpleを受理する。
pub(crate) struct IntegerNetwork {
    network: Network,
}
impl IntegerNetwork {
    pub(super) fn read(path: &Path) -> Result<Self> {
        Ok(Self::decode(&std::fs::read(path)?)?)
    }
    pub(super) fn decode(bytes: &[u8]) -> std::result::Result<Self, Error> {
        let network = Network::decode(bytes).map_err(Error::Format)?;
        Ok(Self { network })
    }
    pub(super) fn feature_set(&self) -> minase::eval::nnue::input::FeatureSet {
        match self.network {
            Network::LayerStack(_) => minase::eval::nnue::input::FeatureSet::A,
            Network::Simple(_) => minase::eval::nnue::input::FeatureSet::HalfKpVm,
        }
    }
    pub(super) fn width(&self) -> usize {
        self.network.width()
    }
    pub(super) fn fv_scale(&self) -> i32 {
        self.network.fv_scale()
    }
    /// ネットの構造に従って両視点を再計算し、指定視点の評価をセンチポーンで返す。
    pub(super) fn evaluate(&self, position: &Position, side: Color) -> Result<i32> {
        Ok(self.network.evaluate_for(position, side)?)
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

use crate::nnue::{layerstack as ls, simple};
use minase::eval::nnue::input::{FeatureSet, HalfKpVm, Indexer};
use minase_core::{PieceCode, Square};

pub(super) enum FloatCheckpoint {
    LayerStack(ls::Checkpoint),
    Simple(simple::Checkpoint),
}
pub(super) enum FloatAccumulator {
    LayerStack(Box<ls::diagnostics::Accumulator>),
    Simple(simple::diagnostics::Accumulator),
}
impl FloatCheckpoint {
    pub(super) fn read(path: &Path, set: FeatureSet) -> Result<Self> {
        Ok(match set {
            FeatureSet::A => Self::LayerStack(ls::Checkpoint::read(path)?),
            FeatureSet::HalfKpVm => Self::Simple(simple::Checkpoint::read(path)?),
        })
    }
    pub(super) fn width(&self) -> usize {
        match self {
            Self::LayerStack(_) => ls::H,
            Self::Simple(cp) => cp.parameters.width(),
        }
    }
    pub(super) fn divisor(&self) -> i32 {
        match self {
            Self::LayerStack(cp) => cp.settings.scale.divisor(),
            Self::Simple(cp) => cp.settings.scale.divisor(),
        }
    }
    pub(super) fn output_scale(&self) -> f64 {
        8128.0 / f64::from(self.divisor())
    }
    pub(super) fn accumulator(&self, position: &Position, side: Color) -> Result<FloatAccumulator> {
        Ok(match self {
            Self::LayerStack(cp) => {
                FloatAccumulator::LayerStack(Box::new(ls::diagnostics::Accumulator::new(
                    &cp.parameters,
                    &ls::Features::from_position(position, side),
                )))
            }
            Self::Simple(cp) => FloatAccumulator::Simple(simple::diagnostics::Accumulator::new(
                &cp.parameters,
                &simple::Features::from_position(position, side)?,
            )),
        })
    }
    pub(super) fn output(&self, acc: &FloatAccumulator) -> Result<f64> {
        match (self, acc) {
            (Self::LayerStack(cp), FloatAccumulator::LayerStack(acc)) => {
                Ok(acc.output(&cp.parameters))
            }
            (Self::Simple(cp), FloatAccumulator::Simple(acc)) => Ok(acc.output(&cp.parameters)),
            _ => Err("checkpoint and accumulator architectures differ".into()),
        }
    }
    pub(super) fn without(
        &self,
        acc: &FloatAccumulator,
        position: &Position,
        side: Color,
        piece: PieceCode,
        square: Square,
    ) -> Result<f64> {
        let bases = [side, side.opposite()]
            .map(|view| minase::eval::features::feature_index(view, piece, square));
        match (self, acc) {
            (Self::LayerStack(cp), FloatAccumulator::LayerStack(acc)) => {
                Ok(acc.without(&cp.parameters, bases).output(&cp.parameters))
            }
            (Self::Simple(cp), FloatAccumulator::Simple(acc)) => {
                let mut rows = [0; 2];
                for (v, view) in [side, side.opposite()].into_iter().enumerate() {
                    rows[v] = HalfKpVm::row(HalfKpVm::context(position, view)?, bases[v])
                        .ok_or("king is not a Simple removal feature")?;
                }
                Ok(acc.without(&cp.parameters, rows).output(&cp.parameters))
            }
            _ => Err("checkpoint and accumulator architectures differ".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_both_architectures_and_rejects_malformed_inputs() {
        use crate::nnue::layerstack::{Parameters, Scale, export};
        let scale = Scale::new(679.03).unwrap();
        let bytes = export(&Parameters::initialize(7, scale), scale).unwrap();
        let net = IntegerNetwork::decode(&bytes).unwrap();
        let reference = Network::decode(&bytes).unwrap();
        assert_eq!(net.width(), 256);
        assert_eq!(net.fv_scale(), 12);
        for side in Color::ALL {
            let position = Position::initial();
            assert_eq!(
                net.evaluate(&position, side).unwrap(),
                reference.evaluate_for(&position, side).unwrap()
            );
        }
        use minase::eval::nnue::{
            input::FeatureSet,
            network::{SimpleNetwork, SimpleParameters},
        };
        let simple = SimpleNetwork::new(SimpleParameters {
            width: 2,
            fv_scale: 8,
            ft_biases: vec![0; 2],
            ft_weights: vec![0; FeatureSet::HalfKpVm.spec().dimensions() * 2],
            l1_biases: vec![0; 32],
            l1_weights: vec![0; 32 * 4],
            l2_biases: vec![0; 32],
            l2_weights: vec![0; 32 * 32],
            output_bias: 0,
            output_weights: vec![0; 32],
        })
        .unwrap()
        .encode();
        let net = IntegerNetwork::decode(&simple).unwrap();
        assert_eq!(net.evaluate(&Position::initial(), Color::Black).unwrap(), 0);
        for bad in [
            &bytes[..3],
            &bytes[..bytes.len() - 1],
            b"invalid network".as_slice(),
        ] {
            assert!(matches!(IntegerNetwork::decode(bad), Err(Error::Format(_))));
        }
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
        let without = Pst::from_weights(&weights, values, 1000.0, "JCSA").unwrap();
        let embeddings = vec![[1; FM_RANK]; FEATURE_COUNT];
        let with = Pst::from_weights_with_fm(
            &weights,
            values,
            1000.0,
            "JCSA",
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
