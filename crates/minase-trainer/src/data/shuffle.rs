const STEP: u64 = 0x9E37_79B9_7F4A_7C15;

fn mix64(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

/// ファイルのシードと対局番号から、対局単位の分割を固定するハッシュ。
/// 式は`mix64(seed ^ game·0x9E3779B97F4A7C15)`で、乗算はu64の折り返し演算である。
/// `mix64`はSplitMix64の出力混合関数であり、状態加算を行わない。
pub fn hash64(seed: u64, game: u32) -> u64 {
    mix64(seed ^ u64::from(game).wrapping_mul(STEP))
}

/// [`hash64(seed, game)`](hash64)が20の倍数の対局を検証集合へ割り当てる。
/// 来歴の`games`が`null`のファイル、すなわち乱数開始の自己対局だけに使う。
/// 棋譜IDを持つファイルは[`Dataset::open`](super::Dataset::open)の棋譜IDの規則で分割する。
pub fn is_validation(seed: u64, game: u32) -> bool {
    hash64(seed, game).is_multiple_of(20)
}

/// 訓練局面の通し番号をSplitMix64によるFisher–Yatesで並べ替える。
///
/// 初期状態は`mix64(seed ^ epoch.wrapping_mul(0x9e3779b97f4a7c15))`。
/// 乱数を得るたびに状態へ同じ定数を足してから混合し、棄却法で剰余の偏りを除く。
/// 入力の順序、seed、epochが同じなら、プラットフォームやワーカー数に依存しない。
pub fn epoch_permutation(training_indices: &[u64], seed: u64, epoch: u64) -> Vec<u64> {
    let mut indices = training_indices.to_vec();
    let mut state = mix64(seed ^ epoch.wrapping_mul(STEP));
    for end in (1..indices.len()).rev() {
        let bound = end as u64 + 1;
        let threshold = bound.wrapping_neg() % bound;
        let selected = loop {
            state = state.wrapping_add(STEP);
            let value = mix64(state);
            if value >= threshold {
                break (value % bound) as usize;
            }
        };
        indices.swap(end, selected);
    }
    indices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash64_matches_python_vectors() {
        // hash64のビット演算から定まる既知値であり、通常のSplitMix64の状態加算は行わない。
        for (seed, game, expected) in [
            (0, 0, 0),
            (1, 0, 0x5692_161D_100B_05E5),
            (0, 1, 0xE220_A839_7B1D_CDAF),
            (u64::MAX, u32::MAX, 0x3A9B_57C2_77B2_2E0A),
        ] {
            assert_eq!(hash64(seed, game), expected);
        }
    }

    #[test]
    fn validation_split_matches_python_games() {
        // hash64(seed, game) % 20 == 0となる0..80の対局番号である。
        for (seed, expected) in [
            (11, vec![4, 15, 22, 41, 48, 59, 70]),
            (22, vec![6, 8, 35, 44, 58, 63, 74]),
        ] {
            let actual: Vec<_> = (0..80).filter(|&game| is_validation(seed, game)).collect();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn epoch_permutation_is_fixed_and_preserves_indices() {
        // epoch_permutationの初期化と棄却法を独立な整数演算で評価した固定列である。
        let indices: Vec<u64> = (0..12).collect();
        for (seed, epoch, expected) in [
            (7, 0, [3, 11, 1, 4, 6, 0, 10, 9, 5, 8, 2, 7]),
            (7, 1, [6, 7, 0, 3, 10, 5, 4, 9, 1, 11, 8, 2]),
            (8, 0, [6, 5, 0, 11, 7, 4, 3, 8, 10, 9, 1, 2]),
            (u64::MAX, u64::MAX, [11, 6, 1, 4, 7, 5, 9, 0, 2, 10, 8, 3]),
        ] {
            let actual = epoch_permutation(&indices, seed, epoch);
            assert_eq!(actual, expected);
            assert_eq!(actual, epoch_permutation(&indices, seed, epoch));
            let mut sorted = actual;
            sorted.sort_unstable();
            assert_eq!(sorted, indices);
        }
        assert!(epoch_permutation(&[], 0, 0).is_empty());
        assert_eq!(epoch_permutation(&[u64::MAX], 0, 0), [u64::MAX]);
        let mut repeated = epoch_permutation(&[5, 5, 1], 0, 0);
        repeated.sort_unstable();
        assert_eq!(repeated, [1, 5, 5]);
    }
}
