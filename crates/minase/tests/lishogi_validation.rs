//! 固定の検証用データの入力契約、lishogi規則による照合とMNSDの保存を検証する。

use minase::training::records::{
    Exclusion, Origin, Outcome, Policy, Reader, TeacherKind, Value as TeacherValue,
};
use minase_core::{Color, Game, GameResult, Rules, WinReason, notation::usi};
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

const CASES: &str = include_str!("fixtures/lishogi_validation_cases.ndjson");
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "minase-lishogi-validation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn input(&self, games: &[Value]) {
        let bytes = games
            .iter()
            .map(|game| format!("{game}\n"))
            .collect::<String>();
        fs::write(self.path("input.ndjson"), bytes).unwrap();
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_minase"));
        command
            .args(["data", "lishogi", "validation", "--input"])
            .arg(self.path("input.ndjson"))
            .arg("--output")
            .arg(self.path("games.mnsd"))
            .arg("--report")
            .arg(self.path("report.json"));
        command
    }

    fn run(&self) -> Output {
        self.command()
            .args(["--seed", "20261113", "--allow-dirty"])
            .output()
            .unwrap()
    }

    fn report(&self) -> Value {
        serde_json::from_reader(File::open(self.path("report.json")).unwrap()).unwrap()
    }

    fn assert_no_output(&self) {
        assert!(!self.path("games.mnsd").exists());
        assert!(!self.path("report.json").exists());
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn fixtures() -> Vec<Value> {
    CASES
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn fixture(status: &str) -> Value {
    fixtures()
        .into_iter()
        .find(|game| game["status"] == status)
        .unwrap()
}

fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn expected_result(game: &Value) -> GameResult {
    GameResult::Win {
        winner: match game["winner"].as_str().unwrap() {
            "sente" => Color::Black,
            "gote" => Color::White,
            other => panic!("unexpected fixture winner {other}"),
        },
        reason: match game["status"].as_str().unwrap() {
            "resign" => WinReason::Resignation,
            "royalsLost" => WinReason::RoyalCapture,
            "bareKing" => WinReason::BareKing,
            other => panic!("unexpected fixture status {other}"),
        },
    }
}

#[test]
fn real_games_round_trip_with_lishogi_headers_moves_and_terminal_entries() {
    let dir = Directory::new();
    fs::write(dir.path("input.ndjson"), CASES).unwrap();
    let output = dir.run();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "accepted: 3\nillegal_move: 0\nfinished_early: 0\nresult_mismatch: 0\n"
    );
    success(output);
    let cases = fixtures();
    let expected_entries: usize = cases
        .iter()
        .map(|game| game["moves"].as_str().unwrap().split_whitespace().count() + 1)
        .sum();
    let mut reader = Reader::new(File::open(dir.path("games.mnsd")).unwrap()).unwrap();
    let header = reader.header();
    assert_eq!(header.rule_set(), Rules::LISHOGI.to_string());
    assert_eq!(header.rules(), Rules::LISHOGI);
    assert_eq!(header.teacher_kind(), TeacherKind::NoSearch);
    assert_eq!(header.network_checksum(), &[0; 32]);
    assert_eq!(header.teacher_search_budget(), 0);
    assert_eq!(header.seed(), 20261113);
    assert_eq!(header.game_count(), 3);
    assert_eq!(header.entry_count(), expected_entries as u64);
    let commit = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(commit.status.success());
    assert_eq!(
        header.generation_commit(),
        String::from_utf8(commit.stdout).unwrap().trim()
    );
    for (index, input) in cases.iter().enumerate() {
        let decoded = reader.read_game().unwrap().unwrap();
        let record = decoded.record();
        let mut game = Game::new(Rules::LISHOGI);
        assert_eq!(record.game_number(), index as u32 + 1);
        assert_eq!(record.origin(), Origin::HumanGame);
        assert_eq!(record.start_ply(), 0);
        assert_eq!(record.start_position(), game.position());
        assert_eq!(record.outcome(), Outcome::Game(expected_result(input)));
        let texts: Vec<_> = input["moves"]
            .as_str()
            .unwrap()
            .split_whitespace()
            .collect();
        assert_eq!(record.entries().len(), texts.len() + 1);
        for (entry, text) in record.entries().iter().zip(texts) {
            let mv = usi::parse(game.position(), text).unwrap();
            assert_eq!(entry.played_move(), Some(mv));
            assert_eq!(entry.policy(), &Policy::PlayedMove);
            game.play(mv).unwrap();
        }
        let last = record.entries().last().unwrap();
        assert_eq!(last.played_move(), None);
        assert_eq!(last.policy(), &Policy::None);
        assert_eq!(decoded.positions().last(), Some(game.position()));
        for entry in record.entries() {
            assert_eq!(entry.value(), TeacherValue::None);
            assert_eq!(entry.visits(), None);
            assert_eq!(entry.exclusion(), Exclusion::None);
        }
        if input["status"] == "resign" {
            assert_eq!(game.result(), None);
        } else {
            assert_eq!(game.result(), Some(expected_result(input)));
        }
    }
    assert!(reader.read_game().unwrap().is_none());
    let ids: Vec<_> = cases.iter().map(|game| game["id"].clone()).collect();
    assert_eq!(
        dir.report(),
        json!({
            "input_games": 3, "accepted": 3, "accepted_ids": ids,
            "excluded": {
                "illegal_move": {"count": 0, "ids": []},
                "finished_early": {"count": 0, "ids": []},
                "result_mismatch": {"count": 0, "ids": []}
            }
        })
    );
}

#[test]
fn replay_exclusions_keep_input_order_and_accepted_numbers_have_no_gaps() {
    let dir = Directory::new();
    let resign = fixture("resign");
    let royal = fixture("royalsLost");
    let bare = fixture("bareKing");
    let mut illegal = resign.clone();
    illegal["id"] = json!("illegal");
    let mut moves: Vec<_> = resign["moves"]
        .as_str()
        .unwrap()
        .split_whitespace()
        .collect();
    moves[10] = "1あ";
    illegal["moves"] = json!(moves.join(" "));
    let mut early = royal.clone();
    early["id"] = json!("early");
    early["moves"] = json!(format!("{} 1あ", royal["moves"].as_str().unwrap()));
    let mut wrong_winner = royal.clone();
    wrong_winner["id"] = json!("wrong-winner");
    wrong_winner["winner"] = json!(if royal["winner"] == "sente" {
        "gote"
    } else {
        "sente"
    });
    let mut unfinished = resign.clone();
    unfinished["id"] = json!("unfinished");
    unfinished["status"] = json!("royalsLost");
    let mut wrong_reason = royal.clone();
    wrong_reason["id"] = json!("wrong-reason");
    wrong_reason["status"] = json!("bareKing");
    let mut finished_resign = royal.clone();
    finished_resign["id"] = json!("finished-resign");
    finished_resign["status"] = json!("resign");
    dir.input(&[
        illegal,
        resign.clone(),
        early,
        wrong_winner,
        royal.clone(),
        unfinished,
        wrong_reason,
        finished_resign,
        bare.clone(),
    ]);
    success(dir.run());
    assert_eq!(
        dir.report(),
        json!({
            "input_games": 9, "accepted": 3, "accepted_ids": [resign["id"], royal["id"], bare["id"]],
            "excluded": {
                "illegal_move": {"count": 1, "ids": ["illegal"]},
                "finished_early": {"count": 1, "ids": ["early"]},
                "result_mismatch": {"count": 4, "ids": ["wrong-winner", "unfinished", "wrong-reason", "finished-resign"]}
            }
        })
    );
    let mut reader = Reader::new(File::open(dir.path("games.mnsd")).unwrap()).unwrap();
    for (number, input) in [(1, resign), (2, royal), (3, bare)] {
        let decoded = reader.read_game().unwrap().unwrap();
        assert_eq!(decoded.record().game_number(), number);
        assert_eq!(
            decoded.record().outcome(),
            Outcome::Game(expected_result(&input))
        );
    }
    assert!(reader.read_game().unwrap().is_none());
}

#[test]
fn resignation_accepts_either_recorded_winner_regardless_of_side_to_move() {
    let dir = Directory::new();
    let mut sente = fixture("resign");
    sente["id"] = json!("sente-wins");
    sente["winner"] = json!("sente");
    let mut gote = sente.clone();
    gote["id"] = json!("gote-wins");
    gote["winner"] = json!("gote");
    dir.input(&[sente, gote]);
    success(dir.run());
    assert_eq!(dir.report()["accepted"], 2);
    let mut reader = Reader::new(File::open(dir.path("games.mnsd")).unwrap()).unwrap();
    for winner in [Color::Black, Color::White] {
        assert_eq!(
            reader.read_game().unwrap().unwrap().record().outcome(),
            Outcome::Game(GameResult::Win {
                winner,
                reason: WinReason::Resignation
            })
        );
    }
}

#[test]
fn invalid_input_contract_reports_line_and_removes_output() {
    let dir = Directory::new();
    let valid = fixture("resign");
    for (field, value) in [
        ("initialSfen", Value::Null),
        ("initialSfen", json!("startpos")),
        ("status", json!("draw")),
        ("winner", Value::Null),
        ("winner", json!("other")),
        ("id", valid["id"].clone()),
        ("id", json!("")),
        ("id", json!("with space")),
    ] {
        let mut invalid = valid.clone();
        invalid["id"] = json!("invalid");
        invalid[field] = value;
        if field == "winner" && invalid[field].is_null() {
            invalid.as_object_mut().unwrap().remove("winner");
        }
        dir.input(&[valid.clone(), invalid]);
        let output = dir.run();
        assert_eq!(output.status.code(), Some(1), "{field}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("NDJSON line 2:"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        dir.assert_no_output();
    }
}

#[test]
fn existing_output_or_report_is_preserved_without_creating_the_other() {
    let dir = Directory::new();
    dir.input(&fixtures());
    for existing in [
        vec!["games.mnsd"],
        vec!["report.json"],
        vec!["games.mnsd", "report.json"],
    ] {
        for name in &existing {
            fs::write(dir.path(name), b"keep these bytes").unwrap();
        }
        let output = dir.run();
        assert_eq!(output.status.code(), Some(1));
        for name in ["games.mnsd", "report.json"] {
            if existing.contains(&name) {
                assert_eq!(fs::read(dir.path(name)).unwrap(), b"keep these bytes");
                fs::remove_file(dir.path(name)).unwrap();
            } else {
                assert!(!dir.path(name).exists());
            }
        }
    }
}

#[test]
fn failed_report_creation_removes_the_new_mnsd() {
    let dir = Directory::new();
    dir.input(&[fixture("resign")]);
    // 報告の親ディレクトリを欠落させ、MNSDを書いた後の失敗を起こす。
    let output = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["data", "lishogi", "validation", "--input"])
        .arg(dir.path("input.ndjson"))
        .arg("--output")
        .arg(dir.path("games.mnsd"))
        .arg("--report")
        .arg(dir.path("missing/report.json"))
        .args(["--seed", "20261113", "--allow-dirty"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    dir.assert_no_output();
}

#[test]
fn seed_is_required_and_must_be_a_nonzero_u64() {
    let dir = Directory::new();
    dir.input(&[fixture("resign")]);
    for args in [
        vec![],
        vec!["--seed", "0"],
        vec!["--seed", "18446744073709551616"],
    ] {
        let output = dir
            .command()
            .args(args)
            .arg("--allow-dirty")
            .output()
            .unwrap();
        assert!(!output.status.success());
        dir.assert_no_output();
    }
}
