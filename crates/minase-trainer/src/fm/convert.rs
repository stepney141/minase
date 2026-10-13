//! 学習済みFMの補正を縮小し、重みファイルだけで評価を再現できる形に保存する。
//! [`quarter`]は指数だけを変え、[`scale`]は実数の出力係数を変えて量子化し直す。

use super::{
    Result, ZeroCorrectionReport,
    npz::FloatWeights,
    quantize,
    report::{ValidationReport, scale_report},
    zero_correction_report,
};
use crate::identity::{FileIdentity, file_identity};
use clap::{Args, Subcommand};
use minase::eval::pst::{FmWeights, Pst};
use serde::Serialize;
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

/// FMの学習と学習済み重みの変換。
#[derive(Subcommand)]
pub enum Command {
    /// 固定した整数PSTへ加えるランク32のFMをCPUで学習する。
    Train(super::Args),
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
    /// `middlegame`と`endgame`が`[13680]`、`V`が`[13680, 32]`、`a`が`[32]`の、dtype `<f4`または`<f8`の有限値を持つNPZを指定する。
    /// `joint train`の`pst-float.npz`を使え、端点のない`fm train`の`fm-float.npz`は使えない。
    #[arg(long)]
    pub float: PathBuf,
    /// 1024を等倍とする整数倍率。0ではFMの次元0のMNPT版3を書く。
    #[arg(long, value_parser = clap::value_parser!(i32).range(0..=1024))]
    pub fm_scale: i32,
    /// 新規に書くMNPT。隣に.scale.jsonの報告も書く。
    #[arg(long)]
    pub output: PathBuf,
    /// MNSDを入力順に指定する。
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
}

/// 補正1/4への変換で保存する検査和、指数、補正0の検査結果。
#[derive(Debug, Serialize)]
pub struct QuarterReport {
    /// 入力MNPTのパスとヘッダ内の検査和。
    pub input: FileIdentity,
    /// 出力MNPTのパスとヘッダ内の検査和。
    pub output: FileIdentity,
    /// 縮小前の指数。
    pub exponent_before: u32,
    /// 縮小後の指数。
    pub exponent_after: u32,
    /// 縮小後の補正0の検査。
    pub zero_check: ZeroCorrectionReport,
}

/// 倍率の焼き込みの記録。検証なしではvalidationを省略する。
#[derive(Debug, Serialize)]
pub struct ScaleReport {
    /// 入力MNPTのパスとヘッダ内の検査和。
    pub input: FileIdentity,
    /// 入力NPZのパスとバイト長。
    pub float: FileIdentity,
    /// 出力MNPTのパスとヘッダ内の検査和。
    pub output: FileIdentity,
    /// 1024を等倍とする倍率。
    pub fm_scale: i32,
    /// 出力MNPTの版。
    pub format_version: u32,
    /// 入力FMの指数。
    pub exponent_before: u32,
    /// 出力FMの指数。FMを持たない場合はnull。
    pub exponent_after: Option<u32>,
    /// 出力埋め込みの最大絶対値。FMを持たない場合はnull。
    pub max_absolute_embedding: Option<i32>,
    /// 検証標本での比較。MNSDを指定した場合だけ保存する。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ValidationReport>,
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

/// 指数を`e + 1`にし、補正を`trunc(N / 2^(2e+3))`へ縮小する。
/// 埋め込み、符号、PST、K、駒価値、規則名は変えない。
/// 指数30の入力と、出力または拡張子を`.quarter.json`に替えた報告先が既存の場合は拒否する。
/// 書いた後に読み直し、バイト列と再符号化した結果が一致することを検査する。
pub fn quarter(args: &QuarterArgs) -> Result<QuarterReport> {
    let report_path = report_path(&args.output, "quarter.json")?;
    let input = fs::read(&args.input)?;
    let source = Pst::decode(&input)?;
    let before = source
        .fm_weights()
        .ok_or("MNPT with FM weights (rank 32) required")?;
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
        input: file_identity(&args.input)?,
        output: crate::identity::mnpt_output_identity(&args.output, &bytes)?,
        exponent_before: before.exponent,
        exponent_after: after.exponent,
        zero_check,
    };
    write_output(&args.output, &report_path, &bytes, &report)?;
    Ok(report)
}

/// 整数倍率`0 ≤ s ≤ 1,024`を`a′ = a * s / 1024`として焼き込み、Vを保って指数を探し直す。
/// 開始前に実数NPZのPSTとFMを量子化し、それぞれ入力MNPTと完全一致することを要求する。
/// `s = 0`ではFMの次元0のMNPT版3、`s = 1024`では入力と同じ重みを出力し、PST、K、駒価値、規則名を保つ。
/// 倍率を推論時に掛けずに重みへ含めることで、エンジンの評価式を変えず重みファイルだけで評価が定まる。
/// 検証標本の計算は保存前に完了し、書いたMNPTは読み直して検査する。
/// 報告は`--output`の拡張子を`.scale.json`に替えたパスへ書く。
/// 出力MNPTか報告のどちらかが既に存在すれば拒否する。
pub fn scale(args: &ScaleArgs) -> Result<ScaleReport> {
    if !(0..=1024).contains(&args.fm_scale) {
        return Err("FM scale must be an integer in 0..1024".into());
    }
    if args.batch == 0 || args.validation_sample == 0 {
        return Err("batch and validation sample size must be positive".into());
    }
    let report_path = report_path(&args.output, "scale.json")?;
    let input = fs::read(&args.input)?;
    let source = Pst::decode(&input)?;
    let before = source
        .fm_weights()
        .ok_or("MNPT with FM weights (rank 32) required")?;
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
        input: file_identity(&args.input)?,
        float: file_identity(&args.float)?,
        output: crate::identity::mnpt_output_identity(&args.output, &bytes)?,
        fm_scale: args.fm_scale,
        format_version: 3,
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
        Command::Train(args) => serde_json::to_string(&super::train(args)?)?,
        Command::Quarter(args) => serde_json::to_string(&quarter(&args)?)?,
        Command::Scale(args) => serde_json::to_string(&scale(&args)?)?,
    };
    println!("{report}");
    Ok(())
}
