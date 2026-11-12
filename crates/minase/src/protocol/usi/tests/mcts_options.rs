//! alphazero-mcts.mdのUSIの契約を、ONNX Runtimeなしで検査する。

use crate::nn::Network;
use crate::protocol::UsiProtocol;
use crate::search::mcts::test_support::SCALE;
use crate::search::mcts::{Config, Engine};
use crate::search::{PrepareError, Searcher};
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
        &lines[4..9],
        &[
            "option name USI_Hash type spin default 256 min 1 max 65536",
            "option name Threads type spin default 1 min 1 max 1",
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
        "info string error: invalid NnBackend value 'vulkan'\ninfo string error: NnBatchSize must be an integer from 1 to 1024\ninfo string error: NnBatchSize must be an integer from 1 to 1024\ninfo string error: Threads must be an integer from 1 to 1\nreadyok\n"
    );
}

#[test]
fn empty_tensorrt_cache_ends_session_without_readyok_or_loading_runtime() {
    let mut searcher = Engine::new(Network::from_bytes(Vec::new()), Config::new(SCALE));
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
