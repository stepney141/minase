/// LayerStackのCPUとCUDAが集める検証統計。
/// 非線形出力はセンチポーン単位である。
#[derive(Clone, Debug, PartialEq)]
pub struct Validation {
    /// 局面ごとのlossの和。
    pub loss_sum: f64,
    /// 引き分けを除き、勝敗とロジットの正負が一致した局面数。
    pub correct: u64,
    /// 引き分けでない局面数。
    pub decisive: u64,
    /// 非線形出力の和。
    pub nonlinear_sum: f64,
    /// 非線形出力の二乗の和。
    pub nonlinear_square_sum: f64,
    /// acc ≤ 0の要素数。
    pub lower: u64,
    /// acc ≥ 1の要素数。
    pub upper: u64,
    /// 局面数。
    pub positions: usize,
    /// 累算器の要素数B×2×H。
    pub activations: usize,
    /// 第1隠れ層のz≤0の要素数。
    pub hidden1_lower: u64,
    /// 第1隠れ層のz≥1の要素数。
    pub hidden1_upper: u64,
    /// 第1隠れ層の要素数B×15。
    pub hidden1_activations: usize,
    /// 第2隠れ層のa≤0の要素数。
    pub hidden2_lower: u64,
    /// 第2隠れ層のa≥1の要素数。
    pub hidden2_upper: u64,
    /// 第2隠れ層の要素数B×32。
    pub hidden2_activations: usize,
    /// 両因子の累算値が正である要素積の個数。
    pub pairwise_positive: u64,
    /// 全局面、全視点の要素積の個数B×H。
    pub pairwise_products: usize,
}
impl Validation {
    /// バッチ平均のloss。
    pub fn loss(&self) -> f64 {
        self.loss_sum / self.positions as f64
    }
    /// 符号一致率。全局面が引き分けならNone。
    pub fn sign_accuracy(&self) -> Option<f64> {
        (self.decisive != 0).then(|| self.correct as f64 / self.decisive as f64)
    }
    /// 非線形出力の母標準偏差、センチポーン。
    pub fn nonlinear_std_cp(&self) -> f64 {
        let mean = self.nonlinear_sum / self.positions as f64;
        (self.nonlinear_square_sum / self.positions as f64 - mean * mean)
            .max(0.0)
            .sqrt()
    }
    /// acc ≤ 0とacc ≥ 1の割合。
    pub fn saturation(&self) -> [f64; 2] {
        [
            self.lower as f64 / self.activations as f64,
            self.upper as f64 / self.activations as f64,
        ]
    }
    /// 幅15のclipped ReLUの枝についてz≤0とz≥1の割合。
    pub fn hidden1_saturation(&self) -> [f64; 2] {
        [
            self.hidden1_lower as f64 / self.hidden1_activations as f64,
            self.hidden1_upper as f64 / self.hidden1_activations as f64,
        ]
    }
    /// 幅32の層についてa≤0とa≥1の割合。
    pub fn hidden2_saturation(&self) -> [f64; 2] {
        [
            self.hidden2_lower as f64 / self.hidden2_activations as f64,
            self.hidden2_upper as f64 / self.hidden2_activations as f64,
        ]
    }
    /// 全要素積のうち両因子の累算値が正である割合。
    pub fn pairwise_both_positive(&self) -> f64 {
        self.pairwise_positive as f64 / self.pairwise_products as f64
    }
    /// CPUとCUDAで共通の12列。
    /// loss和、一致数、勝敗局面数、非線形cp和、二乗和、acc≤0数、acc≥1数、
    /// z≤0数、z≥1数、a≤0数、a≥1数、要素積の両因子が正の個数の順。
    pub(crate) fn from_sums(sums: &[f64; 12], positions: usize, width: usize) -> Self {
        Self {
            loss_sum: sums[0],
            correct: sums[1] as u64,
            decisive: sums[2] as u64,
            nonlinear_sum: sums[3],
            nonlinear_square_sum: sums[4],
            lower: sums[5] as u64,
            upper: sums[6] as u64,
            hidden1_lower: sums[7] as u64,
            hidden1_upper: sums[8] as u64,
            hidden2_lower: sums[9] as u64,
            hidden2_upper: sums[10] as u64,
            pairwise_positive: sums[11] as u64,
            hidden1_activations: positions * 15,
            hidden2_activations: positions * 32,
            pairwise_products: positions * width,
            positions,
            activations: positions * 2 * width,
        }
    }
    pub(crate) fn sums(&self) -> [f64; 12] {
        [
            self.loss_sum,
            self.correct as f64,
            self.decisive as f64,
            self.nonlinear_sum,
            self.nonlinear_square_sum,
            self.lower as f64,
            self.upper as f64,
            self.hidden1_lower as f64,
            self.hidden1_upper as f64,
            self.hidden2_lower as f64,
            self.hidden2_upper as f64,
            self.pairwise_positive as f64,
        ]
    }
}
