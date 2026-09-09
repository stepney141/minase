//! 保存済みFM対局の先頭pairを合法性検査つきで再生し、診断専用MNSDを作る。
use std::fs;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use clap::Parser;
use minase::core::rules::parse_rule_set;
use minase::eval::Pst;
use minase::eval::training_data::{Header, Outcome, Record, Writer};
use minase::notation::usi;
use minase::{Color, Game, GameResult, Rules};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Parser)]
#[command(
    about = "FM対局アーカイブformat_version=3を再生する。出力は診断専用で教師誤差計算には使用不可。"
)]
struct Arguments {
    #[arg(long)]
    run_dir: PathBuf,
    #[arg(long)]
    max_pairs: NonZeroUsize,
    #[arg(long)]
    pst: PathBuf,
    #[arg(long)]
    output_dir: PathBuf,
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a Value, String> {
    value
        .get(key)
        .ok_or_else(|| format!("missing field: {key}"))
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    field(value, key)?
        .as_str()
        .ok_or_else(|| format!("invalid string: {key}"))
}
fn number(value: &Value, key: &str) -> Result<u64, String> {
    field(value, key)?
        .as_u64()
        .ok_or_else(|| format!("invalid integer: {key}"))
}
fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    field(value, key)?
        .as_array()
        .ok_or_else(|| format!("invalid array: {key}"))
}
fn color(value: &str) -> Result<Color, String> {
    match value {
        "black" => Ok(Color::Black),
        "white" => Ok(Color::White),
        _ => Err(format!("invalid color: {value}")),
    }
}
fn play(game: &mut Game, side: Color, movement: &str) -> Result<(), String> {
    if game.position().side_to_move() != side {
        return Err("turn side mismatch".into());
    }
    let mv = usi::parse(game.position(), movement).map_err(|e| e.to_string())?;
    game.play(mv).map_err(|e| e.to_string())?;
    Ok(())
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn termination(game: &Game, saved: &Value) -> Result<GameResult, String> {
    let result = game.result().ok_or("replayed game did not finish")?;
    match (text(saved, "kind")?, result) {
        ("adjudicated_win", GameResult::Win { winner, reason }) => {
            if color(text(saved, "winner")?)? != winner
                || text(saved, "reason")? != format!("{reason:?}")
            {
                return Err("adjudicated winner/reason mismatch".into());
            }
        }
        ("adjudicated_draw", GameResult::Draw { reason }) => {
            if text(saved, "reason")? != format!("{reason:?}") {
                return Err("adjudicated draw reason mismatch".into());
            }
        }
        _ => return Err("unsupported or inconsistent termination".into()),
    }
    Ok(result)
}
fn run(args: Arguments) -> Result<(), String> {
    let manifest_bytes = fs::read(args.run_dir.join("manifest.json")).map_err(|e| e.to_string())?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes).map_err(|e| e.to_string())?;
    if number(&manifest, "format_version")? != 3 {
        return Err("only archive format_version=3 is supported".into());
    }
    let candidate = field(&manifest, "candidate")?;
    let identity = field(candidate, "identity")?;
    if text(identity, "kind")? != "commit" || text(field(candidate, "limit")?, "kind")? != "time" {
        return Err("expected a commit candidate with time control".into());
    }
    let commit = text(identity, "hash")?;
    let seed = number(&manifest, "seed")?;
    let rule_set = text(&manifest, "rules_source")?;
    let codes = parse_rule_set(rule_set).map_err(|e| e.to_string())?;
    let rules = Rules::from_codes(&codes).map_err(|e| e.to_string())?;
    let canonical: Vec<String> = codes.iter().map(ToString::to_string).collect();
    if json!(canonical) != *field(&manifest, "canonical_rules")? {
        return Err("canonical rules mismatch".into());
    }
    let weights = fs::read(&args.pst).map_err(|e| e.to_string())?;
    let pst = Pst::decode(&weights).map_err(|e| e.to_string())?;
    let candidate_weights = Command::new("git")
        .args(["show", &format!("{commit}:nets/pst.bin")])
        .output()
        .map_err(|e| e.to_string())?;
    if !candidate_weights.status.success() {
        return Err("cannot verify candidate nets/pst.bin from local git history".into());
    }
    let archived_pst = Pst::decode(&candidate_weights.stdout).map_err(|e| e.to_string())?;
    if archived_pst.checksum() != pst.checksum() {
        return Err("PST checksum differs from candidate commit".into());
    }
    let mut records = Vec::new();
    let mut provenance = Vec::new();
    let mut sources = Vec::new();
    let mut missing_evaluation = 0_u64;
    let mut non_cp = 0_u64;
    for pair_number in 1..=args.max_pairs.get() {
        let name = format!("pairs/{pair_number:020}.json");
        let bytes = fs::read(args.run_dir.join(&name)).map_err(|e| format!("{name}: {e}"))?;
        let pair: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if number(&pair, "pair_number")? != pair_number as u64 {
            return Err("pair number mismatch".into());
        }
        let source_hash = digest(&bytes);
        sources.push(json!({"file": name, "sha256": source_hash}));
        let opening = array(field(&pair, "opening")?, "moves")?;
        let games = array(&pair, "games")?;
        if games.len() != 2 {
            return Err("each pair must contain 2 games".into());
        }
        for (game_index, saved) in games.iter().enumerate() {
            let candidate_color = color(text(saved, "candidate_color")?)?;
            if candidate_color != [Color::Black, Color::White][game_index] {
                return Err("pair candidate color order mismatch".into());
            }
            let mut game = Game::new(rules);
            for movement in opening {
                let side = game.position().side_to_move();
                play(
                    &mut game,
                    side,
                    movement.as_str().ok_or("invalid opening move")?,
                )?;
            }
            let mut pending = Vec::new();
            for (turn_index, turn) in array(saved, "turns")?.iter().enumerate() {
                let side = color(text(turn, "side")?)?;
                if side != game.position().side_to_move() {
                    return Err(format!(
                        "pair {pair_number} game {game_index} turn {turn_index}: side mismatch"
                    ));
                }
                if side == candidate_color {
                    match field(turn, "evaluation")? {
                        Value::Null => missing_evaluation += 1,
                        evaluation => {
                            if color(text(evaluation, "perspective")?)? != side {
                                return Err("evaluation perspective mismatch".into());
                            }
                            let score = field(evaluation, "score")?;
                            match text(score, "kind")? {
                                "cp" => {
                                    let score = field(score, "value")?
                                        .as_i64()
                                        .ok_or("invalid cp score")?;
                                    let score = i16::try_from(score).map_err(|e| e.to_string())?;
                                    pending.push((
                                        game.position().clone(),
                                        score,
                                        game.ply_count(),
                                        turn_index,
                                        evaluation.clone(),
                                    ));
                                }
                                "mate_in" | "mated_in" => non_cp += 1,
                                _ => return Err("unknown score kind".into()),
                            }
                        }
                    }
                }
                let response = field(turn, "response")?;
                if text(response, "kind")? != "move" {
                    return Err("only recorded move responses are supported".into());
                }
                play(&mut game, side, text(response, "usi")?)?;
            }
            let result = termination(&game, field(saved, "termination")?)?;
            for (position, score, ply, turn_index, evaluation) in pending {
                let index = records.len();
                let game_number = u32::try_from((pair_number - 1) * 2 + game_index + 1)
                    .map_err(|e| e.to_string())?;
                let ply = u16::try_from(ply).map_err(|e| e.to_string())?;
                records.push(Record::from_position(
                    &position,
                    score,
                    Outcome::from_game_result(result, candidate_color),
                    game_number,
                    ply,
                ));
                provenance.push(json!({"index": index, "pair": pair_number, "game": game_index + 1, "game_number": game_number, "ply": ply, "turn_index": turn_index, "source_sha256": source_hash, "evaluation": evaluation}));
            }
        }
    }
    let header = Header::new(rule_set.into(), commit.into(), *pst.checksum(), 0, seed, 0)
        .map_err(|e| e.to_string())?;
    let mut writer =
        Writer::new(std::io::Cursor::new(Vec::new()), header).map_err(|e| e.to_string())?;
    for record in &records {
        writer.write_record(record).map_err(|e| e.to_string())?;
    }
    let data = writer.finish().map_err(|e| e.to_string())?.into_inner();
    let metadata = json!({
        "diagnostic_only": true, "teacher_error_comparison_allowed": false,
        "teacher_nodes": 0, "teacher_nodes_reason": "Archive uses time control; 0 does not indicate a teacher search budget.",
        "score_source": "Candidate's recorded search score at its actual turn; not independent teacher labels.",
        "selection": "First max_pairs complete pairs in pair-number order, all candidate turns with cp scores; no outcome selection.",
        "max_pairs": args.max_pairs.get(), "records": records.len(), "missing_evaluation": missing_evaluation, "non_cp": non_cp,
        "manifest_sha256": digest(&manifest_bytes), "candidate_commit": commit, "pst_sha256": digest(&weights),
        "pst_checksum": pst.checksum().iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "candidate_weights_verified_from_git": true, "mnsd_sha256": digest(&data),
        "sources": sources, "positions": provenance
    });
    fs::create_dir(&args.output_dir).map_err(|e| e.to_string())?;
    fs::write(args.output_dir.join("positions.mnsd"), data).map_err(|e| e.to_string())?;
    fs::write(
        args.output_dir.join("metadata.json"),
        serde_json::to_vec_pretty(&metadata).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    println!(
        "{} positions, {missing_evaluation} missing evaluations, {non_cp} non-cp exclusions",
        records.len()
    );
    Ok(())
}
fn main() -> ExitCode {
    match run(Arguments::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn game() -> Game {
        let codes = parse_rule_set("engine-default").unwrap();
        Game::new(Rules::from_codes(&codes).unwrap())
    }
    #[test]
    fn opening_move_preserves_recorded_position() {
        let mut game = game();
        let from = minase::Square::new(7, 3).unwrap();
        let to = minase::Square::new(7, 4).unwrap();
        let piece = game.position().piece_at(from).unwrap();
        play(&mut game, Color::Black, "5i5h").unwrap();
        assert_eq!(game.position().piece_at(from), None);
        assert_eq!(game.position().piece_at(to), Some(piece));
        assert_eq!(game.position().side_to_move(), Color::White);
        let record = Record::from_position(game.position(), 123, Outcome::Draw, 1, 1);
        let decoded = record.to_position().unwrap();
        assert_eq!(decoded.zobrist(), game.position().zobrist());
        assert_eq!(record.score(), 123);
    }
    #[test]
    fn illegal_move_and_wrong_side_are_rejected_without_mutation() {
        let mut game = game();
        let original = game.position().zobrist();
        assert!(play(&mut game, Color::White, "5i5h").is_err());
        assert!(play(&mut game, Color::Black, "5i5a").is_err());
        assert_eq!(game.position().zobrist(), original);
    }
    #[test]
    fn ongoing_game_cannot_match_saved_win() {
        assert!(
            termination(
                &game(),
                &json!({"kind":"adjudicated_win","winner":"black","reason":"Mate"})
            )
            .is_err()
        );
    }
}
