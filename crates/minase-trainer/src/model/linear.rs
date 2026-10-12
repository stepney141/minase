//! 線形重みの保存範囲と制約。

use super::{Error, Parameter, Result};

pub(crate) const LINEAR_MIN: f32 = -4096.0;
pub(crate) const LINEAR_MAX: f32 = 4095.875;

pub(crate) fn constrain_linear(weights: &mut [f32]) -> Result<()> {
    if weights.iter().any(|v| !v.is_finite()) {
        return Err(Error::NonFinite(Parameter::Linear));
    }
    for value in weights {
        *value = value.clamp(LINEAR_MIN, LINEAR_MAX);
    }
    Ok(())
}
