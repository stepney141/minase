use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use clap::Args;
use serde::Serialize;

use super::Result;
use crate::{
    data::{Dataset, epoch_permutation},
    nn::{Sha256Hasher, hex},
};

#[derive(Args)]
pub(super) struct SampleArgs {
    /// 来歴ファイルを伴うMNSDを入力順に指定する。
    #[arg(long, required = true, num_args = 1..)]
    pub(super) data: Vec<PathBuf>,
    /// 抽出する局面数。
    #[arg(long)]
    pub(super) sample: usize,
    /// SplitMix64の標本抽出用シード。
    #[arg(long)]
    pub(super) seed: u64,
}

impl SampleArgs {
    pub(super) fn open(&self) -> Result<Dataset> {
        // SAFETY: 他の学習器コマンドと同じく、実行中の入力MNSDを変更しない契約で読み込む。
        Ok(unsafe { Dataset::open(&self.data, None, None)? })
    }

    pub(super) fn select(&self, dataset: &Dataset, validation: bool) -> Result<Vec<u64>> {
        let population = if validation {
            dataset.validation_indices().len() as u64
        } else {
            dataset.record_count()
        };
        let minimum = if validation { 1 } else { 2 };
        if self.sample < minimum || self.sample as u64 > population {
            return Err(format!(
                "sample must be in {minimum}..{population} {}positions",
                if validation { "validation " } else { "" }
            )
            .into());
        }
        let mut indices = if validation {
            epoch_permutation(dataset.validation_indices(), self.seed, 0)
        } else {
            let total = usize::try_from(dataset.record_count())?;
            epoch_permutation(&(0..total as u64).collect::<Vec<_>>(), self.seed, 0)
        };
        indices.truncate(self.sample);
        indices.sort_unstable();
        Ok(indices)
    }

    pub(super) fn sampling(&self) -> String {
        "sorted SplitMix64 Fisher-Yates permutation(total, seed, epoch=0)[:sample]".into()
    }
}

#[derive(Serialize)]
pub(super) struct FileIdentity {
    path: PathBuf,
    sha256: String,
}

pub(super) fn file_identity(path: &Path) -> Result<FileIdentity> {
    let path = path.canonicalize()?;
    let mut file = File::open(&path)?;
    let mut hash = Sha256Hasher::default();
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(FileIdentity {
        path,
        sha256: hex(&hash.finalize()),
    })
}

#[derive(Serialize)]
struct InputFile {
    #[serde(flatten)]
    identity: FileIdentity,
    record_count: u64,
}

#[derive(Serialize)]
pub(super) struct SampleIdentity {
    sample_count: usize,
    seed: u64,
    sample_indices_sha256_le_i64: String,
    input_files: Vec<InputFile>,
}

pub(super) fn sample_identity(
    dataset: &Dataset,
    indices: &[u64],
    seed: u64,
) -> Result<SampleIdentity> {
    let mut hash = Sha256Hasher::default();
    for &index in indices {
        hash.update(&i64::try_from(index)?.to_le_bytes());
    }
    let input_files = dataset
        .paths()
        .zip(dataset.headers())
        .map(|(path, header)| {
            Ok(InputFile {
                identity: file_identity(path)?,
                record_count: header.record_count(),
            })
        })
        .collect::<Result<_>>()?;
    Ok(SampleIdentity {
        sample_count: indices.len(),
        seed,
        sample_indices_sha256_le_i64: hex(&hash.finalize()),
        input_files,
    })
}
