//! 一時的なPython照合用の初期値と更新状態。フェーズ6で削除する。

use super::{Adam, Args, Losses, Model, Result, model::V_LEN, npz::floats};
use crate::{
    data::{
        Dataset,
        replay::{Epoch, Replay},
    },
    nn::{
        atomic_write,
        npz::{self, Array},
    },
};
use minase::eval::{features::FEATURE_COUNT, pst::FM_RANK};
use std::path::Path;

pub(super) fn initial(replay: &Replay, mask: &[bool], k: f32) -> Result<Model> {
    let mut arrays = npz::decode(&std::fs::read(replay.directory().join("initial.npz"))?)?;
    if arrays.len() != 3 {
        return Err("FM replay initial keys must be V, a, mask".into());
    }
    let observed = arrays.remove("mask").ok_or("missing FM replay mask")?;
    if observed.dtype != "|b1"
        || observed.shape != [FEATURE_COUNT]
        || observed.data.iter().any(|&v| v > 1)
    {
        return Err("FM replay mask dtype or shape".into());
    }
    if observed
        .data
        .iter()
        .map(|&v| v != 0)
        .ne(mask.iter().copied())
    {
        return Err("FM replay mask differs from training observations".into());
    }
    let mut read = |key, shape: &[usize]| -> Result<Vec<f32>> {
        let array = arrays
            .remove(key)
            .ok_or_else(|| format!("missing FM replay {key}"))?;
        if array.dtype != "<f4" || array.shape != shape {
            return Err(format!("FM replay {key} dtype or shape").into());
        }
        Ok(array
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&b| f32::from_le_bytes(b))
            .collect())
    };
    let v = read("V", &[FEATURE_COUNT, FM_RANK])?;
    let a = read("a", &[FM_RANK])?;
    Model::from_weights(
        v.as_chunks::<FM_RANK>().0,
        a.try_into().expect("checked rank"),
        mask.to_vec(),
        k,
    )
}

pub(super) fn epochs(args: &Args, training_count: usize) -> u64 {
    args.replay_steps.map_or(args.epochs, |n| {
        n.div_ceil(training_count.div_ceil(args.batch) as u64)
    })
}

pub(super) fn order(replay: &Replay, dataset: &Dataset, args: &Args, epoch: u64) -> Result<Epoch> {
    let count = dataset.training_indices().len();
    let required = args.replay_steps.map_or(count, |limit| {
        let steps = count.div_ceil(args.batch) as u64;
        let remaining = limit.saturating_sub((epoch - 1) * steps).min(steps);
        (u128::from(remaining) * args.batch as u128).min(count as u128) as usize
    });
    Ok(replay.read_epoch_prefix(dataset, epoch, required)?)
}

#[derive(Default)]
pub(super) struct Steps {
    losses: Vec<Losses>,
}
impl Steps {
    pub(super) fn record(&mut self, losses: Losses) {
        self.losses.push(losses);
    }
    pub(super) fn write(&self, directory: &Path, model: &Model, optimizer: &Adam) -> Result<()> {
        let mut arrays = Vec::new();
        for (column, name) in ["loss_bce", "loss_reg", "loss_total"]
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
                        .flat_map(|loss| [loss.bce, loss.reg, loss.total()][column].to_le_bytes())
                        .collect(),
                },
            ));
        }
        for (names, values) in [
            (["V", "a"], model.weights.as_slice()),
            (["exp_avg_V", "exp_avg_a"], optimizer.m()),
            (["exp_avg_sq_V", "exp_avg_sq_a"], optimizer.v()),
        ] {
            arrays.push((
                names[0],
                floats(
                    &values[..FEATURE_COUNT * FM_RANK],
                    &[FEATURE_COUNT, FM_RANK],
                ),
            ));
            arrays.push((names[1], floats(&values[V_LEN..], &[FM_RANK])));
        }
        arrays.push((
            "step",
            Array {
                dtype: "<i8".into(),
                shape: vec![],
                data: i64::try_from(optimizer.step())?.to_le_bytes().to_vec(),
            },
        ));
        Ok(atomic_write(
            &directory.join("rust-steps.npz"),
            &npz::encode(&arrays)?,
        )?)
    }
}
