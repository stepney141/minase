//! チェックポイントの進捗JSONと、型・形状・名前を厳密に検査するNPZ入出力。

use super::{
    BestEpoch, Error, Result,
    npz::{self, Array},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// 再開時に照合する`config.json`の3欄。予算と並列度は設定作成時に除外する。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    /// 学習の軌跡を定める設定。
    pub settings: Value,
    /// 正規化したパスと入力の識別情報。
    pub inputs: Value,
    /// 分割、教師用K、観測回数などの前処理結果。
    pub preprocessing: Value,
}
impl Configuration {
    /// 実行記録から必須の比較対象を取り出す。
    pub fn from_document(document: &Value) -> Result<Self> {
        Ok(Self {
            settings: document
                .get("settings")
                .ok_or(Error::Format("missing settings"))?
                .clone(),
            inputs: document
                .get("inputs")
                .ok_or(Error::Format("missing inputs"))?
                .clone(),
            preprocessing: document
                .get("preprocessing")
                .ok_or(Error::Format("missing preprocessing"))?
                .clone(),
        })
    }
    /// 欄ごとに照合し、不一致の欄名をエラーに含める。
    pub fn check(&self, current: &Self) -> Result<()> {
        for (saved, current, reason) in [
            (
                &self.settings,
                &current.settings,
                "checkpoint settings mismatch",
            ),
            (&self.inputs, &current.inputs, "checkpoint inputs mismatch"),
            (
                &self.preprocessing,
                &current.preprocessing,
                "checkpoint preprocessing mismatch",
            ),
        ] {
            if saved != current {
                return Err(Error::Invalid(reason));
            }
        }
        Ok(())
    }
}

/// 4種の学習器が`meta`へ保存する進捗と再開条件。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    /// 最後に完了したエポック。
    pub epoch: u64,
    /// 最良のエポックと検証損失。更新前はnull。
    #[serde(deserialize_with = "Deserialize::deserialize")]
    pub best: Option<BestEpoch>,
    /// 再開前を含む累積秒数。
    pub elapsed_seconds: f64,
    /// 初期化とエポック順列のシード。
    pub seed: u64,
    /// 現在の実行と欄ごとに照合する設定、入力、前処理結果。
    pub config: Configuration,
}
impl Progress {
    pub(crate) fn validate(&self) -> Result<()> {
        if !self.elapsed_seconds.is_finite() || self.elapsed_seconds < 0.0 {
            return Err(Error::Invalid("elapsed time"));
        }
        if let Some(best) = self.best
            && (best.epoch == 0
                || best.epoch > self.epoch
                || !best.validation_loss.is_finite()
                || best.validation_loss < 0.0)
        {
            return Err(Error::Invalid("best epoch"));
        }
        Ok(())
    }
    /// 再開前に設定、入力、前処理結果を照合する。
    pub fn check_config(&self, config: &Configuration) -> Result<()> {
        self.config.check(config)
    }
}

#[derive(Serialize, Deserialize)]
struct Meta {
    kind: String,
    #[serde(flatten)]
    progress: Progress,
}

pub(crate) struct Writer(Vec<(String, Array)>);
impl Writer {
    pub(crate) fn new(kind: &str, progress: &Progress) -> Result<Self> {
        progress.validate()?;
        let data = serde_json::to_vec(&Meta {
            kind: kind.into(),
            progress: progress.clone(),
        })
        .map_err(|_| Error::Format("checkpoint meta JSON"))?;
        Ok(Self(vec![(
            "meta".into(),
            Array {
                dtype: "|u1".into(),
                shape: vec![data.len()],
                data,
            },
        )]))
    }
    pub(crate) fn values<T: Number>(&mut self, name: &str, shape: &[usize], values: &[T]) {
        let mut data = Vec::with_capacity(std::mem::size_of_val(values));
        for &value in values {
            value.append(&mut data);
        }
        self.0.push((
            name.into(),
            Array {
                dtype: T::DTYPE.into(),
                shape: shape.to_vec(),
                data,
            },
        ));
    }
    pub(crate) fn scalar<T: Number>(&mut self, name: &str, value: T) {
        self.values(name, &[], &[value]);
    }
    pub(crate) fn mask(&mut self, name: &str, mask: &[bool]) {
        self.0.push((
            name.into(),
            Array {
                dtype: "|b1".into(),
                shape: vec![mask.len()],
                data: mask.iter().map(|&v| u8::from(v)).collect(),
            },
        ));
    }
    pub(crate) fn encode(self) -> Result<Vec<u8>> {
        let (names, arrays): (Vec<_>, Vec<_>) = self.0.into_iter().unzip();
        npz::encode(
            &names
                .iter()
                .map(String::as_str)
                .zip(arrays)
                .collect::<Vec<_>>(),
        )
    }
}

pub(crate) struct Reader(BTreeMap<String, Array>);
impl Reader {
    pub(crate) fn new(bytes: &[u8], kind: &str) -> Result<(Self, Progress)> {
        let mut reader = Self(npz::decode(bytes)?);
        let meta = reader
            .0
            .remove("meta")
            .ok_or(Error::Format("missing checkpoint meta"))?;
        if meta.dtype != "|u1" || meta.shape != [meta.data.len()] {
            return Err(Error::Format("checkpoint meta dtype or shape"));
        }
        let meta: Meta = serde_json::from_slice(&meta.data)
            .map_err(|_| Error::Format("checkpoint meta JSON"))?;
        if meta.kind != kind {
            return Err(Error::Format("checkpoint kind mismatch"));
        }
        meta.progress.validate()?;
        Ok((reader, meta.progress))
    }
    fn take(&mut self, name: &str, dtype: &str, shape: &[usize]) -> Result<Array> {
        let array = self
            .0
            .remove(name)
            .ok_or(Error::Format("missing checkpoint array"))?;
        if array.dtype != dtype || array.shape != shape {
            return Err(Error::Format("checkpoint array dtype or shape"));
        }
        Ok(array)
    }
    pub(crate) fn values<T: Number>(&mut self, name: &str, shape: &[usize]) -> Result<Vec<T>> {
        Ok(T::read_all(&self.take(name, T::DTYPE, shape)?.data))
    }
    pub(crate) fn scalar<T: Number>(&mut self, name: &str) -> Result<T> {
        Ok(self.values::<T>(name, &[])?[0])
    }
    pub(crate) fn mask(&mut self, name: &str, count: usize) -> Result<Vec<bool>> {
        self.take(name, "|b1", &[count])?
            .data
            .into_iter()
            .map(|v| match v {
                0 => Ok(false),
                1 => Ok(true),
                _ => Err(Error::Format("checkpoint mask must be bool")),
            })
            .collect()
    }
    pub(crate) fn finish(self) -> Result<()> {
        if !self.0.is_empty() {
            return Err(Error::Format("unexpected checkpoint array"));
        }
        Ok(())
    }
}

pub(crate) trait Number: Copy {
    const DTYPE: &'static str;
    fn append(self, bytes: &mut Vec<u8>);
    fn read_all(bytes: &[u8]) -> Vec<Self>;
}
macro_rules! number {
    ($t:ty, $dtype:literal) => {
        impl Number for $t {
            const DTYPE: &'static str = $dtype;
            fn append(self, bytes: &mut Vec<u8>) {
                bytes.extend(self.to_le_bytes());
            }
            fn read_all(bytes: &[u8]) -> Vec<Self> {
                bytes
                    .as_chunks::<{ std::mem::size_of::<$t>() }>()
                    .0
                    .iter()
                    .copied()
                    .map(Self::from_le_bytes)
                    .collect()
            }
        }
    };
}
number!(f32, "<f4");
number!(f64, "<f8");
number!(u64, "<u8");

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn binary_arrays_and_json_progress_preserve_float_bits() {
        let config = Configuration {
            settings: json!({"lr": f64::from_bits(0x3f73_1f58_aefe_dac3)}),
            inputs: json!({"files": []}),
            preprocessing: json!({"k": f64::from_bits(0x4082_3456_789a_bcde)}),
        };
        let progress = Progress {
            epoch: 2,
            best: Some(BestEpoch {
                epoch: 1,
                validation_loss: f64::from_bits(0x3fe1_2345_6789_abcd),
            }),
            elapsed_seconds: f64::from_bits(0x4012_3456_789a_bcde),
            seed: u64::MAX,
            config,
        };
        let f32s = [
            -0.0,
            f32::from_bits(1),
            f32::from_bits(0x3eaa_aaab),
            f32::MAX,
        ];
        let f64s = [
            -0.0,
            f64::from_bits(1),
            f64::from_bits(0x3fe1_2345_6789_abcd),
            f64::MAX,
        ];
        let mut writer = Writer::new("pst", &progress).unwrap();
        writer.values("f32", &[2, 2], &f32s);
        writer.values("f64", &[4], &f64s);
        writer.scalar("step", u64::MAX);
        let bytes = writer.encode().unwrap();
        let (mut reader, restored) = Reader::new(&bytes, "pst").unwrap();
        assert_eq!(restored.config, progress.config);
        assert_eq!(restored.seed, u64::MAX);
        assert_eq!(
            restored.elapsed_seconds.to_bits(),
            progress.elapsed_seconds.to_bits()
        );
        assert_eq!(
            restored.best.unwrap().validation_loss.to_bits(),
            progress.best.unwrap().validation_loss.to_bits()
        );
        assert_eq!(
            reader
                .values::<f32>("f32", &[2, 2])
                .unwrap()
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            f32s.map(f32::to_bits)
        );
        assert_eq!(
            reader
                .values::<f64>("f64", &[4])
                .unwrap()
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            f64s.map(f64::to_bits)
        );
        assert_eq!(reader.scalar::<u64>("step").unwrap(), u64::MAX);
        reader.finish().unwrap();
    }
}
