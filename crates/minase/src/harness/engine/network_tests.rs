//! ネットの指定とチェックサムの契約を、USIを話す子プロセスで検査する。

use super::{EngineProcess, InitializationError, probe_engine_defaults, probe_usi_options};
use crate::harness::*;
use minase_core::{Color, Game, Rules};
use std::num::NonZeroU64;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);
const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn player(script: String) -> PlayerConfig {
    let mut player = resolve_player(
        parse_player_spec("sh").unwrap(),
        parse_search_limit("nodes=1").unwrap(),
        None,
        "JCSA",
        Vec::new(),
        None,
    )
    .unwrap();
    player.args = vec!["-c".to_owned(), script];
    player
}

fn reporting(lines: &str) -> PlayerConfig {
    let mut config = player(
        r#"
while IFS= read -r line; do
    case "$line" in
        usi) printf 'option name Threads type spin default 1 min 1 max 1\nusiok\n' ;;
        isready) printf '%s\n' "$1"; echo readyok ;;
        go*) echo 'bestmove resign' ;;
    esac
done
"#
        .to_owned(),
    );
    config
        .args
        .extend(["checksum-test".to_owned(), lines.to_owned()]);
    config
}

// 初期化の順序は外部との契約なので、事前と局ごとの両方で受信列を検査する。
#[test]
fn probe_and_game_start_send_the_same_initialization_sequence_with_optional_hash() {
    for hash in [None, Some(8)] {
        let mut expected = vec!["setoption name RuleSet value JCSA".to_owned()];
        if let Some(hash) = hash {
            expected.push(format!("setoption name USI_Hash value {hash}"));
        }
        expected.extend([
            "setoption name ResignValue value 99999".to_owned(),
            "setoption name NnModel value path with spaces.onnx".to_owned(),
            "setoption name NnBatchSize value 7".to_owned(),
            "isready".to_owned(),
        ]);
        let mut script = r#"
IFS= read -r line
[ "$line" = usi ] || exit 11
printf 'option name Threads type spin default 1 min 1 max 1\noption name USI_Hash type spin default 256 min 1 max 65536\noption name NnModel type string default <empty>\noption name NnBatchSize type spin default 256 min 1 max 1024\nusiok\n'
"#.to_owned();
        for line in expected {
            script.push_str(&format!(
                "IFS= read -r line\n[ \"$line\" = '{line}' ] || exit 12\n"
            ));
        }
        script.push_str(&format!(
            "printf 'info string network_sha256 {HASH}\\nreadyok\\n'\n"
        ));
        script.push_str(
            "while IFS= read -r line; do [ \"$line\" = usinewgame ] && echo initialized; done\n",
        );
        let mut config = player(script);
        config.hash_mb = hash;
        config.options = vec![
            ("NnModel".into(), "path with spaces.onnx".into()),
            ("NnBatchSize".into(), "7".into()),
        ];
        let defaults = probe_engine_defaults(&config, true, TIMEOUT).unwrap();
        assert_eq!(defaults.threads, Some(1));
        assert_eq!(defaults.hash_mb, Some(256));
        assert_eq!(
            defaults.network_sha256,
            Some(HASH.to_owned().try_into().unwrap())
        );
        config.network_sha256 = defaults.network_sha256;
        let process = EngineProcess::start(&config, 1, TIMEOUT).unwrap();
        process.receive_until(|line| line == "initialized").unwrap();
    }
}

#[test]
fn initialization_rejects_duplicate_and_malformed_checksums_including_uppercase() {
    let invalid = [
        "a".repeat(63),
        "a".repeat(65),
        "g".repeat(64),
        "A".repeat(64),
        String::new(),
        format!("{HASH} extra"),
    ];
    for value in invalid {
        let config = reporting(&format!("info string network_sha256 {value}"));
        let error = probe_engine_defaults(&config, true, TIMEOUT).unwrap_err();
        assert!(matches!(
            error
                .get_ref()
                .unwrap()
                .downcast_ref::<InitializationError>(),
            Some(InitializationError::Network(NetworkError::Invalid { .. }))
        ));
        assert!(matches!(
            EngineProcess::start(&config, 1, TIMEOUT).err().unwrap(),
            InitializationError::Network(NetworkError::Invalid { .. })
        ));
    }
    for second in [HASH, "invalid"] {
        let config = reporting(&format!(
            "info string network_sha256 {HASH}\ninfo string network_sha256 {second}"
        ));
        assert!(matches!(
            EngineProcess::start(&config, 1, TIMEOUT).err().unwrap(),
            InitializationError::Network(NetworkError::Duplicate { .. })
        ));
        let error = probe_engine_defaults(&config, true, TIMEOUT).unwrap_err();
        assert!(matches!(
            error
                .get_ref()
                .unwrap()
                .downcast_ref::<InitializationError>(),
            Some(InitializationError::Network(NetworkError::Duplicate { .. }))
        ));
    }
}

#[test]
fn only_successful_readyok_can_mean_no_checksum_and_spsa_probe_stops_at_handshake() {
    assert_eq!(
        probe_engine_defaults(&reporting("info string loading"), true, TIMEOUT)
            .unwrap()
            .network_sha256,
        None
    );
    let config = player(
        r#"
while IFS= read -r line; do
    case "$line" in
        usi) echo usiok ;;
        isready) exit 23 ;;
    esac
done
"#
        .to_owned(),
    );
    assert!(probe_engine_defaults(&config, true, TIMEOUT).is_err());
    assert!(
        probe_usi_options(&config.path, &config.args, TIMEOUT)
            .unwrap()
            .is_empty()
    );
    for (is_random, protocol) in [(true, Protocol::Usi), (false, Protocol::Cecp)] {
        let mut config = config.clone();
        config.path = "/nonexistent-engine".into();
        config.is_random = is_random;
        config.protocol = protocol;
        assert_eq!(
            probe_engine_defaults(&config, true, TIMEOUT)
                .unwrap()
                .network_sha256,
            None
        );
    }
}

#[test]
fn play_game_returns_network_errors_instead_of_forfeits_for_both_sides_and_null_changes() {
    for (recorded, received) in [
        (Some(HASH.to_owned()), Some("f".repeat(64))),
        (None, Some(HASH.to_owned())),
        (Some(HASH.to_owned()), None),
    ] {
        for side in [EngineSide::Candidate, EngineSide::Baseline] {
            let lines = received
                .as_ref()
                .map(|value| format!("info string network_sha256 {value}"));
            let mut changed = reporting(lines.as_deref().unwrap_or(""));
            changed.network_sha256 = recorded.clone().map(|value| value.try_into().unwrap());
            let other = reporting("");
            let (candidate, baseline) = match side {
                EngineSide::Candidate => (&changed, &other),
                EngineSide::Baseline => (&other, &changed),
            };
            let result = play_game(
                Game::new(Rules::JCSA),
                Vec::new(),
                Vec::new(),
                100,
                Color::Black,
                candidate,
                NonZeroU64::new(1).unwrap(),
                baseline,
                NonZeroU64::new(2).unwrap(),
                TIMEOUT,
                false,
                &AtomicBool::new(false),
            );
            let error = result.err().unwrap();
            assert_eq!(error.side, side);
            assert!(matches!(error.source, NetworkError::Mismatch { .. }));
            let text = error.to_string();
            assert!(text.contains(&side.to_string()));
            assert!(text.contains(&format!(
                "recorded {}",
                recorded.as_deref().unwrap_or("null")
            )));
            assert!(text.contains(&format!(
                "received {}",
                received.as_deref().unwrap_or("null")
            )));
        }
    }
    for lines in [
        "info string network_sha256 wrong".to_owned(),
        format!("info string network_sha256 {HASH}\ninfo string network_sha256 {HASH}"),
    ] {
        let config = reporting(&lines);
        let result = play_game(
            Game::new(Rules::JCSA),
            Vec::new(),
            Vec::new(),
            100,
            Color::Black,
            &config,
            NonZeroU64::new(1).unwrap(),
            &reporting(""),
            NonZeroU64::new(2).unwrap(),
            TIMEOUT,
            false,
            &AtomicBool::new(false),
        );
        assert!(matches!(
            result,
            Err(MatchEngineError {
                side: EngineSide::Candidate,
                ..
            })
        ));
    }
}

#[test]
fn second_game_checksum_mismatch_aborts_the_pair() {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let counter = std::env::temp_dir().join(format!(
        "minase-pair-network-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&counter, "0\n").unwrap();
    let mut config = player(
        r#"
while IFS= read -r line; do
    case "$line" in
        usi) echo usiok ;;
        isready)
            IFS= read -r count < "$1"
            count=$((count + 1))
            printf '%s\n' "$count" > "$1"
            if [ "$count" = 1 ]; then value="$2"; else value="$3"; fi
            printf 'info string network_sha256 %s\nreadyok\n' "$value" ;;
        go*) echo 'bestmove resign' ;;
    esac
done
"#
        .to_owned(),
    );
    config.args.extend([
        "pair-test".into(),
        counter.to_str().unwrap().into(),
        HASH.into(),
        "f".repeat(64),
    ]);
    config.network_sha256 = Some(HASH.to_owned().try_into().unwrap());
    let result = run_pair(
        Rules::JCSA,
        "JCSA",
        20260811,
        1,
        100,
        &config,
        &reporting(""),
        TIMEOUT,
        false,
        &AtomicBool::new(false),
    );
    let count = std::fs::read_to_string(&counter).unwrap();
    std::fs::remove_file(counter).unwrap();
    assert_eq!(count, "2\n");
    assert!(matches!(
        result,
        Err(MatchEngineError {
            side: EngineSide::Candidate,
            source: NetworkError::Mismatch { .. }
        })
    ));
}
