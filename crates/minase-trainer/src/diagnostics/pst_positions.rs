//! ライブラリでの評価、非王駒の除去、合法な成り手とMNSD成果物。
//!
//! 標本、代表局面、駒を除いた局面を、1局面1対局の合成MNSD版2として保存する。
//! `diagnostic-samples.bin`は全帯の標本を通算番号順に、`representatives.bin`は初期局面と帯番号順の代表局面を保存する。
//! この2ファイルの検査和は基準MNPTから取り、`removals.bin`の検査和は全0とする。
//! 除去局面は代表局面の順、升番号の順に並べる。
//! 対局番号は各ファイル内で1から増加し、価値と方策は持たない。
//! 除去局面が検証を通らない場合も診断計算では数えるが、ファイルには書かない。

use super::{
    Result,
    pst_reference::{Features, Models, Weights, piece},
};
use crate::data::{Outcome, Sample};
use minase::{
    eval::pst::evaluate_for_diagnostics,
    training::records_v2::{
        Entry, Exclusion, GameRecord, Header, Origin, Outcome as GameOutcome, Policy, TeacherKind,
        Value, Writer,
    },
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

/// 診断用のMNSD版2を新規作成する。
/// ヘッダは規則セット名`"L0,P0,R1,E0"`、生成コミット`"0"`×40、ノード上限0、シード0、αβ教師、渡された検査和である。
/// 各成果物の記録順と検査和の選択は[`pst_positions`](self)のモジュールdocコメントに従う。
pub(super) fn write_mnsd(path: &Path, records: &[Sample], checksum: [u8; 32]) -> Result<()> {
    let header = Header::new(
        "L0,P0,R1,E0".into(),
        "0".repeat(40),
        checksum,
        TeacherKind::AlphaBeta,
        0,
        0,
    )?;
    let file = BufWriter::new(OpenOptions::new().write(true).create_new(true).open(path)?);
    let mut writer = Writer::new(file, header)?;
    for (index, record) in records.iter().enumerate() {
        let number = u32::try_from(index)
            .ok()
            .and_then(|n| n.checked_add(1))
            .ok_or(minase::training::records_v2::Error::CountOverflow)?;
        writer.write_game(&GameRecord::new(
            number,
            Origin::Synthetic,
            GameOutcome::NotFromGame,
            record.ply(),
            record.to_position()?,
            vec![Entry::new(
                None,
                Value::None,
                Exclusion::None,
                Policy::None,
                None,
            )?],
        )?)?;
    }
    writer.finish()?.flush()?;
    Ok(())
}

pub(super) fn check(weights: &Weights, records: &[Sample], name: &str) -> Result<usize> {
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

/// `report.json`の`rust_removal_agreement`のモデルごとの件数。
/// `checked`はエンジン評価との一致を確かめた除去数、`skipped`は局面として復元できなかった除去数である。
#[derive(Default, Serialize)]
pub(super) struct Agreement {
    checked: usize,
    skipped: usize,
}
/// 除去局面を復元できない場合の理由。
/// 失敗時だけ`base`と`candidate`に理由の文字列を保存し、復元できれば空のJSONオブジェクトとなる。
#[derive(Default, Serialize)]
struct Skipped {
    #[serde(skip_serializing_if = "Option::is_none")]
    base: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate: Option<String>,
}
/// 代表局面から駒を1枚除いた結果。
/// `square`は升番号0〜143、`piece_byte`はMNSDの駒コード、`relative_color`は手番側0、相手側1、`state`は駒状態番号である。
/// `delta_cp`は除去後から除去前を引いたPST参照の評価差を`{base, candidate}`で持つ。
/// 除去で位相も変わるため、その駒固有の価値と同一視しない。
/// `total_delta_cp`は局面を復元してエンジン全体の評価と参照の一致を検査した場合に、同じ差分をモデルごとに保存する。
/// 復元できなければ両モデルの値は`null`となり、`skipped`に[`Skipped`]の理由を持つ。
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
/// `report.json`の`representatives`の1要素。
/// `label`は初期局面なら`"initial"`、それ以外は`"band{帯番号}"`、`index`は通算番号で初期局面では`null`である。
/// `piece_count`は盤上駒数、`phase_numerator`は位相分子q、`evaluations`は`{base, candidate}`の独立整数PST参照評価である。
/// `removals`は[`Removal`]の配列である。
/// `total_evaluations`はエンジン全体の評価との一致を確認した`evaluations`と同じ値である。
/// 呼び手はFMを持たない重みだけを受理するため、全体評価にFMを含まない。
/// `promotions`は`{base, candidate}`の各値に[`Promotion`]の配列を持ち、成り手がなければその値を`null`とする。
/// `promotion_reason`は基準に合法な成り手がなければ`"no legal promotion"`、あれば`null`である。
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
/// 成り手を適用した後の局面。
/// `board`はMNSD駒コード144バイト、`stm`は先手0か後手1の着手後の手番、`lion`は先獅子の対象升番号で、対象升がなければ255である。
/// `eval`は[`Promotion`]の`after_opposite`、`eval_pst`は`after_opposite_pst`である。
#[derive(Serialize)]
struct AfterPosition {
    board: Vec<u8>,
    stm: u8,
    lion: u8,
    eval: i32,
    eval_pst: i32,
}
/// 合法な成り手の評価差。単位はセンチポーンである。
/// 着手前の手番側から見た評価を`before`、同じ側から見た着手後評価を`after_same`、反対側から見た着手後評価を`after_opposite`とする。
/// `move`はUSI表記、`capture`は盤上駒数が減ったか、`promote`は常に真、`terminal`は着手後に終局したかである。
/// `delta = −after_opposite − before`、`placement = after_same − before`、`turn = −(after_opposite + after_same)`を保存する。
/// `after_same`と`after_opposite`も保存し、各評価と差分の`_pst`版はFMを除いたPSTだけの値である。
/// FMを持たない重みでは両系列の値は等しく、`after`に[`AfterPosition`]を持つ。
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

/// 代表局面の合法な成り手をエンジン既定の`L0,P0,R1,E0`で実際に適用し、[`Promotion`]を作る。
/// 着手後の独立整数PST参照が`after_opposite`と異なる場合、または参照から求めた差が`delta`と異なる場合は失敗する。
/// モデルごとの照合した手の総数を、呼び手が`rust_promotion_agreement`に数える。
fn promotions(model: &Weights, record: &Sample) -> Result<Vec<Promotion>> {
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
        let record = Sample::from_position(
            after.position(),
            0,
            Outcome::Draw,
            1,
            0,
            minase::training::records_v2::Origin::Synthetic,
        );
        let bytes = record.board_codes();
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
                board: bytes.iter().map(|code| code & 127).collect(),
                stm: record.side_to_move() as u8,
                lion: record.lion_square().map_or(255, |s| s.dense_index() as u8),
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

/// 初期局面と各帯の代表局面で、除去と成り手の評価を照合する。
/// 呼び手は各帯の抽出済み標本を全教師分類で合わせ、その中で通算番号が最小の局面を選んで帯番号順に渡す。
/// 代表局面と復元できた除去局面でエンジン評価と独立整数PST参照が1件でも異なれば失敗する。
/// 基準と候補の除去差分の積が負の件数を数え、呼び手が`pst_removal_sign_reversals`として記録する。
/// 除去局面を復元できない場合は[`Agreement`]と[`Skipped`]へ記録して続ける。
/// MNSD成果物の内容は[`pst_positions`](self)のモジュールdocコメントに従う。
pub(super) fn diagnose(
    models: &Models<Weights>,
    selected: &[(usize, u64, Sample)],
    directory: &Path,
) -> Result<PositionReport> {
    // 初期局面は手数0とし、書き出す際に合成対局の番号と結果を付ける。
    let initial = Sample::from_position(
        &Position::initial(),
        0,
        Outcome::Loss,
        0,
        0,
        minase::training::records_v2::Origin::Synthetic,
    );
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
        let bytes = record.board_codes();
        for (square, &byte) in bytes[..144].iter().enumerate() {
            if byte == 0 {
                continue;
            }
            let (state, color) = piece(byte & 127);
            if [11, 21].contains(&state) {
                continue;
            }
            let variant = record.without_piece(square);
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
                    variants.push(variant.clone());
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
            removals.push(Removal {
                square,
                piece_byte: byte & 127,
                relative_color: u8::from(color != record.side_to_move() as u8),
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
