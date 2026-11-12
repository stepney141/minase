//! ランダム対局で合法手適用と終局の不変条件を検査し、結果を行単位で報告する。

mod game;
mod seed;
mod summary;
mod text;

use minase_core::{RuleCode, Rules, RulesError};
use std::{
    fmt, io,
    num::{NonZeroU32, NonZeroU64},
    time::Instant,
};

pub use game::InvariantFailure;
use game::run_game;
use seed::time_seed;
use summary::{Summary, report_game};
use text::rules_text;

/// 1局を打ち切る手数上限の既定値。
///
/// 2026年8月10日の実測で、代表9規則セット各300局とR1の追加1,000局は打ち切り0局で、
/// 最大手数は1,363手だった。その約3倍を4,096手とする。
/// 上限への到達は異常にせず、終局とは別に打ち切りとして数える。
pub const DEFAULT_MAX_PLY: NonZeroU32 = const { NonZeroU32::new(4096).unwrap() };

/// ランダム対局の検査済み引数。局数と局番号の同時指定は認めない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arguments {
    games: NonZeroU64,
    seed: Option<u64>,
    game: Option<NonZeroU64>,
    max_ply: NonZeroU32,
    verify_all: bool,
    verbose: bool,
}

/// 局数と局番号を同時に指定した。
#[derive(Debug)]
pub struct ArgumentError;

impl fmt::Display for ArgumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("games and game cannot be specified together")
    }
}
impl std::error::Error for ArgumentError {}

impl Arguments {
    /// 排他指定を検査して構築する。局数の省略時は1局、シードの省略時は実行時刻を使う。
    pub fn new(
        games: Option<NonZeroU64>,
        seed: Option<u64>,
        game: Option<NonZeroU64>,
        max_ply: NonZeroU32,
        verify_all: bool,
        verbose: bool,
    ) -> Result<Self, ArgumentError> {
        if games.is_some() && game.is_some() {
            return Err(ArgumentError);
        }
        let games = match games {
            Some(games) => games,
            None => NonZeroU64::MIN,
        };
        Ok(Self {
            games,
            seed,
            game,
            max_ply,
            verify_all,
            verbose,
        })
    }
}

/// ランダム対局を継続できない原因。
#[derive(Debug)]
pub enum Error {
    /// 規則コードの組合せが不正。
    Rules(RulesError),
    /// 現在時刻からシードを作れない。
    Seed(std::time::SystemTimeError),
    /// 対局の不変条件が破られた。
    Invariant(Box<InvariantFailure>),
    /// 1局も集計されていない。
    NoGames,
    /// 報告の行を出力できない。
    Output(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rules(error) => error.fmt(f),
            Self::Seed(error) => write!(
                f,
                "failed to generate a seed from the current time: {error}"
            ),
            Self::Invariant(failure) => failure.fmt(f),
            Self::NoGames => f.write_str("random-play summary has no games"),
            Self::Output(error) => write!(f, "failed to report random-play: {error}"),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Rules(error) => Some(error),
            Self::Seed(error) => Some(error),
            Self::Invariant(failure) => Some(failure.as_ref()),
            Self::Output(error) => Some(error),
            Self::NoGames => None,
        }
    }
}

/// 指定した規則で独立した対局を進め、各局の結果、指定時は全手順、および集計を出力する。
/// 局番号は1起算で、同じ規則、シード、局番号、および手数上限なら全手順を再現する。
pub fn run(
    arguments: Arguments,
    codes: &[RuleCode],
    output: &mut dyn FnMut(&str) -> io::Result<()>,
) -> Result<(), Error> {
    let rules = Rules::from_codes(codes).map_err(Error::Rules)?;
    let base_seed = match arguments.seed {
        Some(seed) => seed,
        None => time_seed().map_err(Error::Seed)?,
    };
    let rules_text = rules_text(codes);
    output(&format!("rules: {rules_text}")).map_err(Error::Output)?;
    output(&format!("seed: {base_seed}")).map_err(Error::Output)?;

    let start = Instant::now();
    let mut summary = Summary::default();
    let mut execute = |game_number| {
        let completed = run_game(
            rules,
            &rules_text,
            base_seed,
            game_number,
            arguments.max_ply,
            arguments.verify_all,
        )
        .map_err(Error::Invariant)?;
        report_game(
            game_number,
            completed,
            arguments.verbose,
            &mut summary,
            output,
        )
        .map_err(Error::Output)
    };
    match arguments.game {
        Some(game_number) => execute(game_number)?,
        None => {
            for game_number in 1..=arguments.games.get() {
                // RangeInclusiveは上限がu64::MAXでも桁あふれせず、先頭は1である。
                let game_number = NonZeroU64::new(game_number).ok_or(Error::NoGames)?;
                execute(game_number)?;
            }
        }
    }
    summary.print(start.elapsed(), output)
}
