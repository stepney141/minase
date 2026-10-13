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

#[cfg(test)]
mod tests {
    use super::*;

    // [`game_seed`]の再現性の契約を検査する。
    // 参照値はdocコメントの式から、実装を経由せずに計算した。
    #[test]
    fn game_seed_follows_the_documented_splitmix64_derivation() {
        // splitmix64(1 + 1) = splitmix64(2)
        assert_eq!(game_seed(1, 1).get(), 0x9758_35DE_1C97_56CE);
        assert_eq!(game_seed(0xDEAD_BEEF, 41).get(), 0xA472_D968_8D97_8A28);
        // 加算はラッピング演算: u64::MAX + 3 ≡ 2 ≡ 1 + 1 (mod 2^64)
        assert_eq!(game_seed(u64::MAX, 3), game_seed(1, 1));
        // 派生値0は固定の非ゼロ定数0x9E37_79B9_7F4A_7C15へ置換される。
        // 仕様式の各段は全単射なので出力0の原像は一意で、逆算により
        // 0x61C8_8646_80B5_83EB(增分定数の2の補数)である。
        assert_eq!(
            game_seed(0x61C8_8646_80B5_83EB - 3, 3).get(),
            0x9E37_79B9_7F4A_7C15
        );
    }
}
