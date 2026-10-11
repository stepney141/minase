const STEP: u64 = 0x9E37_79B9_7F4A_7C15;

fn mix64(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

/// Pythonの`data.mnsd.hash64`と同じ、ファイルのseedと対局番号のハッシュ。
pub fn hash64(seed: u64, game: u32) -> u64 {
    mix64(seed ^ u64::from(game).wrapping_mul(STEP))
}

/// 自己対局の記録を、対局単位で20分の1の検証集合へ割り当てる。
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
        // tools/train/tests/test_mnsd.pyの既知値。加算を伴う通常のSplitMix64とは異なる。
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
        // 同じPythonテストが固定した、0..80の検証用対局番号。
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
        // 上記の初期化・棄却法を独立なPython整数演算で評価した固定列。
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
