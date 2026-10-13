//! ライブラリでの評価、非王駒の除去、合法な成り手とMNSD成果物。

use super::{
    Result,
    pst_reference::{Features, Models, Weights, piece},
};
use minase::{
    eval::pst::evaluate_for_diagnostics,
    training::records::{Header, Outcome, Record},
};
use minase_core::{Game, MoveGenerator, Position, Rules, notation::usi};
use serde::Serialize;
use std::{
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::Path,
};

pub(super) fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.flush()?;
    Ok(())
}

pub(super) fn write_mnsd(path: &Path, records: &[Record], checksum: [u8; 32]) -> Result<()> {
    let header = Header::new(
        "L0,P0,R1,E0".into(),
        "0".repeat(40),
        checksum,
        0,
        0,
        records.len() as u64,
    )?;
    let mut file = BufWriter::new(OpenOptions::new().write(true).create_new(true).open(path)?);
    file.write_all(&header.encode())?;
    for record in records {
        file.write_all(&record.encode())?;
    }
    file.flush()?;
    Ok(())
}

pub(super) fn check(weights: &Weights, records: &[Record], name: &str) -> Result<usize> {
    for (index, record) in records.iter().enumerate() {
        let position = record
            .to_position()
            .map_err(|error| format!("{name} record {index}: {error}"))?;
        let engine =
            evaluate_for_diagnostics(&weights.engine, &position, position.side_to_move()).0;
        let reference = weights.evaluate(&Features::new(record));
        if reference != engine {
            return Err(format!("engine evaluation disagrees with the independent integer reference for {name} record {index}: {engine} != {reference}").into());
        }
    }
    Ok(records.len())
}

#[derive(Default, Serialize)]
pub(super) struct Agreement {
    checked: usize,
    skipped: usize,
}
#[derive(Default, Serialize)]
struct Skipped {
    #[serde(skip_serializing_if = "Option::is_none")]
    base: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate: Option<String>,
}
#[derive(Serialize)]
struct Removal {
    square: usize,
    piece_byte: u8,
    relative_color: u8,
    state: usize,
    delta_cp: Models<i32>,
    total_delta_cp: Models<Option<i32>>,
    skipped: Skipped,
}
#[derive(Serialize)]
pub(super) struct Representative {
    label: String,
    index: Option<u64>,
    piece_count: usize,
    phase_numerator: u8,
    evaluations: Models<i32>,
    removals: Vec<Removal>,
    total_evaluations: Models<i32>,
    promotions: Models<Option<Vec<Promotion>>>,
    promotion_reason: Option<&'static str>,
}
#[derive(Serialize)]
struct AfterPosition {
    board: Vec<u8>,
    stm: u8,
    lion: u8,
    eval: i32,
    eval_pst: i32,
}
#[derive(Serialize)]
struct Promotion {
    r#move: String,
    capture: bool,
    promote: bool,
    terminal: bool,
    delta: i32,
    delta_pst: i32,
    after_same: i32,
    after_same_pst: i32,
    after_opposite: i32,
    after_opposite_pst: i32,
    placement: i32,
    placement_pst: i32,
    turn: i32,
    turn_pst: i32,
    after: AfterPosition,
}

fn promotions(model: &Weights, record: &Record) -> Result<Vec<Promotion>> {
    let position = record.to_position()?;
    let side = position.side_to_move();
    let before = evaluate_for_diagnostics(&model.engine, &position, side);
    let generator = MoveGenerator::new(Rules::ENGINE_DEFAULT.moves);
    let game = Game::from_position(Rules::ENGINE_DEFAULT, position.clone());
    let mut rows = Vec::new();
    for mv in game.legal_moves() {
        if !mv.promote {
            continue;
        }
        let text = usi::text(&position, mv, &generator)?;
        let mut after = game.clone();
        after.play(mv)?;
        let same = evaluate_for_diagnostics(&model.engine, after.position(), side);
        let opposite = evaluate_for_diagnostics(&model.engine, after.position(), side.opposite());
        let record = Record::from_position(after.position(), 0, Outcome::Draw, 1, 0);
        let bytes = record.encode();
        let reference = model.evaluate(&Features::new(&record));
        let delta = -opposite.0 - before.0;
        if reference != opposite.0 || -reference - before.0 != delta {
            return Err(format!("engine promotion evaluation or delta disagrees with the independent integer reference: {text}").into());
        }
        rows.push(Promotion {
            r#move: text,
            capture: after.position().occupied().popcount() < position.occupied().popcount(),
            promote: true,
            terminal: after.result().is_some(),
            delta,
            delta_pst: -opposite.1 - before.1,
            after_same: same.0,
            after_same_pst: same.1,
            after_opposite: opposite.0,
            after_opposite_pst: opposite.1,
            placement: same.0 - before.0,
            placement_pst: same.1 - before.1,
            turn: -(opposite.0 + same.0),
            turn_pst: -(opposite.1 + same.1),
            after: AfterPosition {
                board: bytes[..144].to_vec(),
                stm: bytes[144],
                lion: bytes[145],
                eval: opposite.0,
                eval_pst: opposite.1,
            },
        });
    }
    Ok(rows)
}

pub(super) struct PositionReport {
    pub representatives: Vec<Representative>,
    pub reversals: usize,
    pub removals: Models<Agreement>,
    pub promotions: Models<usize>,
}

pub(super) fn diagnose(
    models: &Models<Weights>,
    selected: &[(usize, u64, Record)],
    directory: &Path,
) -> Result<PositionReport> {
    // 初期局面の記録は盤面と先獅子以外の欄を0にし、結果と対局番号も0で保存する。
    let initial = Record::from_position(&Position::initial(), 0, Outcome::Loss, 0, 0);
    let records: Vec<_> = std::iter::once(initial)
        .chain(selected.iter().map(|(_, _, record)| record.clone()))
        .collect();
    write_mnsd(
        &directory.join("representatives.bin"),
        &records,
        *models.base.engine.checksum(),
    )?;
    for (name, model) in models.each() {
        check(model, &records, name)?;
    }
    let mut report = PositionReport {
        representatives: Vec::new(),
        reversals: 0,
        removals: Models::default(),
        promotions: Models::default(),
    };
    let mut variants = Vec::new();
    for (index, record) in records.iter().enumerate() {
        let features = Features::new(record);
        let before = Models {
            base: models.base.evaluate(&features),
            candidate: models.candidate.evaluate(&features),
        };
        let mut removals = Vec::new();
        let bytes = record.encode();
        for (square, &byte) in bytes[..144].iter().enumerate() {
            if byte == 0 {
                continue;
            }
            let (state, color) = piece(byte);
            if [11, 21].contains(&state) {
                continue;
            }
            let mut removed = bytes;
            removed[square] = 0;
            let variant = Record::decode(&removed)?;
            let removed_features = Features::new(&variant);
            let scores = Models {
                base: models.base.evaluate(&removed_features),
                candidate: models.candidate.evaluate(&removed_features),
            };
            let delta = Models {
                base: scores.base - before.base,
                candidate: scores.candidate - before.candidate,
            };
            if i64::from(delta.base) * i64::from(delta.candidate) < 0 {
                report.reversals += 1;
            }
            let mut total = Models::default();
            let mut skipped = Skipped::default();
            match variant.to_position() {
                Err(error) => {
                    skipped.base = Some(error.to_string());
                    skipped.candidate = Some(error.to_string());
                    report.removals.base.skipped += 1;
                    report.removals.candidate.skipped += 1;
                }
                Ok(position) => {
                    for (name, model, expected) in [
                        ("base", &models.base, scores.base),
                        ("candidate", &models.candidate, scores.candidate),
                    ] {
                        let engine = evaluate_for_diagnostics(
                            &model.engine,
                            &position,
                            position.side_to_move(),
                        )
                        .0;
                        if engine != expected {
                            return Err(
                                format!("engine removal evaluation disagrees for {name}").into()
                            );
                        }
                    }
                    total.base = Some(delta.base);
                    total.candidate = Some(delta.candidate);
                    report.removals.base.checked += 1;
                    report.removals.candidate.checked += 1;
                }
            }
            variants.push(variant);
            removals.push(Removal {
                square,
                piece_byte: byte,
                relative_color: u8::from(color != bytes[144]),
                state,
                delta_cp: delta,
                total_delta_cp: total,
                skipped,
            });
        }
        let base_promotions = promotions(&models.base, record)?;
        let candidate_promotions = promotions(&models.candidate, record)?;
        report.promotions.base += base_promotions.len();
        report.promotions.candidate += candidate_promotions.len();
        let promotion_reason = base_promotions.is_empty().then_some("no legal promotion");
        report.representatives.push(Representative {
            label: if index == 0 {
                "initial".into()
            } else {
                format!("band{}", selected[index - 1].0)
            },
            index: (index != 0).then(|| selected[index - 1].1),
            piece_count: features.count,
            phase_numerator: features.q,
            total_evaluations: Models {
                base: before.base,
                candidate: before.candidate,
            },
            evaluations: before,
            removals,
            promotions: Models {
                base: (!base_promotions.is_empty()).then_some(base_promotions),
                candidate: (!candidate_promotions.is_empty()).then_some(candidate_promotions),
            },
            promotion_reason,
        });
    }
    write_mnsd(&directory.join("removals.bin"), &variants, [0; 32])?;
    Ok(report)
}
