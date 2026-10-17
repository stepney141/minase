const STEP: u64 = 0x9E37_79B9_7F4A_7C15;

pub(super) fn mix64(mut value: u64) -> u64 {
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
/// すべての由来で、MNSDヘッダのシードと記録の対局番号を使う。
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

/// SplitMix64の状態を進め、棄却法で0以上bound未満の整数を得る。
fn bounded(state: &mut u64, bound: u64) -> u64 {
    let threshold = bound.wrapping_neg() % bound;
    loop {
        *state = state.wrapping_add(STEP);
        let value = mix64(*state);
        if value >= threshold {
            return value % bound;
        }
    }
}

/// 各訓練局面に独立で一様なbucket番号を割り当てる乱数列。
pub struct BucketAssigner {
    state: u64,
    buckets: u32,
}

impl BucketAssigner {
    /// シード、エポック番号と正のbucket数から割り当て列を作る。
    pub fn new(seed: u64, epoch: u64, buckets: u32) -> Self {
        assert!(buckets > 0, "bucket count must be positive");
        Self {
            state: mix64(seed ^ epoch.wrapping_mul(STEP) ^ 0x6275_636B_6574_7300),
            buckets,
        }
    }

    /// 次の局面のbucket番号を返す。
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> u32 {
        bounded(&mut self.state, u64::from(self.buckets)) as u32
    }
}

/// bucket内の0..lenを、用途とbucket番号で分離した乱数列で並べ替える。
/// 返り値`order`のj番目は、bucket内でj番目に渡す局面の元の位置である（呼び手は`group[order[j]]`と読む）。
pub fn bucket_permutation(len: usize, seed: u64, epoch: u64, bucket: u32) -> Vec<usize> {
    let mut state = mix64(
        mix64(seed ^ epoch.wrapping_mul(STEP) ^ 0x7065_726D_7574_6500)
            ^ (u64::from(bucket) + 1).wrapping_mul(STEP),
    );
    let mut order: Vec<_> = (0..len).collect();
    for end in (1..len).rev() {
        let selected = bounded(&mut state, end as u64 + 1) as usize;
        order.swap(end, selected);
    }
    order
}

/// 訓練局面0..nをRao–Sandelius方式で並べる、検証用の純関数。
/// 割り当てたbucket内を並べ替え、bucket番号順に連結する。
pub fn rao_sandelius_order(n: usize, buckets: u32, seed: u64, epoch: u64) -> Vec<usize> {
    let mut assigner = BucketAssigner::new(seed, epoch, buckets);
    let mut groups = vec![Vec::new(); buckets as usize];
    for i in 0..n {
        groups[assigner.next() as usize].push(i);
    }
    groups
        .iter()
        .enumerate()
        .flat_map(|(k, group)| {
            bucket_permutation(group.len(), seed, epoch, k as u32)
                .into_iter()
                .map(|i| group[i])
        })
        .collect()
}

/// Floydの方法で0..populationからk件を重複なく選び、昇順で返す。
/// kが母集団を超えた場合はエラーにする。
pub fn floyd_sample(population: u64, k: usize, seed: u64) -> super::Result<Vec<u64>> {
    if k as u64 > population {
        return Err(super::Error::Invalid("sample size exceeds population"));
    }
    let mut state = mix64(seed ^ 0x7361_6D70_6C65_0000);
    let mut selected = std::collections::BTreeSet::new();
    for j in population - k as u64..population {
        let value = bounded(&mut state, j + 1);
        if !selected.insert(value) {
            selected.insert(j);
        }
    }
    Ok(selected.into_iter().collect())
}

#[cfg(test)]
mod bucket_tests {
    use super::*;

    #[test]
    fn direct_assignment_matches_reference_order() {
        // 仕様2.5: 学習経路の2つの操作と参照順列は同じ列を作る。
        for n in [0, 1, 5, 31] {
            for buckets in [1, 3, 64] {
                for epoch in 0..8 {
                    let mut assigner = BucketAssigner::new(7, epoch, buckets);
                    let mut groups = vec![Vec::new(); buckets as usize];
                    for i in 0..n {
                        groups[assigner.next() as usize].push(i);
                    }
                    let mut actual = Vec::new();
                    for (k, group) in groups.iter().enumerate() {
                        for i in bucket_permutation(group.len(), 7, epoch, k as u32) {
                            actual.push(group[i]);
                        }
                    }
                    assert_eq!(actual, rao_sandelius_order(n, buckets, 7, epoch));
                    actual.sort_unstable();
                    assert_eq!(actual, (0..n).collect::<Vec<_>>());
                }
            }
        }
    }

    #[test]
    fn floyd_samples_are_sorted_unique_and_bounded() {
        // 仕様2.5: 0件と全件も許し、母集団を超える要求は拒否する。
        for population in [0, 1, 5, 100, u64::MAX] {
            for k in 0..=population.min(20) as usize {
                let selected = floyd_sample(population, k, 17).unwrap();
                assert_eq!(selected.len(), k);
                assert!(selected.windows(2).all(|w| w[0] < w[1]));
                assert!(selected.iter().all(|&i| i < population));
                assert_eq!(selected, floyd_sample(population, k, 17).unwrap());
            }
        }
        assert!(floyd_sample(3, 4, 0).is_err());
    }
}
