//! 固定局面の診断とランダム対局を、USIの契約とコミット済みの局面集で検査する。

use super::*;
use crate::diagnostics::{nn_eval, nn_fixture, test_support::TempDir};
use crate::nn::{Evaluator, NnError, Query};
use crate::search::mcts::{Config, Engine as MctsEngine};
use std::error::Error;
use std::fs;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

const FIXTURES: &str = include_str!("../../../../tests/fixtures/nn-positions.jsonl");

fn assert_ended(output: &str) {
    assert!(!output.contains("error:"), "{output}");
    assert!(output.lines().all(|line| line.starts_with("info string ")));
    assert_eq!(output.lines().last(), Some("info string end"));
}

#[test]
fn nn_fixture_works_with_mcts_without_rules_or_preparation_and_overwrites() {
    let directory = TempDir::new();
    let path = directory.path().join("fixture out with spaces.jsonl");
    fs::write(&path, "old contents").unwrap();
    let mut protocol = UsiProtocol::with_searcher(Searcher::mcts().unwrap());
    let command = format!("nn-fixture out {}\n", path.display()).replace("out with", "out   with");
    let output = run(&mut protocol, &command);
    assert_eq!(
        output,
        format!(
            "info string nn-fixture wrote 288 positions to {}\ninfo string end\n",
            path.display()
        )
    );
    assert_eq!(fs::read(path).unwrap(), FIXTURES.as_bytes());
    assert!(protocol.engine().is_err());
}

#[test]
fn nn_fixture_write_failure_keeps_path_and_source_and_ends_session() {
    let directory = TempDir::new();
    let path = directory.path().join("absent/positions.jsonl");
    let mut output = Vec::new();
    let error = UsiProtocol::new()
        .run(
            &mut Cursor::new(format!("nn-fixture out {}\nusi\n", path.display())),
            &mut output,
        )
        .unwrap_err();
    let error = error
        .get_ref()
        .unwrap()
        .downcast_ref::<nn_fixture::Error>()
        .unwrap();
    assert!(matches!(error, nn_fixture::Error::Write { path: actual, .. } if actual == &path));
    assert_eq!(
        error
            .source()
            .unwrap()
            .downcast_ref::<io::Error>()
            .unwrap()
            .kind(),
        io::ErrorKind::NotFound
    );
    let output = String::from_utf8(output).unwrap();
    assert_eq!(output.lines().count(), 1);
    assert!(output.starts_with("info string error: failed to write NN fixtures to "));
}

#[test]
fn nn_eval_requires_mcts_and_preparation_without_creating_files() {
    let directory = TempDir::new();
    let path = directory.path().join("evaluation.json");
    for (searcher, message) in [
        (Searcher::alpha_beta(), "nn-eval requires --search mcts"),
        (Searcher::mcts().unwrap(), "nn-eval requires isready"),
    ] {
        let output = run(
            &mut UsiProtocol::with_searcher(searcher),
            &format!("nn-eval out {}\nusi\n", path.display()),
        );
        assert!(
            output.starts_with(&format!("info string error: {message}\n")),
            "{output}"
        );
        assert_eq!(error_lines(&output).len(), 1);
        assert!(!output.contains("info string end"));
        assert!(output.ends_with("usiok\n"));
        assert!(!path.exists());
    }
}

/// 各依頼と各合法手で異なる値を返し、JSONの順序とバッチ境界を観測する。
struct RecordingEvaluator {
    batches: Arc<Mutex<Vec<usize>>>,
    next_position: usize,
}
impl Evaluator for RecordingEvaluator {
    fn evaluate(
        &mut self,
        queries: &[Query<'_>],
        logits: &mut Vec<f32>,
        values: &mut Vec<f32>,
    ) -> Result<(), NnError> {
        self.batches.lock().unwrap().push(queries.len());
        logits.clear();
        values.clear();
        for query in queries {
            let index = self.next_position;
            values.push(index as f32 / 512.0);
            logits.extend(
                (0..query.moves.len()).map(|move_index| index as f32 + move_index as f32 / 1024.0),
            );
            self.next_position += 1;
        }
        Ok(())
    }
}

#[test]
fn nn_eval_uses_prepared_inference_and_preserves_fixture_and_batch_order() {
    for batch in [1, 7, 256] {
        let directory = TempDir::new();
        let path = directory.path().join("evaluation with spaces.json");
        fs::write(&path, "old contents").unwrap();
        let batches = Arc::new(Mutex::new(Vec::new()));
        let evaluator = RecordingEvaluator {
            batches: Arc::clone(&batches),
            next_position: 0,
        };
        let engine = MctsEngine::with_evaluator(
            Some(Box::new(evaluator)),
            Config {
                batch_size: NonZeroUsize::new(batch).unwrap(),
                ..Config::new(600.0)
            },
        );
        let mut protocol = UsiProtocol::with_searcher(Searcher::Mcts(engine));
        // 同値の設定は準備済み推論器を保持する。対局の規則は固定局面集へ影響しない。
        let setup = run(
            &mut protocol,
            &format!(
                "setoption name RuleSet value L0,P1,R1,E0\nsetoption name NnBatchSize value {batch}\nsetoption name NnBackend value tensorrt\nisready\nposition startpos\n"
            ),
        );
        assert!(!setup.contains("error:"), "{setup}");
        let before = run(&mut protocol, "state\nd\n");
        let output = run(&mut protocol, &format!("nn-eval out {}\n", path.display()));
        assert_ended(&output);
        assert_eq!(output.lines().count(), 2);
        assert!(output.starts_with("info string NN inference (288 positions): "));
        assert_eq!(run(&mut protocol, "state\nd\n"), before);
        let bytes = fs::read(&path).unwrap();
        assert_eq!(bytes.last(), Some(&b'\n'));
        let text = std::str::from_utf8(&bytes).unwrap();
        let keys = [
            "network_sha256",
            "backend",
            "batch",
            "positions",
            "sfen",
            "value",
            "moves",
        ]
        .map(|key| text.find(&format!("\"{key}\":")).unwrap());
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            value["network_sha256"],
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert!(setup.contains(value["network_sha256"].as_str().unwrap()));
        assert_eq!(value["backend"], "tensorrt");
        assert_eq!(value["batch"], batch);
        let positions = value["positions"].as_array().unwrap();
        assert_eq!(positions.len(), 288);
        for (index, (actual, fixture)) in positions.iter().zip(FIXTURES.lines()).enumerate() {
            let fixture: serde_json::Value = serde_json::from_str(fixture).unwrap();
            assert_eq!(actual["sfen"], fixture["sfen"]);
            assert_eq!(
                serde_json::from_value::<f32>(actual["value"].clone()).unwrap(),
                index as f32 / 512.0
            );
            let moves = actual["moves"].as_array().unwrap();
            let expected = fixture["moves"].as_array().unwrap();
            assert_eq!(moves.len(), expected.len());
            for (move_index, (mv, expected)) in moves.iter().zip(expected).enumerate() {
                assert_eq!(mv[0], expected[4]);
                assert_eq!(
                    serde_json::from_value::<f32>(mv[1].clone()).unwrap(),
                    index as f32 + move_index as f32 / 1024.0
                );
            }
        }
        let expected_batches = match batch {
            1 => vec![1; 288],
            7 => [vec![7; 41], vec![1]].concat(),
            256 => vec![256, 32],
            _ => unreachable!(),
        };
        assert_eq!(*batches.lock().unwrap(), expected_batches);
        fs::remove_file(&path).unwrap();
        let output = run(
            &mut protocol,
            &format!(
                "setoption name NnModel value changed.onnx\nnn-eval out {}\nusi\n",
                path.display()
            ),
        );
        assert!(output.starts_with("info string error: nn-eval requires isready\n"));
        assert!(output.ends_with("usiok\n"));
        assert!(!path.exists());
    }
}

struct FailingEvaluator;
impl Evaluator for FailingEvaluator {
    fn evaluate(
        &mut self,
        _: &[Query<'_>],
        _: &mut Vec<f32>,
        _: &mut Vec<f32>,
    ) -> Result<(), NnError> {
        Err(NnError::RuntimeLibraryPathUnset)
    }
}

#[test]
fn nn_eval_inference_failure_keeps_source_and_ends_session() {
    let directory = TempDir::new();
    let path = directory.path().join("evaluation.json");
    let engine = MctsEngine::with_evaluator(Some(Box::new(FailingEvaluator)), Config::new(600.0));
    let mut protocol = UsiProtocol::with_searcher(Searcher::Mcts(engine));
    let mut output = Vec::new();
    let error = protocol
        .run(
            &mut Cursor::new(format!("nn-eval out {}\nusi\n", path.display())),
            &mut output,
        )
        .unwrap_err();
    assert!(matches!(
        error.get_ref().unwrap().downcast_ref::<nn_eval::Error>(),
        Some(nn_eval::Error::Evaluation(
            search::EvaluationError::Inference(NnError::RuntimeLibraryPathUnset)
        ))
    ));
    let output = String::from_utf8(output).unwrap();
    assert_eq!(output.lines().count(), 1);
    assert!(output.starts_with("info string error: failed to evaluate NN fixtures: "));
    assert!(!path.exists());
}

fn without_elapsed(output: &str) -> Vec<&str> {
    output
        .lines()
        .filter(|line| !line.starts_with("info string elapsed:"))
        .collect()
}

#[test]
fn random_play_reports_three_games_and_reproduces_each_game() {
    let mut protocol = UsiProtocol::new();
    assert_eq!(
        run(&mut protocol, "setoption name RuleSet value JCSA\n"),
        ""
    );
    let first = run(&mut protocol, "random-play games 3 seed 7\n");
    assert_ended(&first);
    assert!(first.starts_with("info string rules: L0,P0,R1,E0\ninfo string seed: 7\n"));
    assert_eq!(
        first
            .lines()
            .filter(|line| line.starts_with("info string game "))
            .count(),
        3
    );
    assert!(first.contains("info string summary: games=3\n"));
    assert_eq!(
        without_elapsed(&first),
        without_elapsed(&run(&mut protocol, "random-play seed 7 games 3\n"))
    );
    let second = run(&mut protocol, "random-play seed 7 game 2\n");
    assert_ended(&second);
    assert_eq!(
        first
            .lines()
            .find(|line| line.starts_with("info string game 2:")),
        second
            .lines()
            .find(|line| line.starts_with("info string game 2:"))
    );
}

#[test]
fn random_play_uses_last_accepted_rules_without_changing_the_current_game() {
    let mut protocol = UsiProtocol::new();
    run(
        &mut protocol,
        "setoption name RuleSet value JCSA\nposition startpos moves 6i6h\nsetoption name RuleSet value lishogi\n",
    );
    let before = run(&mut protocol, "state\nd\nmoves\n");
    let output = run(
        &mut protocol,
        "random-play max-ply 2 seed 7 verbose verify-all\n",
    );
    assert_ended(&output);
    assert!(output.starts_with("info string rules: L1,L2,P0,P3,R1,E1,E3\n"));
    assert!(
        output.contains("info string game 1: plies=2 result=cutoff\ninfo string game 1 moves:\n")
    );
    assert_eq!(run(&mut protocol, "state\nd\nmoves\n"), before);
    assert!(run(&mut protocol, "setoption name RuleSet value lishogi,P1\n").contains("error:"));
    let rejected = run(
        &mut protocol,
        "random-play max-ply 2 seed 7 verbose verify-all\n",
    );
    assert_eq!(without_elapsed(&output), without_elapsed(&rejected));
}
