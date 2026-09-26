//! 完了した調整記録を検査し、係数表の既定値だけを書き換える。

use super::{
    model::{Settings, validate_settings},
    params::{ParameterError, validate_parameters},
    storage::{read_iterations, read_manifest, validate_chain},
};
use minase::harness::lock_run_directory;
use std::{
    collections::BTreeSet,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    ops::Range,
    path::Path,
};

/// 適用前の検査失敗と、改名が確定した後の失敗を区別する。
#[derive(Debug)]
pub(super) enum ApplyError {
    Lock(io::Error),
    Read(io::Error),
    InvalidSettings(io::Error),
    Incomplete {
        completed: u64,
        total: u64,
    },
    Chain(io::Error),
    InvocationCount(usize),
    MissingEnd,
    InvalidLine {
        line: usize,
        text: String,
    },
    MissingParameter(String),
    DuplicateParameter(String),
    RangeMismatch(String),
    StartMismatch {
        name: String,
        default: i32,
        start: f64,
    },
    IntegerOutOfRange {
        name: String,
        value: f64,
    },
    RewriteMismatch(String),
    Write(io::Error),
    AfterWrite(io::Error),
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lock(e) => write!(
                f,
                "cannot lock run directory: {}",
                e.to_string().escape_debug()
            ),
            Self::Read(e) => write!(f, "cannot read input: {}", e.to_string().escape_debug()),
            Self::InvalidSettings(e) => write!(
                f,
                "invalid session settings: {}",
                e.to_string().escape_debug()
            ),
            Self::Incomplete { completed, total } => {
                write!(f, "incomplete session: {completed} of {total} iterations")
            }
            Self::Chain(e) => write!(
                f,
                "invalid iteration chain: {}",
                e.to_string().escape_debug()
            ),
            Self::InvocationCount(count) => {
                write!(f, "expected one parameters invocation, found {count}")
            }
            Self::MissingEnd => write!(f, "parameters invocation has no closing line"),
            Self::InvalidLine { line, text } => {
                write!(f, "unrecognized table line {line}: {text:?}")
            }
            Self::MissingParameter(name) => write!(f, "parameter missing from table: {name:?}"),
            Self::DuplicateParameter(name) => write!(f, "duplicate table parameter: {name:?}"),
            Self::RangeMismatch(name) => write!(f, "session range exceeds table range: {name:?}"),
            Self::StartMismatch {
                name,
                default,
                start,
            } => write!(
                f,
                "table default differs from session start for {name:?}: default={default} start={start}"
            ),
            Self::IntegerOutOfRange { name, value } => write!(
                f,
                "rounded value outside parameter range for {name:?}: {value}"
            ),
            Self::RewriteMismatch(name) => {
                write!(f, "rewritten default does not match for {name:?}")
            }
            Self::Write(e) => write!(
                f,
                "cannot replace source file: {}",
                e.to_string().escape_debug()
            ),
            Self::AfterWrite(e) => write!(
                f,
                "source file was replaced, but a subsequent operation failed: {}",
                e.to_string().escape_debug()
            ),
        }
    }
}

impl std::error::Error for ApplyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lock(e)
            | Self::Read(e)
            | Self::InvalidSettings(e)
            | Self::Chain(e)
            | Self::Write(e)
            | Self::AfterWrite(e) => Some(e),
            _ => None,
        }
    }
}

fn invalid_parameters(error: ParameterError) -> ApplyError {
    ApplyError::InvalidSettings(io::Error::new(io::ErrorKind::InvalidData, error))
}

/// 解析済み宣言。既定値のバイト範囲は解析器だけが構築する。
#[derive(Debug)]
pub(super) struct Entry {
    pub name: String,
    pub default: i32,
    pub min: i32,
    pub max: i32,
    literal: Range<usize>,
}

fn identifier(text: &str) -> bool {
    let mut bytes = text.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn decimal(text: &str) -> Option<i32> {
    let digits = text.trim_start_matches(['+', '-']);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn declaration(line: &str, offset: usize) -> Option<Entry> {
    let row = line.trim_start();
    let (name, rest) = row.split_once('(')?;
    let (accessor, rest) = rest.split_once("):")?;
    if !identifier(name) || !identifier(accessor) {
        return None;
    }
    let rest = rest.trim_start();
    let begin = offset + line.len() - rest.len();
    let (default, rest) = rest.split_once(',')?;
    let (min, rest) = rest.trim_start().split_once(',')?;
    let (max, tail) = rest.trim_start().split_once(';')?;
    if !tail.trim().is_empty() {
        return None;
    }
    Some(Entry {
        name: name.to_owned(),
        default: decimal(default)?,
        min: decimal(min)?,
        max: decimal(max)?,
        literal: begin..begin + default.len(),
    })
}

/// 指定された行文法を検査し、呼び出しの外側には触れない。
pub(super) fn parse_table(text: &str) -> Result<Vec<Entry>, ApplyError> {
    let count = text
        .lines()
        .filter(|line| line.trim() == "parameters! {")
        .count();
    if count != 1 {
        return Err(ApplyError::InvocationCount(count));
    }
    let mut inside = false;
    let mut offset = 0;
    let mut entries = Vec::new();
    let mut names = BTreeSet::new();
    for (index, line) in text.split_inclusive('\n').enumerate() {
        if !inside {
            inside = line.trim() == "parameters! {";
        } else if line.trim() == "}" {
            return Ok(entries);
        } else if !line.trim().is_empty() && !line.trim_start().starts_with("///") {
            let entry = declaration(line, offset).ok_or_else(|| ApplyError::InvalidLine {
                line: index + 1,
                text: line.trim_end_matches(['\r', '\n']).to_owned(),
            })?;
            if !names.insert(entry.name.clone()) {
                return Err(ApplyError::DuplicateParameter(entry.name));
            }
            entries.push(entry);
        }
        offset += line.len();
    }
    Err(ApplyError::MissingEnd)
}

/// 丸める前に表との整合を検査し、各係数の整数と報告を作る。
fn replacements(
    entries: &[Entry],
    settings: &Settings,
    theta: &[f64],
) -> Result<Vec<(String, i32)>, ApplyError> {
    let matched = settings
        .parameters
        .iter()
        .map(|p| {
            entries
                .iter()
                .find(|e| e.name == p.name)
                .map(|entry| (p, entry))
                .ok_or_else(|| ApplyError::MissingParameter(p.name.clone()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    for (p, entry) in &matched {
        if entry.min > p.min || entry.max < p.max {
            return Err(ApplyError::RangeMismatch(p.name.clone()));
        }
        if f64::from(entry.default) != p.start {
            return Err(ApplyError::StartMismatch {
                name: p.name.clone(),
                default: entry.default,
                start: p.start,
            });
        }
    }
    matched
        .iter()
        .zip(theta)
        .map(|((p, entry), value)| {
            let integer = value.round();
            if !integer.is_finite()
                || integer < f64::from(p.min)
                || integer > f64::from(p.max)
                || integer < f64::from(entry.min)
                || integer > f64::from(entry.max)
            {
                return Err(ApplyError::IntegerOutOfRange {
                    name: p.name.clone(),
                    value: integer,
                });
            }
            Ok((p.name.clone(), integer as i32))
        })
        .collect()
}

fn rewrite(text: &str, entries: &[Entry], values: &[(String, i32)]) -> Result<String, ApplyError> {
    let mut output = String::new();
    let mut previous = 0;
    for entry in entries {
        if let Some((_, value)) = values.iter().find(|(name, _)| *name == entry.name) {
            output.push_str(
                text.get(previous..entry.literal.start)
                    .ok_or_else(|| ApplyError::RewriteMismatch(entry.name.clone()))?,
            );
            output.push_str(&value.to_string());
            previous = entry.literal.end;
        }
    }
    output.push_str(
        text.get(previous..)
            .ok_or_else(|| ApplyError::RewriteMismatch(String::new()))?,
    );
    let parsed = parse_table(&output)?;
    for (name, value) in values {
        if !parsed
            .iter()
            .any(|entry| entry.name == *name && entry.default == *value)
        {
            return Err(ApplyError::RewriteMismatch(name.clone()));
        }
    }
    Ok(output)
}

fn report(
    entries: &[Entry],
    settings: &Settings,
    theta: &[f64],
    values: &[(String, i32)],
    source: &Path,
) -> String {
    let mut output = String::new();
    let mut changed = 0;
    let mut unchanged = 0;
    for ((p, final_value), (_, integer)) in settings.parameters.iter().zip(theta).zip(values) {
        let status = if f64::from(*integer) == p.start {
            unchanged += 1;
            "unchanged"
        } else {
            changed += 1;
            "changed"
        };
        output.push_str(&format!(
            "{}: start={} final={final_value} integer={integer} {status}\n",
            p.name, p.start
        ));
    }
    for entry in entries {
        if !values.iter().any(|(name, _)| *name == entry.name) {
            output.push_str(&format!("{}: not in session\n", entry.name));
        }
    }
    output.push_str(&format!(
        "changed={changed} unchanged={unchanged} source={}\n",
        source.display()
    ));
    output
}

/// 同じディレクトリで書き込みを完了してから改名し、確定後の失敗を区別する。
fn replace_source(source: &Path, text: &str) -> Result<(), ApplyError> {
    let source = fs::canonicalize(source).map_err(ApplyError::Write)?;
    let parent = source.parent().ok_or_else(|| {
        ApplyError::Write(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source has no parent directory",
        ))
    })?;
    let name = source.file_name().ok_or_else(|| {
        ApplyError::Write(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source has no filename",
        ))
    })?;
    let mut temporary_name = std::ffi::OsString::from(".");
    temporary_name.push(name);
    temporary_name.push(format!(".{}.tmp", std::process::id()));
    let temporary = parent.join(temporary_name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(ApplyError::Write)?;
    let result = (|| -> io::Result<()> {
        file.set_permissions(fs::metadata(&source)?.permissions())?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &source)
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(ApplyError::Write(error));
    }
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(ApplyError::AfterWrite)
}

/// 完了記録から最終θを復元して適用し、標準出力用の報告を返す。
pub(super) fn apply(run_dir: &Path, source: &Path) -> Result<String, ApplyError> {
    let _lock = lock_run_directory(run_dir).map_err(ApplyError::Lock)?;
    let manifest = read_manifest(run_dir).map_err(ApplyError::Read)?;
    let settings = &manifest.settings;
    validate_parameters(&settings.parameters).map_err(invalid_parameters)?;
    validate_settings(settings).map_err(ApplyError::InvalidSettings)?;
    let records = read_iterations(run_dir, settings.iterations).map_err(ApplyError::Chain)?;
    // 読み込みは番号の範囲と重複を検査済みなので、個数の一致で全番号を確認できる。
    if records.len() as u64 != settings.iterations {
        return Err(ApplyError::Incomplete {
            completed: records.len() as u64,
            total: settings.iterations,
        });
    }
    let theta = validate_chain(settings, &records).map_err(ApplyError::Chain)?;
    let text = fs::read_to_string(source).map_err(ApplyError::Read)?;
    let entries = parse_table(&text)?;
    let values = replacements(&entries, settings, &theta)?;
    let rewritten = rewrite(&text, &entries, &values)?;
    let report = report(&entries, settings, &theta, &values, source);
    replace_source(source, &rewritten)?;
    Ok(report)
}
