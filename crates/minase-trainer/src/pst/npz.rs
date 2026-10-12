use super::{Error, Result};
use crate::nn::npz::{self, Array};
use minase::eval::features::{FEATURE_COUNT, mirror_feature};
use std::path::Path;

fn validate(initial: &[[f32; 2]]) -> Result<()> {
    if initial.len() != FEATURE_COUNT
        || initial
            .iter()
            .flatten()
            .any(|value| !value.is_finite() || !(-4096.0..=4095.875).contains(value))
    {
        return Err(Error::Invalid(
            "initial PST must be finite [13680, 2] in -4096..4095.875",
        ));
    }
    if initial
        .iter()
        .enumerate()
        .any(|(base, row)| *row != initial[mirror_feature(base)])
    {
        return Err(Error::Invalid("initial PST mirror sharing violated"));
    }
    Ok(())
}

/// 2端点をmiddlegame、endgameのf32配列として保存する。鏡映共有と保存範囲を検査する。
pub fn encode_pst_npz(initial: &[[f32; 2]]) -> Result<Vec<u8>> {
    validate(initial)?;
    let arrays = ["middlegame", "endgame"].map(|name| {
        (
            name,
            Array {
                dtype: "<f4".into(),
                shape: vec![FEATURE_COUNT],
                data: Vec::with_capacity(FEATURE_COUNT * 4),
            },
        )
    });
    let mut arrays = arrays;
    for (e, (_, array)) in arrays.iter_mut().enumerate() {
        for row in initial {
            array.data.extend(row[e].to_le_bytes());
        }
    }
    Ok(npz::encode(&arrays)?)
}

/// 圧縮なしNPZのPST端点を読み、dtype、13,680要素、鏡映共有と保存範囲を検査する。
pub fn load_initial(path: impl AsRef<Path>) -> Result<Vec<[f32; 2]>> {
    let mut arrays = npz::decode(&std::fs::read(path)?)?;
    let mut initial = vec![[0.0; 2]; FEATURE_COUNT];
    for (e, key) in ["middlegame", "endgame"].into_iter().enumerate() {
        let array = arrays
            .remove(key)
            .ok_or(Error::Format("missing NPZ endpoint"))?;
        if array.dtype != "<f4" || array.shape != [FEATURE_COUNT] {
            return Err(Error::Format("PST NPZ dtype or shape"));
        }
        for (row, &bytes) in initial.iter_mut().zip(array.data.as_chunks::<4>().0) {
            row[e] = f32::from_le_bytes(bytes);
        }
    }
    validate(&initial)?;
    Ok(initial)
}
