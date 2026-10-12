/// ロジットと0以上1以下の教師値に対する二値交差エントロピー。
/// 有限な入力をf64のまま、最大値、教師項、補正項の順で計算する。
pub fn bce_with_logits(logit: f64, target: f64) -> f64 {
    logit.max(0.0) - logit * target + (-logit.abs()).exp().ln_1p()
}

/// 二値交差エントロピーのロジットに関する微分。平均化や尺度変換は行わない。
pub fn bce_with_logits_gradient(logit: f64, target: f64) -> f64 {
    1.0 / (1.0 + (-logit).exp()) - target
}
