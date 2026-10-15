//! 完了した更新回数から、3群に共通する学習率の倍率を求める。

use crate::nnue::model::{Error, Result};
use serde::Serialize;
use std::{fmt, str::FromStr};

/// 更新ごとに基準学習率へ掛ける倍率m(t)。tは完了した更新回数で、最初の更新では0。
/// Uは1エポックの更新回数で、`ceil(訓練局面数 / バッチサイズ)`とする。
/// 引数は`constant:<c>`、`exponential:<h>:<f>`、`cosine:<T>:<f>`の形で指定する。
/// 既定値は`cosine:3:0.1`で、総長3エポック、倍率の下限0.1とする。
/// c、h、Tは有限かつ正、下限fは有限かつ`0 < f ≤ 1`でなければならない。
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LrSchedule {
    /// `m(t) = c`。
    Constant {
        /// 一定の倍率c。
        multiplier: f64,
    },
    /// `m(t) = max(f, 2^(−t / (h·U)))`。
    Exponential {
        /// 半減期hをエポック数で指定する。
        half_life_epochs: f64,
        /// 倍率の下限f。
        floor: f64,
    },
    /// `m(t) = f + (1 − f)·(1 + cos(π·min(t / (T·U), 1))) / 2`。
    Cosine {
        /// 総長Tをエポック数で指定する。学習のエポック数の上限とは独立である。
        horizon_epochs: f64,
        /// 総長に達した後も使う倍率の下限f。
        floor: f64,
    },
}

impl Default for LrSchedule {
    fn default() -> Self {
        Self::Cosine {
            horizon_epochs: 3.0,
            floor: 0.1,
        }
    }
}

impl FromStr for LrSchedule {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let number = |value: &str| {
            value
                .parse::<f64>()
                .map_err(|_| Error::Invalid("invalid learning-rate schedule number"))
        };
        let schedule = match value.split(':').collect::<Vec<_>>().as_slice() {
            ["constant", c] => Self::Constant {
                multiplier: number(c)?,
            },
            ["exponential", h, f] => Self::Exponential {
                half_life_epochs: number(h)?,
                floor: number(f)?,
            },
            ["cosine", t, f] => Self::Cosine {
                horizon_epochs: number(t)?,
                floor: number(f)?,
            },
            _ => {
                return Err(Error::Invalid(
                    "learning-rate schedule must be constant:<c>, exponential:<h>:<f>, or cosine:<T>:<f>",
                ));
            }
        };
        schedule.validate()?;
        Ok(schedule)
    }
}

impl fmt::Display for LrSchedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Constant { multiplier } => write!(f, "constant:{multiplier}"),
            Self::Exponential {
                half_life_epochs,
                floor,
            } => write!(f, "exponential:{half_life_epochs}:{floor}"),
            Self::Cosine {
                horizon_epochs,
                floor,
            } => write!(f, "cosine:{horizon_epochs}:{floor}"),
        }
    }
}

impl LrSchedule {
    /// 倍率、期間、下限の有限性と範囲を検査する。
    pub fn validate(&self) -> Result<()> {
        let (duration, floor) = match *self {
            Self::Constant { multiplier } => {
                if !multiplier.is_finite() || multiplier <= 0.0 {
                    return Err(Error::Invalid(
                        "learning-rate multiplier must be finite and positive",
                    ));
                }
                return Ok(());
            }
            Self::Exponential {
                half_life_epochs,
                floor,
            } => (half_life_epochs, floor),
            Self::Cosine {
                horizon_epochs,
                floor,
            } => (horizon_epochs, floor),
        };
        if !duration.is_finite() || duration <= 0.0 {
            return Err(Error::Invalid(
                "learning-rate schedule duration must be finite and positive",
            ));
        }
        if !floor.is_finite() || !(0.0 < floor && floor <= 1.0) {
            return Err(Error::Invalid(
                "learning-rate schedule floor must be finite and in (0, 1]",
            ));
        }
        Ok(())
    }

    /// 完了済み更新回数tと1エポックの更新回数Uから、次の更新の倍率を倍精度で求める。
    /// 呼び手は事前に[`Self::validate`]を通す。一定の倍率はUに依存しない。
    ///
    /// # Panics
    /// Uが0なら、呼び手の誤りとしてパニックする。
    pub fn multiplier(&self, completed_steps: u64, steps_per_epoch: u64) -> f64 {
        assert!(steps_per_epoch > 0, "steps per epoch must be positive");
        match *self {
            Self::Constant { multiplier } => multiplier,
            Self::Exponential {
                half_life_epochs,
                floor,
            } => (-(completed_steps as f64) / (half_life_epochs * steps_per_epoch as f64))
                .exp2()
                .max(floor),
            Self::Cosine {
                horizon_epochs,
                floor,
            } => {
                let fraction =
                    (completed_steps as f64 / (horizon_epochs * steps_per_epoch as f64)).min(1.0);
                floor + (1.0 - floor) * (1.0 + (std::f64::consts::PI * fraction).cos()) / 2.0
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing_and_display_round_trip() {
        for (text, expected) in [
            ("cosine:3:0.1", LrSchedule::default()),
            ("constant:1", LrSchedule::Constant { multiplier: 1.0 }),
            ("constant:0.37", LrSchedule::Constant { multiplier: 0.37 }),
            (
                "exponential:1:0.1",
                LrSchedule::Exponential {
                    half_life_epochs: 1.0,
                    floor: 0.1,
                },
            ),
            (
                "cosine:3.5:0.1",
                LrSchedule::Cosine {
                    horizon_epochs: 3.5,
                    floor: 0.1,
                },
            ),
        ] {
            let parsed: LrSchedule = text.parse().unwrap();
            assert_eq!(parsed, expected);
            assert_eq!(parsed.to_string(), text);
            assert_eq!(parsed.to_string().parse::<LrSchedule>().unwrap(), parsed);
        }
    }

    #[test]
    fn parsing_rejects_invalid_shapes_and_numbers() {
        for text in [
            "",
            "1",
            "constant",
            "constant:",
            "constant:1:",
            "constant:1:2",
            "exponential:1",
            "exponential:1:",
            "exponential:1:0.1:2",
            "cosine:3",
            "cosine::0.1",
            "cosine:3:0.1:",
            "linear:1:0.1",
            "constant:no",
            "constant:１",
            "定数:1",
            "constant:1e400",
            "exponential:no:0.1",
            "cosine:3:no",
            "constant:NaN",
            "constant:inf",
            "constant:0",
            "constant:-1",
            "exponential:0:0.1",
            "exponential:inf:0.1",
            "exponential:1:0",
            "cosine:NaN:0.1",
            "cosine:3:NaN",
            "cosine:3:1.1",
        ] {
            assert!(
                matches!(text.parse::<LrSchedule>(), Err(Error::Invalid(_))),
                "{text}"
            );
        }
    }

    #[test]
    fn constant_is_independent_of_update_and_epoch_length() {
        let schedule = LrSchedule::Constant { multiplier: 0.37 };
        for t in [0, 1, 10, u64::MAX] {
            for u in [1, 9, u64::MAX] {
                assert_eq!(schedule.multiplier(t, u).to_bits(), 0.37_f64.to_bits());
            }
        }
    }

    #[test]
    fn exponential_halves_after_its_half_life_and_stops_at_the_floor() {
        let schedule = LrSchedule::Exponential {
            half_life_epochs: 1.5,
            floor: 0.1,
        };
        assert!((schedule.multiplier(0, 8) - 1.0).abs() < 1e-15);
        assert!((schedule.multiplier(12, 8) - 0.5).abs() < 1e-15);
        assert!((schedule.multiplier(24, 8) - 0.25).abs() < 1e-15);
        assert!((schedule.multiplier(1200, 8) - 0.1).abs() < 1e-15);
    }

    #[test]
    fn cosine_reaches_its_midpoint_and_keeps_the_floor_after_its_horizon() {
        let schedule = LrSchedule::Cosine {
            horizon_epochs: 3.0,
            floor: 0.1,
        };
        assert!((schedule.multiplier(0, 8) - 1.0).abs() < 1e-15);
        assert!((schedule.multiplier(12, 8) - 0.55).abs() < 1e-15);
        assert!((schedule.multiplier(24, 8) - 0.1).abs() < 1e-15);
        assert!((schedule.multiplier(25, 8) - 0.1).abs() < 1e-15);
        assert!((schedule.multiplier(u64::MAX, 8) - 0.1).abs() < 1e-15);
    }

    #[test]
    fn validation_checks_finite_positive_parameters_and_floor_boundaries() {
        for value in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for schedule in [
                LrSchedule::Constant { multiplier: value },
                LrSchedule::Exponential {
                    half_life_epochs: value,
                    floor: 0.1,
                },
                LrSchedule::Cosine {
                    horizon_epochs: value,
                    floor: 0.1,
                },
                LrSchedule::Exponential {
                    half_life_epochs: 1.0,
                    floor: value,
                },
                LrSchedule::Cosine {
                    horizon_epochs: 3.0,
                    floor: value,
                },
            ] {
                assert!(matches!(schedule.validate(), Err(Error::Invalid(_))));
            }
        }
        for floor in [f64::MIN_POSITIVE, 0.1, 1.0, 1.1] {
            for schedule in [
                LrSchedule::Exponential {
                    half_life_epochs: 1.0,
                    floor,
                },
                LrSchedule::Cosine {
                    horizon_epochs: 3.0,
                    floor,
                },
            ] {
                assert_eq!(schedule.validate().is_ok(), floor <= 1.0);
                if floor == 1.0 {
                    assert_eq!(schedule.multiplier(0, 9), 1.0);
                    assert_eq!(schedule.multiplier(1000, 9), 1.0);
                }
            }
        }
        assert!(LrSchedule::Constant { multiplier: 2.0 }.validate().is_ok());
    }

    #[test]
    #[should_panic(expected = "steps per epoch must be positive")]
    fn zero_steps_per_epoch_is_a_caller_error() {
        LrSchedule::default().multiplier(0, 0);
    }
}
