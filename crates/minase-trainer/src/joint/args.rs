use super::Result;
use crate::fm::batch::check_size;
use crate::{
    data::{Lookahead, validate_lambda_override},
    nn::AdamWGroup,
};
use serde::Serialize;
use std::path::PathBuf;

/// PSTとFMの同時学習の学習条件と保存先。
#[derive(Clone, Debug, clap::Args, Serialize)]
pub struct Args {
    /// 来歴付きMNSDを、学習中に変更しない入力として指定する。
    #[arg(long, required = true, num_args = 1..)]
    pub data: Vec<PathBuf>,
    /// 初期PST、探索用駒価値、K、規則名を持つMNPT版3。
    #[arg(long)]
    pub base: PathBuf,
    /// 初期の実数PST端点を持つnpz。
    #[arg(long)]
    pub pst_init: PathBuf,
    /// 初期の実数V、a、maskを持つnpz。
    #[arg(long)]
    pub fm_init: PathBuf,
    /// 縮小前のFMを持つMNPT版3。
    #[arg(long)]
    pub fm_reference: PathBuf,
    /// 新規の空の出力ディレクトリ。再開時は--resumeと同じ場所。
    #[arg(long)]
    pub output_dir: PathBuf,
    /// 正の有限な学習率。
    #[arg(long, default_value_t = 1e-3)]
    pub fm_lr: f64,
    /// PST群の学習率。重み減衰は0。
    #[arg(long, default_value_t = 0.03)]
    pub pst_lr: f64,
    /// 再開前を含むエポック数の上限。1以上200以下。
    #[arg(long, default_value_t = 200)]
    pub epochs: u64,
    /// 改善がないエポックを許す数。1以上。
    #[arg(long, default_value_t = 5)]
    pub patience: u64,
    /// バッチの局面数。端数も学習する。
    #[arg(long, default_value_t = 16_384)]
    pub batch: usize,
    /// 順列と量子化検査の標本に使うシード。
    #[arg(long, default_value_t = 1)]
    pub seed: u64,
    /// AdamWの分離した重み減衰。
    #[arg(long, default_value_t = 1e-4)]
    pub fm_weight_decay: f64,
    /// 係数を掛ける前の駒除去損失への倍率。
    #[arg(long, default_value_t = 1000.0)]
    pub removal_penalty: f64,
    /// FM補正の二乗平均に掛ける正則化係数。
    #[arg(long, default_value_t = 1e-4)]
    pub lambda_res: f64,
    /// 自己対局のλを上書きする。省略時は来歴のλを使う。
    #[arg(long)]
    pub lambda_override: Option<f64>,
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
            lr: self.fm_lr,
            weight_decay: self.fm_weight_decay,
        }
        .validate()?;
        AdamWGroup {
            lr: self.pst_lr,
            weight_decay: 0.0,
        }
        .validate()?;
        if !self.removal_penalty.is_finite() || self.removal_penalty < 0.0 {
            return Err("--removal-penalty must be finite and nonnegative".into());
        }
        validate_lambda_override(self.lambda_override)?;
        if self.data.is_empty() {
            return Err("--data requires at least one file".into());
        }
        if !(1..=200).contains(&self.epochs) {
            return Err("--epochs must be in 1..200".into());
        }
        if self.patience == 0 {
            return Err("--patience must be positive".into());
        }
        check_size(self.batch)?;
        if self.validation_sample == 0 || self.threads == Some(0) {
            return Err("--validation-sample and --threads must be positive".into());
        }
        if !self.lambda_res.is_finite() || self.lambda_res < 0.0 {
            return Err("--lambda-res must be finite and nonnegative".into());
        }
        self.lookahead()?;
        Ok(())
    }

    pub(super) fn lookahead(&self) -> Result<Option<Lookahead>> {
        match (self.lookahead_gamma, self.lookahead_plies) {
            (None, None) => Ok(None),
            (Some(gamma), Some(plies)) => Ok(Some(Lookahead::new(gamma, plies)?)),
            _ => Err("lookahead requires both --lookahead-gamma and --lookahead-plies".into()),
        }
    }
}
