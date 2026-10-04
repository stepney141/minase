# minase-core

`minase-core` is a Rust library that implements the rules of chu shogi.
It provides the board, pieces, positions, legal-move generation, rule sets, game adjudication (checkmate, repetition, and bare-king endings), and the SFEN, USI, and CECP move notations.
The library contains no search or evaluation, and it is the rules layer of the [Minase](https://github.com/stepney141/minase) engine.

## Rules

The rules follow the Japan Chu Shogi Association by default.
Local rules used by lishogi, HaChu, and other sources are selected through rule codes from four groups (lion rules L, promotion rules P, repetition rules R, and bare-king rules E), or through the presets `engine-default` and `lishogi`.
The rule book that defines each code and cites its sources is [RULES.md](https://github.com/stepney141/minase/blob/master/docs/rules/RULES.md) (Japanese) in the repository.

## Example

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

## Performance

Cargo applies only the profile of the crate being built, so the settings below must go in your own `Cargo.toml`.
Legal-move generation in this crate relies on inlining across its internal functions, and Cargo's default release profile leaves much of it undone.

```toml
[profile.release]
lto = true
codegen-units = 1
```

With both settings, a perft of the initial position to depth 5 ran 1.63 times as fast as with the default release profile (30.1 versus 18.5 million nodes per second on one performance core of an Intel Core Ultra 7 265KF).
LTO accounts for nearly all of the gain; `codegen-units = 1` alone does not help.

## Features

- `invariants`: checks the internal consistency of positions after every move. The crate's own tests always enable it; ordinary builds leave it off.
- `test-util`: exposes the `test_util` module of test positions and helpers for dependent crates' tests.

## License

MIT. See [LICENSE-MIT](LICENSE-MIT).
