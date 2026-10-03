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
