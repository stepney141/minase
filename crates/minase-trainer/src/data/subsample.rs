//! 前処理済み訓練集合から、更新に使う対局または局面を決定的に抽出する。

use super::{Error, shuffle::mix64};
use serde::{Serialize, Serializer};
use std::{fmt, str::FromStr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unit {
    Games,
    Positions,
}

/// `games:k:r`または`positions:k:r`で指定する訓練集合の抽出条件。
/// kは2以上のu64、rは0以上k未満であり、文字列からの構築時に検査する。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrainSubsample {
    unit: Unit,
    k: u64,
    r: u64,
}

impl FromStr for TrainSubsample {
    type Err = Error;

    fn from_str(spec: &str) -> Result<Self, Self::Err> {
        let invalid = || {
            Error::Invalid(
                "--train-subsample requires games:<k>:<r> or positions:<k>:<r> with integer k >= 2 and 0 <= r < k (u64)",
            )
        };
        let mut parts = spec.split(':');
        let unit = match parts.next() {
            Some("games") => Unit::Games,
            Some("positions") => Unit::Positions,
            _ => return Err(invalid()),
        };
        let number = |part: Option<&str>| {
            let part = part.ok_or_else(invalid)?;
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid());
            }
            part.parse::<u64>().map_err(|_| invalid())
        };
        let k = number(parts.next())?;
        let r = number(parts.next())?;
        if parts.next().is_some() || k < 2 || r >= k {
            return Err(invalid());
        }
        Ok(Self { unit, k, r })
    }
}

impl fmt::Display for TrainSubsample {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let unit = match self.unit {
            Unit::Games => "games",
            Unit::Positions => "positions",
        };
        write!(f, "{unit}:{}:{}", self.k, self.r)
    }
}

impl Serialize for TrainSubsample {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl TrainSubsample {
    /// fileは入力順の0始まり番号。plyは先読み教師と同じMNSDの手数を使う。
    pub(crate) fn keeps(self, file: u64, game: u32, ply: u16) -> bool {
        let game = mix64((file << 32) | u64::from(game));
        let hash = match self.unit {
            Unit::Games => game,
            Unit::Positions => mix64(game ^ u64::from(ply).wrapping_mul(0x9E37_79B9_7F4A_7C15)),
        };
        hash % self.k == self.r
    }
}
