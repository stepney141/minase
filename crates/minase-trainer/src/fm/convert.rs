use super::{
    Result, ZeroCorrectionReport,
    npz::FloatWeights,
    quantize,
    report::{ValidationReport, scale_report},
    zero_correction_report,
};
use clap::{Args, Subcommand};
use minase::eval::pst::{FmWeights, Pst};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

/// 学習を伴わないFM重みの変換。
#[derive(Subcommand)]
pub enum Command {
    /// 指数だけを1増やし、補正を1/4にする。
    Quarter(QuarterArgs),
    /// 実数の出力係数へ倍率を掛けて量子化し直す。
    Scale(ScaleArgs),
}

/// FM補正を1/4にする入出力。
#[derive(Args)]
pub struct QuarterArgs {
    /// 入力のMNPT版3。
    #[arg(long)]
    pub input: PathBuf,
    /// 新規に書くMNPT版3。隣に.quarter.jsonの報告も書く。
    #[arg(long)]
    pub output: PathBuf,
}

/// FMの倍率の焼き込みと、任意の検証標本による比較。
#[derive(Args)]
pub struct ScaleArgs {
    /// 入力のMNPT版3。
    #[arg(long)]
    pub input: PathBuf,
    /// 入力MNPTと同じ学習で保存した実数のNPZ。
    #[arg(long)]
    pub float: PathBuf,
    /// 1024を等倍とする整数倍率。0ではMNPT版2を書く。
    #[arg(long, value_parser = clap::value_parser!(i32).range(0..=1024))]
    pub fm_scale: i32,
    /// 新規に書くMNPT。隣に.scale.jsonの報告も書く。
    #[arg(long)]
    pub output: PathBuf,
    /// 来歴ファイルを伴うMNSDを入力順に指定する。
    #[arg(long, num_args = 1..)]
    pub data: Vec<PathBuf>,
    /// 検証を処理するバッチの局面数。
    #[arg(long, default_value_t = 4096)]
    pub batch: usize,
    /// 検証集合から非復元抽出する最大局面数。
    #[arg(long, default_value_t = 10000)]
    pub validation_sample: usize,
    /// SplitMix64の標本抽出用シード。
    #[arg(long, default_value_t = 1)]
    pub seed: u64,
    /// 照合用の昇順little-endian i64標本番号。一時的な引数で、フェーズ6で削除する。
    #[arg(long, requires = "data")]
    pub replay_sample: Option<PathBuf>,
}

/// Pythonのquarter報告と同じ項目と順序。
#[derive(Debug, Serialize)]
pub struct QuarterReport {
    /// 入力ファイル全体のSHA-256。
    pub input_sha256: String,
    /// 出力ファイル全体のSHA-256。
    pub output_sha256: String,
    /// 縮小前の指数。
    pub exponent_before: u32,
    /// 縮小後の指数。
    pub exponent_after: u32,
    /// 縮小後の補正0の検査。
    pub zero_check: ZeroCorrectionReport,
}

/// Pythonのscale報告と同じ項目と順序。検証なしではvalidationを省略する。
#[derive(Debug, Serialize)]
pub struct ScaleReport {
    /// 入力MNPT全体のSHA-256。
    pub input_sha256: String,
    /// 入力NPZ全体のSHA-256。
    pub float_sha256: String,
    /// 出力MNPT全体のSHA-256。
    pub output_sha256: String,
    /// 1024を等倍とする倍率。
    pub fm_scale: i32,
    /// 出力MNPTの版。
    pub format_version: u32,
    /// 入力FMの指数。
    pub exponent_before: u32,
    /// 出力FMの指数。版2ではnull。
    pub exponent_after: Option<u32>,
    /// 出力埋め込みの最大絶対値。版2ではnull。
    pub max_absolute_embedding: Option<i32>,
    /// 検証標本での比較。MNSDを指定した場合だけ保存する。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ValidationReport>,
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn require_new(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(format!("output already exists: {}", path.display()).into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn report_path(output: &Path, extension: &str) -> Result<PathBuf> {
    let report = output.with_extension(extension);
    if output == report {
        return Err("MNPT output and report paths must differ".into());
    }
    require_new(output)?;
    require_new(&report)?;
    Ok(report)
}

fn with_fm(source: &Pst, fm: Option<FmWeights<'_>>) -> Result<Pst> {
    let values = std::array::from_fn(|state| source.piece_value_of_state(state));
    Ok(match fm {
        Some(fm) => {
            Pst::from_weights_with_fm(source.weights(), values, source.k(), source.rule_set(), fm)?
        }
        None => Pst::from_weights(source.weights(), values, source.k(), source.rule_set())?,
    })
}

fn write_output(
    output: &Path,
    report_path: &Path,
    bytes: &[u8],
    report: &impl Serialize,
) -> Result<()> {
    let mut json = serde_json::to_vec_pretty(report)?;
    json.push(b'\n');
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let mut file = File::create_new(output)?;
    let mut report_file = match File::create_new(report_path) {
        Ok(file) => file,
        Err(error) => {
            drop(file);
            fs::remove_file(output)?;
            return Err(error.into());
        }
    };
    file.write_all(bytes)?;
    drop(file);
    let written = fs::read(output)?;
    if written != bytes || Pst::decode(&written)?.encode() != bytes {
        return Err("written MNPT differs from exported weights".into());
    }
    report_file.write_all(&json)?;
    Ok(())
}

/// 指数以外の学習済み配列とメタデータを保ち、1/4の補正と報告を書き出す。
pub fn quarter(args: &QuarterArgs) -> Result<QuarterReport> {
    let report_path = report_path(&args.output, "quarter.json")?;
    let input = fs::read(&args.input)?;
    let source = Pst::decode(&input)?;
    let before = source.fm_weights().ok_or("MNPT version 3 required")?;
    if before.exponent == 30 {
        return Err("quarter correction exponent would exceed 30".into());
    }
    let after = FmWeights {
        exponent: before.exponent + 1,
        ..before
    };
    let zero_check = zero_correction_report(after.embeddings, after.signs, after.exponent)?;
    let bytes = with_fm(&source, Some(after))?.encode();
    let report = QuarterReport {
        input_sha256: hash(&input),
        output_sha256: hash(&bytes),
        exponent_before: before.exponent,
        exponent_after: after.exponent,
        zero_check,
    };
    write_output(&args.output, &report_path, &bytes, &report)?;
    Ok(report)
}

/// PSTとFMの再量子化が入力に完全一致することを確認し、倍率を焼き込む。
/// 検証標本の計算は保存前に完了し、書いたMNPTはエンジンの読み手で検査する。
pub fn scale(args: &ScaleArgs) -> Result<ScaleReport> {
    if !(0..=1024).contains(&args.fm_scale) {
        return Err("FM scale must be an integer in 0..1024".into());
    }
    if args.batch == 0 || args.validation_sample == 0 {
        return Err("batch and validation sample size must be positive".into());
    }
    if args.replay_sample.is_some() && args.data.is_empty() {
        return Err("--replay-sample requires --data".into());
    }
    let report_path = report_path(&args.output, "scale.json")?;
    let input = fs::read(&args.input)?;
    let source = Pst::decode(&input)?;
    let before = source.fm_weights().ok_or("MNPT version 3 required")?;
    let float_bytes = fs::read(&args.float)?;
    let weights = FloatWeights::decode(&float_bytes)?;
    if crate::pst::export::quantize_weights(&weights.endpoints)? != source.weights().as_slice() {
        return Err("floating PST does not quantize to the input MNPT endpoints".into());
    }
    let expected = quantize(&weights.v, &weights.a, f64::from(source.k()))?;
    if expected.as_weights() != before {
        return Err("floating FM does not quantize to the input MNPT FM".into());
    }
    let scaled_a = weights.a.map(|a| a * f64::from(args.fm_scale) / 1024.0);
    let output_fm = if args.fm_scale == 0 {
        None
    } else {
        Some(quantize(&weights.v, &scaled_a, f64::from(source.k()))?)
    };
    let output = with_fm(&source, output_fm.as_ref().map(|fm| fm.as_weights()))?;
    let validation = if args.data.is_empty() {
        None
    } else {
        Some(scale_report(&source, &output, &weights, &scaled_a, args)?)
    };
    let bytes = output.encode();
    let report = ScaleReport {
        input_sha256: hash(&input),
        float_sha256: hash(&float_bytes),
        output_sha256: hash(&bytes),
        fm_scale: args.fm_scale,
        format_version: if output_fm.is_some() { 3 } else { 2 },
        exponent_before: before.exponent,
        exponent_after: output_fm.as_ref().map(|fm| fm.exponent),
        max_absolute_embedding: output_fm.as_ref().map(|fm| {
            fm.embeddings
                .iter()
                .flatten()
                .map(|&u| i32::from(u).abs())
                .max()
                .expect("nonempty FM")
        }),
        validation,
    };
    write_output(&args.output, &report_path, &bytes, &report)?;
    Ok(report)
}

/// コマンドを実行し、保存した報告と同じ内容を標準出力へ1行で出す。
pub fn run(command: Command) -> Result<()> {
    let report = match command {
        Command::Quarter(args) => serde_json::to_string(&quarter(&args)?)?,
        Command::Scale(args) => serde_json::to_string(&scale(&args)?)?,
    };
    println!("{report}");
    Ok(())
}
