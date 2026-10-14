//! フェーズ5指示書の受理条件、再生境界、並列書き出しと開始局面の契約。
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use minase::training::records::{
    Entry, Exclusion, GameRecord, Header, Origin, Outcome, Policy, Reader, TeacherKind,
    Value as TeacherValue, Writer,
};
use minase_core::notation::{sfen::parse_extended_sfen, usi};
use minase_core::{Color, Game, GameResult, Rules, WinReason};
use serde_json::{Value, json};

const CASES: &str = include_str!("fixtures/lishogi_import_cases.ndjson");
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "minase-phase5-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn json_file(path: &Path) -> Value {
    serde_json::from_reader(File::open(path).unwrap()).unwrap()
}
fn import(directory: &Directory, mode: &str, name: &str, concurrency: &str) -> Output {
    let input = directory.path("input.ndjson");
    let mut command = Command::new(env!("CARGO_BIN_EXE_minase"));
    command.args(["data", "lishogi"]);
    command
        .args([mode, "--input"])
        .arg(input)
        .arg("--output")
        .arg(directory.path(name))
        .arg("--report")
        .arg(directory.path(&format!("{name}.report.json")))
        .args([
            "--nodes",
            "200",
            "--hash-mb",
            "1",
            "--concurrency",
            concurrency,
        ]);
    if mode == "games" {
        command.args(["--seed", "290501", "--allow-dirty"]);
    }
    command.output().unwrap()
}
fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn games_filter_in_specified_order_and_preserve_details_and_records() {
    let dir = Directory::new();
    fs::write(dir.path("input.ndjson"), CASES).unwrap();
    success(import(&dir, "games", "one.mnsd", "1"));
    success(import(&dir, "games", "two.mnsd", "2"));
    for suffix in ["", ".report.json"] {
        assert_eq!(
            fs::read(dir.path(&format!("one.mnsd{suffix}"))).unwrap(),
            fs::read(dir.path(&format!("two.mnsd{suffix}"))).unwrap()
        );
    }
    let report = json_file(&dir.path("one.mnsd.report.json"));
    assert_eq!(
        report["accepted_ids"],
        json!(["SNjoPiHz", "Lqyj1bLC", "gZ0HcfLK"])
    );
    assert_eq!(report["accepted"], 3);
    let expected = BTreeMap::from([
        ("variant", vec!["variant"]),
        ("initial_sfen", vec!["initial"]),
        ("bot", vec!["bot"]),
        ("unrated", vec!["unrated"]),
        ("missing_speed", vec!["missing-speed"]),
        ("rating", vec!["provisional", "missing-rating"]),
        ("status_outoftime", vec!["time", "opening-prefix-2u7dwJf9"]),
        ("status_draw", vec!["UUnYczs0"]),
        ("unknown_status", vec!["unknown"]),
        ("winner", vec!["missing-winner"]),
        ("duplicate", vec!["SNjoPiHz"]),
        ("illegal_default_rules", vec!["illegal"]),
        ("winner_mismatch", vec!["royals-not-lost"]),
        ("resigner_mismatch", vec!["wrong-winner"]),
    ]);
    for (reason, entry) in report["excluded"].as_object().unwrap() {
        let ids = expected.get(reason.as_str()).cloned().unwrap_or_default();
        assert_eq!(entry["count"], ids.len(), "{reason}");
        assert_eq!(entry["ids"], json!(ids), "{reason}");
    }
    // 入力1、MNSD2、報告2の5ファイルだけを作る。
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 5);
    let mut reader = Reader::new(File::open(dir.path("one.mnsd")).unwrap()).unwrap();
    assert_eq!(reader.header().rule_set(), "L0,P0,R1,E0");
    assert_eq!(reader.header().teacher_scale(), 200);
    let mut counts = [0; 3];
    let mut last_game = 0;
    let mut played_policies = 0;
    let mut candidate_policies = 0;
    while let Some(decoded) = reader.read_game().unwrap() {
        let record = decoded.record();
        assert_eq!(record.origin(), Origin::HumanGame);
        assert_eq!(record.start_ply(), 0);
        assert_eq!(record.start_position(), &minase_core::Position::initial());
        let game = record.game_number();
        assert!(game > last_game);
        last_game = game;
        let winning_color = if game == 2 {
            Color::Black
        } else {
            Color::White
        };
        let id = ["SNjoPiHz", "Lqyj1bLC", "gZ0HcfLK"][game as usize - 1];
        let input: Value = CASES
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .find(|g| g["id"] == id)
            .unwrap();
        let mut replay = Game::new(Rules::ENGINE_DEFAULT);
        let mut moves = Vec::new();
        for text in input["moves"].as_str().unwrap().split_whitespace() {
            if replay.result().is_some() {
                break;
            }
            let mv = usi::parse(replay.position(), text).unwrap();
            replay.play(mv).unwrap();
            moves.push(mv);
        }
        assert_eq!(record.entries().len(), moves.len() + 1);
        assert_eq!(
            record.outcome(),
            Outcome::Game(match replay.result() {
                Some(result) => result,
                None => GameResult::Win {
                    winner: winning_color,
                    reason: WinReason::Resignation
                },
            })
        );
        let last = record.entries().last().unwrap();
        assert_eq!(last.played_move(), None);
        if replay.result().is_some() {
            if id == "SNjoPiHz" {
                assert_eq!(
                    record.outcome(),
                    Outcome::Game(GameResult::Win {
                        winner: Color::White,
                        reason: WinReason::RoyalCapture,
                    })
                );
            }
            assert_eq!(last.value(), TeacherValue::None);
            assert_eq!(last.policy(), &Policy::None);
            assert_eq!(last.exclusion(), Exclusion::None);
        } else {
            assert!(matches!(last.value(), TeacherValue::AlphaBeta(_)));
            assert!(
                matches!(last.policy(), Policy::Candidates(c) if c.len() == 1 && c[0].1 == 65_535)
            );
        }
        let mut exclusions = [0_u64; 4];
        for (k, entry) in record.entries().iter().enumerate() {
            assert_eq!(entry.played_move(), moves.get(k).copied());
            exclusions[entry.exclusion() as usize] += 1;
            // 指示書のノード上限で各局面を独立に探索し、保存した価値と最善手を照合する。
            if entry.value() != TeacherValue::None {
                use minase::search::{
                    DEFAULT_THREADS, HistoryTables, SearchLimits, SearchSnapshot,
                    TranspositionTable, search,
                };
                let position_game =
                    Game::from_position(Rules::ENGINE_DEFAULT, decoded.positions()[k].clone());
                let expected = search(
                    &minase::eval::weights().unwrap(),
                    &SearchSnapshot::from_game(&position_game).unwrap(),
                    &SearchLimits::new(None, Some(200), None, None).unwrap(),
                    DEFAULT_THREADS,
                    &mut TranspositionTable::new(1).unwrap(),
                    &mut HistoryTables::new(DEFAULT_THREADS),
                )
                .unwrap();
                assert_eq!(
                    entry.value(),
                    TeacherValue::AlphaBeta(expected.score.try_into().unwrap())
                );
                if entry.played_move() == Some(expected.best_move) {
                    assert_eq!(entry.policy(), &Policy::PlayedMove);
                } else {
                    assert_eq!(
                        entry.policy(),
                        &Policy::Candidates(vec![(expected.best_move, 65_535)])
                    );
                }
            }
            match entry.policy() {
                Policy::PlayedMove => {
                    played_policies += 1;
                    assert!(entry.played_move().is_some());
                }
                Policy::Candidates(c) => {
                    candidate_policies += 1;
                    assert_eq!(c.len(), 1);
                    assert_eq!(c[0].1, 65_535);
                    assert_ne!(entry.played_move(), Some(c[0].0));
                }
                Policy::None => assert_eq!(k, moves.len()),
            }
            if entry.exclusion() == Exclusion::None && entry.value() != TeacherValue::None {
                counts[game as usize - 1] += 1;
            }
        }
        for (i, reason) in ["mate_band", "tactical", "repetition"].iter().enumerate() {
            assert_eq!(
                report["games"][id]["excluded_positions"][reason],
                exclusions[i + 1]
            );
        }
    }
    assert_eq!(last_game, 3);
    assert!(played_policies > 0);
    assert!(candidate_policies > 0);
    for (count, bound) in counts.into_iter().zip([22, 81, 19]) {
        assert!(count > 0 && count <= bound);
    }
    let details = &report["games"];
    assert_eq!(details["SNjoPiHz"]["sente_rating"], 1800);
    assert_eq!(details["SNjoPiHz"]["clock"]["byoyomi"], 30);
    for (index, id) in ["SNjoPiHz", "Lqyj1bLC", "gZ0HcfLK"].iter().enumerate() {
        assert_eq!(details[id]["recorded_positions"], counts[index]);
    }
}

#[test]
fn openings_keep_unrated_unfinished_games_and_stop_before_illegal_moves() {
    let dir = Directory::new();
    let mut game: Value = CASES
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|g| g["id"] == "opening-prefix-2u7dwJf9")
        .unwrap();
    game["rated"] = json!(false);
    game["winner"] = Value::Null;
    game["players"]["sente"]["provisional"] = json!(true);
    game["moves"] = json!(format!("{} 1あ", game["moves"].as_str().unwrap()));
    fs::write(dir.path("input.ndjson"), format!("{game}\n")).unwrap();
    success(import(&dir, "openings", "openings.txt", "2"));
    let report = json_file(&dir.path("openings.txt.report.json"));
    assert_eq!(report["accepted"], 1);
    assert!(report.get("games").is_none());
    assert_eq!(report["candidates"], 2);
    assert_eq!(report["retained"], 2);
    assert_eq!(report["truncated_illegal"]["opening-prefix-2u7dwJf9"], 61);
    assert_eq!(
        report["retained_plies"]["opening-prefix-2u7dwJf9"],
        json!([40, 60])
    );
    let mut replay = Game::new(Rules::ENGINE_DEFAULT);
    let mut expected = BTreeMap::new();
    for (index, text) in game["moves"]
        .as_str()
        .unwrap()
        .split_whitespace()
        .take(60)
        .enumerate()
    {
        replay
            .play(usi::parse(replay.position(), text).unwrap())
            .unwrap();
        if index + 1 == 40 || index + 1 == 60 {
            expected.insert(index + 1, replay.position().clone());
        }
    }
    for line in fs::read_to_string(dir.path("openings.txt"))
        .unwrap()
        .lines()
    {
        let mut fields = line.splitn(3, ' ');
        assert_eq!(fields.next(), Some("opening-prefix-2u7dwJf9"));
        let ply: usize = fields.next().unwrap().parse().unwrap();
        let setup =
            parse_extended_sfen(fields.next().unwrap(), Rules::ENGINE_DEFAULT.moves).unwrap();
        assert_eq!(setup.next_move_number() as usize, ply + 1);
        assert_eq!(setup.position(), &expected[&ply]);
    }
}

#[test]
fn invalid_json_reports_line_and_does_not_leave_an_output() {
    let dir = Directory::new();
    for invalid in ["{", "{}", "null", "{\"id\":42}"] {
        fs::write(
            dir.path("input.ndjson"),
            format!("{}\n{invalid}\n", CASES.lines().next().unwrap()),
        )
        .unwrap();
        let output = import(&dir, "games", "bad.mnsd", "1");
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains("line 2"));
        assert!(!dir.path("bad.mnsd").exists());
    }
}

#[test]
fn opening_generation_cli_requires_zero_random_moves_and_one_game_per_line() {
    let dir = Directory::new();
    fs::write(dir.path("input.ndjson"), CASES).unwrap();
    success(import(&dir, "openings", "openings.txt", "1"));
    let command = |random_moves: &str, name: &str| {
        Command::new(env!("CARGO_BIN_EXE_minase"))
            .args(["data", "selfplay"])
            .args(["generate", "--openings"])
            .arg(dir.path("openings.txt"))
            .arg("--output")
            .arg(dir.path(name))
            .args([
                "--random-moves",
                random_moves,
                "--seed",
                "290502",
                "--nodes",
                "400",
                "--hash-mb",
                "1",
                "--max-ply",
                "4000",
                "--allow-dirty",
            ])
            .output()
            .unwrap()
    };
    let rejected = command("1", "bad.mnsd");
    assert_eq!(rejected.status.code(), Some(1));
    assert!(!dir.path("bad.mnsd").exists());
    success(command("0", "selfplay.mnsd"));
    let mut reader = Reader::new(File::open(dir.path("selfplay.mnsd")).unwrap()).unwrap();
    assert!(reader.header().game_count() > 0);
    while let Some(decoded) = reader.read_game().unwrap() {
        assert_eq!(decoded.record().origin(), Origin::HumanOpeningSelfplay);
    }
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 4);
}

#[test]
fn all_terminal_reasons_are_distinct_and_earlier_filters_win() {
    let dir = Directory::new();
    let base: Value = serde_json::from_str(CASES.lines().nth(3).unwrap()).unwrap();
    let statuses = [
        ("outoftime", "status_outoftime"),
        ("timeout", "status_timeout"),
        ("aborted", "status_aborted"),
        ("draw", "status_draw"),
        ("repetition", "status_repetition"),
        ("perpetualCheck", "status_perpetual_check"),
        ("bareKing", "status_bare_king"),
        ("created", "status_created"),
        ("started", "status_started"),
        ("mate", "status_mate"),
        ("stalemate", "status_stalemate"),
        ("cheat", "status_cheat"),
        ("noStart", "status_no_start"),
        ("unknownFinish", "status_unknown_finish"),
        ("future-status", "unknown_status"),
    ];
    let mut cases = Vec::new();
    for (status, _) in statuses {
        let mut case = base.clone();
        case["id"] = json!(status);
        case["status"] = json!(status);
        case["winner"] = Value::Null;
        cases.push(case);
    }
    for (id, field, value) in [
        ("no-user", "user", Value::Null),
        ("ai", "aiLevel", json!(1)),
    ] {
        let mut case = base.clone();
        case["id"] = json!(id);
        case["players"]["sente"][field] = value;
        cases.push(case);
    }
    let mut initial = base.clone();
    initial["id"] = json!("null-initial");
    initial["initialSfen"] = Value::Null;
    cases.push(initial);
    let mut duplicate = base.clone();
    duplicate["status"] = json!("draw");
    cases.extend([base, duplicate]);
    fs::write(
        dir.path("input.ndjson"),
        cases.iter().map(|c| format!("{c}\n")).collect::<String>(),
    )
    .unwrap();
    success(import(&dir, "games", "statuses.mnsd", "2"));
    let report = json_file(&dir.path("statuses.mnsd.report.json"));
    for (status, reason) in statuses {
        assert!(
            report["excluded"][reason]["ids"]
                .as_array()
                .unwrap()
                .contains(&json!(status))
        );
    }
    assert_eq!(report["accepted"], 1);
    assert_eq!(report["excluded"]["status_draw"]["count"], 2);
    assert_eq!(report["excluded"]["winner"]["count"], 0);
    assert_eq!(report["excluded"]["duplicate"]["count"], 0);
    assert_eq!(report["excluded"]["bot"]["ids"], json!(["no-user", "ai"]));
    assert_eq!(
        report["excluded"]["initial_sfen"]["ids"],
        json!(["null-initial"])
    );
}

#[test]
fn engine_terminal_winner_is_checked_and_later_moves_are_ignored() {
    let dir = Directory::new();
    let mut good: Value = serde_json::from_str(CASES.lines().next().unwrap()).unwrap();
    let mut bad = good.clone();
    bad["id"] = json!("contradictory");
    bad["winner"] = json!("sente");
    good["moves"] = json!(format!("{} 1あ", good["moves"].as_str().unwrap()));
    fs::write(dir.path("input.ndjson"), format!("{bad}\n{good}\n")).unwrap();
    success(import(&dir, "games", "end.mnsd", "1"));
    let report = json_file(&dir.path("end.mnsd.report.json"));
    assert_eq!(report["accepted_ids"], json!(["SNjoPiHz"]));
    assert_eq!(
        report["excluded"]["winner_mismatch"]["ids"],
        json!(["contradictory"])
    );
    assert_eq!(report["excluded"]["illegal_default_rules"]["count"], 0);
}

#[test]
fn invalid_utf8_has_line_number_and_existing_outputs_are_preserved() {
    let dir = Directory::new();
    let mut input = format!("{}\n", CASES.lines().next().unwrap()).into_bytes();
    input.extend([0xff, b'\n']);
    fs::write(dir.path("input.ndjson"), input).unwrap();
    let output = import(&dir, "games", "bad.mnsd", "1");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("line 2"));
    fs::write(dir.path("input.ndjson"), CASES).unwrap();
    fs::write(dir.path("existing.mnsd"), b"keep").unwrap();
    assert_eq!(
        import(&dir, "games", "existing.mnsd", "1").status.code(),
        Some(1)
    );
    assert_eq!(fs::read(dir.path("existing.mnsd")).unwrap(), b"keep");
}

#[test]
fn inspect_accepts_and_displays_synthetic_records() {
    let dir = Directory::new();
    let path = dir.path("synthetic.mnsd");
    let header = Header::new(
        "L0,P0,R1,E0".into(),
        "0".repeat(40),
        [0; 32],
        TeacherKind::AlphaBeta,
        0,
        0,
    )
    .unwrap();
    let mut writer = Writer::new(File::create(&path).unwrap(), header).unwrap();
    writer
        .write_game(
            &GameRecord::new(
                0,
                Origin::Synthetic,
                Outcome::NotFromGame,
                0,
                minase_core::Position::initial(),
                vec![
                    Entry::new(
                        None,
                        TeacherValue::None,
                        Exclusion::None,
                        Policy::None,
                        None,
                    )
                    .unwrap(),
                ],
            )
            .unwrap(),
        )
        .unwrap();
    writer.finish().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["data", "selfplay", "inspect"])
        .arg(path)
        .args(["--dump", "1"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("origin: Synthetic"));
    assert!(text.contains("games_origin_3: 1"));
    assert!(text.contains("origin_3: 0"));
    assert!(text.contains("eligible_positions: 0"));
    assert!(text.contains("outcome: none"));
}

/// 検査出力の全行を固定し、先頭N局というdumpの単位と全対局の集計を検査する。
#[test]
fn inspect_dump_has_every_entry_field_in_order() {
    let dir = Directory::new();
    let path = dir.path("dump.mnsd");
    for kind in [TeacherKind::AlphaBeta, TeacherKind::Mcts] {
        let header = Header::new(
            "L0,P0,R1,E0".into(),
            "1".repeat(40),
            [0xab; 32],
            kind,
            42,
            7,
        )
        .unwrap();
        let mut writer = Writer::new(File::create(&path).unwrap(), header).unwrap();
        let mut game = Game::new(Rules::ENGINE_DEFAULT);
        let first = usi::parse(game.position(), "1i1h").unwrap();
        let initial = game.position().clone();
        game.play(first).unwrap();
        let last_position = game.position().clone();
        let best = game.legal_moves()[0];
        let (value, visits) = match kind {
            TeacherKind::AlphaBeta => (TeacherValue::AlphaBeta(123), None),
            TeacherKind::Mcts => (TeacherValue::Mcts(123), Some(456)),
        };
        let record = GameRecord::new(
            9,
            Origin::HumanGame,
            Outcome::Game(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::Resignation,
            }),
            4,
            initial.clone(),
            vec![
                Entry::new(
                    Some(first),
                    value,
                    Exclusion::Tactical,
                    Policy::PlayedMove,
                    visits,
                )
                .unwrap(),
                Entry::new(
                    None,
                    value,
                    Exclusion::None,
                    Policy::Candidates(vec![(best, 65_535)]),
                    visits,
                )
                .unwrap(),
            ],
        )
        .unwrap();
        writer.write_game(&record).unwrap();
        writer
            .write_game(
                &GameRecord::new(
                    12,
                    Origin::Synthetic,
                    Outcome::NotFromGame,
                    0,
                    initial.clone(),
                    vec![
                        Entry::new(
                            None,
                            TeacherValue::None,
                            Exclusion::None,
                            Policy::None,
                            visits.map(|_| 0),
                        )
                        .unwrap(),
                    ],
                )
                .unwrap(),
            )
            .unwrap();
        writer.finish().unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_minase"))
            .args(["data", "selfplay", "inspect"])
            .arg(&path)
            .args(["--dump", "1"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        let visits_line = if kind == TeacherKind::Mcts {
            "root_visits: 456\n"
        } else {
            ""
        };
        let expected = format!(
            "header:\nmagic: MNSD\nformat_version: 2\nrule_set: L0,P0,R1,E0\ngeneration_commit: {}\nnetwork_checksum: {}\nteacher_kind: {kind:?}\nteacher_scale: 42\nseed: 7\ngame_count: 2\nentry_count: 3\n\
entry: 9 0\nsfen: {}\nside_to_move: Black\nlion_square: none\nlion_by_kirin_promotion: false\nvalue: 123\nexclusion: tactical\npolicy: played\n{visits_line}played_move: 1i1h\noutcome: Win\ngame_number: 9\nply: 4\norigin: HumanGame\n\
entry: 9 1\nsfen: {}\nside_to_move: White\nlion_square: none\nlion_by_kirin_promotion: false\nvalue: 123\nexclusion: none\npolicy: 1 {}:65535\n{visits_line}played_move: none\noutcome: Loss\ngame_number: 9\nply: 5\norigin: HumanGame\n\
summary:\ngames: 2\nentries: 3\ngames_origin_0: 0\ngames_origin_1: 0\ngames_origin_2: 1\ngames_origin_3: 1\nexclusion_none: 2\nexclusion_mate_band: 0\nexclusion_tactical: 1\nexclusion_repetition: 0\nvalue_none: 1\npolicy_none: 1\npolicy_played: 1\npolicy_candidates: 1\npolicy_candidate_count_1: 1\neligible_positions: 1\norigin_0: 0\norigin_1: 0\norigin_2: 1\norigin_3: 0\noutcome_loss: 1\noutcome_draw: 0\noutcome_win: 0\nscore_minimum: 123\nscore_maximum: 123\nscore_average: 123.000000\nblack_to_move: 0\nwhite_to_move: 1\n",
            "1".repeat(40),
            "ab".repeat(32),
            minase_core::to_sfen(&initial),
            minase_core::to_sfen(&last_position),
            usi::text_generated(&last_position, best)
        );
        assert_eq!(text, expected);
        // 2局目だけを途中で切り、失敗位置と非0終了を検査する。
        let mut bytes = fs::read(&path).unwrap();
        bytes.pop();
        fs::write(&path, bytes).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_minase"))
            .args(["data", "selfplay", "inspect"])
            .arg(&path)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains("game at file index 2:"));
    }
}

/// 保存対局だけの除外数と学習対象数はファイルから数え直して一致し、打ち切り対局は書かれない。
#[test]
fn selfplay_summary_counts_saved_entries_and_discards_capped_games() {
    let dir = Directory::new();
    for cap in [1200, 1] {
        let path = dir.path(&format!("selfplay-{cap}.mnsd"));
        let output = Command::new(env!("CARGO_BIN_EXE_minase"))
            .args(["data", "selfplay", "generate", "--output"])
            .arg(&path)
            .args([
                "--games",
                "1",
                "--seed",
                "7",
                "--nodes",
                "100",
                "--random-moves",
                "4",
                "--hash-mb",
                "1",
                "--concurrency",
                "1",
                "--max-ply",
                &cap.to_string(),
                "--allow-dirty",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        let summary: BTreeMap<_, _> = text.lines().filter_map(|l| l.split_once(": ")).collect();
        let mut reader = Reader::new(File::open(path).unwrap()).unwrap();
        let mut eligible = 0;
        let mut exclusions = [0; 4];
        while let Some(decoded) = reader.read_game().unwrap() {
            for entry in decoded.record().entries() {
                exclusions[entry.exclusion() as usize] += 1;
                if entry.exclusion() == Exclusion::None && entry.value() != TeacherValue::None {
                    eligible += 1;
                }
            }
        }
        assert_eq!(
            summary["recorded_positions"].parse::<u64>().unwrap(),
            eligible
        );
        for (index, reason) in ["mate_band", "tactical", "repetition"].iter().enumerate() {
            assert_eq!(
                summary[format!("excluded_{reason}_in_saved_games").as_str()]
                    .parse::<u64>()
                    .unwrap(),
                exclusions[index + 1]
            );
        }
        if cap == 1 {
            assert_eq!(reader.header().game_count(), 0);
            assert_eq!(reader.header().entry_count(), 0);
            assert_eq!(summary["games_discarded_max_ply"], "1");
        } else {
            assert_eq!(reader.header().game_count(), 1);
            assert!(eligible > 0);
            assert!(exclusions[1..].iter().sum::<u64>() > 0);
        }
    }
}
