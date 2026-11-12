//! 診断コマンドの引数を検査し、行ごとにフラッシュして診断結果を返す。

use std::fmt;
use std::num::ParseIntError;

use crate::diagnostics::{bench, perft};

use super::*;

/// 実行前に拒否できる診断コマンドの引数の誤り。
#[derive(Debug)]
enum ArgumentError {
    Missing(&'static str),
    Unknown(String),
    Duplicate(&'static str),
    Integer {
        name: &'static str,
        value: String,
        source: ParseIntError,
    },
    Perft(perft::ArgumentError),
    Bench(bench::ArgumentError),
}

impl fmt::Display for ArgumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(name) => write!(f, "{name} requires a value"),
            Self::Unknown(token) => write!(f, "unknown diagnostic argument '{token}'"),
            Self::Duplicate(name) => write!(f, "duplicate argument {name}"),
            Self::Integer {
                name,
                value,
                source,
            } => write!(f, "invalid {name} '{value}': {source}"),
            Self::Perft(error) => error.fmt(f),
            Self::Bench(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ArgumentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Integer { source, .. } => Some(source),
            Self::Perft(error) => Some(error),
            Self::Bench(error) => Some(error),
            _ => None,
        }
    }
}

fn integer<T: std::str::FromStr<Err = ParseIntError>>(
    name: &'static str,
    value: &str,
) -> Result<T, ArgumentError> {
    value.parse().map_err(|source| ArgumentError::Integer {
        name,
        value: value.to_owned(),
        source,
    })
}

fn parse_perft(tokens: &[&str]) -> Result<perft::Arguments, ArgumentError> {
    let (depth, options) = tokens
        .split_first()
        .ok_or(ArgumentError::Missing("perft depth"))?;
    let depth = integer("perft depth", depth)?;
    let mut divide = false;
    for &option in options {
        if option != "divide" {
            return Err(ArgumentError::Unknown(option.to_owned()));
        }
        if divide {
            return Err(ArgumentError::Duplicate("divide"));
        }
        divide = true;
    }
    perft::Arguments::new(depth, divide).map_err(ArgumentError::Perft)
}

fn parse_bench(tokens: &[&str]) -> Result<bench::Arguments, ArgumentError> {
    let mut depth = bench::DEFAULT_DEPTH;
    let mut threads = search::DEFAULT_THREADS;
    let mut repetitions = std::num::NonZeroUsize::MIN;
    let mut seen = std::collections::HashSet::new();
    let mut tokens = tokens.iter().copied();
    while let Some(token) = tokens.next() {
        let name = match token {
            "depth" => "depth",
            "threads" => "threads",
            "repetitions" => "repetitions",
            _ => return Err(ArgumentError::Unknown(token.to_owned())),
        };
        if !seen.insert(name) {
            return Err(ArgumentError::Duplicate(name));
        }
        let value = tokens.next().ok_or(ArgumentError::Missing(name))?;
        match name {
            "depth" => depth = integer(name, value)?,
            "threads" => threads = integer(name, value)?,
            _ => repetitions = integer(name, value)?,
        }
    }
    bench::Arguments::new(depth, threads, repetitions).map_err(ArgumentError::Bench)
}

/// 出力先は行を受け取るたびに読み手へ届ける。
fn write_line(output: &mut dyn Write, line: &str) -> io::Result<()> {
    writeln!(output, "info string {line}")?;
    output.flush()
}

/// 成功時だけ終端行を出し、実行時の失敗は原因の型を保持してセッションへ返す。
fn finish<E: std::error::Error + Send + Sync + 'static>(
    result: Result<(), E>,
    output: &mut dyn Write,
) -> io::Result<()> {
    match result {
        Ok(()) => write_line(output, "end"),
        Err(error) => {
            write_error(output, &error.to_string())?;
            output.flush()?;
            Err(io::Error::other(error))
        }
    }
}

impl UsiProtocol {
    /// 現局面を変更せず、適用中の着手規則でperftを実行する。
    pub(super) fn handle_perft(&self, tokens: &[&str], output: &mut dyn Write) -> io::Result<()> {
        let arguments = match parse_perft(tokens) {
            Ok(arguments) => arguments,
            Err(error) => return write_error(output, &error.to_string()),
        };
        let engine = self.engine()?;
        if engine.lifecycle() == EngineLifecycle::AwaitingStart {
            return write_error(output, "perft requires an active or finished game");
        }
        let result = perft::run(
            arguments,
            engine.game().position(),
            engine.active_rules().moves,
            &mut |line| write_line(output, line),
        );
        finish(result, output)
    }

    /// セッションの探索器や対局に触れず、専用のαβ探索で測定する。
    pub(super) fn handle_bench(&self, tokens: &[&str], output: &mut dyn Write) -> io::Result<()> {
        let arguments = match parse_bench(tokens) {
            Ok(arguments) => arguments,
            Err(error) => return write_error(output, &error.to_string()),
        };
        #[cfg(feature = "tuning")]
        TUNING_LOCKED.store(true, Ordering::Relaxed);
        let result = bench::run(arguments, &mut |line| write_line(output, line));
        finish(result, output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::{NonZeroU32, NonZeroUsize};

    #[test]
    fn bench_defaults_and_option_order_follow_the_contract() {
        let defaults = bench::Arguments::new(
            NonZeroU32::new(3).unwrap(),
            NonZeroUsize::MIN,
            NonZeroUsize::MIN,
        )
        .unwrap();
        assert_eq!(parse_bench(&[]).unwrap(), defaults);
        assert_eq!(
            parse_bench(&["threads", "1", "depth", "3", "repetitions", "1"]).unwrap(),
            defaults
        );
    }

    #[test]
    fn depth_accepts_only_one_through_max_ply() {
        for depth in [1, search::MAX_PLY] {
            assert!(parse_bench(&["depth", &depth.to_string()]).is_ok());
        }
        for depth in [0, search::MAX_PLY + 1] {
            assert!(parse_bench(&["depth", &depth.to_string()]).is_err());
        }
    }

    #[test]
    fn arguments_accept_thread_and_repetition_boundaries() {
        for (threads, repetitions) in [(1, 1), (256, 2), (1, usize::MAX)] {
            let expected = bench::Arguments::new(
                bench::DEFAULT_DEPTH,
                NonZeroUsize::new(threads).unwrap(),
                NonZeroUsize::new(repetitions).unwrap(),
            )
            .unwrap();
            assert_eq!(
                parse_bench(&[
                    "threads",
                    &threads.to_string(),
                    "repetitions",
                    &repetitions.to_string()
                ])
                .unwrap(),
                expected
            );
        }
        assert!(parse_perft(&[&u32::MAX.to_string()]).is_ok());
    }

    #[test]
    fn arguments_reject_invalid_threads_and_repetitions() {
        for value in ["0", "257", "not-a-number"] {
            assert!(parse_bench(&["threads", value]).is_err());
        }
        for value in ["0", "not-a-number"] {
            assert!(parse_bench(&["repetitions", value]).is_err());
        }
    }

    /// 診断の失敗は終端行を出さず、フラッシュ済みのエラー行と型つきの原因を返す。
    #[test]
    fn runtime_failure_returns_its_typed_source() {
        let mut output = Vec::new();
        let error = UsiProtocol::new()
            .run(
                &mut io::Cursor::new(format!("bench depth 1 repetitions {}\nusi\n", usize::MAX)),
                &mut output,
            )
            .unwrap_err();
        assert!(matches!(
            error.get_ref().unwrap().downcast_ref::<bench::Error>(),
            Some(bench::Error::Allocation(_))
        ));
        let output = String::from_utf8(output).unwrap();
        assert_eq!(output.lines().count(), 1);
        assert!(
            output.starts_with("info string error: bench allocation failed: "),
            "{output}"
        );
    }
}
