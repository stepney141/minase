//! 一時的なPython照合用のPST初期値と更新状態。照合後に削除する。

use super::{Adam, Error, Losses, Model, Result, load_initial, model::canonical};
use crate::{
    data::{Dataset, replay::Replay},
    nn::{
        atomic_write,
        npz::{self, Array},
    },
};
use minase::eval::features::FEATURE_COUNT;
use std::path::Path;

pub(super) fn initial(replay: &Replay, expected: &Model) -> Result<Model> {
    let values = load_initial(replay.directory().join("initial.npz"))?;
    if values
        .iter()
        .flatten()
        .zip(expected.expanded().iter().flatten())
        .any(|(actual, expected)| actual.to_bits() != expected.to_bits())
    {
        return Err(Error::Invalid("replay initial PST differs from --init"));
    }
    Model::initialize(&values, expected.k())
}

pub(super) fn order(replay: &Replay, dataset: &Dataset, epoch: u64) -> Result<Vec<u64>> {
    let epoch = replay.read_epoch(dataset, epoch)?;
    if epoch.mirror.iter().any(|&value| value != 0) {
        return Err(Error::Invalid("PST replay mirror bits must all be zero"));
    }
    Ok(epoch.indices)
}

#[derive(Default)]
pub(super) struct Steps {
    losses: Vec<[f64; 3]>,
}

impl Steps {
    pub(super) fn record(&mut self, losses: Losses, coefficient: f64) {
        self.losses.push([
            losses.bce,
            losses.removal,
            losses.bce + coefficient * losses.removal,
        ]);
    }

    pub(super) fn write(&self, directory: &Path, model: &Model, optimizer: &Adam) -> Result<()> {
        let mut arrays = Vec::new();
        for (column, name) in ["loss_bce", "loss_removal", "loss_total"]
            .into_iter()
            .enumerate()
        {
            arrays.push((
                name,
                Array {
                    dtype: "<f8".into(),
                    shape: vec![self.losses.len()],
                    data: self
                        .losses
                        .iter()
                        .flat_map(|row| row[column].to_le_bytes())
                        .collect(),
                },
            ));
        }
        for (names, values) in [
            (["middlegame", "endgame"], model.weights()),
            (["exp_avg_middlegame", "exp_avg_endgame"], optimizer.m()),
            (
                ["exp_avg_sq_middlegame", "exp_avg_sq_endgame"],
                optimizer.v(),
            ),
        ] {
            for (column, name) in names.into_iter().enumerate() {
                arrays.push((
                    name,
                    Array {
                        dtype: "<f4".into(),
                        shape: vec![FEATURE_COUNT],
                        data: (0..FEATURE_COUNT)
                            .flat_map(|feature| {
                                values[2 * canonical(feature) + column].to_le_bytes()
                            })
                            .collect(),
                    },
                ));
            }
        }
        let step = i64::try_from(optimizer.step())
            .map_err(|_| Error::Invalid("replay step exceeds int64"))?;
        arrays.push((
            "step",
            Array {
                dtype: "<i8".into(),
                shape: vec![],
                data: step.to_le_bytes().to_vec(),
            },
        ));
        Ok(atomic_write(
            &directory.join("rust-steps.npz"),
            &npz::encode(&arrays)?,
        )?)
    }
}
