# Minase

Minase is a legal-move generation library and playing engine for chu shogi, written in Rust.
It follows the rules of the Japan Chu Shogi Federation by default, and the local rules used by lishogi, HaChu, and other sources can be selected through rule codes.
The supported rules and their sources are documented in [RULES.md](RULES.md) (Japanese).

## Overview

- Rules. The implementation covers the lion's two-step move (igui and jitto), the sente-lion restriction, protection of the lion (including indirect protection through a discovered attack), promotion rights, repetition, and bare-king endings. Local rules are selected with rule codes (groups L, P, R, and E) or with the presets `engine-default` and `lishogi`.
- Protocols. The engine speaks USI (including the lishogi extensions) and CECP (XBoard).
- Search and evaluation. The search uses alpha-beta, quiescence search, iterative deepening, a transposition table, and Lazy SMP for parallel search. The evaluation is a piece-square table interpolated by game phase.
- Verification. The repository includes unit tests, perft, replay checks against real game records, and an SPRT self-play harness.
- `unsafe` code is forbidden across the crate through the lint settings in Cargo.toml.

## Requirements

Rust 1.88 or later (2024 edition).

## Build

```bash
git clone https://github.com/stepney141/minase.git
cd minase
cargo build --release
```

The engine binary is written to `target/release/minase`.

## Usage

### Running the engine

Both `--protocol` and `--rules` are required.

```bash
# USI, standard rules
./target/release/minase --protocol usi --rules engine-default

# USI, lishogi rules
./target/release/minase --protocol usi --rules lishogi

# CECP, explicit rule codes
./target/release/minase --protocol cecp --rules L0,P0,R1,E0
```

`--rules` accepts one of the following.

- `engine-default`: the standard rules (L0,P0,R1,E0)
- `lishogi`: the rules used on lishogi (L1,L2,P0,P3,R1,E1,E3)
- A comma-separated list of rule codes. The list must contain exactly one code from each of the groups L, P, R, and E (RULES.md, Article 33).

### GUIs and bots

In a USI-compatible GUI, register the engine path with the arguments `--protocol usi --rules engine-default`.
A development GUI lives in the separate repository minase-gui, and the setup for running the engine as a bot on lishogi lives in minase-lishogi-bot.

## Using Minase as a library

Add the crate to Cargo.toml to use position handling and legal-move generation from Rust.

```rust
use minase::{Game, MoveGenerator, Position};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initial position
    let pos = Position::initial();

    // Legal-move generation
    let movegen = MoveGenerator::standard();
    let mut legal_moves = Vec::new();
    movegen.generate_moves(&pos, &mut legal_moves);
    println!("Legal moves in the initial position: {}", legal_moves.len());

    // Playing a game and adjudicating the result
    let mut game = Game::with_default_rules();
    if let Some(&mv) = legal_moves.first() {
        let status = game.play(mv)?;
        println!("Game status after the move: {status:?}");
    }

    Ok(())
}
```

## Bundled tools

The following binaries are included for development and measurement.

| Binary | Purpose |
|---|---|
| `minase` | The engine |
| `match_runner` | SPRT harness for self-play and matches against external engines |
| `match_report` | Recomputes Elo and statistics from saved match records |
| `selfplay_gen` | Generates self-play data for evaluation training |
| `perft` | Verifies and times legal-move generation |
| `random_play` | Checks invariants with random play |
| `bench` | Benchmarks search and move generation |
| `usi_random` | Random-move engine for calibrating the harness |
| `pst_probe` | Diagnostics for PST training (cross-checks position evaluations from a weight file) |
