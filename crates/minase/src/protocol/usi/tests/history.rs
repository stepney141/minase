//! history表の対局内持ち越しとUSIライフサイクルの検査。
//! 理由は`docs/adr/carry-histories-and-correction-histories-within-a-game.md`にある。

use super::*;
use crate::search::history_decay_for_test;
use std::sync::atomic::Ordering as AtomicOrdering;

const CORRECTION_INDEX: usize = 17;

fn setup() -> (Engine, UsiProtocol) {
    let mut engine = make_engine(&[RuleCode::R1]);
    let mut protocol = UsiProtocol::new(&engine);
    run(
        &mut protocol,
        &mut engine,
        "setoption name USI_Hash value 1\nposition startpos\n",
    );
    (engine, protocol)
}

fn seed(protocol: &mut UsiProtocol) {
    let histories = protocol.searcher.as_mut().unwrap().histories_mut_for_test();
    histories.correction.values()[CORRECTION_INDEX].store(321, AtomicOrdering::Relaxed);
    for (worker, history) in histories.continuation_correction.iter_mut().enumerate() {
        history.store(CORRECTION_INDEX, worker as i16 + 400);
    }
    for (worker, history) in histories.capture.iter_mut().enumerate() {
        history.values_mut().fill(worker as i32 + 100);
    }
    for (worker, history) in histories.workers.iter_mut().enumerate() {
        history[0][60][60] = 160 + worker as i32 * 16;
        history[1][10][11] = -7;
    }
}

fn assert_seed(protocol: &UsiProtocol, command: &str) {
    let histories = protocol.searcher.as_ref().unwrap().histories_for_test();
    assert_eq!(histories.workers.len(), 4, "{command}");
    assert_eq!(histories.capture.len(), 4, "{command}");
    assert_eq!(
        histories.correction.values()[CORRECTION_INDEX].load(AtomicOrdering::Relaxed),
        321,
        "{command}: position correction history"
    );
    for (worker, history) in histories.continuation_correction.iter().enumerate() {
        assert_eq!(
            i32::from(history.values()[CORRECTION_INDEX]),
            worker as i32 + 400,
            "{command}: continuation correction worker {worker}"
        );
    }
    for (worker, history) in histories.capture.iter().enumerate() {
        assert!(
            history
                .values()
                .iter()
                .all(|&value| value == worker as i32 + 100),
            "{command}: capture worker {worker}"
        );
    }
    for (worker, history) in protocol
        .searcher
        .as_ref()
        .unwrap()
        .histories_for_test()
        .workers
        .iter()
        .enumerate()
    {
        assert_eq!(
            history[0][60][60],
            160 + worker as i32 * 16,
            "{command}: quiet worker {worker}"
        );
        assert_eq!(history[1][10][11], -7, "{command}: quiet worker {worker}");
    }
}

#[test]
fn history_carry_usi_preserves_rejected_settings_and_unchanged_threads() {
    let (mut engine, mut protocol) = setup();
    run(
        &mut protocol,
        &mut engine,
        "setoption name Threads value 4\n",
    );
    seed(&mut protocol);
    for command in [
        "setoption name RuleSet value XX9",
        "setoption name RuleSet value L1,E1",
        "setoption name Threads value 0",
        "setoption name Threads value 257",
        "setoption name Threads value nope",
        "setoption name USI_Hash value 0",
    ] {
        let output = run(&mut protocol, &mut engine, command);
        assert!(output.contains("info string error:"), "{command}: {output}");
        assert_seed(&protocol, command);
    }
    for command in [
        "setoption name Threads value 4",
        "setoption name DrawValueBlack value 0",
        "setoption name DrawValueWhite value 0",
        "position startpos",
        "isready",
    ] {
        run(&mut protocol, &mut engine, command);
        assert_seed(&protocol, command);
    }
}

#[test]
fn history_carry_usi_clears_new_games_accepted_settings_and_changed_threads() {
    for (command, workers) in [
        ("usinewgame", 4),
        ("setoption name RuleSet value L0,P0,R2,E0", 4),
        ("setoption name Threads value 2", 2),
        ("setoption name USI_Hash value 1", 4),
        ("setoption name USI_Hash value 2", 4),
        ("setoption name DrawValueBlack value -50", 4),
        ("setoption name DrawValueWhite value 50", 4),
    ] {
        let (mut engine, mut protocol) = setup();
        run(
            &mut protocol,
            &mut engine,
            "setoption name Threads value 4\n",
        );
        seed(&mut protocol);
        let output = run(&mut protocol, &mut engine, command);
        assert!(!output.contains("error"), "{command}: {output}");
        let histories = protocol.searcher.as_ref().unwrap().histories_for_test();
        assert_eq!(histories.workers.len(), workers, "{command}");
        assert_eq!(histories.capture.len(), workers, "{command}");
        assert_eq!(
            histories.correction.values()[CORRECTION_INDEX].load(AtomicOrdering::Relaxed),
            -5,
            "{command}: position correction history"
        );
        assert!(
            histories
                .continuation_correction
                .iter()
                .all(|history| history.values()[CORRECTION_INDEX] == 5),
            "{command}: continuation correction history"
        );
        assert!(
            histories
                .capture
                .iter()
                .all(|history| history.values().iter().all(|&value| value == -742)),
            "{command}: capture history"
        );
        assert!(
            histories.workers.iter().all(|history| history
                .iter()
                .flatten()
                .flatten()
                .all(|&v| v == -5)),
            "{command}: quiet history"
        );
    }
}

// 的中・不的中の両方で、実行中と結果保留中の回収経路を通す。
#[test]
fn history_carry_usi_returns_ponder_history_on_hit_and_miss() {
    // 返却値は開始時の履歴を1回だけ減衰させた商である。
    for held in [false, true] {
        for hit in [false, true] {
            let (mut engine, mut protocol) = setup();
            run(
                &mut protocol,
                &mut engine,
                "setoption name Threads value 4\n",
            );
            seed(&mut protocol);
            let mut output = Vec::new();
            let mut active = protocol
                .start_go(&engine, &["ponder", "depth", "3"], &mut output)
                .unwrap();
            if held {
                while matches!(active, Some(ActiveSearch::Running { .. })) {
                    protocol
                        .wait_search_event(&engine, &mut active, &mut output)
                        .unwrap();
                }
                assert!(matches!(active, Some(ActiveSearch::AwaitingStop { .. })));
            }
            if hit {
                protocol
                    .ponderhit(&engine, &mut active, &mut output)
                    .unwrap();
                if active.is_some() {
                    protocol
                        .finish_search(&engine, &mut active, &mut output, false)
                        .unwrap();
                }
            } else {
                protocol.discard_search(&mut active).unwrap();
            }
            assert!(active.is_none());
            let histories = protocol.searcher.as_ref().unwrap().histories_for_test();
            assert_eq!(histories.workers.len(), 4);
            for (worker, history) in histories.workers.iter().enumerate() {
                let numerator = (160 + worker as i32 * 16) * history_decay_for_test();
                let residual = numerator - history[0][60][60] * 100;
                assert!((0..100).contains(&residual), "held={held}, hit={hit}");
            }
        }
    }
}
