use super::{AnchorError, FeatureSet, Indexer};
use crate::eval::features::FEATURE_COUNT;
use minase_core::{Color, Position};

/// 基底特徴をそのまま第1層の行へ写すP型の番号付け。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct P;

impl Indexer for P {
    const FEATURE_SET: FeatureSet = FeatureSet::P;
    type Context = ();

    fn context(_: &Position, _: Color) -> Result<(), AnchorError> {
        Ok(())
    }

    fn row((): (), base: usize) -> Option<usize> {
        assert!(base < FEATURE_COUNT, "base feature out of range");
        Some(base)
    }

    fn check_search_root(_: &Position) -> Result<(), AnchorError> {
        Ok(())
    }
}
