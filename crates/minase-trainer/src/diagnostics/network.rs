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
/// オフセット4のu32の版番号が3であることを要求し、版2を拒否する。
pub(super) fn read_baseline(path: &Path) -> Result<Pst> {
    let bytes = std::fs::read(path)?;
    let pst = Pst::decode(&bytes)?;
    // 復号済みなのでヘッダ長と版の位置は検査済み。
    if bytes[4..8] != 3_u32.to_le_bytes() {
        return Err(format!("{}: baseline must be MNPT version 3", path.display()).into());
    }
    Ok(pst)
}
