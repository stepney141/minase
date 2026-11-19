## v3.0.0-alpha.1 (2026-11-20)

### Features

- **eval:** Embed the NNUE trained from a piece-value linear initialization
- **nn:** Add network input encoding, policy labels, and evaluator trait
- **nn:** Run network inference through ONNX Runtime
- **search:** Add the position-free MCTS tree with PUCT and Gumbel selection
- **search:** Drive MCTS over positions with repetition rules and time limits
- **usi:** Select MCTS with --search and append engine arguments in matches
- **data:** Add the AlphaZero self-play generator
- **match:** Pass USI options to the candidate and baseline engines and record them
- **search:** Parameterize the Gumbel Q scale and temper the final move noise
- **nn:** Add per-kind pseudo control and control count input planes
- **zero:** Add a check-only mate-in-three detector and its cost tool
- **zero:** Randomize self-play playout caps and stop at mate-in-three wins
- **zero:** Close self-play cycles at the first chunk prefix reaching the target positions
- **zero:** Raise the self-play ply limit to 3000
- **trainer:** Add nnue fit-linear to regress the linear path on game outcomes
- **eval:** Embed the NNUE with the linear path regressed on game outcomes
- **eval:** Replace the NNUE heads with pairwise multiplication and two hidden layers
- **eval:** Hold the NNUE hidden-layer weights in 16 bits with scale 256
- **eval:** Embed the NNUE trained with the fan-in first-layer initialization
- **trainer:** Export the handcrafted evaluation as a linear-only MNUE and read --base from MNUE
- **search:** Prove MCTS nodes whose side to move can capture the last royal
- **search:** Add the PolicyTemperature option to the MCTS engine
- **search:** Set the MCTS policy temperature to 1.4 by default
- **zero:** Choose the self-play start position by a required argument
- **nn:** Spread zero self-play threads over several identical GPUs (**breaking**)

  `ExecutionProvider::TensorRt` and `ExecutionProvider::Cuda` carry a `device_id`, `tensorrt_cache::query_gpu` is replaced by `query_gpus` returning `GpuInventory`, `parse_gpu_query` returns `GpuInventory`, and `GpuQueryError::DeviceCount` is replaced by `NoDevices`, `MixedDevices` and `CudaVisibleDevicesSet`.

- **usi:** Require the RuleSet option and remove the --rules startup argument (**breaking**)

  `minase --protocol <usi|cecp> --rules <rules>` is no longer accepted. Start the engine with `--protocol` only and send `setoption name RuleSet value <rules>` before `isready`, or `option RuleSet=<rules>` before `new` in CECP (XBoard: `-firstOptions "RuleSet=<rules>"`). The RuleSet option now declares `default <empty>`. Match run directories written with manifest format 7 must be resumed and reported by the harness that started them; commits from before this change need `--baseline-args '--rules <codes>'`.

- **usi:** Select the zero network with the NnModel option and verify its checksum in matches (**breaking**)

  `--nn-model` is no longer an engine startup argument; send `setoption name NnModel value <path>` before `isready`. In matches, write `--candidate-args '--search mcts' --candidate-option 'NnModel=<path>'`. Version 8 manifests require the `network_sha256` field.

- **usi:** Run perft and bench as USI commands and remove the dev subcommands (**breaking**)

  `minase dev perft` and `minase dev bench` are removed. Send `perft` or `bench` to an engine started with `--protocol usi`. bench_compare.py handles only commits from this one on.

- **usi:** Run nn-eval, nn-fixture and random-play as USI commands and remove the dev subcommands (**breaking**)

  `minase dev nn-eval`, `minase dev nn-fixture` and `minase dev random-play` are removed. Send `nn-eval`, `nn-fixture` or `random-play` to an engine started with `--protocol usi`.

- **search:** Search every rule code in MCTS without encoding deferred promotion (**breaking**)

  nn::encode no longer returns a Result, and NnError::PromotionDeferred, mcts::Error::PromotionDeferred and EvaluationError::Input are removed. MCTS now accepts rule sets with P1, P2 and P5 instead of reporting "MCTS cannot encode rules P1, P2 or P5".

- **selfplay:** Commit teacher games in fixed-size chunks and resume after interruption
- **datagen:** Exclude checks, screen openings, adjudicate decided games, and generate under lishogi
- **data:** Write lishogi validation games as no-search MNSD records (**breaking**)

  `minase::training::records::TeacherKind` gains the `NoSearch` variant, so exhaustive matches on it must handle the new kind.

- **selfplay:** Start zero self-play from the initial position and calibrate resignation like AobaZero
- **selfplay:** Generate zero self-play under the lishogi rule set
- **usi:** Run bench with PV-MCTS when launched with --search mcts
- **search:** Validate the PV-MCTS tree under the invariants feature
- **usi:** Print PV-MCTS root move statistics with VerboseMoveStats
- **usi:** Carry ranked PV lines in search events and declare MultiPV (**breaking**)

  SearchEvent::Progress loses depth, score, and pv, and SearchEvent::Finished loses pv; both gain lines: Vec<PvLine>.

- **search:** Read N ranked PV lines in alpha-beta search for MultiPV
- **search:** Report N ranked PV lines from PV-MCTS for MultiPV
- **eval:** Read the NNUE network from the EvalFile option (**breaking**)

  The alpha-beta engine requires the USI option EvalFile (or the CECP option EvalFile) before isready or new, and --search mcts requires the USI option NnCpPerLogit before isready.

- **eval:** Take the NNUE network of data commands from --eval-file (**breaking**)

  The library no longer exposes eval::weights(), and NetworkFile exposes the network and its checksum only through methods.

- **eval:** Add P and HalfKP_vm feature set specs and a shared first-layer accumulator
- **eval:** Add LayerStack and Simple networks in the tatara file format
- **eval:** Search with tatara-format LayerStack and Simple networks (**breaking**)

  EvalFile no longer accepts MNUE files; it reads tatara LayerStack (P) and Simple (HalfKP_vm) networks only.

- **trainer:** Export v0 as a LayerStack network and remove MNUE

### Bug fixes

- **search:** Address review findings on MCTS reuse, batch timing, and Gumbel
- **data:** Keep self-play running when the final Gumbel move is unvisited
- **search:** Adjudicate MCTS leaves with the referee before the repetition approximation
- **match:** Keep the sign of mate -0 in the harness score records
- **nn:** Keep the ONNX Runtime environment alive until process exit
- **selfplay:** Check chunk summary numbers and return thread spawn failures
- **trainer:** Print validation losses and stop reasons as plain values and keep game numbers in generator errors

### Performance

- **zero:** Try likely defenses first in the mate-in-three detector
- **eval:** Hold the NNUE accumulator in 16-bit integers
- **eval:** Fold the mirrored pairwise products before the dot products
- **search:** Precompute continuation history rows for quiet ordering
- **search:** Skip the qsearch SEE floor after a stricter threshold passed
- **movegen:** Share the attack state among check tests of a node
- **movegen:** Find capture targets from the first blockers of the rays
- **search:** Narrow attacker lookups and reuse attacker data in SEE
- **search:** Filter the game history before scanning it for repetitions
- **search:** Update the NNUE accumulator only when a node needs it
- **search:** Reuse per-piece values while scoring quiet moves
- **search:** Build the lesser-piece threats by value rank without allocation
- **search:** Generate quiet moves only after the killer stages
- **search:** Stop sorting and returning quiets once late move pruning holds
- **eval:** Expand the NNUE head input to i16 once per evaluation
- **eval:** Pad the second hidden layer to 32 inputs for inference
- **eval:** Use fixed-length arrays for the width-256 mirrored head
- **search:** Skip the second quiet SEE when a stricter threshold passed
- **search:** Store the capture history in 16 bits
- **search:** Share the transposition move legality within a node
- **search:** Compute capture information once per candidate
- **nn:** Receive GPU inference outputs in pinned memory through IoBinding
- **nn:** Send zero network inputs as uint8 and cast them in the graph (**breaking**)

  Networks exported with a float32 input are rejected; re-export them from their checkpoints with minase_zero export-onnx.

- **nn:** Return the zero policy output as FP16 (**breaking**)

  Networks exported with a float32 policy are rejected; re-export them from their checkpoints with minase_zero export-onnx.

- **nn:** Pack the zero policy output to the labels that can be legal (**breaking**)

  Networks without the policy_labels metadata are rejected; re-export them from their checkpoints with minase_zero export-onnx.

### Refactoring

- **search:** Move per-game search state behind a Searcher enum
- Share the random opening, repetition cutoff check, and TensorRT cache root
- **data:** Move the teacher statistics into minase data teacher-stats
- **search:** Move shared items to the module that owns them
- **search:** Use the softmax of the network module in MCTS
- **nn:** Move test-only code out of production types and functions (**breaking**)

  `minase::nn::UniformEvaluator` is now `minase::nn::test_util::UniformEvaluator` and requires the `test-util` feature.

- **nn:** Move the fixed position fixture into the engine crate
- **rules:** Rename the engine-default rule set to JCSA (**breaking**)

  The rule set name `engine-default` is no longer accepted; write `JCSA`. In minase-core, `Rules::ENGINE_DEFAULT` is now `Rules::JCSA`, and `Game::with_default_rules()` is removed; call `Game::new(Rules::JCSA)`. Match and SPSA runs whose manifest stores `engine-default` as the rule text cannot be resumed by this harness.

- **datagen:** Share the output lock and atomic saves with zero-selfplay

## v2.6.0 (2026-10-25)

### Features

- **match:** Add a fixed movetime limit to the match harness
- **search:** Search movetime-only limits to the deadline
- **search:** Play partial results in movetime-only searches

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
