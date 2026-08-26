//! 合法手生成の検証用perftコマンド。深さごとの経路数を数える。

use std::process;
use std::time::{Duration, Instant};

use clap::{CommandFactory, Parser, error::ErrorKind};
use minase::{Move, MoveGenerator, Position, Square, parse_sfen};

/// グローバルアロケータ。benchの実測（docs/plans/search.md 実施状況）に基づきmimallocを使う。
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// perftコマンドの引数。
#[derive(Parser)]
#[command(name = "perft")]
struct Arguments {
    /// 数える最大深さ。
    depth: u32,
    /// 開始局面のSFEN。省略時は初期局面。
    #[arg(long)]
    sfen: Option<String>,
    /// ルート合法手ごとの内訳を出力するかどうか。
    #[arg(long)]
    divide: bool,
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

/// ルート合法手ごとの末端ノード数を出力しながら深さ別合計を数える。
fn run_divide(
    generator: &MoveGenerator,
    position: &Position,
    depth: u32,
) -> Result<Vec<u64>, minase::IllegalMove> {
    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    let mut counts = vec![0; depth as usize];
    counts[0] = moves.len() as u64;

    for mv in moves {
        let mut child = position.clone();
        child.try_make_move(mv, generator)?;
        let mut child_counts = vec![0; (depth - 1) as usize];
        perft(generator, &mut child, &mut child_counts);
        let nodes = child_counts.last().copied().unwrap_or(1);
        println!("{}: {nodes}", move_text(mv));
        for (count, child_count) in counts[1..].iter_mut().zip(&child_counts) {
            *count += child_count;
        }
    }

    Ok(counts)
}

/// 深さごとの経路数を出力する。
fn print_counts(counts: &[u64]) {
    for (index, count) in counts.iter().enumerate() {
        println!("perft({}): {count}", index + 1);
    }
}

/// 総ノード数・経過時間・秒間ノード数を出力する。
fn print_summary(total: u64, elapsed: Duration) {
    let seconds = elapsed.as_secs_f64();
    let nodes_per_second = total as f64 / seconds;

    println!("total nodes: {total}");
    if seconds < 0.1 {
        println!("elapsed: {:.3} ms", seconds * 1e3);
    } else {
        println!("elapsed: {seconds:.6} s");
    }
    println!("nodes/second: {nodes_per_second:.0}");
}

/// 深さ1から`counts.len()`までの合法手経路数を数える。
///
/// 深さkの結果を`counts[k - 1]`へ加算し、返る前に`position`を復元する。
fn perft(generator: &MoveGenerator, position: &mut Position, counts: &mut [u64]) {
    let Some((current, deeper)) = counts.split_first_mut() else {
        return;
    };

    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    *current += moves.len() as u64;
    if deeper.is_empty() {
        return;
    }

    for mv in moves {
        let mut child = position.clone();
        child
            .try_make_move(mv, generator)
            .expect("generated moves are legal");
        perft(generator, &mut child, deeper);
    }
}

/// 引数を検証し、perftまたはdivideを実行して集計を出力する。
fn main() {
    let arguments = Arguments::parse();
    if arguments.divide && arguments.depth == 0 {
        Arguments::command()
            .error(
                ErrorKind::ValueValidation,
                "--divide requires <DEPTH> to be at least 1",
            )
            .exit();
    }
    let mut position = match arguments.sfen {
        Some(sfen) => match parse_sfen(&sfen) {
            Ok(position) => position,
            Err(error) => {
                eprintln!("failed to parse SFEN: {error}");
                process::exit(1);
            }
        },
        None => Position::initial(),
    };
    let generator = MoveGenerator::standard();
    let start = Instant::now();
    let counts = if arguments.divide {
        match run_divide(&generator, &position, arguments.depth) {
            Ok(counts) => counts,
            Err(error) => {
                eprintln!("failed to apply a generated root move: {error}");
                process::exit(1);
            }
        }
    } else {
        let mut counts = vec![0; arguments.depth as usize];
        perft(&generator, &mut position, &mut counts);
        counts
    };
    let elapsed = start.elapsed();
    print_counts(&counts);
    print_summary(counts.last().copied().unwrap_or(1), elapsed);
}
