use super::{BatchSlot, InputFeatureSet, MAX_BATCH_SIZE};
use crate::data::{Outcome, Sample};
use crate::nnue::gpu::{Error, Result};
use minase_core::{Color, PieceCode, PieceKind, PositionBuilder, Square};
pub(crate) struct Random(pub(crate) u64);
impl Random {
    fn next(&mut self) -> u64 {
        let result = minase_core::rng::splitmix64(self.0);
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        result
    }
    pub(crate) fn centered(&mut self) -> f32 {
        ((self.next() >> 40) as f32 / 8_388_608.0) - 1.0
    }
}
pub(crate) fn synthetic_batch(set: InputFeatureSet, n: usize, seed: u64) -> Result<BatchSlot> {
    if !(1..=MAX_BATCH_SIZE).contains(&n) {
        return Err(Error::Invalid("synthetic batch size"));
    }
    let mut random = Random(seed);
    let mut records = Vec::with_capacity(n);
    let mut scores = Vec::with_capacity(n);
    let mut classes = Vec::with_capacity(n);
    for b in 0..n {
        let side = if b % 2 == 0 {
            Color::Black
        } else {
            Color::White
        };
        let mut builder = PositionBuilder::new(side);
        let mut squares: Vec<_> = Square::all().collect();
        for j in (1..squares.len()).rev() {
            squares.swap(j, (random.next() % (j + 1) as u64) as usize);
        }
        let count = [2, 3, 4, 45, 90, 91, 92, 93, 144][b % 9];
        for (j, &square) in squares[..count].iter().enumerate() {
            let (color, kind) = if j < 2 {
                (if j == 0 { side } else { side.opposite() }, PieceKind::King)
            } else {
                let kinds = [
                    PieceKind::Pawn,
                    PieceKind::Rook,
                    PieceKind::Lion,
                    PieceKind::SilverGeneral,
                    PieceKind::Bishop,
                    PieceKind::GoldGeneral,
                ];
                (
                    if random.next() & 1 == 0 {
                        side
                    } else {
                        side.opposite()
                    },
                    kinds[(random.next() % kinds.len() as u64) as usize],
                )
            };
            let piece = PieceCode::new(color, kind).ok_or(Error::Invalid("synthetic piece"))?;
            builder
                .put(square, piece)
                .map_err(|_| Error::Invalid("synthetic placement"))?;
        }
        let mut position = builder
            .finish()
            .map_err(|_| Error::Invalid("synthetic position"))?;
        if b % 2 == 0 {
            // 相手の王の升は必ず手番側の駒がなく、先獅子状態の不変条件を満たす。
            position
                .set_lion_capture(Some(squares[1]))
                .map_err(|_| Error::Invalid("synthetic lion capture"))?;
        }
        let score = (random.centered() * 2000.0) as i16;
        records.push(Sample::from_position(
            &position,
            score,
            [Outcome::Loss, Outcome::Draw, Outcome::Win][b % 3],
            (b / 60000) as u32,
            (b % 60000) as u16,
            minase::training::records::Origin::RandomSelfplay,
        )?);
        scores.push(f32::from(score));
        classes.push((b % 3) as u32);
    }
    Ok(BatchSlot::from_records(set, &records, &scores, &classes)?)
}
