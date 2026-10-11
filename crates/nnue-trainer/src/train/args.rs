use super::{Error, Result};
use crate::{
    data::{InputFeatureSet, Lookahead, MAX_BATCH_SIZE},
    model::Shape,
};
use serde::Serialize;
use std::{collections::HashSet, path::PathBuf};

/// GPU学習の入力と実行条件。最適化器の係数は固定する。
#[derive(Clone, Debug, clap::Args, Serialize)]
pub struct Args {
    /// 入力の特徴集合を指定する。
    #[arg(long, value_enum)]
    pub feature_set: InputFeatureSet,
    /// 隠れ層の幅を2以上512以下の偶数で指定する。
    #[arg(long)]
    pub width: usize,
    /// 来歴ファイルを伴うMNSDファイルを、全体のレコード順に指定する。
    #[arg(long, required = true, num_args = 1..)]
    pub data: Vec<PathBuf>,
    /// 浮動小数点の初期駒配置評価表（PST）を格納した無圧縮NPZファイルを指定する。
    #[arg(long)]
    pub init_pst: PathBuf,
    /// 初期PST、駒価値、出力用K、規則セット名の基準となるMNPTファイルを指定する。
    #[arg(long)]
    pub base: PathBuf,
    /// 設定、ログ、チェックポイント、学習済みネットワークの保存先を指定する。
    #[arg(long)]
    pub output_dir: PathBuf,
    /// 先読みの割引率を0より大きく1より小さい値で指定する。--lookahead-pliesも必要。
    #[arg(long, requires = "lookahead_plies")]
    pub lookahead_gamma: Option<f64>,
    /// 先読み幅を1以上4096以下の手数で指定する。--lookahead-gammaも必要。
    #[arg(long, requires = "lookahead_gamma")]
    pub lookahead_plies: Option<u16>,
    /// 1更新の最大局面数を指定する。末尾のバッチは指定数より少なくてもよい。
    #[arg(long, default_value_t = 16_384)]
    pub batch: usize,
    /// 再開前の完了分を含むエポック数の上限を1以上50以下で指定する。
    #[arg(long, default_value_t = 50)]
    pub epochs: u64,
    /// 改善しないエポックが指定数だけ連続して完了したら停止する。
    #[arg(long, default_value_t = 5)]
    pub patience: u64,
    /// 初期化、順列作成、検証標本抽出の乱数シードを指定する。
    #[arg(long, default_value_t = 1)]
    pub seed: u64,
    /// 前処理、検証、再開前の実行を含む累積時間の上限を24時間以下で指定する。
    #[arg(long, default_value_t = 24.0)]
    pub time_limit_hours: f64,
    /// 量子化誤差の検査に使う検証局面数の上限を指定する。
    #[arg(long, default_value_t = 10_000)]
    pub validation_sample: usize,
    /// バッチ供給のワーカー数を指定する。省略時は物理コア数から2を引き、最小値を1とする。
    #[arg(long)]
    pub threads: Option<usize>,
    /// 指定ディレクトリの最新チェックポイントから再開する。--output-dirと同じ場所を指定する。
    #[arg(long)]
    pub resume: Option<PathBuf>,
}

impl Args {
    /// ファイルやGPUに触れる前に値と引数の組合せを検査する。
    pub fn validate(&self) -> Result<()> {
        Shape::new(self.feature_set, self.width, 1.0)?;
        if self.data.is_empty() {
            return Err(Error::Invalid("--data requires at least one file").into());
        }
        if !(1..=MAX_BATCH_SIZE).contains(&self.batch) {
            return Err(
                Error::Invalid("--batch exceeds the reverse-index capacity or is zero").into(),
            );
        }
        if !(1..=50).contains(&self.epochs) {
            return Err(Error::Invalid("--epochs must be in 1..=50").into());
        }
        if self.patience == 0 || self.validation_sample == 0 || self.threads == Some(0) {
            return Err(Error::Invalid(
                "--patience, --validation-sample and --threads must be positive",
            )
            .into());
        }
        if !self.time_limit_hours.is_finite()
            || !(0.0 < self.time_limit_hours && self.time_limit_hours <= 24.0)
        {
            return Err(Error::Invalid("--time-limit-hours must be finite and in (0, 24]").into());
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
            )
            .into()),
        }
    }

    pub(super) fn workers(&self) -> Result<usize> {
        if let Some(n) = self.threads {
            return Ok(n);
        }
        let cpuinfo = std::fs::read_to_string("/proc/cpuinfo")?;
        physical_workers(&cpuinfo)
    }
}

fn physical_workers(cpuinfo: &str) -> Result<usize> {
    let mut cores = HashSet::new();
    for processor in cpuinfo.split("\n\n").filter(|s| !s.trim().is_empty()) {
        let field = |key: &str| {
            processor.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                (name.trim() == key).then_some(value.trim())
            })
        };
        let package = field("physical id").ok_or(Error::Invalid(
            "CPU topology has no physical id; set --threads explicitly",
        ))?;
        let core = field("core id").ok_or(Error::Invalid(
            "CPU topology has no core id; set --threads explicitly",
        ))?;
        cores.insert((package, core));
    }
    if cores.is_empty() {
        return Err(Error::Invalid("CPU topology is empty; set --threads explicitly").into());
    }
    Ok(cores.len().saturating_sub(2).max(1))
}
