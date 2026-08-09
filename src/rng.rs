//! 決定的な擬似乱数生成器。

/// xorshift64擬似乱数生成器。
pub struct XorShift64 {
    /// 乱数列の内部状態。
    state: u64,
}

impl XorShift64 {
    /// シードから生成器を作る。シードは0以外でなければならない。
    pub fn new(seed: u64) -> Self {
        assert_ne!(seed, 0);
        Self { state: seed }
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
    pub fn index(&mut self, length: usize) -> usize {
        self.next() as usize % length
    }
}
