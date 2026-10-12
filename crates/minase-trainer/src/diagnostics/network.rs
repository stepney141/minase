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

pub(super) enum IntegerNetwork {
    P(Network<P, DirectScrelu>),
    HalfKp(Network<HalfKpHmB8, DirectScrelu>),
}

impl IntegerNetwork {
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

    pub(super) fn k(&self) -> f64 {
        match self {
            Self::P(net) => f64::from(net.k()),
            Self::HalfKp(net) => f64::from(net.k()),
        }
    }

    pub(super) fn evaluate(&self, position: &Position, side: Color) -> i32 {
        match self {
            Self::P(net) => nnue::evaluate_for_diagnostics(net, position, side),
            Self::HalfKp(net) => nnue::evaluate_for_diagnostics(net, position, side),
        }
    }
}

pub(super) fn read_baseline(path: &Path) -> Result<Pst> {
    let bytes = std::fs::read(path)?;
    let pst = Pst::decode(&bytes)?;
    // 復号済みなのでヘッダ長と版の位置は検査済み。
    if bytes[4..8] != 3_u32.to_le_bytes() {
        return Err(format!("{}: baseline must be MNPT version 3", path.display()).into());
    }
    Ok(pst)
}
