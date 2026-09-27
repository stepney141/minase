//! strength-stage12.md「項目5」「検証」のUSIライフサイクル。

use super::*;

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
    for (worker, history) in protocol
        .histories
        .as_mut()
        .unwrap()
        .workers
        .iter_mut()
        .enumerate()
    {
        history[0][60][60] = 160 + worker as i32 * 16;
        history[1][10][11] = -7;
    }
}

fn assert_seed(protocol: &UsiProtocol) {
    for (worker, history) in protocol
        .histories
        .as_ref()
        .unwrap()
        .workers
        .iter()
        .enumerate()
    {
        assert_eq!(history[0][60][60], 160 + worker as i32 * 16);
        assert_eq!(history[1][10][11], -7);
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
        assert_seed(&protocol);
    }
    for command in [
        "setoption name Threads value 4",
        "position startpos",
        "isready",
    ] {
        run(&mut protocol, &mut engine, command);
        assert_seed(&protocol);
    }
}

#[test]
fn history_carry_usi_clears_new_games_accepted_rules_and_changed_threads() {
    for command in [
        "usinewgame",
        "setoption name RuleSet value L0,P0,R2,E0",
        "setoption name Threads value 2",
        "setoption name USI_Hash value 1",
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
        let histories = protocol.histories.as_ref().unwrap();
        assert_eq!(
            histories.workers.len(),
            if command.ends_with('2') { 2 } else { 4 }
        );
        assert!(
            histories.workers.iter().all(|history| history
                .iter()
                .flatten()
                .flatten()
                .all(|&v| v == 0))
        );
    }
}

#[test]
fn history_carry_usi_keeps_history_between_go_commands() {
    let (mut engine, mut protocol) = setup();
    seed(&mut protocol);
    for expected in [120, 90] {
        let mut output = Vec::new();
        let mut active = protocol
            .start_go(&engine, &["depth", "3"], &mut output)
            .unwrap();
        assert!(active.is_some());
        assert!(protocol.histories.is_none());
        protocol
            .finish_search(&engine, &mut active, &mut output, false)
            .unwrap();
        assert!(active.is_none());
        assert_eq!(
            protocol.histories.as_ref().unwrap().workers[0][0][60][60],
            expected
        );
        run(&mut protocol, &mut engine, "position startpos\n");
    }
}

// 的中・不的中の両方で、実行中と結果保留中の回収経路を通す。
#[test]
fn history_carry_usi_returns_ponder_history_on_hit_and_miss() {
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
            let histories = protocol.histories.as_ref().unwrap();
            assert_eq!(histories.workers.len(), 4);
            for (worker, history) in histories.workers.iter().enumerate() {
                assert_eq!(
                    history[0][60][60],
                    120 + worker as i32 * 12,
                    "held={held}, hit={hit}"
                );
            }
        }
    }
}
