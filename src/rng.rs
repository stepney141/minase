//! 決定的な擬似乱数生成器。

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
