#![allow(dead_code)]

#[path = "mnsd.rs"]
pub mod mnsd;
pub mod training;
use minase::training::records::Header;
use minase_core::{Color, PieceCode, PieceKind, Position, PositionBuilder, Square};
use minase_trainer::data::{Outcome, Sample};
use minase_trainer::{
    data::Dataset,
    nnue::{
        data::{BatchSlot, InputFeatureSet, build_batches},
        model::{Masks, Model, Shape},
    },
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
pub struct Fixture {
    pub root: PathBuf,
    pub dataset: Dataset,
}
impl Fixture {
    pub fn new(positions: &[Position]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "nnue-reference-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("records.bin");
        let header = Header::new(
            "L0,P0,R1,E0".into(),
            "1".repeat(40),
            [2; 32],
            minase::training::records::TeacherKind::AlphaBeta,
            200,
            11,
        )
        .unwrap();
        let mut samples = Vec::new();
        for (i, position) in positions.iter().enumerate() {
            let record = Sample::from_position(
                position,
                [120, -80, 30][i % 3],
                [Outcome::Win, Outcome::Loss, Outcome::Draw][i % 3],
                1,
                i as u16,
                minase::training::records::Origin::RandomSelfplay,
            )
            .unwrap();
            samples.push(record.clone());
        }
        fs::write(&path, mnsd::independent(header, &samples)).unwrap();
        Self {
            dataset: Dataset::open(&[path], None, std::num::NonZeroUsize::new(1).unwrap())
                .unwrap()
                .with_lambda(1.0)
                .unwrap(),
            root,
        }
    }
    pub fn batch(&self, set: InputFeatureSet, indices: &[u64]) -> BatchSlot {
        build_batches(&self.dataset, set, &[indices])
            .unwrap()
            .remove(0)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

pub fn position(
    pieces: &[(u8, u8, Color, PieceKind)],
    side: Color,
    lion: Option<(u8, u8)>,
) -> Position {
    let mut builder = PositionBuilder::new(side);
    for &(file, rank, color, kind) in pieces {
        builder
            .put(
                Square::new(file, rank).unwrap(),
                PieceCode::new(color, kind).unwrap(),
            )
            .unwrap();
    }
    let mut position = builder.finish().unwrap();
    if let Some((file, rank)) = lion {
        position
            .set_lion_capture(Some(Square::new(file, rank).unwrap()))
            .unwrap();
    }
    position
}
pub fn positions() -> Vec<Position> {
    use Color::{Black as B, White as W};
    use PieceKind::{King, Pawn, Rook, SilverGeneral};
    vec![
        position(
            &[
                (4, 0, B, King),
                (7, 11, W, King),
                (2, 4, B, Pawn),
                (9, 4, B, Pawn),
                (8, 6, W, Rook),
            ],
            B,
            Some((5, 5)),
        ),
        position(
            &[
                (5, 0, B, King),
                (5, 10, W, King),
                (3, 4, B, Pawn),
                (1, 8, W, Pawn),
                (8, 8, W, SilverGeneral),
            ],
            W,
            None,
        ),
        position(
            &[
                (11, 2, B, King),
                (2, 10, W, King),
                (7, 4, B, Rook),
                (6, 8, W, Pawn),
            ],
            B,
            Some((9, 5)),
        ),
    ]
}
pub fn mirror(position: &Position) -> Position {
    let mut builder = PositionBuilder::new(position.side_to_move());
    for square in Square::all() {
        if let Some(piece) = position.piece_at(square) {
            builder
                .put(
                    Square::new(11 - square.file(), square.rank()).unwrap(),
                    piece,
                )
                .unwrap();
        }
    }
    let mut result = builder.finish().unwrap();
    if let Some(trigger) = position.lion_taken_by_non_lion() {
        result
            .set_lion_capture(Some(
                Square::new(11 - trigger.square.file(), trigger.square.rank()).unwrap(),
            ))
            .unwrap();
    }
    result
}
pub fn all_observed(set: InputFeatureSet) -> Masks {
    let mut canonical = vec![true; 6841];
    canonical[6840] = false;
    let mut input = vec![true; set.dimensions() + 1];
    input[set.dimensions()] = false;
    Masks::new(set, canonical, input).unwrap()
}
pub fn model(set: InputFeatureSet, width: usize) -> Model {
    model_with_head(
        set,
        width,
        minase_trainer::nnue::model::HeadKind::DirectScrelu,
    )
}
pub fn model_with_head(
    set: InputFeatureSet,
    width: usize,
    head: minase_trainer::nnue::model::HeadKind,
) -> Model {
    Model::initialize(
        Shape::new(set, width, 1000.0, head).unwrap(),
        &vec![[0.0; 2]; 13680],
        all_observed(set),
        71,
        0.05,
    )
    .unwrap()
}
