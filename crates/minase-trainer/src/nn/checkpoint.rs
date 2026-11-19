//! チェックポイントの進捗JSONと、型・形状・名前を厳密に検査するNPZ入出力。

use super::{
    BestEpoch, Error, Result,
    npz::{self, Array},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
};

/// 再開時に不一致だった設定欄。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MismatchField {
    /// `device` の値。
    Device,
    /// `lr_linear` の値。
    LrLinear,
    /// `lr_schedule` の値。
    LrSchedule,
    /// `weight_decay_linear` の値。
    WeightDecayLinear,
    /// `lr_ft` の値。
    LrFt,
    /// `weight_decay_ft` の値。
    WeightDecayFt,
    /// `lr_head` の値。
    LrHead,
    /// `weight_decay_head` の値。
    WeightDecayHead,

    /// 設定全体の構造またはモデル定義。
    Settings,
    /// 入力の識別情報。
    Inputs,
    /// 前処理結果。
    Preprocessing,
    /// `lr` の値。
    Lr,
    /// `weight_decay` の値。
    WeightDecay,
    /// `lambda_res` の値。
    LambdaRes,
    /// `batch` の値。
    Batch,
    /// `seed` の値。
    Seed,
    /// `lambda` の値。
    Lambda,
    /// `lookahead_gamma` の値。
    LookaheadGamma,
    /// `lookahead_plies` の値。
    LookaheadPlies,
    /// `validation_sample` の値。
    ValidationSample,
    /// `pst_lr` の値。
    PstLr,
    /// `fm_lr` の値。
    FmLr,
    /// `fm_weight_decay` の値。
    FmWeightDecay,
    /// `removal_penalty` の値。
    RemovalPenalty,
    /// `removal_reference` の値。
    RemovalReference,
}

impl std::fmt::Display for MismatchField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Device
            | Self::LrLinear
            | Self::LrSchedule
            | Self::WeightDecayLinear
            | Self::LrFt
            | Self::WeightDecayFt
            | Self::LrHead
            | Self::WeightDecayHead
            | Self::Settings
            | Self::Lr
            | Self::WeightDecay
            | Self::LambdaRes
            | Self::Batch
            | Self::Seed
            | Self::Lambda
            | Self::LookaheadGamma
            | Self::LookaheadPlies
            | Self::ValidationSample
            | Self::PstLr
            | Self::FmLr
            | Self::FmWeightDecay
            | Self::RemovalPenalty
            | Self::RemovalReference => "settings",
            Self::Inputs => "inputs",
            Self::Preprocessing => "preprocessing",
        })
    }
}

/// 再開時に照合する`config.json`の3欄。予算と並列度は設定作成時に除外する。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    /// 学習の軌跡を定める設定。
    pub settings: Value,
    /// 正規化したパスと入力の識別情報。
    pub inputs: Value,
    /// 分割、教師の勝率尺度（1ロジットあたりのcp）、観測回数などの前処理結果。
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
        for (key, field) in [
            ("device", MismatchField::Device),
            ("lr_linear", MismatchField::LrLinear),
            ("lr_schedule", MismatchField::LrSchedule),
            ("weight_decay_linear", MismatchField::WeightDecayLinear),
            ("lr_ft", MismatchField::LrFt),
            ("weight_decay_ft", MismatchField::WeightDecayFt),
            ("lr_head", MismatchField::LrHead),
            ("weight_decay_head", MismatchField::WeightDecayHead),
            ("lr", MismatchField::Lr),
            ("weight_decay", MismatchField::WeightDecay),
            ("lambda_res", MismatchField::LambdaRes),
            ("batch", MismatchField::Batch),
            ("seed", MismatchField::Seed),
            ("lambda", MismatchField::Lambda),
            ("lookahead_gamma", MismatchField::LookaheadGamma),
            ("lookahead_plies", MismatchField::LookaheadPlies),
            ("validation_sample", MismatchField::ValidationSample),
            ("pst_lr", MismatchField::PstLr),
            ("fm_lr", MismatchField::FmLr),
            ("fm_weight_decay", MismatchField::FmWeightDecay),
            ("removal_penalty", MismatchField::RemovalPenalty),
            ("removal_reference", MismatchField::RemovalReference),
        ] {
            if let (Some(saved), Some(current)) =
                (self.settings.get(key), current.settings.get(key))
                && saved != current
            {
                return Err(Error::CheckpointMismatch(field));
            }
        }
        for (saved, current, field) in [
            (&self.settings, &current.settings, MismatchField::Settings),
            (&self.inputs, &current.inputs, MismatchField::Inputs),
            (
                &self.preprocessing,
                &current.preprocessing,
                MismatchField::Preprocessing,
            ),
        ] {
            if saved != current {
                return Err(Error::CheckpointMismatch(field));
            }
        }
        Ok(())
    }
}

/// PST、FMと同時学習の学習器が`meta`へ保存する進捗と再開条件。
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
struct Meta<P> {
    kind: String,
    #[serde(flatten)]
    progress: P,
}

pub(crate) struct Writer<W>(npz::Writer<W>);
impl<W: Write> Writer<W> {
    pub(crate) fn new(output: W, kind: &str, progress: &Progress) -> Result<Self> {
        progress.validate()?;
        Self::with_metadata(output, kind, progress)
    }
    /// 学習器専用の進捗を、配列の形式を変えずに保存する。
    pub(crate) fn with_metadata<P: Serialize>(output: W, kind: &str, progress: &P) -> Result<Self> {
        let data = serde_json::to_vec(&Meta {
            kind: kind.into(),
            progress,
        })
        .map_err(|_| Error::Format("checkpoint meta JSON"))?;
        let mut writer = Self(npz::Writer::new(output));
        writer
            .0
            .member("meta", "|u1", &[data.len()], data.len(), |out| {
                out.write_all(&data)
            })?;
        Ok(writer)
    }
    pub(crate) fn values<T: Number>(
        &mut self,
        name: &str,
        shape: &[usize],
        values: &[T],
    ) -> Result<()> {
        self.0.member(
            name,
            T::DTYPE,
            shape,
            std::mem::size_of_val(values),
            |out| {
                for &value in values {
                    value.write(out)?;
                }
                Ok(())
            },
        )
    }
    pub(crate) fn scalar<T: Number>(&mut self, name: &str, value: T) -> Result<()> {
        self.values(name, &[], &[value])
    }
    pub(crate) fn mask(&mut self, name: &str, mask: &[bool]) -> Result<()> {
        self.0
            .member(name, "|b1", &[mask.len()], mask.len(), |out| {
                for &value in mask {
                    out.write_all(&[u8::from(value)])?;
                }
                Ok(())
            })
    }
    pub(crate) fn finish(self) -> Result<()> {
        self.0.finish()
    }
}

pub(crate) struct Reader(BTreeMap<String, Array>);
impl Reader {
    pub(crate) fn new(input: impl Read, kind: &str) -> Result<(Self, Progress)> {
        let (reader, progress): (_, Progress) = Self::with_metadata(input, kind)?;
        progress.validate()?;
        Ok((reader, progress))
    }
    /// 学習器専用の進捗を読み、共通の種別と配列の形状を検査する。
    pub(crate) fn with_metadata<P: serde::de::DeserializeOwned>(
        input: impl Read,
        kind: &str,
    ) -> Result<(Self, P)> {
        let mut reader = Self(npz::read(input)?);
        let meta = reader
            .0
            .remove("meta")
            .ok_or(Error::Format("missing checkpoint meta"))?;
        if meta.dtype != "|u1" || meta.shape != [meta.data.len()] {
            return Err(Error::Format("checkpoint meta dtype or shape"));
        }
        let meta: Meta<P> = serde_json::from_slice(&meta.data)
            .map_err(|_| Error::Format("checkpoint meta JSON"))?;
        if meta.kind != kind {
            return Err(Error::Format("checkpoint kind mismatch"));
        }
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
    fn write(self, output: &mut dyn Write) -> std::io::Result<()>;
    fn read_all(bytes: &[u8]) -> Vec<Self>;
}
macro_rules! number {
    ($t:ty, $dtype:literal) => {
        impl Number for $t {
            const DTYPE: &'static str = $dtype;
            fn write(self, output: &mut dyn Write) -> std::io::Result<()> {
                output.write_all(&self.to_le_bytes())
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
            preprocessing: json!({"cp_per_logit": f64::from_bits(0x4082_3456_789a_bcde)}),
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
        let mut bytes = Vec::new();
        let mut writer = Writer::new(&mut bytes, "pst", &progress).unwrap();
        writer.values("f32", &[2, 2], &f32s).unwrap();
        writer.values("f64", &[4], &f64s).unwrap();
        writer.scalar("step", u64::MAX).unwrap();
        writer.finish().unwrap();
        let (mut reader, restored) = Reader::new(bytes.as_slice(), "pst").unwrap();
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
    #[test]
    fn streamed_file_matches_npz_bytes_for_all_checkpoint_types() {
        use std::{fs::File, io::BufReader};
        let path = std::env::temp_dir().join(format!(
            "minase-checkpoint-stream-{}.npz",
            std::process::id()
        ));
        let f32s = [-0.0_f32, f32::from_bits(1), f32::MAX, 0.25];
        let f64s = [-0.0_f64, f64::from_bits(1), f64::MAX];
        let integers = [0_u64, 1, u64::MAX];
        let mask = [true, false, true];
        let metadata = json!({"epoch": 3});
        let meta_bytes = br#"{"kind":"test","epoch":3}"#.to_vec();
        let arrays = vec![
            (
                "meta",
                Array {
                    dtype: "|u1".into(),
                    shape: vec![meta_bytes.len()],
                    data: meta_bytes,
                },
            ),
            (
                "f32",
                Array {
                    dtype: "<f4".into(),
                    shape: vec![2, 2],
                    data: f32s.iter().flat_map(|v| v.to_le_bytes()).collect(),
                },
            ),
            (
                "f64",
                Array {
                    dtype: "<f8".into(),
                    shape: vec![3],
                    data: f64s.iter().flat_map(|v| v.to_le_bytes()).collect(),
                },
            ),
            (
                "u64",
                Array {
                    dtype: "<u8".into(),
                    shape: vec![3],
                    data: integers.iter().flat_map(|v| v.to_le_bytes()).collect(),
                },
            ),
            (
                "mask",
                Array {
                    dtype: "|b1".into(),
                    shape: vec![3],
                    data: vec![1, 0, 1],
                },
            ),
            (
                "step",
                Array {
                    dtype: "<u8".into(),
                    shape: vec![],
                    data: u64::MAX.to_le_bytes().to_vec(),
                },
            ),
            (
                "empty",
                Array {
                    dtype: "<f4".into(),
                    shape: vec![0, 2],
                    data: vec![],
                },
            ),
        ];
        super::super::atomic_write_with(&path, |output| -> Result<()> {
            let mut writer = Writer::with_metadata(output, "test", &metadata)?;
            writer.values("f32", &[2, 2], &f32s)?;
            writer.values("f64", &[3], &f64s)?;
            writer.values("u64", &[3], &integers)?;
            writer.mask("mask", &mask)?;
            writer.scalar("step", u64::MAX)?;
            writer.values::<f32>("empty", &[0, 2], &[])?;
            writer.finish()
        })
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), npz::encode(&arrays).unwrap());
        let (mut reader, restored): (_, Value) =
            Reader::with_metadata(BufReader::new(File::open(&path).unwrap()), "test").unwrap();
        assert_eq!(restored, metadata);
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
                .values::<f64>("f64", &[3])
                .unwrap()
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            f64s.map(f64::to_bits)
        );
        assert_eq!(reader.values::<u64>("u64", &[3]).unwrap(), integers);
        assert_eq!(reader.mask("mask", 3).unwrap(), mask);
        assert_eq!(reader.scalar::<u64>("step").unwrap(), u64::MAX);
        assert!(reader.values::<f32>("empty", &[0, 2]).unwrap().is_empty());
        reader.finish().unwrap();
        std::fs::remove_file(path).unwrap();
    }

    fn archive() -> Vec<u8> {
        let meta = br#"{"kind":"test","epoch":3}"#.to_vec();
        npz::encode(&[
            (
                "meta",
                Array {
                    dtype: "|u1".into(),
                    shape: vec![meta.len()],
                    data: meta,
                },
            ),
            (
                "data",
                Array {
                    dtype: "<f4".into(),
                    shape: vec![1],
                    data: 1.25_f32.to_le_bytes().to_vec(),
                },
            ),
        ])
        .unwrap()
    }

    fn rejection(bytes: &[u8], expected: &str) {
        assert_eq!(npz::decode(bytes).unwrap_err().to_string(), expected);
        // A short-reading source exercises the same Read path used by files.
        struct ShortRead<'a>(&'a [u8]);
        impl Read for ShortRead<'_> {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                let count = output.len().min(3);
                self.0.read(&mut output[..count])
            }
        }
        let error = Reader::with_metadata::<Value>(ShortRead(bytes), "test")
            .err()
            .unwrap();
        assert_eq!(error.to_string(), expected);
    }

    #[test]
    fn checkpoint_reader_rejects_zip_corruption_with_the_existing_errors() {
        let bytes = archive();
        let local = bytes.windows(4).rposition(|b| b == b"PK\x03\x04").unwrap();
        let central = bytes.windows(4).position(|b| b == b"PK\x01\x02").unwrap();
        let end = bytes.len() - 22;
        for (offset, expected) in [
            (4, "ZIP must be stored with no flags"),
            (6, "ZIP must be stored with no flags"),
            (8, "ZIP must be stored with no flags"),
            (14, "ZIP CRC32"),
            (18, "stored ZIP sizes differ"),
            (central - 1, "ZIP CRC32"),
            (central, "ZIP central directory"),
            (central + 8, "ZIP central directory"),
            (central + 10, "ZIP central directory"),
            (end, "ZIP end directory"),
            (end + 4, "ZIP end directory"),
            (end + 6, "ZIP end directory"),
            (end + 8, "ZIP end directory"),
            (end + 10, "ZIP end directory"),
            (end + 12, "ZIP end directory"),
            (end + 16, "ZIP end directory"),
        ] {
            let mut bad = bytes.clone();
            bad[offset] ^= 1;
            rejection(&bad, expected);
        }
        for cut in 0..bytes.len() {
            rejection(&bytes[..cut], "truncated input");
        }
        let mut bad = bytes.clone();
        bad.push(0);
        rejection(&bad, "trailing ZIP bytes");
        let mut bad = bytes.clone();
        bad[local + 30..local + 34].copy_from_slice(b"meta");
        rejection(&bad, "invalid or duplicate NPZ key");
        for byte in [b'/', b'\\', 0] {
            let mut bad = bytes.clone();
            bad[30] = byte;
            rejection(&bad, "invalid or duplicate NPZ key");
        }
        let mut bad = bytes.clone();
        bad[30] = 255;
        rejection(&bad, "NPZ key encoding");
        let mut bad = bytes;
        bad[37] = b'z';
        rejection(&bad, "NPZ member is not NPY");
    }

    // ZIP64 changes only the local size fields and their extra field. Central
    // offsets must still account for the inserted bytes.
    fn zip64(extra: &[u8], compressed: bool, uncompressed: bool) -> Vec<u8> {
        let mut bytes = archive();
        let end = bytes.len() - 22;
        let central = u32::from_le_bytes(bytes[end + 16..end + 20].try_into().unwrap()) as usize;
        for (offset, enabled) in [(18, compressed), (22, uncompressed)] {
            if enabled {
                bytes[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            }
        }
        bytes[4..6].copy_from_slice(&45_u16.to_le_bytes());
        bytes[28..30].copy_from_slice(&(extra.len() as u16).to_le_bytes());
        // The second central entry follows the first 46-byte header and name.
        let second = central + 46 + 8;
        let offset = u32::from_le_bytes(bytes[second + 42..second + 46].try_into().unwrap());
        bytes[second + 42..second + 46]
            .copy_from_slice(&(offset + extra.len() as u32).to_le_bytes());
        bytes[end + 16..end + 20]
            .copy_from_slice(&(central as u32 + extra.len() as u32).to_le_bytes());
        bytes.splice(38..38, extra.iter().copied());
        bytes
    }

    #[test]
    fn checkpoint_reader_accepts_zip64_sizes_and_rejects_bad_extra_fields() {
        let bytes = archive();
        let size = u32::from_le_bytes(bytes[22..26].try_into().unwrap()) as u64;
        for (compressed, uncompressed) in [(true, true), (true, false), (false, true)] {
            let count = usize::from(compressed) + usize::from(uncompressed);
            let mut extra = vec![1, 0, (count * 8) as u8, 0];
            for _ in 0..count {
                extra.extend(size.to_le_bytes());
            }
            let bytes = zip64(&extra, compressed, uncompressed);
            let (mut reader, metadata): (_, Value) =
                Reader::with_metadata(bytes.as_slice(), "test").unwrap();
            assert_eq!(metadata, json!({"epoch": 3}));
            assert_eq!(reader.values::<f32>("data", &[1]).unwrap(), [1.25]);
            reader.finish().unwrap();
        }
        rejection(&zip64(&[], true, true), "missing ZIP64 size");
        rejection(&zip64(&[2, 0, 0, 0], true, true), "missing ZIP64 size");
        rejection(&zip64(&[1, 0, 8, 0], true, true), "truncated input");
        let mut extra = vec![1, 0, 16, 0];
        extra.extend(size.to_le_bytes());
        extra.extend((size + 1).to_le_bytes());
        rejection(&zip64(&extra, true, true), "stored ZIP sizes differ");
        extra[12..20].copy_from_slice(&size.to_le_bytes());
        extra.extend([1, 0, 0, 0]);
        rejection(&zip64(&extra, true, true), "duplicate ZIP64 size");
        // The independently generated NumPy fixture has ZIP64 local headers too.
        let numpy = include_bytes!("../../tests/fixtures/nn-arrays.npz");
        assert!(matches!(
            Reader::with_metadata::<Value>(&numpy[..], "test"),
            Err(Error::Format("missing checkpoint meta"))
        ));
    }

    #[test]
    fn failed_stream_does_not_replace_the_committed_checkpoint() {
        let path = std::env::temp_dir().join(format!(
            "minase-checkpoint-failed-{}.npz",
            std::process::id()
        ));
        let original = archive();
        std::fs::write(&path, &original).unwrap();
        let result = super::super::atomic_write_with(&path, |output| -> Result<()> {
            let mut writer = Writer::with_metadata(output, "test", &json!({"epoch": 3}))?;
            writer.values("wrong-shape", &[2], &[1.0_f32])?;
            writer.finish()
        });
        assert!(matches!(result, Err(Error::Format("NPY array length"))));
        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::remove_file(&path).unwrap();
        let mut temporary = path.into_os_string();
        temporary.push(".tmp");
        std::fs::remove_file(temporary).unwrap();
    }
}
