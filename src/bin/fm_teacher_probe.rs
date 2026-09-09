//! 独立PST探索によるFM/PST提案手の診断。MNSD以前の反復履歴は利用できない。
use clap::Parser;
use minase::core::rules::parse_rule_set;
use minase::eval::{Pst, training_data::Reader};
use minase::notation::usi;
use minase::search::{
    MATE_THRESHOLD, SearchEvent, SearchLimits, SearchSnapshot, TranspositionTable, start_search,
};
use minase::{Color, Game, GameResult, Move, MoveGenerator, Rules};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Cursor, Write},
    num::{NonZeroU64, NonZeroUsize},
    path::PathBuf,
    process::ExitCode,
    sync::Arc,
};

#[derive(Parser)]
#[command(about = "FM/PST提案手の独立教師探索。MNSD以前の反復履歴なし。各探索Threads=1、新規TT。")]
struct Arguments {
    #[arg(long)]
    positions: PathBuf,
    #[arg(long)]
    candidate_pst: PathBuf,
    #[arg(long)]
    teacher_pst: PathBuf,
    #[arg(long)]
    proposal_nodes: NonZeroU64,
    #[arg(long, value_delimiter=',', num_args=1..)]
    teacher_nodes: Vec<NonZeroU64>,
    #[arg(long)]
    hash_mb: NonZeroUsize,
}
#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Score {
    Cp {
        value: i32,
    },
    /// 子探索のmate値には親から着手した1手を含める。
    Mate {
        value: i32,
    },
    Terminal {
        outcome: &'static str,
        reason: String,
    },
    Incomplete {
        value: i32,
    },
}
fn searched_score(value: i32, depth: u32, negate: bool) -> Score {
    let value = if negate { -value } else { value };
    if depth == 0 {
        Score::Incomplete { value }
    } else if value.abs() >= MATE_THRESHOLD {
        Score::Mate {
            value: if negate {
                value - value.signum()
            } else {
                value
            },
        }
    } else {
        Score::Cp { value }
    }
}
fn terminal_score(result: GameResult, parent: Color) -> Score {
    match result {
        GameResult::Win { winner, reason } => Score::Terminal {
            outcome: if winner == parent { "win" } else { "loss" },
            reason: format!("{reason:?}"),
        },
        GameResult::Draw { reason } => Score::Terminal {
            outcome: "draw",
            reason: format!("{reason:?}"),
        },
    }
}
#[derive(Serialize)]
struct Search {
    best_move: Option<String>,
    score: Score,
    depth: u32,
    nodes: u64,
    elapsed_ms: f64,
    pv: Vec<String>,
    stop_reason: String,
}
#[derive(Serialize)]
struct Proposals {
    fm: Search,
    pst: Search,
}
#[derive(Serialize)]
struct TeacherRoot {
    budget: u64,
    search: Search,
}
#[derive(Serialize)]
struct Child {
    r#move: String,
    budget: u64,
    search: Search,
}
#[derive(Clone, Serialize)]
struct Metadata {
    candidate_sha256: String,
    teacher_sha256: String,
    positions_sha256: String,
}
#[derive(Serialize)]
struct Row {
    metadata: Metadata,
    index: usize,
    game_number: u32,
    ply: u16,
    history_available: bool,
    proposal_nodes: u64,
    teacher_nodes: Vec<u64>,
    proposals: Proposals,
    teacher_roots: Vec<TeacherRoot>,
    candidates: Vec<String>,
    children: Vec<Child>,
}
fn validate_budgets(budgets: &[NonZeroU64]) -> Result<(), String> {
    if budgets.is_empty() || budgets.windows(2).any(|p| p[0] >= p[1]) {
        return Err("teacher budgets must be nonempty, unique, and increasing".into());
    }
    Ok(())
}
fn include_candidate(candidates: &mut Vec<Move>, mv: Move, legal: &[Move]) -> Result<(), String> {
    if !legal.contains(&mv) {
        return Err("search proposed an illegal move".into());
    }
    if !candidates.contains(&mv) {
        candidates.push(mv)
    }
    Ok(())
}
fn move_text(game: &Game, mv: Move) -> Result<String, String> {
    usi::text(game.position(), mv, &MoveGenerator::new(game.rules().moves))
        .map_err(|e| e.to_string())
}
fn run_search(
    game: &Game,
    pst: &Arc<Pst>,
    budget: u64,
    hash_mb: usize,
    negate: bool,
) -> Result<(Move, Search), String> {
    let snapshot = SearchSnapshot::from_game(game).map_err(|e| e.to_string())?;
    let limits = SearchLimits::new(None, Some(budget), None, None).map_err(|e| e.to_string())?;
    let tt = TranspositionTable::new(hash_mb).map_err(|e| e.to_string())?;
    let handle = start_search(
        Arc::clone(pst),
        snapshot,
        limits,
        0,
        NonZeroUsize::new(1).unwrap(),
        tt,
    );
    let finished = loop {
        let event = handle
            .events()
            .recv()
            .map_err(|_| "search ended without Finished event")?;
        if let SearchEvent::Finished {
            best_move,
            score,
            depth,
            nodes,
            elapsed,
            pv,
            stop_reason,
            ..
        } = event
        {
            break (best_move, score, depth, nodes, elapsed, pv, stop_reason);
        }
    };
    handle.join().map_err(|_| "search worker panicked")?;
    let (best_move, score, depth, nodes, elapsed, pv, stop_reason) = finished;
    if !game.legal_moves().contains(&best_move) {
        return Err("search returned an illegal best move".into());
    }
    let generator = MoveGenerator::new(game.rules().moves);
    let mut position = game.position().clone();
    let mut text_pv = Vec::new();
    for mv in pv {
        text_pv.push(usi::text(&position, mv, &generator).map_err(|e| e.to_string())?);
        position
            .try_make_move(mv, &generator)
            .map_err(|e| e.to_string())?;
    }
    Ok((
        best_move,
        Search {
            best_move: Some(move_text(game, best_move)?),
            score: searched_score(score, depth, negate),
            depth,
            nodes,
            elapsed_ms: elapsed.as_secs_f64() * 1000.0,
            pv: text_pv,
            stop_reason: format!("{stop_reason:?}"),
        },
    ))
}
fn decode_pst(path: &PathBuf) -> Result<(Arc<Pst>, String), String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    Ok((
        Arc::new(Pst::decode(&bytes).map_err(|e| e.to_string())?),
        format!("{:x}", Sha256::digest(&bytes)),
    ))
}
fn run(args: Arguments) -> Result<(), String> {
    validate_budgets(&args.teacher_nodes)?;
    let (candidate, candidate_sha256) = decode_pst(&args.candidate_pst)?;
    let (teacher, teacher_sha256) = decode_pst(&args.teacher_pst)?;
    // ハッシュ対象と探索入力を同じバイト列に固定する。
    let positions = fs::read(&args.positions).map_err(|e| e.to_string())?;
    let metadata = Metadata {
        candidate_sha256,
        teacher_sha256,
        positions_sha256: format!("{:x}", Sha256::digest(&positions)),
    };
    let mut reader = Reader::new(Cursor::new(positions)).map_err(|e| e.to_string())?;
    let codes = parse_rule_set("engine-default").map_err(|e| e.to_string())?;
    let rules = Rules::from_codes(&codes).map_err(|e| e.to_string())?;
    let input_codes = parse_rule_set(reader.header().rule_set()).map_err(|e| e.to_string())?;
    if Rules::from_codes(&input_codes).map_err(|e| e.to_string())? != rules {
        return Err("positions must use engine-default rules".into());
    }

    let mut output = io::stdout().lock();
    let mut index = 0;
    while let Some(record) = reader.read_record().map_err(|e| e.to_string())? {
        let game = Game::from_position(rules, record.to_position().map_err(|e| e.to_string())?);
        let legal = game.legal_moves();
        let mut candidates = Vec::new();
        let budget = args.proposal_nodes.get();
        let (fm_move, fm) = run_search(&game, &candidate, budget, args.hash_mb.get(), false)?;
        let (pst_move, pst) = run_search(&game, &teacher, budget, args.hash_mb.get(), false)?;
        include_candidate(&mut candidates, fm_move, &legal)?;
        include_candidate(&mut candidates, pst_move, &legal)?;
        let mut teacher_roots = Vec::new();
        for budget in &args.teacher_nodes {
            let (mv, search) =
                run_search(&game, &teacher, budget.get(), args.hash_mb.get(), false)?;
            include_candidate(&mut candidates, mv, &legal)?;
            teacher_roots.push(TeacherRoot {
                budget: budget.get(),
                search,
            });
        }
        let mut children = Vec::new();
        for &mv in &candidates {
            let mut child = game.clone();
            child.play(mv).map_err(|e| e.to_string())?;
            for budget in &args.teacher_nodes {
                let search = match child.result() {
                    Some(result) => Search {
                        best_move: None,
                        score: terminal_score(result, game.position().side_to_move()),
                        depth: 0,
                        nodes: 0,
                        elapsed_ms: 0.0,
                        pv: Vec::new(),
                        stop_reason: "Terminal".into(),
                    },
                    None => run_search(&child, &teacher, budget.get(), args.hash_mb.get(), true)?.1,
                };
                children.push(Child {
                    r#move: move_text(&game, mv)?,
                    budget: budget.get(),
                    search,
                });
            }
        }
        let row = Row {
            metadata: metadata.clone(),
            index,
            game_number: record.game_number(),
            ply: record.ply(),
            history_available: false,
            proposal_nodes: budget,
            teacher_nodes: args.teacher_nodes.iter().map(|n| n.get()).collect(),
            proposals: Proposals { fm, pst },
            teacher_roots,
            candidates: candidates
                .iter()
                .map(|&mv| move_text(&game, mv))
                .collect::<Result<_, _>>()?,
            children,
        };
        serde_json::to_writer(&mut output, &row).map_err(|e| e.to_string())?;
        writeln!(output).map_err(|e| e.to_string())?;
        output.flush().map_err(|e| e.to_string())?;
        index += 1;
    }
    Ok(())
}
fn main() -> ExitCode {
    match run(Arguments::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use minase::{DrawReason, WinReason};
    #[test]
    fn child_scores_use_parent_sign_and_separate_mate_and_incomplete() {
        assert_eq!(searched_score(-135, 8, true), Score::Cp { value: 135 });
        assert_eq!(searched_score(135, 8, false), Score::Cp { value: 135 });
        assert_eq!(
            searched_score(29999, 8, true),
            Score::Mate { value: -29998 }
        );
        assert_eq!(
            searched_score(135, 0, true),
            Score::Incomplete { value: -135 }
        );
    }
    #[test]
    fn terminal_results_use_parent_color_without_numeric_cp() {
        let win = GameResult::Win {
            winner: Color::White,
            reason: WinReason::Mate,
        };
        assert!(matches!(
            terminal_score(win, Color::White),
            Score::Terminal { outcome: "win", .. }
        ));
        assert!(matches!(
            terminal_score(win, Color::Black),
            Score::Terminal {
                outcome: "loss",
                ..
            }
        ));
        assert!(matches!(
            terminal_score(
                GameResult::Draw {
                    reason: DrawReason::Repetition
                },
                Color::Black
            ),
            Score::Terminal {
                outcome: "draw",
                ..
            }
        ));
    }
    #[test]
    fn candidate_union_is_unique_and_rejects_illegal_moves() {
        let codes = parse_rule_set("engine-default").unwrap();
        let game = Game::new(Rules::from_codes(&codes).unwrap());
        let legal = game.legal_moves();
        let mut selected = Vec::new();
        for mv in [legal[0], legal[1], legal[0], legal[1]] {
            include_candidate(&mut selected, mv, &legal).unwrap()
        }
        assert_eq!(selected, vec![legal[0], legal[1]]);
        let mut illegal = legal[0];
        illegal.to = illegal.from;
        assert!(include_candidate(&mut selected, illegal, &legal).is_err());
        assert_eq!(selected.len(), 2);
    }
    #[test]
    fn budgets_must_be_positive_unique_and_increasing() {
        let n = |x| NonZeroU64::new(x).unwrap();
        assert!(validate_budgets(&[]).is_err());
        assert!(validate_budgets(&[n(100), n(1000)]).is_ok());
        assert!(validate_budgets(&[n(100), n(100)]).is_err());
        assert!(validate_budgets(&[n(1000), n(100)]).is_err());
        assert!(
            Arguments::try_parse_from([
                "probe",
                "--positions",
                "x",
                "--candidate-pst",
                "y",
                "--teacher-pst",
                "z",
                "--proposal-nodes",
                "0",
                "--teacher-nodes",
                "100",
                "--hash-mb",
                "64"
            ])
            .is_err()
        );
    }
}
