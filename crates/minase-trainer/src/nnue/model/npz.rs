use super::{Error, Result, validate_initial};
use crate::nn::npz;
use minase::eval::features::FEATURE_COUNT;
use std::path::Path;

/// 圧縮なしNPZからNNUEの線形初期値を読む。ほかのメンバーは読み飛ばす。
/// middlegameとendgameのdtype、13,680要素、有限性、保存範囲と鏡映共有を検査する。
pub fn load_initial(path: impl AsRef<Path>) -> Result<Vec<[f32; 2]>> {
    let mut arrays = npz::decode_selected(&std::fs::read(path)?, &["middlegame", "endgame"])?;
    let mut initial = vec![[0.0; 2]; FEATURE_COUNT];
    for (endpoint, key) in ["middlegame", "endgame"].into_iter().enumerate() {
        let array = arrays
            .remove(key)
            .ok_or(Error::Format("missing NPZ endpoint"))?;
        if array.dtype != "<f4" || array.shape != [FEATURE_COUNT] {
            return Err(Error::Format("initial NPZ dtype or shape"));
        }
        for (row, &bytes) in initial.iter_mut().zip(array.data.as_chunks::<4>().0) {
            row[endpoint] = f32::from_le_bytes(bytes);
        }
    }
    validate_initial(&initial)?;
    Ok(initial)
}
