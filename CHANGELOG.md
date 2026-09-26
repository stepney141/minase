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
