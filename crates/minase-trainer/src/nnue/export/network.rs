use minase::eval::nnue::{
    self, Network,
    feature_set::{FeatureSet, HalfKpHmB8, P},
    head::PairwiseL16L32,
};
use minase_core::{Color, Position};

use crate::nnue::data::InputFeatureSet;

/// MNUE版5の2つの入力特徴集合とpairwise-l16-l32出力部を扱う書き出し検査用の整数ネット。
pub(crate) struct IntegerNetwork {
    network: Box<dyn DiagnosticNetwork>,
}

trait DiagnosticNetwork {
    fn cp_per_logit(&self) -> f64;
    fn evaluate(&self, position: &Position, side: Color) -> i32;
}

impl<F: FeatureSet> DiagnosticNetwork for Network<F, PairwiseL16L32> {
    fn cp_per_logit(&self) -> f64 {
        f64::from(self.cp_per_logit())
    }
    fn evaluate(&self, position: &Position, side: Color) -> i32 {
        nnue::evaluate_for_diagnostics(self, position, side)
    }
}

impl IntegerNetwork {
    pub(crate) fn decode(bytes: &[u8]) -> std::result::Result<Self, nnue::Error> {
        let architecture = nnue::architecture(bytes)?;
        let feature = architecture.split(';').next();
        let feature_set = match feature {
            Some("features=P") => InputFeatureSet::P,
            Some("features=HalfKP-hm-b8") => InputFeatureSet::HalfKpHmB8,
            _ => {
                return Err(nnue::Error::UnexpectedArchitecture {
                    actual: architecture.into(),
                });
            }
        };
        let network: Box<dyn DiagnosticNetwork> = match feature_set {
            InputFeatureSet::P => Box::new(Network::<P, PairwiseL16L32>::decode(bytes)?),
            InputFeatureSet::HalfKpHmB8 => {
                Box::new(Network::<HalfKpHmB8, PairwiseL16L32>::decode(bytes)?)
            }
        };
        Ok(Self { network })
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

/// この学習器の書き出し形式の識別情報を返す。
pub(crate) fn file_identity(path: &std::path::Path) -> std::io::Result<serde_json::Value> {
    use std::io::Read;
    let mut header = [0; 176];
    std::fs::File::open(path)?.read_exact(&mut header)?;
    Ok(
        serde_json::json!({"path": path.canonicalize()?, "checksum": crate::nn::hex(&header[144..176])}),
    )
}
