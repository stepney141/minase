//! B≥2のbucketによる一様シャッフル（一時ファイルの経路）の受け入れテスト。
//!
//! 期待値は設計書`docs/plans/out-of-core-shuffle.md`の「設計判断」「検証」と、
//! フェーズ2の仕様（一時ファイルの配置、読み戻しの経路、乱数と順列、学習器の変更）から導く。
//! B=1の経路（`Dataset`）を等価性の基準に使い、一時ファイルの経路の出力は写さない。

mod common;

use clap::Parser;
use common::{mnsd, training};
use minase::training::records::{Header, Origin, TeacherKind};
use minase_core::{Color, PieceCode, PieceKind, PositionBuilder, Square};
use minase_trainer::data::{
    BucketAssigner, Dataset, EpochReader, Error, Lookahead, Outcome, Sample, SpillRecord,
    SpillStore, bucket_permutation, floyd_sample, rao_sandelius_order,
};
use minase_trainer::nnue::data::{InputFeatureSet, Observations};
use std::{
    collections::BTreeSet,
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// テストごとの一時ディレクトリ。破棄時に中身ごと消す。
struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "spill-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const LAMBDA: f64 = 0.75;

fn lookahead() -> Option<Lookahead> {
    Some(Lookahead::new(0.9, 40).unwrap())
}

fn one() -> NonZeroUsize {
    NonZeroUsize::new(1).unwrap()
}

fn far() -> Instant {
    Instant::now() + Duration::from_secs(3600)
}

/// 0..gamesの対局番号を持つ独立な局面を1局面1対局で並べたMNSD。
/// 検証分割はヘッダのシードと対局番号で決まるので、`independent`が区分を保つ。
fn records(seed: u64, network: [u8; 32], games: u32) -> Vec<u8> {
    let header = Header::new(
        "L0,P0,R1,E0".into(),
        "1".repeat(40),
        network,
        TeacherKind::AlphaBeta,
        200,
        seed,
    )
    .unwrap();
    let mut samples = Vec::new();
    for game in 0..games {
        let side = if game % 2 == 0 {
            Color::Black
        } else {
            Color::White
        };
        let mut builder = PositionBuilder::new(side);
        for (file, rank, color, kind) in [
            (4, 0, Color::Black, PieceKind::King),
            (7, 11, Color::White, PieceKind::King),
            ((game % 12) as u8, 3, Color::Black, PieceKind::Pawn),
            (
                ((game / 12 + 6) % 12) as u8,
                8,
                Color::White,
                PieceKind::Pawn,
            ),
            (((game + 3) % 12) as u8, 6, Color::Black, PieceKind::Rook),
        ] {
            builder
                .put(
                    Square::new(file, rank).unwrap(),
                    PieceCode::new(color, kind).unwrap(),
                )
                .unwrap();
        }
        let outcome = match (game % 3, side) {
            (0, _) => Outcome::Draw,
            (1, Color::Black) | (2, Color::White) => Outcome::Win,
            _ => Outcome::Loss,
        };
        samples.push(
            Sample::from_position(
                &builder.finish().unwrap(),
                ((game * 97) % 801) as i16 - 400,
                outcome,
                game,
                (game % 8) as u16,
                Origin::RandomSelfplay,
            )
            .unwrap(),
        );
    }
    mnsd::independent(header, &samples)
}

/// 入力ファイル群と一時ファイルの置き場所。
struct Data {
    temp: Temp,
    paths: Vec<PathBuf>,
    scratch: PathBuf,
}

impl Data {
    /// (ヘッダのシード, 重みの検査和, 対局数)ごとに1ファイルを書く。
    fn new(files: &[(u64, [u8; 32], u32)]) -> Self {
        let temp = Temp::new();
        let mut paths = Vec::new();
        for (i, &(seed, network, games)) in files.iter().enumerate() {
            let path = temp.join(&format!("records-{i}.bin"));
            fs::write(&path, records(seed, network, games)).unwrap();
            paths.push(path);
        }
        let scratch = temp.join("scratch");
        fs::create_dir(&scratch).unwrap();
        Self {
            temp,
            paths,
            scratch,
        }
    }
    /// 2つの教師の分類にまたがる2ファイル。
    fn two_files() -> Self {
        Self::new(&[(11, [2; 32], 150), (12, [3; 32], 140)])
    }
    fn dataset(&self) -> Dataset {
        Dataset::open(&self.paths, lookahead(), one())
            .unwrap()
            .with_lambda(LAMBDA)
            .unwrap()
    }
    fn store(&self, buckets: u32, bucket_positions: u64) -> SpillStore {
        SpillStore::create(
            &self.paths,
            lookahead(),
            one(),
            &self.scratch,
            buckets,
            bucket_positions,
        )
        .unwrap()
        .with_lambda(LAMBDA)
        .unwrap()
    }
    fn scratch_entries(&self) -> Vec<PathBuf> {
        fs::read_dir(&self.scratch)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect()
    }
}

/// 読み戻した局面の教師情報が、同じ通し番号のB=1の局面とビット単位で一致することを確かめる。
fn assert_same_as_dataset(dataset: &Dataset, record: &SpillRecord) {
    assert_eq!(record.sample, dataset.record(record.index).unwrap());
    assert_eq!(
        record.teacher_score.to_bits(),
        dataset.teacher_score(record.index).unwrap().to_bits(),
        "index {}",
        record.index
    );
    assert_eq!(
        record.teacher_class,
        dataset.teacher_class(record.index).unwrap()
    );
}

/// 訓練局面の通し番号をファイル順に集める。
fn file_order(store: &SpillStore) -> Vec<u64> {
    let mut indices = Vec::new();
    store
        .for_each_training(|record| {
            indices.push(record.index);
            Ok(())
        })
        .unwrap();
    indices
}

/// 読み手からbatch件ずつ全チャンクを取り出す。
fn drain(mut reader: EpochReader<'_>, batch: usize) -> Vec<Vec<SpillRecord>> {
    let mut chunks = Vec::new();
    while let Some(chunk) = reader.next_chunk(batch).unwrap() {
        chunks.push(chunk);
    }
    chunks
}

fn epoch_chunks(store: &SpillStore, seed: u64, epoch: u64, batch: usize) -> Vec<Vec<SpillRecord>> {
    drain(
        store
            .epoch(seed, epoch, far())
            .unwrap()
            .expect("the deadline is far away"),
        batch,
    )
}

/// 仕様の順列: ファイル順の訓練局面0..nをrao_sandelius_orderで並べ、通し番号へ写す。
fn expected_order(training: &[u64], buckets: u32, seed: u64, epoch: u64) -> Vec<u64> {
    rao_sandelius_order(training.len(), buckets, seed, epoch)
        .into_iter()
        .map(|i| training[i])
        .collect()
}

/// ファイル順の訓練局面に振られるbucketごとの局面数。
fn bucket_sizes(n: usize, buckets: u32, seed: u64, epoch: u64) -> Vec<u64> {
    let mut assigner = BucketAssigner::new(seed, epoch, buckets);
    let mut sizes = vec![0_u64; buckets as usize];
    for _ in 0..n {
        let bucket = assigner.next();
        assert!(bucket < buckets);
        sizes[bucket as usize] += 1;
    }
    sizes
}

/// 最後以外のチャンクが満杯で、チャンク数がceil(N / batch)であることを確かめる。
fn assert_chunk_shape(chunks: &[Vec<SpillRecord>], n: usize, batch: usize) {
    assert_eq!(chunks.len(), n.div_ceil(batch), "n={n}, batch={batch}");
    for chunk in &chunks[..chunks.len() - 1] {
        assert_eq!(chunk.len(), batch);
    }
    assert_eq!(chunks.last().unwrap().len(), n - batch * (chunks.len() - 1));
}

fn indices(chunks: &[Vec<SpillRecord>]) -> Vec<u64> {
    chunks.iter().flatten().map(|record| record.index).collect()
}

fn sha256_hex(indices: &[u64]) -> String {
    let bytes: Vec<u8> = indices
        .iter()
        .flat_map(|index| index.to_le_bytes())
        .collect();
    minase_trainer::nn::hex(&minase_trainer::nn::sha256(&bytes))
}

// ---- 検証2: 各訓練局面がちょうど1回ずつ、B=1と同じ教師情報で渡される ----

#[test]
fn b3_epochs_present_each_training_position_once_in_full_batches() {
    let data = Data::two_files();
    let dataset = data.dataset();
    let n = dataset.training_indices().len();
    let store = data.store(3, n as u64);
    let training = file_order(&store);
    assert_eq!(training, dataset.training_indices());
    let batch = 64;
    // 最後の端数のバッチが生じる大きさである。
    assert_ne!(n % batch, 0);
    for epoch in 0..3 {
        let sizes = bucket_sizes(n, 3, 1, epoch);
        // bucket 0と1の境界が1つのチャンクの内側にある（チャンクがbucketをまたぐ）。
        assert!(sizes[0] > 0 && sizes[1] > 0);
        assert_ne!(sizes[0] % batch as u64, 0, "epoch {epoch}: {sizes:?}");
        let chunks = epoch_chunks(&store, 1, epoch, batch);
        assert_chunk_shape(&chunks, n, batch);
        let presented = indices(&chunks);
        let mut sorted = presented.clone();
        sorted.sort_unstable();
        let mut expected = dataset.training_indices().to_vec();
        expected.sort_unstable();
        assert_eq!(
            sorted, expected,
            "epoch {epoch}: each position exactly once"
        );
        assert_eq!(presented, expected_order(&training, 3, 1, epoch));
        for record in chunks.iter().flatten() {
            assert_same_as_dataset(&dataset, record);
        }
    }
}

#[test]
fn spill_store_matches_the_in_memory_dataset_bit_for_bit() {
    let data = Data::two_files();
    let dataset = data.dataset();
    let store = data.store(3, 1_000);
    let training = dataset.training_indices();
    let validation = dataset.validation_indices();
    assert!(!validation.is_empty());

    assert_eq!(store.training_count(), training.len() as u64);
    assert_eq!(store.validation_count(), validation.len() as u64);
    assert_eq!(store.record_count(), dataset.record_count());
    assert_eq!(store.buckets(), 3);
    assert_eq!(store.bucket_positions(), 1_000);
    assert_eq!(
        store.paths().collect::<Vec<_>>(),
        dataset.paths().collect::<Vec<_>>()
    );
    assert_eq!(
        format!("{:?}", store.headers().collect::<Vec<_>>()),
        format!("{:?}", dataset.headers().collect::<Vec<_>>())
    );
    assert_eq!(store.teacher_classes(), dataset.teacher_classes());
    assert_eq!(store.teacher_classes().len(), 2);
    assert_eq!(store.teacher_lambdas(), dataset.teacher_lambdas());
    assert_eq!(store.training_indices_sha256(), sha256_hex(training));
    assert_eq!(store.validation_indices_sha256(), sha256_hex(validation));

    let mut seen = Vec::new();
    store
        .for_each_training(|record| {
            assert_same_as_dataset(&dataset, &record);
            seen.push(record.index);
            Ok(())
        })
        .unwrap();
    assert_eq!(seen, training);

    let chunk = 7;
    let chunks = store
        .validation_chunks(chunk)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_chunk_shape(&chunks, validation.len(), chunk);
    assert_eq!(indices(&chunks), validation);
    for record in chunks.iter().flatten() {
        assert_same_as_dataset(&dataset, record);
    }

    let spilled = store.estimate_teacher_cp_per_logit().unwrap();
    let memory = dataset.estimate_teacher_cp_per_logit().unwrap();
    assert_eq!(spilled.len(), memory.len());
    for (left, right) in spilled.iter().zip(&memory) {
        let left = left.expect("λ > 0 estimates every class");
        let right = right.unwrap();
        assert_eq!(left.value.to_bits(), right.value.to_bits());
        assert_eq!(left.at_search_bound, right.at_search_bound);
    }

    for feature_set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let memory = Observations::compute(&dataset, feature_set).unwrap();
        let spilled = Observations::from_positions(
            store.training_records().map(|record| {
                Ok::<_, minase_trainer::nnue::data::Error>(record?.sample.to_position()?)
            }),
            feature_set,
        )
        .unwrap();
        assert_eq!(spilled.canonical_counts(), memory.canonical_counts());
        assert_eq!(spilled.input_counts(), memory.input_counts());
        assert_eq!(spilled.virtual_mask(), memory.virtual_mask());
    }
}

#[test]
fn empty_single_and_spanned_buckets_follow_the_rao_sandelius_order() {
    // seed=11では0..4の対局はすべて訓練局面なので、N=4である。
    let data = Data::new(&[(11, [2; 32], 4)]);
    let dataset = data.dataset();
    let n = dataset.training_indices().len();
    assert_eq!(n, 4);
    let store = data.store(3, n as u64);
    let training = file_order(&store);
    let batch = 2;
    let (mut empty, mut single, mut spanned) = (false, false, false);
    for epoch in 0..60 {
        let sizes = bucket_sizes(n, 3, 7, epoch);
        empty |= sizes.contains(&0);
        single |= sizes.contains(&1);
        // 非空のbucketの累積の境界がバッチの途中にあれば、1つのチャンクが2つのbucketをまたぐ。
        let mut cumulative = 0;
        for &size in &sizes[..2] {
            cumulative += size;
            spanned |= size > 0 && cumulative < n as u64 && cumulative % batch as u64 != 0;
        }
        let chunks = epoch_chunks(&store, 7, epoch, batch);
        assert_chunk_shape(&chunks, n, batch);
        assert_eq!(
            indices(&chunks),
            expected_order(&training, 3, 7, epoch),
            "epoch {epoch}: {sizes:?}"
        );
    }
    assert!(
        empty && single && spanned,
        "the scan must cover each boundary"
    );
}

#[test]
fn more_buckets_than_positions_still_presents_every_position() {
    let data = Data::new(&[(11, [2; 32], 3)]);
    let n = data.dataset().training_indices().len();
    assert_eq!(n, 3);
    for buckets in [5, 4096] {
        let store = data.store(buckets, n as u64);
        let training = file_order(&store);
        for epoch in 0..4 {
            assert!(bucket_sizes(n, buckets, 3, epoch).contains(&0));
            for batch in [1, 2, 3, 64] {
                let chunks = epoch_chunks(&store, 3, epoch, batch);
                assert_chunk_shape(&chunks, n, batch);
                assert_eq!(
                    indices(&chunks),
                    expected_order(&training, buckets, 3, epoch)
                );
            }
        }
    }
}

// ---- 検証3: 順列の一様性と再現性 ----

/// 0..5の順列をLehmer符号で0..120へ写す。
fn rank(permutation: &[usize]) -> usize {
    let mut rank = 0;
    for (i, &value) in permutation.iter().enumerate() {
        let smaller = permutation[i + 1..].iter().filter(|&&v| v < value).count();
        rank = rank * (permutation.len() - i) + smaller;
    }
    rank
}

#[test]
fn rao_sandelius_order_is_uniform_over_the_120_permutations_of_five() {
    const TRIALS: u64 = 1_200_000;
    let mut counts = [0_u64; 120];
    for epoch in 0..TRIALS {
        let order = rao_sandelius_order(5, 3, 20_261_016, epoch);
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, [0, 1, 2, 3, 4], "epoch {epoch}");
        counts[rank(&order)] += 1;
    }
    let expected = TRIALS as f64 / 120.0;
    let chi_square: f64 = counts
        .iter()
        .map(|&count| (count as f64 - expected).powi(2) / expected)
        .sum();
    assert!(counts.iter().all(|&count| count > 0));
    // 自由度119、有意水準0.001の臨界値。
    assert!(chi_square < 173.6, "chi-square {chi_square}");
}

#[test]
fn bucket_assigner_draws_each_bucket_uniformly() {
    const DRAWS: u64 = 700_000;
    let mut assigner = BucketAssigner::new(5, 9, 7);
    let mut counts = [0_u64; 7];
    for _ in 0..DRAWS {
        counts[assigner.next() as usize] += 1;
    }
    let expected = DRAWS as f64 / 7.0;
    let chi_square: f64 = counts
        .iter()
        .map(|&count| (count as f64 - expected).powi(2) / expected)
        .sum();
    // 自由度6、有意水準0.001の臨界値。
    assert!(chi_square < 22.46, "chi-square {chi_square}: {counts:?}");
}

#[test]
fn rao_sandelius_order_is_reproducible_and_composed_of_bucket_permutations() {
    for (n, buckets, seed, epoch) in [(0, 3, 1, 0), (1, 3, 1, 0), (37, 3, 4, 2), (200, 16, 9, 5)] {
        let order = rao_sandelius_order(n, buckets, seed, epoch);
        assert_eq!(order, rao_sandelius_order(n, buckets, seed, epoch));
        // 仕様: 各局面iにBucketAssignerのi番目の値を振り、bucketごとにiを元の順に集め、
        // 各bucketをbucket_permutationで並べ替え、bucket番号順に連結する。
        let mut assigner = BucketAssigner::new(seed, epoch, buckets);
        let mut groups = vec![Vec::new(); buckets as usize];
        for i in 0..n {
            groups[assigner.next() as usize].push(i);
        }
        let mut composed = Vec::new();
        for (k, group) in groups.iter().enumerate() {
            let permutation = bucket_permutation(group.len(), seed, epoch, k as u32);
            let mut sorted = permutation.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, (0..group.len()).collect::<Vec<_>>());
            assert_eq!(
                permutation,
                bucket_permutation(group.len(), seed, epoch, k as u32)
            );
            composed.extend(permutation.iter().map(|&j| group[j]));
        }
        assert_eq!(order, composed, "n={n}, B={buckets}");
    }
    // エポックが変われば順列も変わる（固定順にならない）。
    assert_ne!(
        rao_sandelius_order(200, 3, 1, 0),
        rao_sandelius_order(200, 3, 1, 1)
    );
}

// ---- 検証用の標本: Floydの方法 ----

#[test]
fn floyd_sample_returns_sorted_distinct_members_and_rejects_oversized_requests() {
    for (population, k) in [(1, 1), (10, 0), (10, 3), (10, 10), (1_000_000, 50)] {
        let sample = floyd_sample(population, k, 77).unwrap();
        assert_eq!(sample.len(), k);
        assert!(sample.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(sample.iter().all(|&value| value < population));
        assert_eq!(sample, floyd_sample(population, k, 77).unwrap());
    }
    assert_eq!(floyd_sample(6, 6, 3).unwrap(), (0..6).collect::<Vec<u64>>());
    assert!(floyd_sample(5, 6, 3).is_err());
    assert!(floyd_sample(0, 1, 3).is_err());
}

#[test]
fn floyd_sample_chooses_every_subset_uniformly() {
    const TRIALS: u64 = 200_000;
    // 0..5から2個を選ぶ10通りの部分集合。
    let mut counts = std::collections::BTreeMap::new();
    for seed in 0..TRIALS {
        *counts
            .entry(floyd_sample(5, 2, seed).unwrap())
            .or_insert(0_u64) += 1;
    }
    assert_eq!(counts.len(), 10);
    let expected = TRIALS as f64 / 10.0;
    let chi_square: f64 = counts
        .values()
        .map(|&count| (count as f64 - expected).powi(2) / expected)
        .sum();
    // 自由度9、有意水準0.001の臨界値。
    assert!(chi_square < 27.88, "chi-square {chi_square}");
}

#[test]
fn validation_sample_reads_the_floyd_selection_in_ascending_order() {
    let data = Data::two_files();
    let dataset = data.dataset();
    let store = data.store(3, 1_000);
    let validation = dataset.validation_indices();
    let v = validation.len();
    assert!(v > 4);
    for (k, seed) in [(1, 5), (4, 5), (4, 6), (v, 1)] {
        let records = store.validation_sample(k, seed).unwrap();
        let expected: Vec<u64> = floyd_sample(v as u64, k, seed)
            .unwrap()
            .into_iter()
            .map(|i| validation[i as usize])
            .collect();
        assert_eq!(
            records.iter().map(|r| r.index).collect::<Vec<_>>(),
            expected
        );
        for record in &records {
            assert_same_as_dataset(&dataset, record);
        }
    }
    // 検証局面の数を超える要求は全検証局面を昇順に返す。
    let all = store.validation_sample(v + 10, 5).unwrap();
    assert_eq!(all.iter().map(|r| r.index).collect::<Vec<_>>(), validation);
    assert!(matches!(
        store.validation_sample(0, 5),
        Err(Error::Invalid(_))
    ));
}

// ---- 検証4: 上限超過、切断、後始末 ----

#[test]
fn bucket_overflow_reports_the_bucket_its_count_the_limit_and_b() {
    let data = Data::new(&[(11, [2; 32], 150)]);
    let n = data.dataset().training_indices().len();
    // 上限を超えるbucketが1つだけになるシードを選び、そのbucketを特定する。
    let (seed, sizes) = (0..1_000)
        .map(|seed| (seed, bucket_sizes(n, 3, seed, 0)))
        .find(|(_, sizes)| {
            let max = *sizes.iter().max().unwrap();
            sizes.iter().filter(|&&size| size == max).count() == 1
        })
        .unwrap();
    let max = *sizes.iter().max().unwrap();
    let bucket = sizes.iter().position(|&size| size == max).unwrap() as u32;
    let store = data.store(3, max - 1);
    let directory = store.directory().to_path_buf();
    let error = store.epoch(seed, 0, far()).err().expect("overflow");
    match &error {
        Error::BucketOverflow {
            bucket: b,
            positions,
            limit,
            buckets,
        } => {
            assert_eq!(
                (*b, *positions, *limit, *buckets),
                (bucket, max, max - 1, 3)
            );
        }
        other => panic!("expected BucketOverflow, got {other:?}"),
    }
    let message = error.to_string();
    for number in [bucket as u64, max, max - 1, 3] {
        assert!(message.contains(&number.to_string()), "{message}");
    }
    drop(store);
    assert!(!directory.exists());
    assert!(data.scratch.is_dir());
    assert!(data.scratch_entries().is_empty());
}

#[test]
fn any_bucket_above_the_limit_stops_the_epoch() {
    let data = Data::new(&[(11, [2; 32], 150)]);
    let n = data.dataset().training_indices().len() as u64;
    // 鳩の巣原理により、どの割り当てでも上限を超えるbucketがある。
    let limit = n.div_ceil(3) - 1;
    let store = data.store(3, limit);
    for epoch in 0..3 {
        let sizes = bucket_sizes(n as usize, 3, 1, epoch);
        match store.epoch(1, epoch, far()) {
            Err(Error::BucketOverflow {
                bucket,
                positions,
                limit: reported,
                buckets,
            }) => {
                assert_eq!(buckets, 3);
                assert_eq!(reported, limit);
                assert!(positions > limit);
                assert_eq!(positions, sizes[bucket as usize]);
            }
            Err(other) => panic!("expected BucketOverflow, got {other:?}"),
            Ok(_) => panic!("epoch {epoch} must overflow: {sizes:?}"),
        }
    }
}

/// 一時ファイルを末尾からcutバイト切り詰める。
fn truncate(path: &Path, cut: u64) {
    let file = fs::OpenOptions::new().write(true).open(path).unwrap();
    let len = file.metadata().unwrap().len();
    assert!(len > cut, "{}", path.display());
    file.set_len(len - cut).unwrap();
}

fn assert_spill_error<T: std::fmt::Debug>(result: Result<T, Error>, path: &Path) {
    match result {
        Err(Error::Spill { path: reported, .. }) => assert_eq!(reported, path),
        other => panic!("expected Spill for {}, got {other:?}", path.display()),
    }
}

/// エポックの開始から全チャンクの読み出しまでを1つの結果にする。
fn read_whole_epoch(store: &SpillStore) -> Result<usize, Error> {
    let mut reader = store.epoch(2, 0, far())?.expect("the deadline is far away");
    let mut count = 0;
    while let Some(chunk) = reader.next_chunk(16)? {
        count += chunk.len();
    }
    Ok(count)
}

const CUTS: [u64; 2] = [minase_trainer::data::spill::RECORD_LEN as u64, 1];

#[test]
fn truncated_training_spill_is_detected_before_a_short_epoch() {
    for cut in CUTS {
        let data = Data::new(&[(11, [2; 32], 60)]);
        let store = data.store(3, 1_000);
        let path = store.directory().join("train-0.bin");
        truncate(&path, cut);
        assert_spill_error(read_whole_epoch(&store), &path);
        assert_spill_error(store.for_each_training(|_| Ok(())), &path);
        let directory = store.directory().to_path_buf();
        drop(store);
        assert!(!directory.exists(), "cut={cut}");
    }
}

#[test]
fn truncated_validation_spill_is_detected() {
    for cut in CUTS {
        let data = Data::two_files();
        let store = data.store(3, 1_000);
        let path = store.directory().join("validation-0.bin");
        truncate(&path, cut);
        assert_spill_error(
            store.validation_chunks(5).collect::<Result<Vec<_>, _>>(),
            &path,
        );
        let v = store.validation_count() as usize;
        assert_spill_error(store.validation_sample(v, 1), &path);
    }
}

#[test]
fn truncated_scale_spill_is_detected() {
    // 1件は先読み後の探索値f32と結果u8である。
    for cut in [5, 1] {
        let data = Data::two_files();
        let store = data.store(3, 1_000);
        let path = store.directory().join("scale-0.bin");
        truncate(&path, cut);
        assert_spill_error(store.estimate_teacher_cp_per_logit(), &path);
    }
}

#[test]
fn truncated_bucket_spill_is_detected_before_it_is_read() {
    for cut in CUTS {
        let data = Data::new(&[(11, [2; 32], 60)]);
        let store = data.store(3, 1_000);
        let n = store.training_count() as usize;
        let sizes = bucket_sizes(n, 3, 2, 0);
        assert!(sizes[2] > 0, "{sizes:?}");
        let path = store.directory().join("epoch").join("bucket-2.bin");
        let mut reader = store.epoch(2, 0, far()).unwrap().unwrap();
        truncate(&path, cut);
        let result = loop {
            match reader.next_chunk(16) {
                Ok(Some(_)) => {}
                Ok(None) => break Ok(()),
                Err(error) => break Err(error),
            }
        };
        assert_spill_error(result, &path);
    }
}

#[test]
fn run_directory_layout_and_its_removal_on_drop() {
    let data = Data::new(&[(11, [2; 32], 60)]);
    let dataset = data.dataset();
    let first = data.store(3, 1_000);
    let second = data.store(3, 1_000);
    assert_ne!(first.directory(), second.directory());
    for store in [&first, &second] {
        let directory = store.directory();
        assert_eq!(directory.parent().unwrap(), data.scratch);
        let name = directory.file_name().unwrap().to_str().unwrap();
        let parts: Vec<_> = name.split('-').collect();
        assert_eq!(parts.len(), 4, "{name}");
        assert_eq!(parts[0], "nnue");
        assert!(parts[1].parse::<u64>().is_ok(), "{name}");
        assert_eq!(parts[2], std::process::id().to_string());
        assert!(parts[3].parse::<u64>().is_ok(), "{name}");
        // 仕様はpidを10進で書くとだけ定めるので、末尾の改行は許す。
        assert_eq!(
            fs::read_to_string(directory.join("lock"))
                .unwrap()
                .trim_end_matches('\n'),
            std::process::id().to_string()
        );
        let record = minase_trainer::data::spill::RECORD_LEN as u64;
        assert_eq!(record, 138);
        let length = |name: &str| fs::metadata(directory.join(name)).unwrap().len();
        assert_eq!(
            length("train-0.bin"),
            dataset.training_indices().len() as u64 * record
        );
        assert_eq!(
            length("validation-0.bin"),
            dataset.validation_indices().len() as u64 * record
        );
    }
    let directories = [
        first.directory().to_path_buf(),
        second.directory().to_path_buf(),
    ];
    epoch_chunks(&first, 1, 0, 8);
    drop(first);
    assert!(!directories[0].exists());
    assert!(directories[1].exists());
    drop(second);
    assert!(!directories[1].exists());
    assert!(data.scratch.is_dir());
    assert!(data.scratch_entries().is_empty());
}

// ---- 検証5（ストアの範囲）: 振り分けの途中の中断 ----

#[test]
fn expired_deadline_abandons_the_distribution_and_a_later_epoch_restarts_it() {
    let data = Data::two_files();
    let store = data.store(3, 1_000);
    let training = file_order(&store);
    let past = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
    for epoch in [0, 4] {
        assert!(
            store.epoch(9, epoch, past).unwrap().is_none(),
            "epoch {epoch}: an expired deadline must not yield a reader"
        );
        let chunks = epoch_chunks(&store, 9, epoch, 50);
        assert_chunk_shape(&chunks, training.len(), 50);
        assert_eq!(indices(&chunks), expected_order(&training, 3, 9, epoch));
    }
}

// ---- 検証4と5（学習コマンド）----

#[derive(Parser)]
struct TrainArguments {
    #[command(flatten)]
    args: minase_trainer::nnue::train::Args,
}

/// `nnue train`を`--device cpu`の小さな学習として準備する。
struct Training {
    data: Data,
}

impl Training {
    fn new() -> Self {
        // 320対局のうちseed=11の検証対局を除いた訓練局面を33件ずつのバッチに分ける。
        let data = Data::new(&[(11, [2; 32], 320)]);
        let base = include_bytes!("../../minase/nets/pst.bin");
        fs::write(data.temp.join("pst.bin"), base).unwrap();
        // MNPT版3の本体はヘッダ80バイトの後に序中盤、終盤のi16[13680]を順に置き、単位は1/8cpである。
        // 初期値は基準のPSTを浮動小数へ戻したものとし、再量子化で基準と一致させる。
        const FEATURES: usize = 13_680;
        let weight =
            |at: usize| f32::from(i16::from_le_bytes([base[80 + 2 * at], base[81 + 2 * at]])) / 8.0;
        let initial: Vec<[f32; 2]> = (0..FEATURES)
            .map(|feature| [weight(feature), weight(FEATURES + feature)])
            .collect();
        fs::write(
            data.temp.join("pst-float.npz"),
            minase_trainer::pst::encode_pst_npz(&initial).unwrap(),
        )
        .unwrap();
        Self { data }
    }
    fn path(&self, name: &str) -> String {
        self.data.temp.join(name).to_str().unwrap().to_owned()
    }
    fn args(&self, output: &str, extra: &[&str]) -> minase_trainer::nnue::train::Args {
        let fixed = [
            "minase-trainer".to_owned(),
            "--feature-set".into(),
            "P".into(),
            "--width".into(),
            "8".into(),
            "--data".into(),
            self.data.paths[0].to_str().unwrap().into(),
            "--init-pst".into(),
            self.path("pst-float.npz"),
            "--base".into(),
            self.path("pst.bin"),
            "--output-dir".into(),
            self.path(output),
            "--teacher-cp-per-logit".into(),
            "calibrate".into(),
            "--lambda".into(),
            "0.75".into(),
            "--device".into(),
            "cpu".into(),
            "--batch".into(),
            "33".into(),
            "--validation-sample".into(),
            "24".into(),
            "--lookahead-gamma".into(),
            "0.9".into(),
            "--lookahead-plies".into(),
            "40".into(),
        ];
        let threads = if extra.contains(&"--threads") {
            vec![]
        } else {
            vec!["--threads".to_owned(), "2".to_owned()]
        };
        let values = fixed
            .into_iter()
            .chain(threads)
            .chain(extra.iter().map(|value| (*value).to_owned()));
        TrainArguments::try_parse_from(values).unwrap().args
    }
    fn scratch(&self) -> String {
        self.data.scratch.to_str().unwrap().to_owned()
    }
}

fn find_source<'a, E: std::error::Error + 'static>(
    mut error: &'a (dyn std::error::Error + 'static),
) -> Option<&'a E> {
    loop {
        if let Some(found) = error.downcast_ref::<E>() {
            return Some(found);
        }
        error = error.source()?;
    }
}

#[test]
fn cpu_spill_training_resumed_after_epoch_one_matches_the_uninterrupted_run() {
    let training = Training::new();
    let scratch = training.scratch();
    let other_scratch = training.path("other-scratch");
    fs::create_dir(&other_scratch).unwrap();
    let buckets = [
        "--buckets",
        "3",
        "--bucket-positions",
        "1000",
        "--scratch-dir",
        &scratch,
    ];
    let mut continuous = buckets.to_vec();
    continuous.extend(["--epochs", "2"]);
    minase_trainer::nnue::train::run(training.args("continuous", &continuous)).unwrap();

    let mut first = buckets.to_vec();
    first.extend(["--epochs", "1"]);
    minase_trainer::nnue::train::run(training.args("resumed", &first)).unwrap();
    assert_eq!(
        training::logs(&training.data.temp.join("resumed"))
            .iter()
            .map(|row| row["epoch"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [0, 1]
    );
    // 順列を変えない容量と作業ディレクトリは再開時に変えてよい。
    let resumed_dir = training.path("resumed");
    minase_trainer::nnue::train::run(training.args(
        "resumed",
        &[
            "--buckets",
            "3",
            "--bucket-positions",
            "2000",
            "--scratch-dir",
            &other_scratch,
            "--epochs",
            "2",
            "--threads",
            "3",
            "--resume",
            &resumed_dir,
        ],
    ))
    .unwrap();

    let left = training.data.temp.join("continuous");
    let right = training.data.temp.join("resumed");
    for file in ["latest.npz", "best.npz"] {
        training::compare_checkpoints(&left.join(file), &right.join(file));
    }
    assert_eq!(
        fs::read(left.join("nnue.bin")).unwrap(),
        fs::read(right.join("nnue.bin")).unwrap()
    );
    let continuous_log = training::logs(&left);
    let resumed_log = training::logs(&right);
    assert_eq!(
        continuous_log
            .iter()
            .map(|row| row["epoch"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(continuous_log.len(), resumed_log.len());
    for (a, b) in continuous_log.iter().zip(&resumed_log) {
        for key in ["epoch", "steps", "total_steps"] {
            assert_eq!(a[key], b[key], "{key}");
        }
        for key in ["train_loss", "validation_loss"] {
            assert_eq!(
                a[key].as_f64().map(f64::to_bits),
                b[key].as_f64().map(f64::to_bits),
                "{key}"
            );
        }
    }
    // 1エポックの更新回数はceil(N / batch)である。
    let n = training.data.dataset().training_indices().len() as u64;
    let steps = n.div_ceil(33);
    assert_eq!(continuous_log[1]["steps"].as_u64().unwrap(), steps);
    assert_eq!(
        continuous_log[2]["total_steps"].as_u64().unwrap(),
        2 * steps
    );

    let config = training::document(right.join("config.json"));
    assert_eq!(config["settings"]["buckets"], 3);
    assert!(config["settings"].get("bucket_positions").is_none());
    assert!(config["settings"].get("scratch_dir").is_none());
    assert_eq!(config["resumes"].as_array().unwrap().len(), 1);
    assert_eq!(config["resumes"][0]["epoch"], 1);
    assert_eq!(config["resumes"][0]["bucket_positions"], 2000);
    assert_eq!(config["resumes"][0]["scratch_dir"], other_scratch.as_str());

    // 正常終了した学習は作業ディレクトリを残さない。
    assert!(training.data.scratch_entries().is_empty());
    assert_eq!(fs::read_dir(&other_scratch).unwrap().count(), 0);
}

#[test]
fn resume_rejects_a_changed_bucket_count() {
    let training = Training::new();
    let scratch = training.scratch();
    minase_trainer::nnue::train::run(training.args(
        "output",
        &[
            "--buckets",
            "3",
            "--bucket-positions",
            "1000",
            "--scratch-dir",
            &scratch,
            "--epochs",
            "1",
        ],
    ))
    .unwrap();
    let output = training.path("output");
    for changed in [
        vec![
            "--buckets",
            "2",
            "--bucket-positions",
            "1000",
            "--scratch-dir",
            &scratch,
        ],
        vec![],
    ] {
        let mut extra = changed.clone();
        extra.extend(["--epochs", "2", "--resume", &output]);
        let error = minase_trainer::nnue::train::run(training.args("output", &extra))
            .expect_err("a changed bucket count must be rejected");
        assert!(
            matches!(
                find_source::<minase_trainer::nn::Error>(error.as_ref()),
                Some(minase_trainer::nn::Error::CheckpointMismatch(
                    minase_trainer::nn::checkpoint::MismatchField::Settings
                ))
            ),
            "{changed:?}: {error}"
        );
    }
    assert_eq!(
        training::logs(&training.data.temp.join("output")).len(),
        2,
        "rejected resumes must not train"
    );
    assert!(training.data.scratch_entries().is_empty());
}

#[test]
fn missing_or_non_directory_scratch_dir_fails_before_training() {
    let training = Training::new();
    let file = training.path("not-a-directory");
    fs::write(&file, b"").unwrap();
    for (i, scratch) in [training.path("missing"), file].into_iter().enumerate() {
        let output = format!("output-{i}");
        let result = minase_trainer::nnue::train::run(training.args(
            &output,
            &[
                "--buckets",
                "3",
                "--bucket-positions",
                "1000",
                "--scratch-dir",
                &scratch,
                "--epochs",
                "1",
            ],
        ));
        assert!(result.is_err(), "{scratch}");
        let output = training.data.temp.join(&output);
        assert!(!output.join("latest.npz").exists());
        assert!(!output.join("training.jsonl").exists());
    }
    assert!(!training.data.temp.join("missing").exists());
}

#[test]
fn training_removes_its_run_directory_after_a_bucket_overflow() {
    let training = Training::new();
    let scratch = training.scratch();
    let error = minase_trainer::nnue::train::run(training.args(
        "output",
        &[
            "--buckets",
            "3",
            "--bucket-positions",
            "1",
            "--scratch-dir",
            &scratch,
            "--epochs",
            "1",
        ],
    ))
    .expect_err("one position per bucket cannot hold the training set");
    assert!(
        matches!(
            find_source::<Error>(error.as_ref()),
            Some(Error::BucketOverflow {
                limit: 1,
                buckets: 3,
                ..
            })
        ),
        "{error}"
    );
    assert!(training.data.scratch_entries().is_empty());
    assert!(training.data.scratch.is_dir());
}

// 訓練局面と検証局面の通し番号は互いに素で、合わせて0..record_countを覆う。
#[test]
fn training_and_validation_indices_are_disjoint_and_cover_every_record() {
    let data = Data::two_files();
    let store = data.store(3, 1_000);
    let training: BTreeSet<u64> = file_order(&store).into_iter().collect();
    let validation: BTreeSet<u64> = store
        .validation_chunks(1_000)
        .flat_map(|chunk| chunk.unwrap())
        .map(|record| record.index)
        .collect();
    assert!(training.is_disjoint(&validation));
    assert_eq!(
        training.union(&validation).copied().collect::<Vec<_>>(),
        (0..store.record_count()).collect::<Vec<_>>()
    );
}
