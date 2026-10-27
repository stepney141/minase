//! alphazero-mcts.mdの起動設定とUSIの契約を、ONNX Runtimeなしで検査する。

use super::*;
use crate::nn::UniformEvaluator;
use crate::protocol::{Engine as GameEngine, UsiProtocol};
use crate::search::mcts::test_support::{SCALE, initial, nodes};
use minase_core::rules::parse_rule_set;
use std::io;
use std::sync::mpsc;

fn ready_engine() -> Engine {
    let mut engine = Engine::with_evaluator(Some(Box::new(UniformEvaluator)), Config::new(SCALE));
    engine.config.capacity_bytes = 1024 * 1024;
    engine.cache_dir = PathBuf::new();
    engine.default_cache_dir = PathBuf::new();
    engine
}

fn session(searcher: Engine) -> (GameEngine, UsiProtocol) {
    let engine = GameEngine::new(parse_rule_set("engine-default").unwrap()).unwrap();
    let protocol = UsiProtocol::with_searcher(&engine, Searcher::Mcts(searcher));
    (engine, protocol)
}

fn run(
    engine: &mut GameEngine,
    protocol: &mut UsiProtocol,
    input: &str,
) -> (io::Result<()>, String) {
    let (sender, receiver) = mpsc::channel();
    for line in input.lines() {
        sender.send(Ok(line.to_owned())).unwrap();
    }
    drop(sender);
    let mut output = Vec::new();
    let result = protocol.run_channel(engine, &receiver, &mut output);
    (result, String::from_utf8(output).unwrap())
}

#[test]
fn cache_default_uses_only_nonempty_xdg_cache_home() {
    for (input, expected) in [
        (None, ""),
        (Some(""), ""),
        (Some("/cache"), "/cache/minase/tensorrt"),
        (
            Some("/cache with spaces/"),
            "/cache with spaces/minase/tensorrt",
        ),
    ] {
        assert_eq!(
            default_cache_dir(input.map(OsStr::new)),
            PathBuf::from(expected)
        );
    }
}

#[test]
fn mcts_handshake_declares_options_in_order_and_ignores_tuning_names() {
    let (mut game, mut protocol) = session(ready_engine());
    let (result, output) = run(
        &mut game,
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

#[test]
fn inference_options_accept_boundaries_and_map_backends_without_fallback() {
    let mut engine = ready_engine();
    for (value, expected) in [("CuDa", "cuda"), ("CPU", "cpu"), ("TensorRT", "tensorrt")] {
        engine
            .set_option("NnBackend", SearchOptionValue::Combo(value.to_owned()))
            .unwrap();
        engine
            .set_option(
                "NnCacheDir",
                SearchOptionValue::String("/cache with spaces".to_owned()),
            )
            .unwrap();
        assert_eq!(engine.backend, expected);
        let provider = engine.execution_provider().unwrap();
        assert_eq!(provider.name(), expected);
        if let ExecutionProvider::TensorRt { cache_root } = provider {
            assert_eq!(cache_root, PathBuf::from("/cache with spaces"));
        }
    }
    for size in [1, 1024] {
        engine
            .set_option("NnBatchSize", SearchOptionValue::Spin(size))
            .unwrap();
        assert_eq!(engine.config.batch_size.get(), size);
    }
    for size in [0, 1025, usize::MAX] {
        assert!(matches!(
            engine.set_option("NnBatchSize", SearchOptionValue::Spin(size)),
            Err(SetOptionError::OutOfRange { .. })
        ));
        assert_eq!(engine.config.batch_size.get(), 1024);
    }
    assert!(matches!(
        engine.set_option("NnBackend", SearchOptionValue::Combo("vulkan".to_owned())),
        Err(SetOptionError::InvalidValue { .. })
    ));
    assert_eq!(engine.backend, "tensorrt");
}

#[test]
fn changed_inference_options_require_preparation_again_but_identical_values_do_not() {
    for (name, same, changed) in [
        (
            "NnBackend",
            SearchOptionValue::Combo("TENSORRT".to_owned()),
            SearchOptionValue::Combo("cpu".to_owned()),
        ),
        (
            "NnCacheDir",
            SearchOptionValue::String(String::new()),
            SearchOptionValue::String("/cache".to_owned()),
        ),
        (
            "NnBatchSize",
            SearchOptionValue::Spin(256),
            SearchOptionValue::Spin(1),
        ),
    ] {
        let mut engine = ready_engine();
        engine.set_option(name, same).unwrap();
        engine.prepare().unwrap(); // 注入済みの推論器を保持しており、Runtimeは読まない。
        assert!(engine.evaluator.is_some(), "{name}");
        engine.set_option(name, changed).unwrap();
        assert!(
            matches!(
                engine.prepare_driver(initial(), nodes(1)),
                Err(Error::MissingEvaluator)
            ),
            "{name}"
        );
    }
}

#[test]
fn hash_capacity_change_discards_tree_and_keeps_evaluator() {
    let mut engine = ready_engine();
    engine.prepare_driver(initial(), nodes(1)).unwrap();
    assert!(engine.driver.is_some());
    engine
        .set_option("USI_Hash", SearchOptionValue::Spin(2))
        .unwrap();
    assert!(engine.driver.is_none());
    assert_eq!(engine.config.capacity_bytes, 2 * 1024 * 1024);
    assert!(engine.evaluator.is_some());
}

// 同値の設定で推論器を維持できることを利用し、USIから渡された文字列そのものを検査する。
#[test]
fn usi_preserves_cache_paths_with_spaces_and_normalizes_empty_and_backend_case() {
    for (path, value) in [
        ("/cache with spaces", "/cache   with\tspaces"),
        ("", "<empty>"),
    ] {
        let mut searcher = ready_engine();
        searcher.cache_dir = PathBuf::from(path);
        let (mut game, mut protocol) = session(searcher);
        let (result, output) = run(
            &mut game,
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
    let (mut game, mut protocol) = session(ready_engine());
    let (result, output) = run(
        &mut game,
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
    searcher.cache_dir = PathBuf::from("/initial cache");
    let (mut game, mut protocol) = session(searcher);
    let (result, output) = run(
        &mut game,
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
    let mut searcher = ready_engine();
    searcher.evaluator = None;
    let (mut game, mut protocol) = session(searcher);
    let (result, output) = run(&mut game, &mut protocol, "position startpos\ngo nodes 1\n");
    result.unwrap();
    assert_eq!(
        output,
        "info string error: MCTS requires isready before go\n"
    );

    let (mut game, mut protocol) = session(ready_engine());
    let (result, output) = run(
        &mut game,
        &mut protocol,
        "tt\nposition startpos\ntt\ngo depth 1\n",
    );
    result.unwrap();
    assert_eq!(
        output,
        "info string error: tt requires --search alphabeta\ninfo string error: tt requires --search alphabeta\ninfo string error: MCTS does not support a depth limit\n"
    );
}

#[test]
fn every_started_mcts_go_announces_simulation_counts_before_search_output() {
    let (mut game, mut protocol) = session(ready_engine());
    for _ in 0..2 {
        let (result, output) = run(&mut game, &mut protocol, "position startpos\ngo nodes 1\n");
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
