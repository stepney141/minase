use super::Result;
use minase::eval::{
    nnue::network::{FormatError, Network},
    pst::Pst,
};
use minase_core::{Color, Position};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(super) enum Error {
    MissingFmWeights(PathBuf),
    HalfKpVmDeferred,
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
            Self::HalfKpVmDeferred => f.write_str(
                "HalfKP_vm Simple diagnostics are deferred to phase 4; LayerStack is required",
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

/// tatara形式を復号し、診断で対応しているLayerStackだけを受理する。
pub(crate) struct IntegerNetwork {
    network: Network,
}
impl IntegerNetwork {
    pub(super) fn read(path: &Path) -> Result<Self> {
        Ok(Self::decode(&std::fs::read(path)?)?)
    }
    pub(super) fn decode(bytes: &[u8]) -> std::result::Result<Self, Error> {
        let network = Network::decode(bytes).map_err(Error::Format)?;
        match network {
            Network::LayerStack(_) => Ok(Self { network }),
            Network::Simple(_) => Err(Error::HalfKpVmDeferred),
        }
    }
    pub(super) fn width(&self) -> usize {
        self.network.width()
    }
    pub(super) fn fv_scale(&self) -> i32 {
        self.network.fv_scale()
    }
    /// 両視点のPSQTと後段を局面から再計算し、センチポーンで返す。
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_layerstack_and_rejects_simple_and_malformed_inputs() {
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
        assert!(matches!(
            IntegerNetwork::decode(&simple),
            Err(Error::HalfKpVmDeferred)
        ));
        for bad in [&bytes[..3], &bytes[..bytes.len() - 1], b"MNUE".as_slice()] {
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
