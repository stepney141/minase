//! 決定的な擬似乱数生成器。
//!
//! 実行ファイルと試験が共有する道具であり、`#[doc(hidden)]`で公開するが、
//! ライブラリの安定した公開面には含めない。
//! 局面のZobristハッシュの初期化も実行時にこの乱数を使う。

use core::num::{NonZeroU64, NonZeroUsize};

/// splitmix64の增分定数(黄金比の64ビット表現)。
const SPLITMIX_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
/// splitmix64の第1混合乗数。
const SPLITMIX_MIX1: u64 = 0xBF58_476D_1CE4_E5B9;
/// splitmix64の第2混合乗数。
const SPLITMIX_MIX2: u64 = 0x94D0_49BB_1331_11EB;

/// 指定値をsplitmix64の有限混合で変換する。
pub fn splitmix64(value: u64) -> u64 {
    let mut mixed = value.wrapping_add(SPLITMIX_GAMMA);
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(SPLITMIX_MIX1);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(SPLITMIX_MIX2);
    mixed ^ (mixed >> 31)
}

/// 基本シードと系列番号から0でないシードを派生する。
///
/// 式は`splitmix64(base_seed.wrapping_add(number))`であり、
/// 結果が0なら`0x9E37_79B9_7F4A_7C15`に置き換える。
/// この式は再現性の契約であり、各利用箇所で変えると、`random-play`の各局、
/// 対局ハーネスの開始局面、および自己対局生成の開始局面が変わる。
/// 対局ハーネスはこの関数でペアのシードを派生し、そのシードから
/// 派生番号0で開始局面用、1から4で2局の各プレイヤー用のシードを作る。
/// `base_seed + number`を混合するため、隣接する基本シードの系列は1局ずれて重なる。
/// 複数系列を使う際の注意は`docs/lessons/derive-seed-adjacent-collision.md`にある。
pub fn derive_seed(base_seed: u64, number: u64) -> NonZeroU64 {
    match splitmix64(base_seed.wrapping_add(number)) {
        0 => NonZeroU64::new(SPLITMIX_GAMMA).expect("splitmix gamma is non-zero"),
        seed => NonZeroU64::new(seed).expect("the zero seed was handled separately"),
    }
}

/// xorshift64擬似乱数生成器。
pub struct XorShift64 {
    /// 乱数列の内部状態。
    state: u64,
}

impl XorShift64 {
    /// シードから生成器を作る。シードは0以外でなければならない。
    pub const fn new(seed: NonZeroU64) -> Self {
        Self { state: seed.get() }
    }

    /// 次の乱数を返す。
    #[allow(
        clippy::should_implement_trait,
        reason = "設計で定めた乱数生成APIのため"
    )]
    pub fn next(&mut self) -> u64 {
        let mut value = self.state;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.state = value;
        value
    }

    /// 0以上`length`未満の添字を返す。
    pub fn index(&mut self, length: NonZeroUsize) -> usize {
        let length = u64::try_from(length.get()).expect("usize must fit in u64");
        usize::try_from(self.next() % length)
            .expect("an index below a usize length must fit in usize")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [`derive_seed`]の契約により、乱数系列を停止させる0の派生値は非零定数へ置換する。
    #[test]
    fn pair_seed_replaces_the_zero_output() {
        // derive_seedの式を逆算すると、splitmix64の出力0の原像は0x61C8_8646_80B5_83EBである。
        assert_eq!(
            derive_seed(0x61C8_8646_80B5_83EB - 5, 5).get(),
            0x9E37_79B9_7F4A_7C15
        );
    }
}
