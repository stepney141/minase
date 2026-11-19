## minase-core-v0.3.0 (2026-11-20)

### Features

- **search:** Prove MCTS nodes whose side to move can capture the last royal
- **core:** Add a shuffled initial setup for zero self-play start positions
- **datagen:** Exclude checks, screen openings, adjudicate decided games, and generate under lishogi

### Bug fixes

- **search:** Adjudicate MCTS leaves with the referee before the repetition approximation

### Performance

- **game:** Judge royal captures and attacking moves without full generation
- **game:** Stop stalemate and mate judgement at the first escape
- **movegen:** Look up slider attackers by their first blocker
- **movegen:** Judge ordinary-piece legality without generating moves
- **movegen:** Share the attack state among check tests of a node
- **movegen:** Skip lion and capture checks when generating quiet moves
- **movegen:** Find capture targets from the first blockers of the rays
- **search:** Narrow attacker lookups and reuse attacker data in SEE
- **movegen:** Select pawn and go-between origins by shifting destinations
- **search:** Share the transposition move legality within a node

### Refactoring

- **rules:** Rename the engine-default rule set to JCSA (**breaking**)

  The rule set name `engine-default` is no longer accepted; write `JCSA`. `Rules::ENGINE_DEFAULT` is now `Rules::JCSA`, and `Game::with_default_rules()` is removed; call `Game::new(Rules::JCSA)`.

## minase-core-v0.2.0 (2026-10-23)

### Features

- **core:** Classify root moves by the R1 repetition adjudication
- **search:** Score R1 repetitions inside the tree by the attacking side
- **search:** Skip the moves that R2 and R3 forbid inside the tree (**breaking**)

  `Game::search_key_history` is replaced by `Game::repetition_key_history`, whose values depend on the repetition rule.

- **movegen:** Tell whether a move gives check
- **search:** Order quiet moves by the full Stockfish history sum
- **search:** Score quiet checks by direct check squares

### Bug fixes

- **core:** Adjudicate piece exhaustion as in the 16th edition rules (**breaking**)

  Games under the standard exhaustion rule (E0) can end differently: an immediate win after a non-capturing pawn or go-between promotion now waits for the opponent's reply, and a king plus prince against a lone royal is now exhaustion rather than a continued game.

- **movegen:** Judge a lion's foot by the guard on the lion's square (**breaking**)

  Under L0, a lion, horned falcon, or soaring eagle can no longer recapture a protected lion on the first step of a two-step move (including igui), even if its second step blocks or captures the protecting piece.

- **core:** Treat a move that ends the game in a draw as escaping mate (**breaking**)

  Without E1, a position in which the side in check can end the game at once in a draw (by R1 repetition, the article 22 item 8 bare-royals draw, or the E3 draw) is no longer adjudicated as mate.

- **core:** Draw bare royals even with a far-rank immobile pawn or lance (**breaking**)

  Under E0, a position with one royal each plus only immobile far-rank pawns or lances is now an automatic draw instead of a continued game.

- **core:** Ignore dead pieces in exhaustion and require safe royals for its draw (**breaking**)

  Under E0, a side whose only other pieces besides one non-royal piece are dead far-rank pawns or lances now wins by exhaustion, and one royal each with a royal under attack is no longer an immediate draw.

### Refactoring

- **search:** Drop the test-only probes from the production sources

## minase-core-v0.1.1 (2026-10-04)

### Documentation

- Recommend LTO and one codegen unit in dependent crates

  Cargo builds a dependency with the dependent's profile, and with Cargo's default release profile legal-move generation runs at about 61% of its speed with LTO. The README now gives the profile to put in the dependent's own `Cargo.toml`.

- Add crates.io keywords and categories

## minase-core-v0.1.0 (2026-10-04)

### Refactoring

- Split the library into the minase-core crate (**breaking**)

  The rules library moved out of `minase` into this crate. Library types moved from `minase::` to `minase_core::` (for example, `minase::Position` is now `minase_core::Position`), and the former `minase::core::` modules sit directly under `minase_core::`.


