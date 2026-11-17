//! 外部ネットの準備、識別子、再設定、およびプロトコルの失敗契約。
use minase::eval::nnue::{NetworkFileError, read_network_file};
use minase::protocol::{CecpProtocol, Protocol, UsiProtocol};
use minase::search::{PrepareError, Searcher};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Cursor},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

const NETWORK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/nets/nnue-epoch0.bin");
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "minase-network-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn run(protocol: &mut dyn Protocol, input: &str) -> (io::Result<()>, String) {
    let mut output = Vec::new();
    let result = protocol.run(&mut Cursor::new(input), &mut output);
    (result, String::from_utf8(output).unwrap())
}
fn ready(bytes: &[u8]) -> String {
    format!(
        "info string network_sha256 {:x}\nreadyok\n",
        Sha256::digest(bytes)
    )
}

#[test]
fn preparation_rejects_missing_unreadable_and_invalid_networks_with_typed_causes() {
    let dir = Directory::new();
    let absent = dir.0.join("absent.bin");
    let empty = dir.0.join("empty.bin");
    let truncated = dir.0.join("truncated.bin");
    fs::write(&empty, []).unwrap();
    fs::write(&truncated, &fs::read(NETWORK).unwrap()[..192]).unwrap();
    for (path, expected) in [
        (None, "required option EvalFile is not set".to_owned()),
        (
            Some(&absent),
            format!("failed to read NNUE network file '{}':", absent.display()),
        ),
        (
            Some(&empty),
            format!("invalid NNUE network file '{}':", empty.display()),
        ),
        (
            Some(&truncated),
            format!("invalid NNUE network file '{}':", truncated.display()),
        ),
    ] {
        for cecp in [false, true] {
            let mut protocol: Box<dyn Protocol> = if cecp {
                Box::new(CecpProtocol::new())
            } else {
                Box::new(UsiProtocol::new())
            };
            let mut commands = if cecp {
                "option RuleSet=JCSA\nmemory 1\n"
            } else {
                "setoption name RuleSet value JCSA\nsetoption name USI_Hash value 1\n"
            }
            .to_owned();
            if let Some(path) = path {
                commands.push_str(&if cecp {
                    format!("option EvalFile={}\n", path.display())
                } else {
                    format!("setoption name EvalFile value {}\n", path.display())
                });
            }
            commands.push_str(if cecp {
                "new\nping 1\n"
            } else {
                "isready\nusi\n"
            });
            let (result, output) = run(protocol.as_mut(), &commands);
            let error = result.unwrap_err();
            let cause = error
                .get_ref()
                .unwrap()
                .downcast_ref::<PrepareError>()
                .unwrap();
            assert!(
                output.starts_with(&format!(
                    "{} {expected}",
                    if cecp {
                        "tellusererror"
                    } else {
                        "info string error:"
                    }
                )),
                "{output}"
            );
            assert!(!output.contains("readyok") && !output.contains("pong"));
            match path {
                None => assert!(matches!(cause, PrepareError::MissingEvalFile)),
                Some(p) if p == &absent => assert!(matches!(
                    cause,
                    PrepareError::Nnue(NetworkFileError::Read { .. })
                )),
                _ => assert!(matches!(
                    cause,
                    PrepareError::Nnue(NetworkFileError::Decode { .. })
                )),
            }
        }
    }
}

#[test]
fn same_path_reset_discards_network_and_hashes_the_entire_replacement() {
    let dir = Directory::new();
    let path = dir.0.join("network with spaces.bin");
    let mut bytes = fs::read(NETWORK).unwrap();
    fs::write(&path, &bytes).unwrap();
    let mut protocol = UsiProtocol::new();
    let setup = format!(
        "setoption name RuleSet value JCSA\nsetoption name USI_Hash value 1\nsetoption name EvalFile value {}\nisready\n",
        path.display()
    );
    let (result, output) = run(&mut protocol, &setup);
    result.unwrap();
    assert_eq!(output, ready(&bytes));
    // MNUE版5の勝率尺度だけを変える。本体のチェックサムは同じでも識別子は変わる。
    let old = read_network_file(&path).unwrap();
    bytes[108..112].copy_from_slice(&1234.0_f32.to_le_bytes());
    fs::write(&path, &bytes).unwrap();
    let new = read_network_file(&path).unwrap();
    assert_eq!(old.network().checksum(), new.network().checksum());
    assert_ne!(old.sha256(), new.sha256());
    let (result, output) = run(
        &mut protocol,
        &format!(
            "setoption name EvalFile value {}\nposition startpos\nbench depth 1\neval\ngo nodes 1\nisready\n",
            path.display()
        ),
    );
    result.unwrap();
    assert_eq!(
        output,
        format!(
            "info string error: bench requires isready\ninfo string error: eval requires isready\ninfo string error: go requires isready\n{}",
            ready(&bytes)
        )
    );
}

#[test]
fn alpha_beta_commands_require_preparation_and_cecp_new_prepares() {
    let mut protocol = UsiProtocol::new();
    let (result, output) = run(
        &mut protocol,
        "setoption name RuleSet value JCSA\nposition startpos\nbench depth 1\neval\ngo nodes 1\n",
    );
    result.unwrap();
    assert_eq!(
        output,
        "info string error: bench requires isready\ninfo string error: eval requires isready\ninfo string error: go requires isready\n"
    );
    let mut protocol = CecpProtocol::new();
    let (result, output) = run(
        &mut protocol,
        &format!("option EvalFile={NETWORK}\noption RuleSet=JCSA\nmemory 1\nnew\nping 1\n"),
    );
    result.unwrap();
    assert_eq!(output, "pong 1\n");
}

#[test]
fn cecp_prepares_at_setboard_go_and_after_changing_network_before_usermove() {
    let dir = Directory::new();
    let missing = dir.0.join("missing.bin");
    let board = minase_core::notation::sfen::to_sfen(&minase_core::Position::initial());
    let board = board.split_whitespace().next().unwrap();
    for commands in [
        format!(
            "option RuleSet=JCSA\noption EvalFile={}\nsd 1\nsetboard {board} w\ngo\n",
            missing.display()
        ),
        format!(
            "option RuleSet=JCSA\noption EvalFile={NETWORK}\nmemory 1\nsd 1\nnew\noption EvalFile={}\nusermove g4g5\n",
            missing.display()
        ),
    ] {
        let (result, output) = run(&mut CecpProtocol::new(), &commands);
        assert!(result.is_err(), "{output}");
        assert!(
            output.starts_with("tellusererror failed to read NNUE network file"),
            "{output}"
        );
        assert!(!output.contains("move "));
    }
}

#[test]
fn mcts_validates_scale_before_reading_model() {
    for value in [
        None,
        Some("0"),
        Some("-1"),
        Some("nan"),
        Some("inf"),
        Some("abc"),
    ] {
        let mut protocol = UsiProtocol::with_searcher(Searcher::mcts());
        let mut commands =
            "setoption name RuleSet value JCSA\nsetoption name NnModel value /missing/model.onnx\n"
                .to_owned();
        let expected = match value {
            None => "required option NnCpPerLogit is not set".to_owned(),
            Some(value) => {
                commands.push_str(&format!("setoption name NnCpPerLogit value {value}\n"));
                format!("NnCpPerLogit must be a positive finite number: '{value}'")
            }
        };
        commands.push_str("isready\n");
        let (result, output) = run(&mut protocol, &commands);
        assert!(result.is_err());
        assert_eq!(output, format!("info string error: {expected}\n"));
    }
}
