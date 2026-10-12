//! Pythonとの一度きりの照合に使う再生ファイル。照合後に削除する。

use super::{Dataset, Error, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
struct Manifest {
    format: String,
    trainer: String,
    files: Vec<Input>,
    batch: usize,
    epochs: u64,
}

#[derive(Deserialize)]
struct Input {
    path: PathBuf,
    sha256: String,
}

/// 一時的な再生ディレクトリと、読み込んだreplay.jsonの同一性。
pub struct Replay {
    directory: PathBuf,
    sha256: String,
    epochs: u64,
}

/// 訓練集合をちょうど1回ずつ含む列と、同じ順序の鏡映ビット。
pub struct Epoch {
    /// MNSDの入力順に付けた通し番号。
    pub indices: Vec<u64>,
    /// 各局面を左右鏡映する場合は1、しない場合は0。
    pub mirror: Vec<u8>,
}

fn invalid(path: &Path, reason: &'static str) -> Error {
    Error::Replay {
        path: path.to_owned(),
        reason,
    }
}

/// 昇順little-endian i64の標本番号を読み、件数、範囲、重複と検証集合を検査する。
/// Pythonとの照合用の一時的な読み手であり、フェーズ6で削除する。
pub fn read_sample(
    path: &Path,
    dataset: &Dataset,
    sample: usize,
    validation: bool,
) -> Result<Vec<u64>> {
    let bytes = std::fs::read(path)?;
    if bytes.len() % 8 != 0 || bytes.len() / 8 != sample {
        return Err(invalid(
            path,
            "replay sample count does not match requested sample (expected little-endian i64)",
        ));
    }
    let mut indices = Vec::with_capacity(sample);
    for chunk in bytes.as_chunks::<8>().0 {
        let index = i64::from_le_bytes(*chunk);
        if index < 0 || index as u64 >= dataset.record_count() {
            return Err(invalid(path, "replay sample index is out of range"));
        }
        let index = index as u64;
        if indices.last().is_some_and(|&previous| previous >= index) {
            return Err(invalid(
                path,
                "replay sample indices must be strictly ascending without duplicates",
            ));
        }
        if validation && dataset.validation_indices().binary_search(&index).is_err() {
            return Err(invalid(
                path,
                "replay sample index is outside the validation split",
            ));
        }
        indices.push(index);
    }
    Ok(indices)
}

impl Replay {
    /// 一時的な再生入力の版、学習器、入力順、SHA-256、バッチとエポック数を検査する。
    /// 局面列と鏡映ビットは`read_epoch`で検査する。
    pub fn open(
        directory: &Path,
        trainer: &str,
        dataset: &Dataset,
        batch: usize,
        epochs: u64,
    ) -> Result<Self> {
        let directory = directory.canonicalize()?;
        let path = directory.join("replay.json");
        let bytes = std::fs::read(&path)?;
        let manifest: Manifest =
            serde_json::from_slice(&bytes).map_err(|_| invalid(&path, "invalid replay.json"))?;
        if manifest.format != "minase-replay-1" || manifest.trainer != trainer {
            return Err(invalid(&path, "replay format or trainer mismatch"));
        }
        if manifest.batch != batch || batch == 0 {
            return Err(invalid(&path, "replay batch mismatch"));
        }
        if manifest.epochs < epochs || epochs == 0 {
            return Err(invalid(&path, "replay has insufficient epochs"));
        }
        if manifest.files.len() != dataset.paths().count() {
            return Err(invalid(&path, "replay input file count mismatch"));
        }
        for (input, actual) in manifest.files.iter().zip(dataset.paths()) {
            if !input.path.is_absolute() || input.path.canonicalize()? != actual {
                return Err(invalid(&path, "replay input file order or path mismatch"));
            }
            let mut file = File::open(actual)?;
            let mut hash = Sha256::new();
            let mut buffer = [0_u8; 65536];
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                hash.update(&buffer[..count]);
            }
            if !input
                .sha256
                .eq_ignore_ascii_case(&format!("{:x}", hash.finalize()))
            {
                return Err(invalid(&path, "replay input SHA-256 mismatch"));
            }
        }
        Ok(Self {
            directory,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            epochs: manifest.epochs,
        })
    }

    /// 正規化した再生ディレクトリのパス。
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// 読み込んだreplay.jsonのSHA-256。
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// 1から数えるエポックを読み、欠落、重複、集合外の局面と不正な鏡映ビットを拒否する。
    pub fn read_epoch(&self, dataset: &Dataset, epoch: u64) -> Result<Epoch> {
        let path = self.directory.join(format!("epoch-{epoch:03}.positions"));
        if epoch == 0 || epoch > self.epochs {
            return Err(invalid(&path, "replay epoch outside manifest"));
        }
        let bytes = std::fs::read(&path)?;
        let (records, tail) = bytes.as_chunks::<8>();
        let training = dataset.training_indices();
        if !tail.is_empty() || records.len() != training.len() {
            return Err(invalid(
                &path,
                "replay position count or record length mismatch",
            ));
        }
        let mut offsets = vec![0_u64];
        for header in dataset.headers() {
            offsets.push(offsets.last().unwrap() + header.record_count());
        }
        let mut seen = vec![false; training.len()];
        let mut indices = Vec::with_capacity(training.len());
        for record in records {
            let file = u32::from_le_bytes(record[..4].try_into().unwrap()) as usize;
            let local = u64::from(u32::from_le_bytes(record[4..].try_into().unwrap()));
            if file >= offsets.len() - 1 || local >= offsets[file + 1] - offsets[file] {
                return Err(invalid(&path, "replay file or record index out of range"));
            }
            let index = offsets[file] + local;
            let slot = training
                .binary_search(&index)
                .map_err(|_| invalid(&path, "replay position outside training set"))?;
            if seen[slot] {
                return Err(invalid(&path, "duplicate replay position"));
            }
            seen[slot] = true;
            indices.push(index);
        }
        let path = self.directory.join(format!("epoch-{epoch:03}.mirror"));
        let mirror = std::fs::read(&path)?;
        if mirror.len() != indices.len() || mirror.iter().any(|&value| value > 1) {
            return Err(invalid(&path, "replay mirror length or bit value mismatch"));
        }
        Ok(Epoch { indices, mirror })
    }
}
