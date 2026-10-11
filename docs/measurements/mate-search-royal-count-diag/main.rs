//! 棋譜を再生し、王駒2枚（王将/玉将＋太子）の出現頻度と終局理由を集計する診断ツール。
//! 使い方:
//!   royal-diag lishogi <out.json> <games.ndjson>
//!   royal-diag matches <out.json> <match-dir>...
use minase_core::notation::sfen::to_sfen;
use minase_core::notation::usi;
use minase_core::rules::parse_rule_set;
use minase_core::{Color, Game, GameResult, PieceKind, Rules};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};

#[derive(Default, Serialize)]
struct Counter(BTreeMap<String, u64>);
impl Counter {
    fn add(&mut self, key: impl Into<String>) {
        *self.0.entry(key.into()).or_insert(0) += 1;
    }
}

/// 1局の再生結果。
struct Replayed {
    /// 訪れた各局面の王駒数 (先手, 後手)。初期局面を含む。
    royals: Vec<(u32, u32)>,
    /// 手番の相手が王駒2枚である局面数。
    opp2: u64,
    /// 手番側が王駒2枚である局面数。
    stm2: u64,
    /// 太子がいずれかの局面に現れたか。
    prince_ever: bool,
    result: Option<GameResult>,
    /// 終局後にまだ残っていた着手数。
    remaining_after_result: usize,
    /// 不合法・解析不能の着手があった手数（1始まり）。
    illegal_at: Option<usize>,
    /// 同じ着手列をE1なし（詰みで終局）で再生した双子対局の詰み成立手数。
    twin_mate_ply: Option<u32>,
    twin_result: Option<GameResult>,
    /// 最終着手直前の局面のSFEN。
    sfen_before_last: Option<String>,
    sfen_final: String,
}

fn royal_counts(game: &Game) -> (u32, u32) {
    let p = game.position();
    (
        p.royal_pieces(Color::Black).popcount(),
        p.royal_pieces(Color::White).popcount(),
    )
}

fn observe(game: &Game, r: &mut Replayed) {
    let p = game.position();
    let stm = p.side_to_move();
    let rc = royal_counts(game);
    r.royals.push(rc);
    if p.royal_pieces(stm.opposite()).popcount() == 2 {
        r.opp2 += 1;
    }
    if p.royal_pieces(stm).popcount() == 2 {
        r.stm2 += 1;
    }
    if !p.pieces_of_kind(Color::Black, PieceKind::CrownPrince).is_empty()
        || !p.pieces_of_kind(Color::White, PieceKind::CrownPrince).is_empty()
    {
        r.prince_ever = true;
    }
}

/// `game`（本則）と任意の`twin`（詰みで終局する規則）を同じ着手列で進める。
fn replay<'a>(
    mut game: Game,
    mut twin: Option<Game>,
    moves: impl Iterator<Item = &'a str>,
) -> Replayed {
    let mut r = Replayed {
        royals: Vec::new(),
        opp2: 0,
        stm2: 0,
        prince_ever: false,
        result: None,
        remaining_after_result: 0,
        illegal_at: None,
        twin_mate_ply: None,
        twin_result: None,
        sfen_before_last: None,
        sfen_final: String::new(),
    };
    observe(&game, &mut r);
    for (i, text) in moves.enumerate() {
        if game.result().is_some() {
            r.remaining_after_result += 1;
            continue;
        }
        let mv = match usi::parse(game.position(), text) {
            Ok(mv) => mv,
            Err(_) => {
                r.illegal_at = Some(i + 1);
                break;
            }
        };
        let before = to_sfen(game.position());
        if game.play(mv).is_err() {
            r.illegal_at = Some(i + 1);
            break;
        }
        r.sfen_before_last = Some(before);
        if let Some(t) = twin.as_mut()
            && t.result().is_none()
        {
            // 着手生成規則は同一なので合法性は一致する。
            let tm = usi::parse(t.position(), text).expect("twin parse");
            t.play(tm).expect("twin play");
            if let Some(res) = t.result() {
                r.twin_result = Some(res);
                if matches!(
                    res,
                    GameResult::Win {
                        reason: minase_core::WinReason::Mate,
                        ..
                    }
                ) {
                    r.twin_mate_ply = Some(t.ply_count());
                }
            }
        }
        observe(&game, &mut r);
    }
    r.result = game.result();
    r.sfen_final = to_sfen(game.position());
    r
}

fn result_name(result: Option<GameResult>) -> String {
    match result {
        None => "none".to_owned(),
        Some(GameResult::Win { reason, .. }) => format!("Win:{reason:?}"),
        Some(GameResult::Draw { reason }) => format!("Draw:{reason:?}"),
    }
}

fn result_winner(result: Option<GameResult>) -> Option<Color> {
    match result {
        Some(GameResult::Win { winner, .. }) => Some(winner),
        _ => None,
    }
}

#[derive(Default, Serialize)]
struct Aggregate {
    games_total: u64,
    games_replayed: u64,
    excluded: Counter,
    /// 再生異常。
    illegal_move_games: u64,
    remaining_after_result_games: u64,
    /// 終局理由（情報源の理由 × minaseの裁定）。
    end_reasons: Counter,
    /// 自己対局: 記録の終局と再生の裁定の不一致数。
    adjudication_mismatch: u64,
    /// lishogi: 双子対局（E1なし）での詰み成立と実対局の終わり方。
    twin_mate: Counter,
    /// 勝敗の決まった局について、終局からkプライ前の (敗者王駒, 勝者王駒) の度数。
    royals_at_offset: BTreeMap<String, Counter>,
    /// 引き分け・勝者不明の局の最終局面の (先手王駒, 後手王駒)。
    royals_final_no_winner: Counter,
    /// 全局面数と、手番の相手が王駒2枚の局面数、手番側が王駒2枚の局面数。
    positions_total: u64,
    positions_opp_two_royals: u64,
    positions_stm_two_royals: u64,
    /// 太子が現れた局数。
    games_with_prince: u64,
    /// 終局理由別の太子出現局数。
    games_with_prince_by_reason: Counter,
    /// 王駒捕獲で終わった局の最終着手直前のSFEN等。
    royal_capture_dumps: Vec<Value>,
    plies_total: u64,
}

impl Aggregate {
    fn take(
        &mut self,
        id: &str,
        reason_key: String,
        winner: Option<Color>,
        r: &Replayed,
        dump_capture: bool,
    ) {
        self.games_replayed += 1;
        if r.illegal_at.is_some() {
            self.illegal_move_games += 1;
        }
        if r.remaining_after_result > 0 {
            self.remaining_after_result_games += 1;
        }
        self.end_reasons.add(reason_key.clone());
        self.positions_total += r.royals.len() as u64;
        self.plies_total += (r.royals.len() - 1) as u64;
        self.positions_opp_two_royals += r.opp2;
        self.positions_stm_two_royals += r.stm2;
        if r.prince_ever {
            self.games_with_prince += 1;
            self.games_with_prince_by_reason.add(reason_key.clone());
        }
        let n = r.royals.len();
        match winner {
            Some(w) => {
                for k in [0usize, 1, 10, 20] {
                    let entry = self.royals_at_offset.entry(format!("minus{k}")).or_default();
                    if n > k {
                        let (b, wh) = r.royals[n - 1 - k];
                        let (loser, winr) = match w {
                            Color::Black => (wh, b),
                            Color::White => (b, wh),
                        };
                        entry.add(format!("loser{loser}_winner{winr}"));
                    } else {
                        entry.add("too_short");
                    }
                }
            }
            None => {
                let (b, wh) = r.royals[n - 1];
                self.royals_final_no_winner.add(format!("black{b}_white{wh}"));
            }
        }
        if dump_capture
            && matches!(
                r.result,
                Some(GameResult::Win {
                    reason: minase_core::WinReason::RoyalCapture,
                    ..
                })
            )
        {
            let (b2, w2) = if n >= 2 { r.royals[n - 2] } else { (0, 0) };
            let (b1, w1) = r.royals[n - 1];
            self.royal_capture_dumps.push(serde_json::json!({
                "id": id,
                "winner": format!("{winner:?}"),
                "royals_before_last": {"black": b2, "white": w2},
                "royals_final": {"black": b1, "white": w1},
                "sfen_before_last": r.sfen_before_last,
                "sfen_final": r.sfen_final,
                "plies": n - 1,
            }));
        }
    }
}

fn run_lishogi(path: &str) -> Aggregate {
    let mut agg = Aggregate::default();
    let twin_rules = Rules {
        e1: false,
        ..Rules::LISHOGI
    };
    let reader = BufReader::new(fs::File::open(path).expect("open ndjson"));
    for line in reader.lines() {
        let line = line.expect("read line");
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(&line).expect("json");
        agg.games_total += 1;
        if v.get("variant").and_then(Value::as_str) != Some("chushogi") {
            agg.excluded.add("variant_not_chushogi");
            continue;
        }
        let status = v.get("status").and_then(Value::as_str).unwrap_or("?").to_owned();
        if status == "noStart" {
            agg.excluded.add("status_noStart");
            continue;
        }
        if v.get("initialSfen").is_some_and(|s| !s.is_null()) {
            agg.excluded.add("initialSfen_present");
            continue;
        }
        let moves = v.get("moves").and_then(Value::as_str).unwrap_or("");
        if moves.split_whitespace().next().is_none() {
            agg.excluded.add("no_moves");
            continue;
        }
        let id = v.get("id").and_then(Value::as_str).unwrap_or("?").to_owned();
        let r = replay(
            Game::new(Rules::LISHOGI),
            Some(Game::new(twin_rules)),
            moves.split_whitespace(),
        );
        if r.illegal_at.is_some() {
            agg.illegal_move_games += 1;
            agg.excluded.add("illegal_move_under_LISHOGI");
            agg.games_replayed += 0;
            eprintln!("illegal: {id} at move {:?}", r.illegal_at);
            continue;
        }
        let lishogi_winner = match v.get("winner").and_then(Value::as_str) {
            Some("sente") => Some(Color::Black),
            Some("gote") => Some(Color::White),
            _ => None,
        };
        let minase_winner = result_winner(r.result);
        if minase_winner.is_some() && minase_winner != lishogi_winner {
            agg.adjudication_mismatch += 1;
        }
        let winner = lishogi_winner.or(minase_winner);
        let key = format!("{status} / {}", result_name(r.result));
        // 双子対局の詰みと実対局の終わり方。
        let n_plies = (r.royals.len() - 1) as u32;
        let twin_key = match r.twin_mate_ply {
            None => format!("{status} / twin:no-mate / twin:{}", result_name(r.twin_result)),
            Some(p) if p == n_plies => format!("{status} / twin:mate-at-final-ply"),
            Some(p) => {
                let after = n_plies - p;
                let bucket = if after <= 2 { "1-2" } else if after <= 10 { "3-10" } else { ">10" };
                format!("{status} / twin:mate-then-continued-{bucket}-plies")
            }
        };
        agg.twin_mate.add(twin_key);
        agg.take(&id, key, winner, &r, false);
    }
    agg
}

fn run_matches(dirs: &[String]) -> Aggregate {
    let mut agg = Aggregate::default();
    for dir in dirs {
        let manifest: Value =
            serde_json::from_str(&fs::read_to_string(format!("{dir}/manifest.json")).unwrap())
                .unwrap();
        let codes: Vec<String> = manifest["canonical_rules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_str().unwrap().to_owned())
            .collect();
        let rules = Rules::from_codes(&parse_rule_set(&codes.join(",")).unwrap()).unwrap();
        eprintln!("{dir}: rules {codes:?}");
        let mut files: Vec<_> = fs::read_dir(format!("{dir}/pairs"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        files.sort();
        for f in files {
            let pair: Value = serde_json::from_str(&fs::read_to_string(&f).unwrap()).unwrap();
            let opening: Vec<&str> = pair["opening"]["moves"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m.as_str().unwrap())
                .collect();
            for (gi, g) in pair["games"].as_array().unwrap().iter().enumerate() {
                agg.games_total += 1;
                let term = &g["termination"];
                let kind = term["kind"].as_str().unwrap();
                if kind == "cutoff" {
                    agg.excluded.add("cutoff_max_ply");
                    continue;
                }
                let turns: Vec<&str> = g["turns"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|t| {
                        (t["response"]["kind"] == "move").then(|| t["response"]["usi"].as_str().unwrap())
                    })
                    .collect();
                let id = format!("{}#{gi}", f.display());
                let r = replay(
                    Game::new(rules),
                    None,
                    opening.iter().copied().chain(turns.iter().copied()),
                );
                if r.illegal_at.is_some() {
                    eprintln!("illegal: {id} at {:?}", r.illegal_at);
                }
                let saved_winner = match term["winner"].as_str() {
                    Some("black") => Some(Color::Black),
                    Some("white") => Some(Color::White),
                    _ => None,
                };
                let saved = match kind {
                    "adjudicated_win" => format!("Win:{}", term["reason"].as_str().unwrap()),
                    "adjudicated_draw" => format!("Draw:{}", term["reason"].as_str().unwrap()),
                    other => other.to_owned(),
                };
                let actual = result_name(r.result);
                if saved != actual || saved_winner != result_winner(r.result) {
                    agg.adjudication_mismatch += 1;
                    eprintln!("mismatch: {id} saved {saved} {saved_winner:?} actual {actual}");
                }
                agg.take(&id, saved, saved_winner, &r, true);
            }
        }
    }
    agg
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let agg = match args.get(1).map(String::as_str) {
        Some("lishogi") => run_lishogi(&args[3]),
        Some("matches") => run_matches(&args[3..]),
        _ => {
            eprintln!("usage: royal-diag lishogi <out.json> <ndjson> | matches <out.json> <dir>...");
            std::process::exit(2);
        }
    };
    let out = &args[2];
    assert!(out.ends_with(".json") && !out.ends_with(".ndjson"), "output must be a .json file");
    fs::write(out, serde_json::to_string_pretty(&agg).unwrap()).unwrap();
    eprintln!("wrote {out}");
}
