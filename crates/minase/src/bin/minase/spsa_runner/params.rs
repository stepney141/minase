//! USI宣言と6欄の係数ファイルの検査。
//!
//! # 係数ファイル
//!
//! UTF-8の各有効行を`名前, 開始値, 最小値, 最大値, c_end, r_end`の順でカンマ区切りにする。
//! 前後の空白を除いた空行と`#`で始まる行を読み飛ばし、各欄の前後の空白も除く。
//! 引用符によるエスケープ、見出し行、行末コメントは扱わない。
//! 名前は`Tune_`を除いたエンジンの宣言名で、重複を認めない。
//! 開始値は有限のf64、最小値と最大値はi32で、`最小値 ≤ 開始値 ≤ 最大値`かつ宣言の範囲内でなければならない。
//! `c_end`と`r_end`は有限の正のf64で、意味は[`Parameter`]に従う。
//! 有効行を少なくとも1行要求する。
//! `params`サブコマンドが標準出力へ生成する行の欄と既定値は[`super::summary::parameter_line`]に従う。

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fmt,
    num::{ParseFloatError, ParseIntError},
};

/// 係数ファイル1行の6欄。fishtestの入力形式に対応する。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Parameter {
    /// `Tune_`を除いた係数名。エンジンの宣言と照合する。
    pub name: String,
    /// 調整開始時のθ。係数の範囲内の有限値である。
    pub start: f64,
    /// 係数の下限。整数化と更新時の切り詰めに使う。
    pub min: i32,
    /// 係数の上限。整数化と更新時の切り詰めに使う。
    pub max: i32,
    /// 最終反復の摂動幅。係数と同じ単位の有限の正値である。
    pub c_end: f64,
    /// 最終反復の`a_k / c_k²`を定める有限の正値。
    pub r_end: f64,
}

#[derive(Debug)]
pub(super) struct Declaration {
    pub name: String,
    pub default: i32,
    pub min: i32,
    pub max: i32,
}

/// 入力の誤りを分類し、数値の解析原因も保持する。
#[derive(Debug)]
pub(super) enum ParameterError {
    NoDeclarations,
    InvalidDeclaration(String),
    Empty,
    FieldCount {
        line: usize,
        actual: usize,
    },
    UnknownName(String),
    Duplicate(String),
    Range(String),
    NonPositiveRate(String),
    Integer {
        line: usize,
        source: ParseIntError,
    },
    Float {
        line: usize,
        source: ParseFloatError,
    },
}

impl fmt::Display for ParameterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDeclarations => {
                write!(f, "engine declares no Tune_ options; use a tuning build")
            }
            Self::InvalidDeclaration(line) => write!(f, "invalid tuning declaration: {line}"),
            Self::Empty => write!(f, "parameter file contains no parameters"),
            Self::FieldCount { line, actual } => {
                write!(f, "line {line}: expected 6 fields, got {actual}")
            }
            Self::UnknownName(name) => write!(f, "unknown parameter: {name}"),
            Self::Duplicate(name) => write!(f, "duplicate parameter: {name}"),
            Self::Range(name) => write!(f, "invalid start or bounds for parameter: {name}"),
            Self::NonPositiveRate(name) => {
                write!(f, "c_end and r_end must be finite and positive: {name}")
            }
            Self::Integer { line, source } => write!(f, "line {line}: invalid integer: {source}"),
            Self::Float { line, source } => write!(f, "line {line}: invalid number: {source}"),
        }
    }
}
impl std::error::Error for ParameterError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Integer { source, .. } => Some(source),
            Self::Float { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub(super) fn declarations(lines: &[String]) -> Result<Vec<Declaration>, ParameterError> {
    let mut result = Vec::new();
    let mut names = BTreeSet::new();
    for line in lines {
        let words: Vec<_> = line.split_whitespace().collect();
        if words.get(..2) != Some(["option", "name"].as_slice())
            || !words.get(2).is_some_and(|name| name.starts_with("Tune_"))
        {
            continue;
        }
        let invalid = || ParameterError::InvalidDeclaration(line.clone());
        let [
            "option",
            "name",
            name,
            "type",
            "spin",
            "default",
            default,
            "min",
            min,
            "max",
            max,
        ] = words.as_slice()
        else {
            return Err(invalid());
        };
        let name = name.strip_prefix("Tune_").ok_or_else(invalid)?.to_owned();
        let default = default.parse::<i32>().map_err(|_| invalid())?;
        let min = min.parse::<i32>().map_err(|_| invalid())?;
        let max = max.parse::<i32>().map_err(|_| invalid())?;
        if name.is_empty() || min >= max || !(min..=max).contains(&default) {
            return Err(invalid());
        }
        if !names.insert(name.clone()) {
            return Err(ParameterError::Duplicate(name));
        }
        result.push(Declaration {
            name,
            default,
            min,
            max,
        });
    }
    if result.is_empty() {
        return Err(ParameterError::NoDeclarations);
    }
    Ok(result)
}

/// `名前, 開始値, 最小, 最大, c_end, r_end`の6欄を解析する。
///
/// 空行と、前後の空白を除いて`#`で始まる行を無視する。
/// 全係数がエンジンの`Tune_`宣言に存在し、範囲が宣言内に収まり、名前が重複しないことを検査する。
/// minaseは未知のオプションを黙って無視するため、この検査で名前の誤りを対局前に拒否する。
/// 各欄の意味は[`Parameter`]に従う。
pub(super) fn parse_parameters(
    text: &str,
    declarations: &[Declaration],
) -> Result<Vec<Parameter>, ParameterError> {
    if declarations.is_empty() {
        return Err(ParameterError::NoDeclarations);
    }
    let mut parameters = Vec::new();
    for (index, row) in text.lines().enumerate() {
        let row = row.trim();
        if row.is_empty() || row.starts_with('#') {
            continue;
        }
        let line = index + 1;
        let fields: Vec<_> = row.split(',').map(str::trim).collect();
        if fields.len() != 6 {
            return Err(ParameterError::FieldCount {
                line,
                actual: fields.len(),
            });
        }
        let name = fields[0].to_owned();
        let declaration = declarations
            .iter()
            .find(|d| d.name == name)
            .ok_or_else(|| ParameterError::UnknownName(name.clone()))?;
        let float = |text: &str| {
            text.parse::<f64>()
                .map_err(|source| ParameterError::Float { line, source })
        };
        let integer = |text: &str| {
            text.parse::<i32>()
                .map_err(|source| ParameterError::Integer { line, source })
        };
        let p = Parameter {
            name,
            start: float(fields[1])?,
            min: integer(fields[2])?,
            max: integer(fields[3])?,
            c_end: float(fields[4])?,
            r_end: float(fields[5])?,
        };
        if p.min < declaration.min || p.max > declaration.max {
            return Err(ParameterError::Range(p.name));
        }
        parameters.push(p);
    }
    validate_parameters(&parameters)?;
    Ok(parameters)
}

/// 係数ファイルと保存された実行条件に共通する妥当性を検査する。
pub(super) fn validate_parameters(parameters: &[Parameter]) -> Result<(), ParameterError> {
    if parameters.is_empty() {
        return Err(ParameterError::Empty);
    }
    let mut names = BTreeSet::new();
    for p in parameters {
        if !names.insert(&p.name) {
            return Err(ParameterError::Duplicate(p.name.clone()));
        }
        if !p.start.is_finite()
            || p.min > p.max
            || p.start < f64::from(p.min)
            || p.start > f64::from(p.max)
        {
            return Err(ParameterError::Range(p.name.clone()));
        }
        if !p.c_end.is_finite() || !p.r_end.is_finite() || p.c_end <= 0.0 || p.r_end <= 0.0 {
            return Err(ParameterError::NonPositiveRate(p.name.clone()));
        }
    }
    Ok(())
}
