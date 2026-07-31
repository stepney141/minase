use std::env;
use std::ffi::OsStr;
use std::process;
use std::time::{Duration, Instant};

use minase::{Move, MoveGenerator, Position, Square, parse_sfen, perft};

const USAGE: &str = "Usage: perft <depth> [--sfen \"<sfen>\"] [--divide]";

struct Arguments {
    depth: u32,
    sfen: Option<String>,
    divide: bool,
}

fn parse_arguments() -> Result<Arguments, ()> {
    let mut arguments = env::args_os().skip(1);
    let depth = arguments
        .next()
        .and_then(|argument| argument.into_string().ok())
        .and_then(|argument| argument.parse().ok())
        .ok_or(())?;
    let mut sfen = None;
    let mut divide = false;

    while let Some(argument) = arguments.next() {
        if argument == OsStr::new("--sfen") {
            if sfen.is_some() {
                return Err(());
            }
            sfen = Some(
                arguments
                    .next()
                    .and_then(|value| value.into_string().ok())
                    .ok_or(())?,
            );
        } else if argument == OsStr::new("--divide") {
            if divide {
                return Err(());
            }
            divide = true;
        } else {
            return Err(());
        }
    }

    if divide && depth == 0 {
        return Err(());
    }
    Ok(Arguments {
        depth,
        sfen,
        divide,
    })
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
) -> Result<u64, minase::IllegalMove> {
    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    let mut total = 0;

    for mv in moves {
        let mut child = position.clone();
        child.try_make_move(mv, generator)?;
        let nodes = perft(generator, &mut child, depth - 1);
        println!("{}: {nodes}", move_text(mv));
        total += nodes;
    }

    Ok(total)
}

fn print_summary(total: u64, elapsed: Duration) {
    let seconds = elapsed.as_secs_f64();
    let nodes_per_second = total as f64 / seconds;

    println!("total nodes: {total}");
    println!("elapsed: {seconds:.6} s");
    println!("nodes/second: {nodes_per_second:.0}");
}

fn main() {
    let arguments = match parse_arguments() {
        Ok(arguments) => arguments,
        Err(()) => {
            eprintln!("{USAGE}");
            process::exit(2);
        }
    };
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
    let total = if arguments.divide {
        match run_divide(&generator, &position, arguments.depth) {
            Ok(total) => total,
            Err(error) => {
                eprintln!("failed to apply a generated root move: {error}");
                process::exit(1);
            }
        }
    } else {
        perft(&generator, &mut position, arguments.depth)
    };
    print_summary(total, start.elapsed());
}
