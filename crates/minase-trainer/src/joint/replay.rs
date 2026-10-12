//! 一時的なPython照合用の初期値と更新状態。フェーズ6で削除する。

use super::{Adam, Args, Losses, Model, Result};
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
use crate::{
    fm::{model::V_LEN, npz::floats},
    pst::model::canonical,
};
use minase::eval::{features::FEATURE_COUNT, pst::FM_RANK};
use std::path::Path;

pub(super) fn initial(replay: &Replay, expected: &Model) -> Result<()> {
    let actual = npz::decode(&std::fs::read(replay.directory().join("initial.npz"))?)?;
    let expected = super::export::arrays(expected);
    if actual.len() != expected.len() {
        return Err("joint replay initial keys must be middlegame, endgame, V, a, mask".into());
    }
    for (key, value) in expected {
        let other = actual
            .get(key)
            .ok_or_else(|| format!("missing joint replay {key}"))?;
        if other.dtype != value.dtype || other.shape != value.shape || other.data != value.data {
            return Err(format!(
                "joint replay initial {key} differs from initialized weights or observations"
            )
            .into());
        }
    }
    Ok(())
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
        for (column, name) in ["loss_bce", "loss_removal", "loss_reg", "loss_total"]
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
                        .flat_map(|loss| {
                            [loss.bce, loss.removal, loss.reg, loss.total][column].to_le_bytes()
                        })
                        .collect(),
                },
            ));
        }
        for (names, values) in [
            (["middlegame", "endgame"], model.pst.weights()),
            (["exp_avg_middlegame", "exp_avg_endgame"], optimizer.pst.m()),
            (
                ["exp_avg_sq_middlegame", "exp_avg_sq_endgame"],
                optimizer.pst.v(),
            ),
        ] {
            for (e, name) in names.into_iter().enumerate() {
                let expanded: Vec<_> = (0..FEATURE_COUNT)
                    .map(|f| values[2 * canonical(f) + e])
                    .collect();
                arrays.push((name, floats(&expanded, &[FEATURE_COUNT])));
            }
        }
        for (names, values) in [
            (["V", "a"], model.fm.weights.as_slice()),
            (["exp_avg_V", "exp_avg_a"], optimizer.fm.m()),
            (["exp_avg_sq_V", "exp_avg_sq_a"], optimizer.fm.v()),
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
