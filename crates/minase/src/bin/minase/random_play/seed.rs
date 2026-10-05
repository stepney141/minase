//! ランダム対局の基本シードを生成し、局シードを派生する。

use std::{
    num::NonZeroU64,
    time::{SystemTime, UNIX_EPOCH},
};

/// splitmix64の增分定数(黄金比の64ビット表現)。
const SPLITMIX_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
/// splitmix64の第1混合乗数。
const SPLITMIX_MIX1: u64 = 0xBF58_476D_1CE4_E5B9;
/// splitmix64の第2混合乗数。
const SPLITMIX_MIX2: u64 = 0x94D0_49BB_1331_11EB;

/// 現在時刻から基本シードを生成する。
pub(super) fn time_seed() -> Result<u64, std::time::SystemTimeError> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    Ok(nanos as u64 ^ (nanos >> 64) as u64)
}

/// 指定値をsplitmix64の有限混合で変換する。
fn splitmix64(value: u64) -> u64 {
    let mut mixed = value.wrapping_add(SPLITMIX_GAMMA);
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(SPLITMIX_MIX1);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(SPLITMIX_MIX2);
    mixed ^ (mixed >> 31)
}

/// 基本シードと1起算の局番号から局シードを派生する。
pub(super) fn game_seed(base_seed: u64, game_number: u64) -> NonZeroU64 {
    match splitmix64(base_seed.wrapping_add(game_number)) {
        0 => NonZeroU64::new(SPLITMIX_GAMMA).expect("splitmix gamma is non-zero"),
        seed => NonZeroU64::new(seed).expect("the zero seed was handled separately"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // D8-HARN-17(random-play.mdシード派生節): 局シードは
    // splitmix64(base_seed.wrapping_add(n))であり、式は仕様として固定されて
    // いる。参照値は仕様式から独立に(実装を経由せずに)計算した。
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
