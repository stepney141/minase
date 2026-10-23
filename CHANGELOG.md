## v2.5.0 (2026-10-23)

### Features

- **eval:** Evaluate with an embedded MNUE v4 NNUE instead of the PST and FM
- **eval:** Expose the feature indexing and add the MNUE v5 encoder
- **bench:** Count accumulator refreshes and convert Rust float checkpoints to npz
- **eval:** Embed the trained P-type NNUE of width 128
- **trainer:** Port the NNUE removal, distribution and move diagnostics to Rust
- **trainer:** Add the PST learner as minase-trainer pst train
- **trainer:** Add FM quantization, MNPT v3 output, fm quarter and fm scale
- **trainer:** Add the FM learner as minase-trainer fm train
- **training:** Record the position origin in MNSD and drop provenance (**breaking**)

  `minase::training::provenance` and `minase::training::rescore` are removed, `Record::from_position` takes an `Origin`, and MNSD readers reject records whose byte 156 is not 0-3.

- **eval:** Keep a single MNPT version and drop version 4 match reports (**breaking**)

  `Pst::decode` rejects MNPT version 2 and `Pst::encode` always writes version 3.

- **training:** Add the game-chained MNSD v2 reader and writer
- **datagen:** Write self-play and lishogi teacher data as MNSD v2
- **search:** Drop root moves that lose by R1 repetition within two plies
- **search:** Add a path-dependent band for repetition outcomes
- **search:** Score R1 repetitions inside the tree by the attacking side
- **search:** Skip the moves that R2 and R3 forbid inside the tree (**breaking**)

  `Game::search_key_history` in minase-core is replaced by `Game::repetition_key_history`, whose values depend on the repetition rule.

- **search:** Return a signed draw value for R1 repetition draws
- **usi:** Add DrawValueBlack and DrawValueWhite options
- **eval:** Embed the cosine-annealed NNUE trained on the regenerated teacher data
- **eval:** Add phase-interpolated and hidden-layer NNUE output heads
- **eval:** Embed the NNUE trained without the piece-removal auxiliary loss
- **eval:** Embed the NNUE trained on search values without lookahead teachers
- **search:** Evaluate every node once with the correction applied
- **search:** Shape reverse futility pruning after Stockfish
- **search:** Halve the late move pruning limit when not improving
- **search:** Rebuild late move reductions from Stockfish's terms
- **search:** Gate zero-window TT cutoffs and IIR by node type
- **search:** Adjust pruning when the previous iteration seeks a mate
- **search:** Apply the search-pruning-forms SPSA values
- **spsa:** Resume a commit-based session after rebuilding the engine
- **search:** Prune quiet moves by futility per move after Stockfish
- **search:** Apply late move pruning at every depth and at PV nodes
- **search:** Prune losing captures and quiet checks by SEE at any depth
- **search:** Add Stockfish's futility pruning for captures
- **search:** Prune quiet moves that lose material by SEE
- **search:** Razor all nodes with a depth-proportional margin after Stockfish
- **search:** Shape null move pruning after Stockfish and verify it at depth 16
- **search:** Return Stockfish's weighted mean from reverse futility pruning
- **search:** Reduce checks less and other moves more when seeking a mate
- **search:** Apply the shallow-pruning-extensions SPSA values
- **search:** Update histories with Stockfish bonus and malus forms
- **search:** Add continuation, pawn and low ply histories
- **search:** Update histories at alpha raises, TT cutoffs and fail lows
- **search:** Order quiet moves by the full Stockfish history sum
- **search:** Use histories in LMR and quiet move pruning
- **search:** Extend LMR to all later moves and the root
- **search:** Score quiet checks by direct check squares
- **search:** Apply the history forms SPSA values
- **search:** Update the material correction table in Stockfish form
- **search:** Add pawn, minor, and non-pawn correction tables
- **search:** Add continuation correction history
- **search:** Carry correction tables through a game and share them
- **search:** Reduce LMR less where the correction is large
- **search:** Apply the correction history tables SPSA values
- **search:** Add the correction to the quiescence stand-pat
- **search:** Prune qsearch captures by an SEE threshold and floor
- **search:** Drop the qsearch entry delta pruning and raise by estimates
- **search:** Count qsearch move limit by candidate order
- **search:** Exempt recaptures, promotions, and checks in qsearch
- **search:** Probe the table before the qsearch stand-pat
- **search:** Blend qsearch fail-high returns toward beta
- **search:** Apply the qsearch forms SPSA values
- **search:** Center aspiration windows on the previous best move average
- **search:** Widen the initial aspiration window with the score magnitude
- **search:** Move both aspiration edges from the failed score
- **search:** Reduce aspiration re-searches after fail highs
- **search:** Apply the aspiration window forms SPSA
- **search:** Tune the stable best move iteration count
- **search:** Require the opponent's remaining time for clocks without byoyomi (**breaking**)

  A USI `go` or a CECP search with a clock that has no byoyomi and no `movetime` is now rejected when the opponent's remaining time is missing. `ClockLimits::new` takes the opponent's remaining time as a new argument, and `SearchError` has the new variant `MissingOpponentClock`.

- **search:** Scale the soft limit by falling eval
- **search:** Scale the soft limit by best move stability
- **search:** Scale the soft limit by best move instability
- **search:** Scale the soft limit by best move effort
- **search:** Shrink the soft limit when behind on the clock
- **search:** Cap the scaled soft limit at 500 ms with one root move
- **search:** Stop increasing the depth after half of the scaled time
- **search:** Stop timed searches early on short mates
- **search:** Add 25% to the soft limit after pondering
- **search:** Apply the time instability scaling SPSA
- **search:** Restrict instability time scaling to clocks without byoyomi

### Bug fixes

- **match:** Judge GSPRT through one stats function and prune harness tests
- **search:** Reread the clock at the iteration boundary

### Performance

- **search:** Reuse the static eval stored in the transposition table
- Read saved JSON records in one buffer

### Refactoring

- **eval:** Generalize the NNUE over feature sets and output heads with MNUE v5
- Remove the Python trainer, the temporary replay paths and the dev probes
- Remove an unread CECP clock field and a duplicated seed mixer
- **training:** Drop MNSD v1 and load trainer files in parallel
- **search:** Drop the test-only probes from the production sources
- **search:** Read elapsed search time through a clock trait
- **search:** Pass the cut-node prediction down the search tree
- **search:** Record the PV flag in transposition table entries
- **search:** Compute LMR reductions in hundredths and tune the cap
- **search:** Let tests pass their own coefficients to the formulas
- **search:** Give each shallow pruning method its own node condition
- **search:** Record path moves, move counts and pawn keys for histories
- **search:** Keep the correction table in the history tables
- **search:** Pass a signed threshold to see_prunes
- **search:** Keep correction keys and path records in quiescence
- **search:** Add a static eval field and eval-only state to tt entries
- **search:** Floor the aspiration delta at one and always widen it
- **search:** Record per-root-move effort and score averages

## v2.4.0 (2026-10-12)

### Features

- **search:** Penalize searched captures when a quiet move cuts off
- **search:** Try killer moves before a losing first capture in shallow zero-window nodes
- **search:** Order quiescence capture groups by capture history
- **search:** Carry the capture history across root searches
- **search:** Break quiet move history ties by the PST delta
- **search:** Order root moves by the subtree sizes of the previous iteration
- **search:** Add a history malus with separate bonus and malus scales

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
