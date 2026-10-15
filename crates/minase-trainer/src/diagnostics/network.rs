use std::path::Path;

use minase::eval::{
    nnue::{
        self, Network,
        feature_set::{HalfKpHmB8, P},
        head::DirectScrelu,
    },
    pst::Pst,
};
use minase_core::{Color, Position};

use super::Result;
use crate::nnue::data::InputFeatureSet;

/// MNUE版5のP型またはHalfKP-hm-b8を使う診断用の整数ネット。
/// 出力部は直接出力の[`DirectScrelu`]に限り、ほかの構造は受理しない。
pub(super) enum IntegerNetwork {
    P(Network<P, DirectScrelu>),
    HalfKp(Network<HalfKpHmB8, DirectScrelu>),
}

impl IntegerNetwork {
    /// MNUE版5を読み、構造記述文字列が`features=P;`または`features=HalfKP-hm-b8;`で始まるものだけを受理する。
    /// 残りの構造とファイルの整合性も[`Network::decode`]で検査する。
    pub(super) fn read(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        let architecture = nnue::architecture(&bytes)?;
        if architecture.starts_with("features=P;") {
            Ok(Self::P(Network::decode(&bytes)?))
        } else if architecture.starts_with("features=HalfKP-hm-b8;") {
            Ok(Self::HalfKp(Network::decode(&bytes)?))
        } else {
            Err(nnue::Error::UnexpectedArchitecture {
                actual: architecture.into(),
            }
            .into())
        }
    }

    pub(super) fn feature_set(&self) -> InputFeatureSet {
        match self {
            Self::P(_) => InputFeatureSet::P,
            Self::HalfKp(_) => InputFeatureSet::HalfKpHmB8,
        }
    }

    pub(super) fn width(&self) -> usize {
        match self {
            Self::P(net) => net.width(),
            Self::HalfKp(net) => net.width(),
        }
    }

    /// 重みファイルの出力用Kをf64へ広げて返す。
    pub(super) fn k(&self) -> f64 {
        match self {
            Self::P(net) => f64::from(net.k()),
            Self::HalfKp(net) => f64::from(net.k()),
        }
    }

    /// 累算器を局面から作り直し、指定した視点から見た整数評価を返す。
    /// 戻り値はセンチポーン単位のi32である。
    pub(super) fn evaluate(&self, position: &Position, side: Color) -> i32 {
        match self {
            Self::P(net) => nnue::evaluate_for_diagnostics(net, position, side),
            Self::HalfKp(net) => nnue::evaluate_for_diagnostics(net, position, side),
        }
    }
}

/// MNPTを復号し、PSTとFMの両方を持つ版3だけを受理する。
/// FMの次元が32であることを要求し、次元0を拒否する。
pub(super) fn read_baseline(path: &Path) -> Result<Pst> {
    let bytes = std::fs::read(path)?;
    let pst = Pst::decode(&bytes)?;
    if pst.fm_weights().is_none() {
        return Err(format!(
            "{}: baseline must contain FM weights (rank 32)",
            path.display()
        )
        .into());
    }
    Ok(pst)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(error.to_string().contains("must contain FM weights"));
        std::fs::write(&path, with.encode()).unwrap();
        assert!(read_baseline(&path).unwrap().fm_weights().is_some());
    }
}
