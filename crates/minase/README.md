# minase

`minase` is a chu shogi engine that speaks USI (including the lishogi extensions) and CECP (XBoard).
It searches with alpha-beta, quiescence search, iterative deepening, a transposition table, and Lazy SMP, and evaluates positions with a piece-square table interpolated by game phase.
The rules come from the [`minase-core`](https://crates.io/crates/minase-core) library.

## Running the engine

Both `--protocol` and `--rules` are required.

```bash
minase --protocol usi --rules engine-default
minase --protocol usi --rules lishogi
minase --protocol cecp --rules L0,P0,R1,E0
```

`--rules` accepts `engine-default` (the standard rules, L0,P0,R1,E0), `lishogi` (L1,L2,P0,P3,R1,E1,E3), or a comma-separated list with exactly one code from each of the groups L, P, R, and E.
The rule codes are defined in [RULES.md](https://github.com/stepney141/minase/blob/master/docs/rules/RULES.md) (Japanese).

`cargo install minase` installs the single executable `minase`. The development tools used in the [Minase repository](https://github.com/stepney141/minase), such as the SPRT harness (`minase match run`), `minase dev bench`, and `minase dev perft`, run as its subcommands.

## License

GPL-3.0-or-later. See [COPYING](COPYING).
The engine binaries include the MIT-licensed `minase-core`, and the combination is distributed under GPL-3.0-or-later.
