# Minase

Minase is a legal-move generation library and playing engine for chu shogi, written in Rust.
It follows the rules of [Japan Chu Shogi Association](https://www.chushogi-renmei.com/) by default, and the local rules used by lishogi, [HaChu](https://salsa.debian.org/debian/hachu), and other sources can be selected through rule codes.
The supported rules and their sources are documented in [RULES.md](docs/rules/RULES.md) (Japanese).

## Name

Minase is pronounced "mee-nah-seh" with three syllables, and the final "e" is sounded.
The name comes from Minase Kanenari (水無瀬兼成, 1514–1602), a court noble known as a calligrapher who made shogi pieces, including sets for chu shogi.
The Minase family still keeps a chu shogi set inscribed with his age, 86, and Shimamoto Town in Osaka designated the family's Minase pieces, including this set, as a cultural property in 2009.
These facts are documented in Japanese by the [Japanese Wikipedia article on Kanenari](https://ja.wikipedia.org/wiki/%E6%B0%B4%E7%84%A1%E7%80%AC%E5%85%BC%E6%88%90) and a [Shimamoto Town cultural property bulletin](https://www.town.shimamoto.lg.jp/uploaded/attachment/3733.pdf).

## Overview

- Rules: The implementation covers the lion's two-step move (igui and jitto), the sente-lion restriction, protection of the lion (including indirect protection through a discovered attack), promotion rights, repetition, and bare-king endings. Local rules are selected with rule codes (groups L, P, R, and E) or with the presets `engine-default` and `lishogi`.
- Protocols: The engine speaks USI (including the lishogi extensions) and CECP (XBoard).
- Search and evaluation: The search uses alpha-beta, quiescence search, iterative deepening, a transposition table, and Lazy SMP for parallel search. The evaluation is a piece-square table interpolated by game phase.
- Verification: The repository includes unit tests, perft, replay checks against real game records, and an SPRT self-play harness.
- `unsafe` code is forbidden in both crates through the workspace lint settings in Cargo.toml.

## Repository layout

The repository is a Cargo workspace with two crates.

| Crate | Directory | Contents |
|---|---|---|
| `minase-core` | [crates/minase-core](crates/minase-core) | The rules library: board, pieces, positions, legal-move generation, rule sets, game adjudication, and SFEN/USI/CECP notation |
| `minase` | [crates/minase](crates/minase) | The engine: search, evaluation, the USI and CECP front ends, and the development tools listed below |

Development documents, including the rule book, live in [docs/](docs/) and are written in Japanese.

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

## Using Minase as a library

Add `minase-core` to Cargo.toml to use position handling and legal-move generation from Rust. The library does not depend on the engine.

```rust
use minase_core::{Game, MoveGenerator, Position};

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

## License

`crates/minase-core` is licensed under the [MIT License](crates/minase-core/LICENSE-MIT), and `crates/minase` is licensed under the [GNU General Public License, version 3 or later](crates/minase/COPYING). The engine binaries link the library, and their combination is distributed under GPL-3.0-or-later. All other files in this repository, including `docs/`, `tools/`, and `scripts/`, are not licensed, and the copyright holder reserves all rights to them.
