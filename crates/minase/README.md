# minase

`minase` is a chu shogi engine that speaks USI (including the lishogi extensions) and CECP (XBoard).
It searches with alpha-beta, quiescence search, iterative deepening, a transposition table, and Lazy SMP, and evaluates positions with a piece-square table interpolated by game phase.
The rules come from the [`minase-core`](https://crates.io/crates/minase-core) library.

## Running the engine

`--protocol` is required and selects USI or CECP.

```bash
minase --protocol usi
minase --protocol cecp
```

The rule set is given after startup, and the engine has no default rule set.
A USI client sets the option `RuleSet` before `isready` (`setoption name RuleSet value JCSA`), and a CECP client sends `option RuleSet=JCSA` before `new`.
If `isready` or `new` arrives before a rule set, the engine reports the error and exits with status 1.

`RuleSet` accepts `JCSA` (the rules of the Japan Chu Shogi Association with its repetition rule replaced by R1, L0,P0,R1,E0), `lishogi` (L1,L2,P0,P3,R1,E1,E3), or a comma-separated list with exactly one code from each of the groups L, P, R, and E.
The rule codes are defined in [chu-shogi.md](https://github.com/stepney141/minase/blob/master/docs/rules/chu-shogi.md) (Japanese).

`cargo install minase` installs the single executable `minase`. The development tools used in the [Minase repository](https://github.com/stepney141/minase), such as the SPRT harness (`minase match run`) and the parameter tuner (`minase spsa`), run as its subcommands. Diagnostics of the engine itself, such as `bench` and `perft`, are USI commands of the running engine.

## License

GPL-3.0-or-later. See [COPYING](COPYING).
The engine binaries include the MIT-licensed `minase-core`, and the combination is distributed under GPL-3.0-or-later.
