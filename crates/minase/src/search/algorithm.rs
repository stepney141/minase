//! 起動時に選ぶ探索方式。

use core::fmt;
use core::str::FromStr;

/// 起動引数`--search`で選ぶ探索方式。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SearchAlgorithm {
    /// 評価関数と静止探索を使うαβ探索。
    AlphaBeta,
}

impl SearchAlgorithm {
    /// 起動引数で使う方式名を返す。
    const fn name(self) -> &'static str {
        match self {
            Self::AlphaBeta => "alphabeta",
        }
    }
}

impl fmt::Display for SearchAlgorithm {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// 方式名として解釈できない文字列。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct UnknownSearchAlgorithm(String);

impl fmt::Display for UnknownSearchAlgorithm {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unknown search algorithm '{}' (expected alphabeta)",
            self.0
        )
    }
}

impl std::error::Error for UnknownSearchAlgorithm {}

impl FromStr for SearchAlgorithm {
    type Err = UnknownSearchAlgorithm;

    /// 方式名を大文字小文字を区別して解釈する。
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input {
            "alphabeta" => Ok(Self::AlphaBeta),
            _ => Err(UnknownSearchAlgorithm(input.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // alphazero.md「共存の構造」: 方式名は起動引数の文字列と相互に変換できる。
    #[test]
    fn search_algorithm_round_trips_through_its_name() {
        let algorithm: SearchAlgorithm = "alphabeta".parse().unwrap();
        assert_eq!(algorithm, SearchAlgorithm::AlphaBeta);
        assert_eq!(algorithm.to_string(), "alphabeta");
    }

    #[test]
    fn search_algorithm_rejects_unknown_names() {
        for input in ["", "AlphaBeta", "alpha-beta", "alphabeta "] {
            assert_eq!(
                input.parse::<SearchAlgorithm>(),
                Err(UnknownSearchAlgorithm(input.to_owned()))
            );
        }
    }
}
