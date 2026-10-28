use minase_core::{Game, GameStatus, Rules};

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn main() {
    let games: usize = std::env::args().nth(1).map(|s| s.parse().unwrap()).unwrap_or(300);
    let max_ply: usize = std::env::args().nth(2).map(|s| s.parse().unwrap()).unwrap_or(400);
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut counts: Vec<usize> = Vec::new();
    let mut by_ply: Vec<(usize, usize)> = vec![(0, 0); max_ply / 50 + 1];
    let mut total_moves = 0usize;
    let mut promote_moves = 0usize;
    let mut lion_capture_moves = 0usize;
    for _ in 0..games {
        let mut game = Game::new(Rules::ENGINE_DEFAULT);
        for ply in 0..max_ply {
            let moves = game.legal_moves();
            if moves.is_empty() {
                break;
            }
            counts.push(moves.len());
            let bucket = &mut by_ply[ply / 50];
            bucket.0 += moves.len();
            bucket.1 += 1;
            total_moves += moves.len();
            promote_moves += moves.iter().filter(|m| m.promote).count();
            let position = game.position();
            let opponent = position.side_to_move().opposite();
            let lions = position.pieces_of_kind(opponent, minase_core::PieceKind::Lion);
            lion_capture_moves += moves
                .iter()
                .filter(|m| lions.contains(m.to) || m.mid.is_some_and(|mid| lions.contains(mid)))
                .count();
            let mv = moves[(next(&mut state) % moves.len() as u64) as usize];
            match game.play(mv) {
                Ok(GameStatus::Finished(_)) => break,
                Ok(_) => {}
                Err(e) => panic!("{e}"),
            }
        }
    }
    counts.sort_unstable();
    let n = counts.len();
    let pct = |p: f64| counts[((p * n as f64).ceil() as usize).clamp(1, n) - 1];
    println!("positions {n}");
    println!("mean {:.1}", total_moves as f64 / n as f64);
    println!("median {} p90 {} p99 {} max {}", pct(0.5), pct(0.9), pct(0.99), counts[n - 1]);
    println!("promote share {:.3}", promote_moves as f64 / total_moves as f64);
    println!("lion-capture share {:.4}", lion_capture_moves as f64 / total_moves as f64);
    for (i, (sum, cnt)) in by_ply.iter().enumerate() {
        if *cnt > 0 {
            println!("ply {}-{}: mean {:.1} (n={})", i * 50, i * 50 + 49, *sum as f64 / *cnt as f64, cnt);
        }
    }
}
