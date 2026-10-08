## v2.3.0 (2026-10-08)

### Features

- **eval:** Embed the jointly trained PST and FM weights (candidate J75)
- **eval:** Add the FmScale tuning parameter for the FM correction
- **eval:** Embed the J75 weights with the FM correction scaled by 611/1024

## v2.2.0 (2026-10-08)

### Features

- **eval:** Port the factorization-machine correction to the current PST layout
- **eval:** Embed the quarter FM correction trained with the lookahead teacher (candidate Fa)
- **search:** Search to the byoyomi deadline with a ByoyomiMargin option
- **search:** Adopt interrupted-iteration results in byoyomi clocks

## v2.1.0 (2026-10-06)

### Features

- **cli:** Run the auxiliary tools as subcommands of minase
- **eval:** Adopt the PST retrained for 10 more epochs from G23 (Pc)
- **match:** Resume runs from the recorded conditions
- **spsa:** Resume sessions from the recorded conditions
- **eval:** Adopt the PST retrained for 200 epochs from Pc (candidate L)
- **match:** Let gsprt take the hypothesis Elos for choosing between candidates

## v2.0.1 (2026-10-04)

### Bug Fixes

- Build `cargo install minase` with LTO and one codegen unit

  The 2.0.0 package left out the workspace release profile, so binaries installed from crates.io searched about 14% fewer nodes per second.

## v2.0.0 (2026-10-04)

### Refactoring

- Split the library into the minase-core crate (**breaking**)

  The rules library is now the separate crate `minase-core`. Library types moved from `minase::` to `minase_core::` (for example, `minase::Position` is now `minase_core::Position`), and `minase` no longer re-exports them.

## v1.4.0 (2026-10-04)

### Features

- **search:** Revive capture history, qsearch move limit and history carry (**breaking**)

  Carrying butterfly history between searches changes the search handle API of the library.

- **search:** Revive reverse futility pruning, razoring and the null move eval term
- **search:** Revive the improving flag and a redefined late move pruning
- **search:** Set the revival coefficients to their session start values
- **search:** Re-derive the revival ranges and start values on the G23 PST
- **search:** Apply the search-revival-t SPSA values

### Refactoring

- **train:** Split the training tools into a package by function

## v1.3.0 (2026-10-03)

### Features

- Add d, eval, and tt USI commands and rule-aware perft (**breaking**)

  `perft` now requires `--rules` (for example `--rules engine-default`), and unknown USI commands are no longer silently ignored.

- **usi:** Record protocol I/O with --io-log
- Add the invariants feature for full consistency checks
- **search:** Add the search-stats feature for search statistics
- **usi:** Show the d board in kanji like Stockfish and YaneuraOu
- **eval:** Adopt the PST retrained on generations 2 and 3 (G23)

### Bug fixes

- **search:** Stop repetition detection at the latest null move

### Refactoring

- **harness:** Split the harness module by purpose
- **datagen:** Move code shared by selfplay_gen and lishogi_import into the library
- **selfplay:** Split selfplay_gen and lishogi_import by purpose
- **match:** Split match_runner and match_report by purpose
- **spsa:** Move spsa_runner into its own directory
- **random_play:** Split random_play by purpose
- **core:** Group square, direction, and bitboard under board (**breaking**)

  The items of `minase::core::square`, `minase::core::direction`, and `minase::core::bitboard` are now available only as `minase::core::board::*`.

- **core:** Move promotion and lion capture rules out of rules (**breaking**)

  `minase::core::rules::PromotionChoice` and `minase::core::rules::in_promotion_zone` are now `minase::core::promotion::PromotionChoice` and `minase::core::promotion::in_promotion_zone`.

- **movegen:** Split the move generator by purpose
- **core:** Split the position module by purpose
- **core:** Nest adjudication and repetition under game
- **core:** Split the piece module by type

## v1.2.0 (2026-09-26)

### Features

- **search:** Apply SPSA stage9-20260925-c5 values from 9bc6898
- **search:** Apply SPSA stage9-20260923 values to the search parameters
- **eval:** Retrain the learned PST on the lookahead teacher
- **usi:** Add resignation through the ResignValue option
- **match:** Disable engine resignation in the match harness
- **spsa:** Add spsa_runner with resume and `spsa_runner apply`
- **selfplay:** Add teacher rescoring and provenance files to selfplay_gen, and add lishogi_import

### Bug fixes

- **search:** Count the royal-capturing move in mate distance
- **usi:** Report mate distance in shogi convention

### Performance

- **search:** Evaluate the static score at most once per main-search node

### Refactoring

- **search:** Split the search module by purpose
- **eval:** Split the learned PST into submodules
- **harness:** Move the match execution core into the library harness module
- **training:** Move training data formats out of eval (**breaking**)

  `minase::eval::training_data`, `minase::eval::rescore`, and `minase::eval::provenance` are now `minase::training::records`, `minase::training::rescore`, and `minase::training::provenance`.

## v1.1.0 (2026-09-21)

- Support USI ponder
- Add some forward pruning methods

## v1.0.0 (2026-09-20)

- Initial release
