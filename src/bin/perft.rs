use std::process;
use std::time::{Duration, Instant};

use clap::{CommandFactory, Parser, error::ErrorKind};
use minase::{Move, MoveGenerator, Position, Rules, Square, parse_sfen};

#[derive(Parser)]
#[command(name = "perft")]
struct Arguments {
    depth: u32,
    #[arg(long)]
    sfen: Option<String>,
    #[arg(long)]
    divide: bool,
}

fn square_text(square: Square) -> String {
    format!("({},{})", square.file(), square.rank())
}

// Divide coordinates are zero-based `(file,rank)`: files increase from the
// leftmost SFEN column and ranks increase from the bottom SFEN row. `+` marks
// promotion. A captured intermediate square is included for two-stage moves.
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

fn print_counts(counts: &[u64]) {
    for (index, count) in counts.iter().enumerate() {
        println!("perft({}): {count}", index + 1);
    }
}

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

/// Counts legal-move paths at every depth from 1 to `counts.len()`, adding the
/// depth-`k` result to `counts[k - 1]`, and restores `position` before
/// returning.
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
        let undo = position.make_move_unchecked(mv, Rules::standard());
        perft(generator, position, deeper);
        position.unmake_move(undo);
    }
}

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
