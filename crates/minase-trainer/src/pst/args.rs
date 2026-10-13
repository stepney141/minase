use super::{Error, Result, batch::check_size};
use crate::{
    data::{Lookahead, validate_lambda},
    nn::AdamWGroup,
};
use serde::Serialize;
use std::path::PathBuf;

/// 駒除去差分の参照。
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RemovalReferenceKind {
    /// v0の駒価値を使う。
    V0,
    /// 初期MNPTの整数PSTを使う。
    Init,
}

/// 鏡映共有PSTの学習条件と保存先。
#[derive(Clone, Debug, clap::Args, Serialize)]
pub struct Args {
    /// MNSDを、学習中に変更しない入力として指定する。
    #[arg(long, required = true, num_args = 1..)]
    pub data: Vec<PathBuf>,
    /// 初期PST、探索用駒価値、K、規則名を持つMNPT版3。
    #[arg(long)]
    pub init: PathBuf,
    /// 新規の空の出力ディレクトリ。再開時は--resumeと同じ場所。
    #[arg(long)]
    pub output_dir: PathBuf,
    /// 正の有限な学習率。
    #[arg(long, default_value_t = 0.03)]
    pub lr: f64,
    /// 再開前を含むエポック数の上限。1以上200以下。
    #[arg(long)]
    pub epochs: u64,
    /// 改善がないエポックを許す数。1以上。
    #[arg(long)]
    pub patience: u64,
    /// バッチの局面数。端数も学習する。
    #[arg(long, default_value_t = 16_384)]
    pub batch: usize,
    /// 順列と量子化検査の標本に使うシード。
    #[arg(long, default_value_t = 1)]
    pub seed: u64,
    /// 非負の有限な除去損失係数。
    #[arg(long, default_value_t = 1000.0)]
    pub removal_penalty: f64,
    /// 除去差分の参照を明示する。
    #[arg(long, value_enum)]
    pub removal_reference: RemovalReferenceKind,
    /// 自己対局の教師の混合比を有限な0以上1以下で指定する。実戦対局には0を適用する。
    #[arg(long, value_parser = crate::data::parse_lambda)]
    pub lambda: f64,
    /// 先読み割引率。0より大きく1より小さい有限値。
    #[arg(long, requires = "lookahead_plies")]
    pub lookahead_gamma: Option<f64>,
    /// 先読み幅。1以上4096以下。
    #[arg(long, requires = "lookahead_gamma")]
    pub lookahead_plies: Option<u16>,
    /// 量子化誤差を測る検証標本数の上限。
    #[arg(long, default_value_t = 10_000)]
    pub validation_sample: usize,
    /// CPUのスレッド数。省略時は物理コア数−2、最小1。
    #[arg(long)]
    pub threads: Option<usize>,
    /// 指定ディレクトリのlatest.binから再開する。
    #[arg(long)]
    pub resume: Option<PathBuf>,
}

impl Args {
    /// 入力を開く前に、値の範囲と引数の組合せを検査する。
    pub fn validate(&self) -> Result<()> {
        AdamWGroup {
            lr: self.lr,
            weight_decay: 0.0,
        }
        .validate()?;
        validate_lambda(self.lambda)?;
        if self.data.is_empty() {
            return Err(Error::Invalid("--data requires at least one file"));
        }
        if !(1..=200).contains(&self.epochs) {
            return Err(Error::Invalid("--epochs must be in 1..200"));
        }
        if self.patience == 0 {
            return Err(Error::Invalid("--patience must be positive"));
        }
        check_size(self.batch)?;
        if self.validation_sample == 0 || self.threads == Some(0) {
            return Err(Error::Invalid(
                "--validation-sample and --threads must be positive",
            ));
        }
        if !self.removal_penalty.is_finite() || self.removal_penalty < 0.0 {
            return Err(Error::Invalid(
                "--removal-penalty must be finite and nonnegative",
            ));
        }
        self.lookahead()?;
        Ok(())
    }

    pub(super) fn lookahead(&self) -> Result<Option<Lookahead>> {
        match (self.lookahead_gamma, self.lookahead_plies) {
            (None, None) => Ok(None),
            (Some(gamma), Some(plies)) => Ok(Some(Lookahead::new(gamma, plies)?)),
            _ => Err(Error::Invalid(
                "lookahead requires both --lookahead-gamma and --lookahead-plies",
            )),
        }
    }
}
