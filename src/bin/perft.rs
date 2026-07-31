use std::process;
use std::time::{Duration, Instant};

use clap::{CommandFactory, Parser, error::ErrorKind};
use minase::{Move, MoveGenerator, Position, Square, parse_sfen, perft};

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
