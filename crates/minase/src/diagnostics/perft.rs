//! 合法手生成器と局面の複製によって、深さごとの経路数を数える。

use std::fmt;
use std::io;
use std::time::{Duration, Instant};

use minase_core::{IllegalMove, Move, MoveGenerator, MoveRules, Position, Square};

/// perftの検査済み引数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arguments {
    depth: u32,
    divide: bool,
}

/// perftの引数の誤り。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgumentError {
    /// 深さ0では根の着手ごとの内訳を出せない。
    DivideAtZero,
}

impl fmt::Display for ArgumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("divide requires depth to be at least 1")
    }
}

impl std::error::Error for ArgumentError {}

impl Arguments {
    /// 深さ0とdivideの併用を拒否し、引数を構築する。
    pub fn new(depth: u32, divide: bool) -> Result<Self, ArgumentError> {
        if divide && depth == 0 {
            return Err(ArgumentError::DivideAtZero);
        }
        Ok(Self { depth, divide })
    }
}

/// 計数または行の出力に失敗した原因。
#[derive(Debug)]
pub enum Error {
    /// 合法手生成器が返した手を適用できなかった。
    GeneratedMove(IllegalMove),
    /// 経路数がu64に収まらなかった。
    CountOverflow,
    /// 深さが実行環境の配列長に収まらなかった。
    DepthOverflow(std::num::TryFromIntError),
    /// 深さごとの計数領域を確保できなかった。
    Allocation(std::collections::TryReserveError),
    /// 出力先への書き込みに失敗した。
    Output(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GeneratedMove(error) => write!(
                f,
                "perft invariant failure: generated move cannot be applied: {error}"
            ),
            Self::CountOverflow => f.write_str("perft node count overflow"),
            Self::DepthOverflow(error) => {
                write!(f, "perft depth exceeds addressable memory: {error}")
            }
            Self::Allocation(error) => write!(f, "perft allocation failed: {error}"),
            Self::Output(error) => write!(f, "perft output failed: {error}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::GeneratedMove(error) => Some(error),
            Self::DepthOverflow(error) => Some(error),
            Self::Allocation(error) => Some(error),
            Self::Output(error) => Some(error),
            Self::CountOverflow => None,
        }
    }
}

/// 升を0始まりの`(筋,段)`表記へ変換する。
fn square_text(square: Square) -> String {
    format!("({},{})", square.file(), square.rank())
}

/// divide出力用に指し手を文字列へ変換する。
///
/// 座標は0始まりの`(筋,段)`で、筋はSFENの左端列から、段はSFENの最下段から
/// 増える。`+`は成りを表し、2段階移動では捕獲した中間升も含める。
fn move_text(mv: Move) -> String {
    if let Some(mid) = mv.mid {
        format!(
            "double {}->{}->{}{}",
            square_text(mv.from),
            square_text(mid),
            square_text(mv.to),
            if mv.promote { "+" } else { "" }
        )
    } else {
        format!(
            "move {}->{}{}",
            square_text(mv.from),
            square_text(mv.to),
            if mv.promote { "+" } else { "" }
        )
    }
}

/// 深さごとの計数領域を確保する。
fn empty_counts(depth: usize) -> Result<Vec<u64>, Error> {
    let mut counts = Vec::new();
    counts.try_reserve_exact(depth).map_err(Error::Allocation)?;
    counts.resize(depth, 0);
    Ok(counts)
}

/// 根の合法手ごとの末端ノード数を出力しながら深さ別合計を数える。
fn run_divide(
    generator: &MoveGenerator,
    position: &Position,
    counts: &mut [u64],
    output: &mut dyn FnMut(&str) -> io::Result<()>,
) -> Result<(), Error> {
    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    for mv in moves {
        let mut child = position.clone();
        child
            .try_make_move(mv, generator)
            .map_err(Error::GeneratedMove)?;
        let mut child_counts = empty_counts(counts.len())?;
        // 根の1手を深さ1の経路として数え、残りの深さを子局面で数える。
        if let Some((first, deeper)) = child_counts.split_first_mut() {
            *first = 1;
            perft(generator, &child, deeper)?;
        }
        let nodes = leaf_count(&child_counts);
        output(&format!("{}: {nodes}", move_text(mv))).map_err(Error::Output)?;
        for (count, child_count) in counts.iter_mut().zip(&child_counts) {
            *count = count
                .checked_add(*child_count)
                .ok_or(Error::CountOverflow)?;
        }
    }
    Ok(())
}

/// 深さ0では根自身を1経路として数える。
fn leaf_count(counts: &[u64]) -> u64 {
    match counts.last() {
        Some(&count) => count,
        None => 1,
    }
}

/// 総ノード数・経過時間・秒間ノード数を出力する。
fn print_summary(
    total: u64,
    elapsed: Duration,
    output: &mut dyn FnMut(&str) -> io::Result<()>,
) -> Result<(), Error> {
    let seconds = elapsed.as_secs_f64();
    let nodes_per_second = total as f64 / seconds;
    output(&format!("total nodes: {total}")).map_err(Error::Output)?;
    if seconds < 0.1 {
        output(&format!("elapsed: {:.3} ms", seconds * 1e3)).map_err(Error::Output)?;
    } else {
        output(&format!("elapsed: {seconds:.6} s")).map_err(Error::Output)?;
    }
    output(&format!("nodes/second: {nodes_per_second:.0}")).map_err(Error::Output)
}

/// 深さ1から`counts.len()`までの合法手経路数を加算する。
fn perft(generator: &MoveGenerator, position: &Position, counts: &mut [u64]) -> Result<(), Error> {
    let Some((current, deeper)) = counts.split_first_mut() else {
        return Ok(());
    };
    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    *current = current
        .checked_add(moves.len() as u64)
        .ok_or(Error::CountOverflow)?;
    if deeper.is_empty() {
        return Ok(());
    }
    for mv in moves {
        let mut child = position.clone();
        child
            .try_make_move(mv, generator)
            .map_err(Error::GeneratedMove)?;
        perft(generator, &child, deeper)?;
    }
    Ok(())
}

/// 指定局面から経路を数え、接頭辞と終端行を付けずに結果を出力する。
/// 対局の裁定は行わず、終局後でも着手の規則だけに従って計数する。
pub fn run(
    arguments: Arguments,
    position: &Position,
    rules: MoveRules,
    output: &mut dyn FnMut(&str) -> io::Result<()>,
) -> Result<(), Error> {
    let generator = MoveGenerator::new(rules);
    let start = Instant::now();
    let depth = usize::try_from(arguments.depth).map_err(Error::DepthOverflow)?;
    let mut counts = empty_counts(depth)?;
    if arguments.divide {
        run_divide(&generator, position, &mut counts, output)?;
    } else {
        perft(&generator, position, &mut counts)?;
    }
    let elapsed = start.elapsed();
    for (index, count) in counts.iter().enumerate() {
        output(&format!("perft({}): {count}", index + 1)).map_err(Error::Output)?;
    }
    print_summary(leaf_count(&counts), elapsed, output)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 非公開フィールドを通じて、divideの前提を構築時に保証する。
    #[test]
    fn divide_requires_positive_depth() {
        assert_eq!(
            Arguments::new(0, true).unwrap_err(),
            ArgumentError::DivideAtZero
        );
        assert!(Arguments::new(0, false).is_ok());
    }

    /// 呼び出し元の出力エラーを、文字列へ失わずに返す。
    #[test]
    fn output_failures_keep_the_io_error() {
        let error = run(
            Arguments::new(1, false).unwrap(),
            &Position::initial(),
            minase_core::Rules::JCSA.moves,
            &mut |_| Err(io::Error::from(io::ErrorKind::BrokenPipe)),
        )
        .unwrap_err();
        assert!(matches!(error, Error::Output(error) if error.kind() == io::ErrorKind::BrokenPipe));
    }
}
