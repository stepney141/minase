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

// mcts-debugging-tools.md「検証」9の契約をプロトコルの出力で検査する。
fn verbose_protocol() -> UsiProtocol {
    let mut protocol = UsiProtocol::with_searcher(Searcher::Mcts(Engine::ready_for_test()));
    let (result, output) = run(
        &mut protocol,
        "setoption name RuleSet value JCSA\nsetoption name VerboseMoveStats value true\nposition startpos\n",
    );
    result.unwrap();
    assert!(output.is_empty());
    protocol
}

fn stats_field<'a>(line: &'a str, name: &str) -> &'a str {
    let mut words = line.split_whitespace();
    while let Some(word) = words.next() {
        if word == name {
            return words.next().unwrap();
        }
    }
    panic!("missing {name}: {line}");
}

fn stats_root(output: &str) -> &str {
    let roots: Vec<_> = output
        .lines()
        .filter(|line| line.starts_with("info string movestats root "))
        .collect();
    assert_eq!(roots.len(), 1, "{output}");
    roots[0]
}

fn stats_moves(output: &str) -> Vec<&str> {
    output
        .lines()
        .filter(|line| line.starts_with("info string movestats move "))
        .collect()
}

fn stats_bestmove(output: &str) -> &str {
    let best: Vec<_> = output
        .lines()
        .filter_map(|line| line.strip_prefix("bestmove "))
        .collect();
    assert_eq!(best.len(), 1, "{output}");
    best[0].split_whitespace().next().unwrap()
}

fn assert_stats_order(output: &str, game: &Game) {
    assert!(!output.contains("info string error:"), "{output}");
    let root = stats_root(output);
    let moves = stats_moves(output);
    let expected: HashSet<_> = game
        .legal_moves()
        .into_iter()
        .map(|mv| usi::text_generated(game.position(), mv))
        .collect();
    assert_eq!(moves.len(), expected.len());
    let actual: HashSet<_> = moves
        .iter()
        .map(|line| stats_field(line, "move").to_owned())
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(stats_field(moves[0], "move"), stats_bestmove(output));
    let lines: Vec<_> = output.lines().collect();
    let final_info = lines
        .iter()
        .rposition(|line| line.starts_with("info depth "))
        .unwrap();
    let root_index = lines.iter().position(|line| *line == root).unwrap();
    assert_eq!(root_index, final_info + 1);
    assert_eq!(&lines[root_index + 1..root_index + 1 + moves.len()], moves);
    let stop = root_index + 1 + moves.len();
    assert!(lines[stop].starts_with("info string stop "));
    assert!(lines[stop + 1].starts_with("bestmove "));
    assert_eq!(stop + 2, lines.len());
    assert_eq!(
        stats_field(root, "elapsed-ms"),
        stats_field(lines[final_info], "time")
    );
}

#[test]
fn verbose_movestats_handshake_and_check_values() {
    let mut protocol = verbose_protocol();
    let (result, output) = run(&mut protocol, "usi\n");
    result.unwrap();
    let lines: Vec<_> = output.lines().collect();
    let temperature = lines
        .iter()
        .position(|line| line.starts_with("option name PolicyTemperature "))
        .unwrap();
    assert_eq!(
        lines[temperature + 1],
        "option name VerboseMoveStats type check default false"
    );
    for value in ["true", "TRUE", "false", "FaLsE"] {
        let (result, output) = run(
            &mut protocol,
            &format!(
                "setoption name VerboseMoveStats value {value}\nposition startpos\ngo nodes 1\n"
            ),
        );
        result.unwrap();
        assert!(!output.contains("error:"));
        assert_eq!(
            output.contains("info string movestats root "),
            value.eq_ignore_ascii_case("true")
        );
        assert_legal_bestmove(&output, &Game::new(Rules::JCSA));
    }
    // 拒否した入力は直前の有効な設定を変えない。
    for value in ["", " value", " value yes", " value 1", " value 真"] {
        let (result, output) = run(
            &mut protocol,
            &format!(
                "setoption name VerboseMoveStats value true\nsetoption name VerboseMoveStats{value}\nposition startpos\ngo nodes 1\n"
            ),
        );
        result.unwrap();
        let error = if matches!(value, "" | " value") {
            "info string error: VerboseMoveStats requires a value"
        } else {
            "info string error: VerboseMoveStats must be true or false"
        };
        assert_eq!(output.lines().next(), Some(error));
        stats_root(&output);
    }
    let mut alpha = UsiProtocol::new();
    let (result, output) = run(&mut alpha, "usi\n");
    result.unwrap();
    assert!(!output.contains("VerboseMoveStats"));
    let (result, output) = run(&mut alpha, "setoption name VerboseMoveStats value true\n");
    result.unwrap();
    assert!(output.is_empty());
}

#[test]
fn verbose_movestats_natural_finish_and_default_disabled() {
    let mut protocol = UsiProtocol::with_searcher(Searcher::Mcts(Engine::ready_for_test()));
    let (result, output) = run(
        &mut protocol,
        "setoption name RuleSet value JCSA\nposition startpos\ngo nodes 5\n",
    );
    result.unwrap();
    assert!(!output.contains("movestats"));
    assert_legal_bestmove(&output, &Game::new(Rules::JCSA));
    let (result, output) = run(&mut verbose_protocol(), "go nodes 5\n");
    result.unwrap();
    assert_stats_order(&output, &Game::new(Rules::JCSA));
    let root = stats_root(&output);
    assert_eq!(stats_field(root, "soft-ms"), "-");
    assert_eq!(stats_field(root, "hard-ms"), "-");
}

#[test]
fn verbose_movestats_nodes_one_uses_tempered_softmax() {
    use crate::nn::test_util::NonUniformEvaluator;
    use crate::search::mcts::{Config, test_support::SCALE};
    let engine = Engine::with_evaluator(Some(Box::new(NonUniformEvaluator)), Config::new(SCALE));
    let mut protocol = UsiProtocol::with_searcher(Searcher::Mcts(engine));
    let (result, output) = run(
        &mut protocol,
        "setoption name RuleSet value JCSA\nsetoption name VerboseMoveStats value true\nsetoption name PolicyTemperature value 140\nposition startpos\ngo nodes 1\n",
    );
    result.unwrap();
    let game = Game::new(Rules::JCSA);
    assert_stats_order(&output, &game);
    let root = stats_root(&output);
    for (field, expected) in [("simulations", "1"), ("reused", "0"), ("visits", "0")] {
        assert_eq!(stats_field(root, field), expected);
    }
    let legal = game.legal_moves();
    let leading = usi::text_generated(game.position(), legal[0]);
    // logitを温度Tで割ってsoftmaxを取ると、先頭の重みはexp(ln(4)/T)、他は1になる。
    let leading_weight = (4.0_f64.ln() / 1.4).exp();
    let total = leading_weight + (legal.len() - 1) as f64;
    for row in stats_moves(&output) {
        assert_eq!(stats_field(row, "visits"), "0");
        assert_eq!(stats_field(row, "q"), "-");
        let weight = if stats_field(row, "move") == leading {
            leading_weight
        } else {
            1.0
        };
        let expected = weight / total;
        let prior = stats_field(row, "prior").parse::<f64>().unwrap();
        assert!((prior - expected).abs() <= expected * 1e-6, "{row}");
    }
}

#[test]
fn verbose_movestats_reuses_same_root_and_selected_subtree_across_toggle() {
    let mut protocol = verbose_protocol();
    let (result, first) = run(&mut protocol, "go nodes 5\n");
    result.unwrap();
    assert_eq!(stats_field(stats_root(&first), "reused"), "0");
    // 設定の切り替えで木を捨てない契約も、次の探索の持ち越しから確かめる。
    let (result, second) = run(
        &mut protocol,
        "setoption name VerboseMoveStats value false\nsetoption name VerboseMoveStats value true\nposition startpos\ngo nodes 5\n",
    );
    result.unwrap();
    assert_eq!(stats_field(stats_root(&second), "reused"), "4");
    assert_eq!(stats_field(stats_root(&second), "simulations"), "5");

    let mut protocol = verbose_protocol();
    let (result, first) = run(&mut protocol, "go nodes 300\n");
    result.unwrap();
    assert_stats_order(&first, &Game::new(Rules::JCSA));
    let mv = stats_bestmove(&first);
    let visits = stats_field(stats_moves(&first)[0], "visits")
        .parse::<u64>()
        .unwrap();
    // 展開済みの子への辺の訪問は、子の初回評価1回と子の辺の訪問の和である。
    assert!(visits > 1, "subtree reuse must be exercised");
    let (result, second) = run(
        &mut protocol,
        &format!("position startpos moves {mv}\ngo nodes 5\n"),
    );
    result.unwrap();
    assert_eq!(
        stats_field(stats_root(&second), "reused")
            .parse::<u64>()
            .unwrap(),
        visits - 1
    );
    assert_eq!(stats_field(stats_root(&second), "simulations"), "5");
}

fn without_stats_and_timing(output: &str) -> Vec<Vec<String>> {
    output
        .lines()
        .filter(|line| !line.starts_with("info string movestats "))
        .map(|line| {
            let mut words = line.split_whitespace();
            let mut result = Vec::new();
            while let Some(word) = words.next() {
                if line.starts_with("info depth ") && matches!(word, "time" | "nps") {
                    words.next().unwrap();
                } else {
                    result.push(word.to_owned());
                }
            }
            result
        })
        .collect()
}

#[test]
fn verbose_movestats_preserves_search_output_and_following_search() {
    let mut sessions = Vec::new();
    for enabled in [false, true] {
        let mut protocol = verbose_protocol();
        let (result, first) = run(
            &mut protocol,
            &format!(
                "setoption name VerboseMoveStats value {enabled}\nposition startpos\ngo nodes 5\n"
            ),
        );
        result.unwrap();
        assert_eq!(first.contains("movestats"), enabled);
        let mv = stats_bestmove(&first);
        let (result, second) = run(
            &mut protocol,
            &format!("position startpos moves {mv}\ngo nodes 5\n"),
        );
        result.unwrap();
        assert_eq!(second.contains("movestats"), enabled);
        sessions.push((
            mv.to_owned(),
            without_stats_and_timing(&first),
            without_stats_and_timing(&second),
        ));
    }
    assert_eq!(sessions[0], sessions[1]);
}

// 既存の保留結果の試験と同様、完了イベントを明示的に消費して遷移を確定させる。
fn complete_stats_search(
    protocol: &mut UsiProtocol,
    active: &mut Option<crate::protocol::usi::ActiveSearch>,
    output: &mut Vec<u8>,
) {
    use crate::protocol::usi::ActiveSearch;
    let Some(ActiveSearch::Running { handle, .. }) = active.as_ref() else {
        panic!("search must be running")
    };
    let event = handle
        .events()
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert!(matches!(event, crate::search::SearchEvent::Finished { .. }));
    protocol.handle_search_event(active, event, output).unwrap();
}

#[test]
fn verbose_movestats_running_stop_and_infinite_stop_emit_once() {
    for args in [vec!["nodes", "4294967295"], vec!["infinite"]] {
        let mut protocol = verbose_protocol();
        let mut output = Vec::new();
        let mut active = protocol.start_go(&args, &mut output).unwrap();
        assert!(matches!(
            active,
            Some(crate::protocol::usi::ActiveSearch::Running { .. })
        ));
        protocol
            .handle_searching_line(
                &mut active,
                &mut std::collections::VecDeque::new(),
                "stop",
                &mut output,
            )
            .unwrap();
        assert!(active.is_none());
        let output = String::from_utf8(output).unwrap();
        assert_stats_order(&output, &Game::new(Rules::JCSA));
        assert!(output.contains("info string stop external\n"));
        let root = stats_root(&output);
        assert_eq!(stats_field(root, "soft-ms"), "-");
        assert_eq!(stats_field(root, "hard-ms"), "-");
    }
}

#[test]
fn verbose_movestats_completed_ponder_emits_before_release_once() {
    use crate::protocol::usi::ActiveSearch;
    for command in ["ponderhit", "stop", "gameover win", "quit", "eof"] {
        let mut protocol = verbose_protocol();
        let mut output = Vec::new();
        let mut active = protocol
            .start_go(&["ponder", "nodes", "5"], &mut output)
            .unwrap();
        complete_stats_search(&mut protocol, &mut active, &mut output);
        assert!(matches!(active, Some(ActiveSearch::AwaitingStop { .. })));
        let held = String::from_utf8(output.clone()).unwrap();
        stats_root(&held);
        assert_eq!(
            stats_moves(&held).len(),
            Game::new(Rules::JCSA).legal_moves().len()
        );
        assert!(!held.contains("bestmove "));
        assert!(!held.contains("info string stop "));
        let before = output.len();
        if command == "eof" {
            protocol.discard_search(&mut active).unwrap();
        } else {
            protocol
                .handle_searching_line(
                    &mut active,
                    &mut std::collections::VecDeque::new(),
                    command,
                    &mut output,
                )
                .unwrap();
        }
        assert!(active.is_none());
        if matches!(command, "ponderhit" | "stop") {
            assert_stats_order(
                std::str::from_utf8(&output).unwrap(),
                &Game::new(Rules::JCSA),
            );
        } else {
            assert_eq!(output.len(), before);
        }
    }
}

#[test]
fn verbose_movestats_running_ponderhit_and_clock_budget() {
    let mut protocol = verbose_protocol();
    let mut output = Vec::new();
    // 先読み中は時計を使わず、的中してから1 msの予算が有効になる。
    let mut active = protocol
        .start_go(
            &["ponder", "btime", "0", "wtime", "0", "byoyomi", "31"],
            &mut output,
        )
        .unwrap();
    protocol
        .handle_searching_line(
            &mut active,
            &mut std::collections::VecDeque::new(),
            "ponderhit",
            &mut output,
        )
        .unwrap();
    assert!(matches!(
        active,
        Some(crate::protocol::usi::ActiveSearch::Running { .. })
    ));
    complete_stats_search(&mut protocol, &mut active, &mut output);
    assert!(active.is_none());
    let output = String::from_utf8(output).unwrap();
    assert_stats_order(&output, &Game::new(Rules::JCSA));
    let root = stats_root(&output);
    assert_eq!(stats_field(root, "soft-ms").parse::<u64>().unwrap(), 1);
    assert_eq!(stats_field(root, "hard-ms").parse::<u64>().unwrap(), 1);
}

#[test]
fn verbose_movestats_discard_running_search_emits_nothing() {
    for tail in ["gameover win\n", "quit\n", ""] {
        let mut protocol = verbose_protocol();
        let (result, output) = run(&mut protocol, &format!("go infinite\n{tail}"));
        result.unwrap();
        assert!(!output.contains("movestats"), "{output}");
        assert!(!output.contains("bestmove"), "{output}");
        assert!(!output.contains("info depth"), "{output}");
    }
}
