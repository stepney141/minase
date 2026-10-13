//! 全検証集合の二値交差エントロピーを監視する早期終了。
//! 監視量には駒除去損失と正則化を含めず、最良値は厳密な`<`でだけ更新する。
//! エポック0は記録するが最良の候補にせず、最後の最良更新からpatienceエポックで停止し、最良エポックの重みを書き出す。
//! 最適化器を作り直した直後の一時的な悪化で初期値が返ることを避けるため、エポック0を候補から外す。
//! 典拠はGoodfellow、Bengio、Courville『Deep Learning』7.8節のアルゴリズム7.1と、Kerasの`EarlyStopping`の`min_delta=0`、`restore_best_weights=True`である。

use super::{Error, Result};

/// 学習を終えたエポックのうち、検証損失が最小のもの。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BestEpoch {
    /// 1から始まるエポック番号。
    pub epoch: u64,
    /// 検証損失。
    pub validation_loss: f64,
}

/// エポック0を除外し、厳密な `<` で最良値を更新する。同値なら更新しない。
/// epochは呼び出し側が完了したエポック順に渡す。
pub fn update_best(best: &mut Option<BestEpoch>, epoch: u64, validation_loss: f64) -> Result<bool> {
    if !validation_loss.is_finite() || validation_loss < 0.0 {
        return Err(Error::Invalid("non-finite or negative validation loss"));
    }
    if epoch == 0 {
        return Ok(false);
    }
    let improved = best.is_none_or(|best| validation_loss < best.validation_loss);
    if improved {
        *best = Some(BestEpoch {
            epoch,
            validation_loss,
        });
    }
    Ok(improved)
}

/// 最良エポックからpatience回以上のエポックが経過したかを返す。
pub fn patience_reached(best: Option<BestEpoch>, epoch: u64, patience: u64) -> bool {
    best.is_some_and(|best| {
        epoch
            .checked_sub(best.epoch)
            .is_some_and(|elapsed| elapsed >= patience)
    })
}
