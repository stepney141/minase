//! ランダム対局の基本シードを生成し、局シードを派生する。

use std::{
    num::NonZeroU64,
    time::{SystemTime, UNIX_EPOCH},
};

/// 現在時刻から基本シードを生成する。
pub(super) fn time_seed() -> Result<u64, std::time::SystemTimeError> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    Ok(nanos as u64 ^ (nanos >> 64) as u64)
}

/// 基本シードと1起算の局番号から局シードを派生する。
///
/// 再現性の契約は`splitmix64(base_seed.wrapping_add(n))`であり、`n`は1起算の局番号である。
/// 結果が0なら`0x9E37_79B9_7F4A_7C15`に置き換える。
pub(super) fn game_seed(base_seed: u64, game_number: u64) -> NonZeroU64 {
    minase_core::rng::derive_seed(base_seed, game_number)
}
