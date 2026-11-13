//! alphazero-mcts.mdのUSIの契約を、ONNX Runtimeなしで検査する。

use crate::protocol::{Protocol, UsiProtocol};
use crate::search::mcts::Engine;
use crate::search::{PrepareError, Searcher};
use minase_core::notation::{sfen::parse_extended_sfen, usi};
use minase_core::rules::{RuleCode, parse_rule_set};
use minase_core::{Game, Rules};
use std::collections::HashSet;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc;

fn run(protocol: &mut UsiProtocol, input: &str) -> (io::Result<()>, String) {
    let (sender, receiver) = mpsc::channel();
    for line in input.lines() {
        sender.send(Ok(line.to_owned())).unwrap();
    }
    drop(sender);
    let mut output = Vec::new();
    let result = protocol.run_channel(&receiver, &mut output);
    (result, String::from_utf8(output).unwrap())
}

#[test]
fn mcts_handshake_declares_options_in_order_and_ignores_tuning_names() {
    let mut protocol = UsiProtocol::with_searcher(Searcher::Mcts(Engine::ready_for_test()));
    let (result, output) = run(&mut protocol, "setoption name RuleSet value JCSA\n");
    result.unwrap();
    assert!(output.is_empty());
    let (result, output) = run(
        &mut protocol,
        "usi\nsetoption name Tune_DeltaMargin value nope\nsetoption name Tune_Unknown\n",
    );
    result.unwrap();
    let lines: Vec<_> = output.lines().collect();
    assert_eq!(
        &lines[4..10],
        &[
            "option name USI_Hash type spin default 256 min 1 max 65536",
            "option name Threads type spin default 1 min 1 max 1",
            "option name NnModel type string default <empty>",
            "option name NnBackend type combo default tensorrt var tensorrt var cuda var cpu",
            "option name NnCacheDir type string default <empty>",
            "option name NnBatchSize type spin default 256 min 1 max 1024",
        ]
    );
    assert_eq!(lines.last(), Some(&"usiok"));
    assert!(!output.contains("Tune_"));
    assert!(!output.contains("error:"));
}

// 同値の設定で推論器を維持できることを利用し、USIから渡された文字列そのものを検査する。
#[test]
fn usi_preserves_cache_paths_with_spaces_and_normalizes_empty_and_backend_case() {
    for (path, value) in [
        ("/cache with spaces", "/cache   with\tspaces"),
        ("", "<empty>"),
    ] {
        let mut searcher = Engine::ready_for_test();
        searcher.set_cache_dir_for_test(PathBuf::from(path));
        let mut protocol = UsiProtocol::with_searcher(Searcher::Mcts(searcher));
        let (result, output) = run(&mut protocol, "setoption name RuleSet value JCSA\n");
        result.unwrap();
        assert!(output.is_empty());
        let (result, output) = run(
            &mut protocol,
            &format!(
                "setoption name NnBackend value TeNsOrRt\nsetoption name NnCacheDir value {value}\nposition startpos\ngo nodes 1\n"
            ),
        );
        result.unwrap();
        assert!(!output.contains("error:"), "{output}");
        assert_eq!(
            output
                .lines()
                .filter(|line| line.starts_with("bestmove "))
                .count(),
            1
        );
    }
}

#[test]
fn usi_rejects_invalid_combo_and_spin_values_without_changing_settings() {
    let mut protocol = UsiProtocol::with_searcher(Searcher::Mcts(Engine::ready_for_test()));
    let (result, output) = run(&mut protocol, "setoption name RuleSet value JCSA\n");
    result.unwrap();
    assert!(output.is_empty());
    let (result, output) = run(
        &mut protocol,
        "setoption name NnBackend value vulkan\nsetoption name NnBatchSize value 0\nsetoption name NnBatchSize value 1025\nsetoption name Threads value 2\nisready\n",
    );
    result.unwrap();
    assert_eq!(
        output,
        "info string error: invalid NnBackend value 'vulkan'\ninfo string error: NnBatchSize must be an integer from 1 to 1024\ninfo string error: NnBatchSize must be an integer from 1 to 1024\ninfo string error: Threads must be an integer from 1 to 1\ninfo string network_sha256 e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\nreadyok\n"
    );
}

#[test]
fn empty_tensorrt_cache_ends_session_without_readyok_or_loading_runtime() {
    let mut searcher = Engine::ready_for_test();
    searcher.clear_evaluator_for_test();
    searcher.set_cache_dir_for_test(PathBuf::from("/initial cache"));
    let mut protocol = UsiProtocol::with_searcher(Searcher::Mcts(searcher));
    let (result, output) = run(&mut protocol, "setoption name RuleSet value JCSA\n");
    result.unwrap();
    assert!(output.is_empty());
    let (result, output) = run(
        &mut protocol,
        "setoption name NnCacheDir value <empty>\nisready\nusi\n",
    );
    assert!(matches!(
        result
            .unwrap_err()
            .get_ref()
            .unwrap()
            .downcast_ref::<PrepareError>(),
        Some(PrepareError::MissingCacheDir)
    ));
    assert_eq!(
        output,
        "info string error: NnCacheDir must be set for NnBackend tensorrt\n"
    );
}

#[test]
fn mcts_go_requires_isready_and_rejects_depth_and_tt() {
    let mut searcher = Engine::ready_for_test();
    searcher.clear_evaluator_for_test();
    let mut protocol = UsiProtocol::with_searcher(Searcher::Mcts(searcher));
    let (result, output) = run(&mut protocol, "setoption name RuleSet value JCSA\n");
    result.unwrap();
    assert!(output.is_empty());
    let (result, output) = run(&mut protocol, "position startpos\ngo nodes 1\n");
    result.unwrap();
    assert_eq!(
        output,
        "info string error: MCTS requires isready before go\n"
    );

    let mut protocol = UsiProtocol::with_searcher(Searcher::Mcts(Engine::ready_for_test()));
    let (result, output) = run(&mut protocol, "setoption name RuleSet value JCSA\n");
    result.unwrap();
    assert!(output.is_empty());
    let (result, output) = run(&mut protocol, "tt\nposition startpos\ntt\ngo depth 1\n");
    result.unwrap();
    assert_eq!(
        output,
        "info string error: tt requires --search alphabeta\ninfo string error: tt requires --search alphabeta\ninfo string error: MCTS does not support a depth limit\n"
    );
}

#[test]
fn every_started_mcts_go_announces_simulation_counts_before_search_output() {
    let mut protocol = UsiProtocol::with_searcher(Searcher::Mcts(Engine::ready_for_test()));
    let (result, output) = run(&mut protocol, "setoption name RuleSet value JCSA\n");
    result.unwrap();
    assert!(output.is_empty());
    for _ in 0..2 {
        let (result, output) = run(&mut protocol, "position startpos\ngo nodes 1\n");
        result.unwrap();
        let notice = "info string nodes count MCTS simulations, not alpha-beta move applications";
        assert_eq!(output.lines().next(), Some(notice));
        assert_eq!(output.lines().filter(|line| *line == notice).count(), 1);
        assert_eq!(
            output
                .lines()
                .filter(|line| line.starts_with("bestmove "))
                .count(),
            1
        );
    }
}

// cli-to-usi.md「推論のネットの指定」: 規則、ネットの順に必須条件を検査する。
#[test]
fn readiness_requires_rules_then_model_and_preserves_read_failure_sources() {
    use crate::nn::NnError;
    use std::error::Error;
    for (input, expected) in [
        ("isready\n", "required option RuleSet is not set"),
        (
            "setoption name RuleSet value JCSA\nisready\n",
            "required option NnModel is not set",
        ),
        (
            "setoption name RuleSet value JCSA\nsetoption name NnModel value <empty>\nisready\n",
            "required option NnModel is not set",
        ),
    ] {
        let mut protocol = UsiProtocol::with_searcher(Searcher::mcts().unwrap());
        let (result, output) = run(&mut protocol, input);
        assert!(result.is_err());
        assert_eq!(output, format!("info string error: {expected}\n"));
    }
    let mut protocol = UsiProtocol::with_searcher(Searcher::mcts().unwrap());
    let path = std::env::temp_dir().join(format!(
        "minase-missing-network-{}.onnx",
        std::process::id()
    ));
    assert!(!path.exists());
    let (result, output) = run(
        &mut protocol,
        &format!(
            "setoption name RuleSet value JCSA\nsetoption name NnModel value {}\nisready\nusi\n",
            path.display()
        ),
    );
    let error = result.unwrap_err();
    let error = error
        .get_ref()
        .unwrap()
        .downcast_ref::<PrepareError>()
        .unwrap();
    let cause = error.source().unwrap().downcast_ref::<NnError>().unwrap();
    assert!(matches!(cause, NnError::ReadNetwork { .. }));
    assert_eq!(
        cause
            .source()
            .unwrap()
            .downcast_ref::<io::Error>()
            .unwrap()
            .kind(),
        io::ErrorKind::NotFound
    );
    assert!(output.starts_with("info string error: cannot read network "));
    assert_eq!(output.lines().count(), 1);
}

#[test]
fn model_option_is_mcts_only_and_requires_a_value() {
    let mut protocol = UsiProtocol::new();
    let (result, output) = run(
        &mut protocol,
        "setoption name RuleSet value JCSA\nusi\nisready\n",
    );
    result.unwrap();
    assert!(!output.contains("NnModel"));
    assert!(!output.contains("network_sha256"));
    let mut protocol = UsiProtocol::with_searcher(Searcher::mcts().unwrap());
    let (result, output) = run(
        &mut protocol,
        "setoption name RuleSet value JCSA\nsetoption name NnModel\nsetoption name NnModel value\n",
    );
    result.unwrap();
    assert_eq!(
        output,
        "info string error: NnModel requires a value\n".repeat(2)
    );
}

#[test]
fn every_ready_response_reports_the_loaded_bytes_checksum_immediately_before_readyok() {
    let mut protocol = UsiProtocol::with_searcher(Searcher::Mcts(Engine::ready_for_test()));
    let (result, output) = run(
        &mut protocol,
        "setoption name RuleSet value JCSA\nisready\nisready\n",
    );
    result.unwrap();
    // 空のバイト列のSHA-256の既知の値。推論器の補助も同じ空のバイト列を保持する。
    assert_eq!(output, concat!(
        "info string network_sha256 e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n",
        "readyok\n").repeat(2));
}

#[test]
fn model_change_rejects_go_and_eval_until_prepared_and_same_value_preserves_them() {
    let mut protocol = UsiProtocol::with_searcher(Searcher::Mcts(Engine::ready_for_test()));
    let (result, output) = run(
        &mut protocol,
        "setoption name RuleSet value JCSA\nposition startpos\nsetoption name NnModel value test.onnx\neval\ngo nodes 1\n",
    );
    result.unwrap();
    assert!(!output.contains("error:"), "{output}");
    assert!(output.contains("info string end\n"));
    assert!(output.contains("bestmove "));
    let (result, output) = run(
        &mut protocol,
        "setoption name NnModel value other.onnx\ngo nodes 1\neval\nusi\n",
    );
    result.unwrap();
    assert!(output.starts_with("info string error: MCTS requires isready before go\ninfo string error: eval requires isready\n"));
    assert!(!output.contains("info string end"));
    assert!(output.ends_with("usiok\n"));
}

#[test]
fn eval_rejection_checks_game_state_before_preparation_and_keeps_session_open() {
    let mut protocol = UsiProtocol::with_searcher(Searcher::mcts().unwrap());
    let (result, output) = run(
        &mut protocol,
        "eval\nsetoption name RuleSet value JCSA\neval\nposition startpos\neval\nusi\n",
    );
    result.unwrap();
    assert!(output.starts_with(concat!(
        "info string error: required option RuleSet is not set\n",
        "info string error: eval requires an active or finished game\n",
        "info string error: eval requires isready\n"
    )));
    assert!(!output.contains("info string end"));
    assert!(output.ends_with("usiok\n"));
}

#[test]
fn eval_reports_all_current_legal_moves_with_raw_logits_and_tempered_priors() {
    use crate::nn::test_util::NonUniformEvaluator;
    use crate::search::mcts::Config;
    use crate::search::mcts::test_support::SCALE;
    use minase_core::{Game, Rules, notation::usi};
    for (rules_text, rules) in [("JCSA", Rules::JCSA), ("lishogi", Rules::LISHOGI)] {
        for (temperature, leading_weight) in [(100, 4.0_f64), (200, 2.0)] {
            let engine =
                Engine::with_evaluator(Some(Box::new(NonUniformEvaluator)), Config::new(SCALE));
            let mut protocol = UsiProtocol::with_searcher(Searcher::Mcts(engine));
            let (result, output) = run(
                &mut protocol,
                &format!(
                    "setoption name RuleSet value {rules_text}\nsetoption name PolicyTemperature value {temperature}\nposition startpos\neval\n"
                ),
            );
            result.unwrap();
            let game = Game::new(rules);
            let legal = game.legal_moves();
            let leading = usi::text_generated(game.position(), legal[0]);
            let mut expected_moves: Vec<_> = legal
                .iter()
                .map(|&mv| usi::text_generated(game.position(), mv))
                .collect();
            expected_moves.sort();
            let lines: Vec<_> = output.lines().collect();
            assert_eq!(lines.first(), Some(&"info string value 0.500000"));
            assert_eq!(lines.last(), Some(&"info string end"));
            assert_eq!(lines.len(), legal.len() + 2);
            let mut actual_moves = Vec::new();
            let mut rows = Vec::new();
            for line in &lines[1..lines.len() - 1] {
                let words: Vec<_> = line.split_whitespace().collect();
                assert_eq!(&words[..3], ["info", "string", "move"]);
                assert_eq!(words[4], "logit");
                assert_eq!(words[6], "prior");
                assert_eq!(words.len(), 8);
                for number in [words[5], words[7]] {
                    assert_eq!(number.split_once('.').unwrap().1.len(), 6);
                }
                let logit = words[5].parse::<f64>().unwrap();
                let prior = words[7].parse::<f64>().unwrap();
                let weight = if words[3] == leading {
                    leading_weight
                } else {
                    1.0
                };
                let expected_logit = if words[3] == leading {
                    4.0_f64.ln()
                } else {
                    0.0
                };
                assert!((logit - expected_logit).abs() <= 0.000001);
                assert!(
                    (prior - weight / (leading_weight + (legal.len() - 1) as f64)).abs()
                        <= 0.000001
                );
                actual_moves.push(words[3].to_owned());
                rows.push((prior, words[3]));
            }
            assert!(
                rows.windows(2).all(|pair| pair[0].0 > pair[1].0
                    || (pair[0].0 == pair[1].0 && pair[0].1 < pair[1].1))
            );
            assert!(
                (rows.iter().map(|row| row.0).sum::<f64>() - 1.0).abs()
                    <= legal.len() as f64 * 0.000001
            );
            actual_moves.sort();
            assert_eq!(actual_moves, expected_moves);
        }
    }
}

#[test]
fn every_rule_code_starts_an_mcts_search() {
    // USIの契約: 第29条〜第33条の全規則コードでevalとgoを受理する。
    let mut covered = HashSet::new();
    for rules in [
        "JCSA",
        "lishogi",
        "L1,L3,P0,P5,P6,R2,E1,E2",
        "L0,L4,P1,P4,R3,E0",
        "L0,L4,P2,P5,P6,R2,E1,E2",
        "L0,P1,R1,E0",
        "L0,P2,R1,E0",
        "L0,P0,P5,R1,E0",
    ] {
        let codes = parse_rule_set(rules).unwrap();
        covered.extend(codes.iter().copied());
        let game = Game::new(Rules::from_codes(&codes).unwrap());
        let mut protocol = UsiProtocol::with_searcher(Searcher::Mcts(Engine::ready_for_test()));
        let (result, output) = run(
            &mut protocol,
            &format!("setoption name RuleSet value {rules}\nposition startpos\neval\ngo nodes 8\n"),
        );
        result.unwrap();
        assert!(!output.contains("info string error"), "{rules}: {output}");
        assert!(output.lines().any(|line| line == "info string end"));
        assert_legal_bestmove(&output, &game);
    }
    assert_eq!(covered, HashSet::from(RuleCode::ALL));
}

fn assert_legal_bestmove(output: &str, game: &Game) {
    let moves: Vec<_> = output
        .lines()
        .filter_map(|line| line.strip_prefix("bestmove "))
        .map(|line| line.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(moves.len(), 1, "{output}");
    let legal: Vec<_> = game
        .legal_moves()
        .into_iter()
        .map(|mv| usi::text_generated(game.position(), mv))
        .collect();
    assert!(legal.iter().any(|mv| mv == moves[0]), "{output}");
}

fn eval_moves(output: &str) -> HashSet<String> {
    output
        .lines()
        .filter_map(|line| line.strip_prefix("info string move "))
        .map(|line| line.split_whitespace().next().unwrap().to_owned())
        .collect()
}

struct BrokenEvaluator {
    wrong_shape: bool,
}
impl crate::nn::Evaluator for BrokenEvaluator {
    fn evaluate(
        &mut self,
        _: &[crate::nn::Query<'_>],
        logits: &mut Vec<f32>,
        values: &mut Vec<f32>,
    ) -> Result<(), crate::nn::NnError> {
        logits.clear();
        values.clear();
        if self.wrong_shape {
            Ok(())
        } else {
            Err(crate::nn::NnError::MissingOutput { name: "policy" })
        }
    }
}

#[test]
fn eval_inference_failure_and_invalid_output_end_and_flush_the_session() {
    use crate::search::EvaluationError;
    use crate::search::mcts::Config;
    use crate::search::mcts::test_support::SCALE;
    struct Output {
        bytes: Vec<u8>,
        flushed: bool,
    }
    impl std::io::Write for Output {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(bytes);
            self.flushed = false;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.flushed = true;
            Ok(())
        }
    }
    for wrong_shape in [false, true] {
        let searcher = Engine::with_evaluator(
            Some(Box::new(BrokenEvaluator { wrong_shape })),
            Config::new(SCALE),
        );
        let mut protocol = UsiProtocol::with_searcher(Searcher::Mcts(searcher));
        let mut output = Output {
            bytes: Vec::new(),
            flushed: false,
        };
        let result = protocol.run(
            &mut io::Cursor::new(
                "setoption name RuleSet value JCSA\nposition startpos\neval\nusi\n",
            ),
            &mut output,
        );
        let error = result.unwrap_err();
        assert!(matches!(
            error.get_ref().unwrap().downcast_ref::<EvaluationError>(),
            Some(EvaluationError::Inference(_))
        ));
        assert!(output.flushed);
        let text = String::from_utf8(output.bytes).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(text.starts_with("info string error: "));
        assert!(!text.contains("info string end"));
    }
}

#[test]
fn eval_with_no_legal_moves_reports_only_value_and_end() {
    let mut protocol = UsiProtocol::with_searcher(Searcher::Mcts(Engine::ready_for_test()));
    let (result, output) = run(
        &mut protocol,
        "setoption name RuleSet value JCSA\nposition sfen k11/12/12/12/12/12/12/12/12/12/12/12 b - 1\neval\n",
    );
    result.unwrap();
    assert_eq!(output, "info string value 0.500000\ninfo string end\n");
}

#[test]
fn eval_reports_the_legal_moves_of_a_deferred_promotion_position() {
    use minase_core::Move;
    use minase_core::test_util::sq;

    let rules = Rules::from_codes(&parse_rule_set("L0,P1,R1,E0").unwrap()).unwrap();
    let setup = parse_extended_sfen(DEFERRED_SFEN, rules.moves).unwrap();
    let game = Game::from_position(rules, setup.restore_position().unwrap());
    let mut protocol = UsiProtocol::with_searcher(Searcher::Mcts(Engine::ready_for_test()));
    let (result, output) = run(
        &mut protocol,
        &format!(
            "setoption name RuleSet value L0,P1,R1,E0\nposition sfen {DEFERRED_SFEN}\neval\nusi\n"
        ),
    );
    result.unwrap();
    assert!(!output.contains("info string error"), "{output}");
    assert!(
        output
            .lines()
            .any(|line| line.starts_with("info string value "))
    );
    assert!(output.lines().any(|line| line == "info string end"));
    assert!(output.ends_with("usiok\n"));
    let actual = eval_moves(&output);
    let expected: HashSet<_> = game
        .legal_moves()
        .into_iter()
        .map(|mv| usi::text_generated(game.position(), mv))
        .collect();
    assert_eq!(actual, expected);
    // 第30条P1: 保留中の歩兵8cは、もう1回動くまで非捕獲の8bへの着手で成れない。
    assert!(game.position().promotion_deferred().contains(sq(4, 9)));
    let mv = Move {
        from: sq(4, 9),
        mid: None,
        to: sq(4, 10),
        promote: false,
    };
    assert!(actual.contains(&usi::text_generated(game.position(), mv)));
    assert!(!actual.contains(&usi::text_generated(
        game.position(),
        Move {
            promote: true,
            ..mv
        }
    )));
}

const DEFERRED_SFEN: &str = "k11/12/4P7/12/9g2/5+o3n2/12/9R2/12/12/12/11K b 7f 41 8c";

// USIの契約: 第33条第7項のHaChu既定規則で、保留のあるpositionからgoを実行する。
#[test]
fn hachu_default_rules_search_from_position_to_go() {
    use minase_core::Move;
    use minase_core::test_util::sq;

    let sfen = "k11/4p7/4P7/12/9g2/5+o3n2/12/9R2/12/12/12/11K b - 41 8c";
    let rules_text = "L1,L3,P0,P5,P6,R2,E1,E2";
    let rules = Rules::from_codes(&parse_rule_set(rules_text).unwrap()).unwrap();
    let setup = parse_extended_sfen(sfen, rules.moves).unwrap();
    let game = Game::from_position(rules, setup.restore_position().unwrap());
    assert!(game.position().promotion_deferred().contains(sq(4, 9)));
    let mut protocol = UsiProtocol::with_searcher(Searcher::Mcts(Engine::ready_for_test()));
    let (result, output) = run(
        &mut protocol,
        &format!(
            "setoption name RuleSet value {rules_text}\nposition sfen {sfen}\neval\ngo nodes 8\n"
        ),
    );
    result.unwrap();
    assert!(!output.contains("info string error"), "{output}");
    let actual = eval_moves(&output);
    let expected: HashSet<_> = game
        .legal_moves()
        .into_iter()
        .map(|mv| usi::text_generated(game.position(), mv))
        .collect();
    assert_eq!(actual, expected);
    // 第30条P5により、保留中の歩兵は最奥段ではない8bへの捕獲着手で成れない。
    let mv = Move {
        from: sq(4, 9),
        mid: None,
        to: sq(4, 10),
        promote: false,
    };
    assert!(actual.contains(&usi::text_generated(game.position(), mv)));
    assert!(!actual.contains(&usi::text_generated(
        game.position(),
        Move {
            promote: true,
            ..mv
        }
    )));
    assert_legal_bestmove(&output, &game);
}

#[test]
fn model_paths_join_value_words_with_one_space() {
    let mut protocol = UsiProtocol::with_searcher(Searcher::mcts().unwrap());
    let (result, output) = run(
        &mut protocol,
        "setoption name RuleSet value JCSA\nsetoption name NnModel value /nonexistent   path\twith spaces.onnx\nisready\n",
    );
    let error = result.unwrap_err();
    let Some(PrepareError::Network(crate::nn::NnError::ReadNetwork { path, .. })) =
        error.get_ref().unwrap().downcast_ref::<PrepareError>()
    else {
        panic!("{error}")
    };
    assert_eq!(path, &PathBuf::from("/nonexistent path with spaces.onnx"));
    assert_eq!(output.lines().count(), 1);
}
