//! ランダム対局を実行し、合法手適用と終局の不変条件を検査する。

use std::{
    fmt, io,
    num::{NonZeroU32, NonZeroU64, NonZeroUsize},
};

use minase_core::{
    Game, GameError, GameResult, GameStatus, Move, PositionError, Rules, rng::XorShift64, to_sfen,
};

use super::{seed::game_seed, text::move_text};

/// 正常に終了した1局の結果。
pub(super) struct CompletedGame {
    /// 終了時点の手数。
    pub(super) plies: u32,
    /// 終局結果または打ち切り。
    pub(super) outcome: Outcome,
    /// 実際に適用した全手順。
    pub(super) moves: Vec<Move>,
}

/// 終局または手数上限による打ち切りを表す。
#[derive(Clone, Copy)]
pub(super) enum Outcome {
    /// 裁定による終局。
    Finished(GameResult),
    /// 手数上限による打ち切り。
    Cutoff,
}

/// 違反の種類と、失敗した操作が返した原因。
#[derive(Debug)]
enum Issue {
    NoLegalMoves,
    FullVerification(GameError),
    MoveRejected(GameError),
    InvalidPosition(PositionError),
    TerminalMoves(usize),
    AdditionalMove(Result<GameStatus, GameError>),
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoLegalMoves => f.write_str("ongoing game has no legal moves"),
            Self::FullVerification(error) => {
                write!(f, "legal move rejected during full verification: {error}")
            }
            Self::MoveRejected(error) => write!(f, "legal move rejected: {error}"),
            Self::InvalidPosition(error) => write!(f, "position validation failed: {error}"),
            Self::TerminalMoves(count) => write!(f, "finished game returned {count} legal moves"),
            Self::AdditionalMove(result) => {
                write!(f, "additional move after game end returned {result:?}")
            }
        }
    }
}

impl std::error::Error for Issue {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::FullVerification(error)
            | Self::MoveRejected(error)
            | Self::AdditionalMove(Err(error)) => Some(error),
            Self::InvalidPosition(error) => Some(error),
            _ => None,
        }
    }
}

/// 不変条件の違反と、再現に必要な局面および全手順。
#[derive(Debug)]
pub struct InvariantFailure {
    issue: Issue,
    rules: String,
    base_seed: u64,
    game_number: NonZeroU64,
    ply: u64,
    problem_move: Option<Move>,
    previous_sfen: String,
    moves: Vec<Move>,
}

impl fmt::Display for InvariantFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "random-play invariant failure: {}", self.issue)
    }
}
impl std::error::Error for InvariantFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.issue)
    }
}

impl InvariantFailure {
    /// エラーの説明に続ける規則、シード、局番号、手数、着手、直前のSFEN、全手順を出す。
    pub fn write_context(&self, output: &mut dyn FnMut(&str) -> io::Result<()>) -> io::Result<()> {
        output(&format!("rules: {}", self.rules))?;
        output(&format!("seed: {}", self.base_seed))?;
        output(&format!("game: {}", self.game_number))?;
        output(&format!("ply: {}", self.ply))?;
        match self.problem_move {
            Some(mv) => output(&format!("problem move: {}", move_text(mv)))?,
            None => output("problem move: none")?,
        }
        output(&format!("previous SFEN: {}", self.previous_sfen))?;
        output("moves:")?;
        if self.moves.is_empty() {
            output("  (none)")?;
        } else {
            for (index, &mv) in self.moves.iter().enumerate() {
                output(&format!("  {}: {}", index + 1, move_text(mv)))?;
            }
        }
        Ok(())
    }
}

/// 指定局番号のランダム対局を実行して不変条件を検査する。
///
/// 継続中の局では[`Game::legal_moves`]が空でなく、選んだ合法手を[`Game::play`]が受理し、
/// 着手後の局面が`validate`を通ることを確かめる。
/// `verify_all`では全合法手をそれぞれ複製した対局へ適用し、すべて受理されることを検査する。
/// 終局後には合法手が空であり、追加の着手が[`GameError::GameAlreadyOver`]を返すことを確かめる。
///
/// 違反時は原因と再現情報を保持したエラーを返す。
/// 同じ規則、`seed`、`game`、`max-ply`なら着手列は完全に再現される。
/// `verify-all`は複製だけに適用するので、対局の着手列を変えない。
/// 手数上限への到達は異常ではなく[`Outcome::Cutoff`]として返す。
pub(super) fn run_game(
    rules: Rules,
    rules_text: &str,
    base_seed: u64,
    game_number: NonZeroU64,
    max_ply: NonZeroU32,
    verify_all: bool,
) -> Result<CompletedGame, Box<InvariantFailure>> {
    let mut game = Game::new(rules);
    let mut rng = XorShift64::new(game_seed(base_seed, game_number.get()));
    let mut history = Vec::new();

    loop {
        if game.ply_count() >= max_ply.get() {
            return Ok(CompletedGame {
                plies: game.ply_count(),
                outcome: Outcome::Cutoff,
                moves: history,
            });
        }

        let legal_moves = game.legal_moves();
        let previous_sfen = to_sfen(game.position());
        let Some(count) = NonZeroUsize::new(legal_moves.len()) else {
            return Err(Box::new(InvariantFailure {
                issue: Issue::NoLegalMoves,
                rules: rules_text.to_owned(),
                base_seed,
                game_number,
                ply: u64::from(game.ply_count()),
                problem_move: None,
                previous_sfen,
                moves: history,
            }));
        };

        if verify_all {
            for &candidate in &legal_moves {
                let mut probe = game.clone();
                if let Err(error) = probe.play(candidate) {
                    return Err(Box::new(InvariantFailure {
                        issue: Issue::FullVerification(error),
                        rules: rules_text.to_owned(),
                        base_seed,
                        game_number,
                        ply: u64::from(game.ply_count()) + 1,
                        problem_move: Some(candidate),
                        previous_sfen,
                        moves: history,
                    }));
                }
            }
        }

        let selected = legal_moves[rng.index(count)];
        let status = match game.play(selected) {
            Ok(status) => status,
            Err(error) => {
                return Err(Box::new(InvariantFailure {
                    issue: Issue::MoveRejected(error),
                    rules: rules_text.to_owned(),
                    base_seed,
                    game_number,
                    ply: u64::from(game.ply_count()) + 1,
                    problem_move: Some(selected),
                    previous_sfen,
                    moves: history,
                }));
            }
        };
        history.push(selected);

        if let Err(error) = game.position().validate() {
            return Err(Box::new(InvariantFailure {
                issue: Issue::InvalidPosition(error),
                rules: rules_text.to_owned(),
                base_seed,
                game_number,
                ply: u64::from(game.ply_count()),
                problem_move: Some(selected),
                previous_sfen,
                moves: history,
            }));
        }

        let GameStatus::Finished(result) = status else {
            continue;
        };

        let terminal_sfen = to_sfen(game.position());
        let terminal_moves = game.legal_moves();
        if let Some(&candidate) = terminal_moves.first() {
            return Err(Box::new(InvariantFailure {
                issue: Issue::TerminalMoves(terminal_moves.len()),
                rules: rules_text.to_owned(),
                base_seed,
                game_number,
                ply: u64::from(game.ply_count()) + 1,
                problem_move: Some(candidate),
                previous_sfen: terminal_sfen,
                moves: history,
            }));
        }

        match game.play(selected) {
            Err(GameError::GameAlreadyOver) => {}
            unexpected => {
                return Err(Box::new(InvariantFailure {
                    issue: Issue::AdditionalMove(unexpected),
                    rules: rules_text.to_owned(),
                    base_seed,
                    game_number,
                    ply: u64::from(game.ply_count()) + 1,
                    problem_move: Some(selected),
                    previous_sfen: terminal_sfen,
                    moves: history,
                }));
            }
        }

        return Ok(CompletedGame {
            plies: game.ply_count(),
            outcome: Outcome::Finished(result),
            moves: history,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 違反の値だけから、原因と再現に必要な全行を復元できる。
    #[test]
    fn invariant_failure_reports_the_cause_and_complete_reproduction_context() {
        use minase_core::test_util::sq;
        use std::error::Error;
        let mv = Move {
            from: sq(1, 2),
            mid: None,
            to: sq(1, 3),
            promote: false,
        };
        let failure = InvariantFailure {
            issue: Issue::MoveRejected(GameError::GameAlreadyOver),
            rules: "L0,P0,R1,E0".to_owned(),
            base_seed: 7,
            game_number: NonZeroU64::new(2).unwrap(),
            ply: 3,
            problem_move: Some(mv),
            previous_sfen: "12/12/12/12/12/12/12/12/12/12/12/12 b".to_owned(),
            moves: vec![mv, mv],
        };
        let mut lines = vec![failure.to_string()];
        failure
            .write_context(&mut |line| {
                lines.push(line.to_owned());
                Ok(())
            })
            .unwrap();
        assert_eq!(
            lines[0],
            format!(
                "random-play invariant failure: legal move rejected: {}",
                GameError::GameAlreadyOver
            )
        );
        assert_eq!(
            &lines[1..],
            [
                "rules: L0,P0,R1,E0",
                "seed: 7",
                "game: 2",
                "ply: 3",
                "problem move: move (1,2)->(1,3)",
                "previous SFEN: 12/12/12/12/12/12/12/12/12/12/12/12 b",
                "moves:",
                "  1: move (1,2)->(1,3)",
                "  2: move (1,2)->(1,3)",
            ]
        );
        assert!(matches!(
            failure
                .source()
                .unwrap()
                .source()
                .unwrap()
                .downcast_ref::<GameError>(),
            Some(GameError::GameAlreadyOver)
        ));
        let failure = InvariantFailure {
            issue: Issue::NoLegalMoves,
            problem_move: None,
            moves: Vec::new(),
            ..failure
        };
        let mut lines = Vec::new();
        failure
            .write_context(&mut |line| {
                lines.push(line.to_owned());
                Ok(())
            })
            .unwrap();
        assert_eq!(lines[4], "problem move: none");
        assert_eq!(lines.last().unwrap(), "  (none)");
    }

    /// [`run_game`]の再現性と打ち切りの契約を検査する。
    #[test]
    fn same_seed_and_game_number_reproduce_the_same_game() {
        let rules = Rules::JCSA;
        let first = run_game(
            rules,
            "L0,P0,R1,E0",
            20_260_814,
            NonZeroU64::MIN,
            NonZeroU32::new(16).unwrap(),
            false,
        )
        .unwrap();
        // run_gameの契約により、全合法手の適用検査を有効にしても同じ対局になる。
        let second = run_game(
            rules,
            "L0,P0,R1,E0",
            20_260_814,
            NonZeroU64::MIN,
            NonZeroU32::new(16).unwrap(),
            true,
        )
        .unwrap();
        assert_eq!(first.moves, second.moves);
        assert_eq!(first.plies, second.plies);
        assert_eq!(first.plies, 16);
        assert!(matches!(first.outcome, Outcome::Cutoff));
        assert!(matches!(second.outcome, Outcome::Cutoff));
    }
}
