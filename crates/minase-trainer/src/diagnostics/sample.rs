use std::path::PathBuf;

use clap::Args;
use serde::Serialize;

use super::Result;
use crate::{
    data::{Dataset, epoch_permutation},
    nn::{Sha256Hasher, hex},
};

#[derive(Args)]
pub(super) struct SampleArgs {
    /// MNSDを入力順に指定する。
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
    /// 学習と同じ[`Dataset::open`]で、先読みなしでMNSDを開く。
    /// 対局単位の検証分割も学習と同じ規則に従う。
    pub(super) fn open(&self) -> Result<Dataset> {
        Ok(Dataset::open(&self.data, None)?)
    }

    /// 母集団の通算番号から非復元抽出し、昇順に並べて返す。
    /// `nnue-distribution`では全局面の番号`0..記録数`を母集団とし、`sample`は2以上かつ母集団の件数以下とする。
    /// `nnue-removal`では`validation = true`で検証集合を使い、`sample`は1以上かつ検証局面数以下とする。
    /// 範囲外はエラーにする。
    /// 母集団の番号列へ[`epoch_permutation`]をシード`seed`、エポック0で適用し、先頭`sample`件を昇順に並べる。
    /// 順列はSplitMix64によるFisher–Yatesで生成する。
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

    /// 分布診断の`sampling`欄へ保存する固定文字列を返す。
    /// 値は`"sorted SplitMix64 Fisher-Yates permutation(total, seed, epoch=0)[:sample]"`である。
    /// 検証集合を抽出するときの手順も同じだが、`nnue-removal`の報告はこの欄を持たない。
    pub(super) fn sampling(&self) -> String {
        "sorted SplitMix64 Fisher-Yates permutation(total, seed, epoch=0)[:sample]".into()
    }
}

pub(super) use crate::identity::{FileIdentity, file_identity};

#[derive(Serialize)]
struct InputFile {
    #[serde(flatten)]
    identity: FileIdentity,
    record_count: u64,
}

/// 診断報告の最上位へ平坦化して保存する標本の情報。
/// `sample_count`は抽出局面数、`seed`は`--seed`の値である。
/// `sample_indices_sha256_le_i64`は、昇順に並べた通算番号をそれぞれリトルエンディアンi64の8バイトにして連結した列のSHA-256を小文字16進で表す。
/// `input_files`は入力順の配列で、各要素は[`FileIdentity`]のパス、バイト長、ヘッダに、適格な局面数`record_count`を加えた情報を持つ。
#[derive(Serialize)]
pub(super) struct SampleIdentity {
    sample_count: usize,
    seed: u64,
    sample_indices_sha256_le_i64: String,
    input_files: Vec<InputFile>,
}

/// 抽出済みの昇順の通算番号と入力ファイルから[`SampleIdentity`]を作る。
/// 番号列はこの関数では並べ替えず、渡された順にハッシュへ加える。
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
        .zip(dataset.file_sample_counts())
        .map(|(path, count)| {
            Ok(InputFile {
                identity: file_identity(path)?,
                record_count: count,
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
