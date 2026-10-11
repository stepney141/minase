//! 合成RecordだけでMNSDの境界とバッチの外部契約を検証する。

use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use minase::{
    eval::{
        features::{FEATURE_COUNT, feature_index, lion_feature_index},
        nnue::feature_set::{FeatureSet, HalfKpHmB8},
    },
    training::{
        provenance::{Provenance, ResultOrigin, SearchCondition, StartOrigin, Teacher},
        records::{HEADER_LEN, Header, Outcome, RECORD_LEN, Record},
    },
};
use minase_core::{Color, PieceCode, PieceKind, Position, PositionBuilder, Square};
use nnue_trainer::data::{
    BatchSlot, CANONICAL_ROWS, Dataset, Error, FEATURE_COLUMNS, InputFeatureSet, Lookahead,
    Observations, PackedOccurrence, REVERSE_INDEX_SEGMENT_LEN, ReverseIndex, SENTINEL,
    build_batches, decode_p_feature,
};
use sha2::{Digest, Sha256};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "nnue-trainer-data-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn write(
        &self,
        name: &str,
        seed: u64,
        checksum: u8,
        nodes: u32,
        records: &[Record],
    ) -> PathBuf {
        let path = self.0.join(name);
        let header = Header::new(
            "L0,P0,R1,E0".into(),
            "1".repeat(40),
            [checksum; 32],
            nodes,
            seed,
            records.len() as u64,
        )
        .unwrap();
        let mut bytes = header.encode().to_vec();
        for record in records {
            bytes.extend(record.encode());
        }
        fs::write(&path, &bytes).unwrap();
        let provenance = Provenance {
            format: "minase-provenance".into(),
            version: 1,
            mnsd_sha256: format!("{:x}", Sha256::digest(&bytes)),
            teacher: Teacher {
                generation_commit: "1".repeat(40),
                network_checksum: format!("{checksum:02x}").repeat(32),
                nodes,
                rule_set: "L0,P0,R1,E0".into(),
                search_condition: SearchCondition::InGame,
            },
            result_origin: ResultOrigin::Selfplay,
            start_origin: StartOrigin::Random,
            lambda: 0.75,
            games: None,
        };
        provenance
            .write(File::create(sidecar(&path)).unwrap())
            .unwrap();
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn sidecar(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".provenance.json");
    name.into()
}

fn update_provenance(path: &Path, update: impl FnOnce(&mut serde_json::Value)) {
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(sidecar(path)).unwrap()).unwrap();
    update(&mut value);
    fs::write(sidecar(path), serde_json::to_vec(&value).unwrap()).unwrap();
}

fn refresh_checksum(path: &Path) {
    let checksum = format!("{:x}", Sha256::digest(fs::read(path).unwrap()));
    update_provenance(path, |value| value["mnsd_sha256"] = checksum.into());
}

fn sq(file: u8, rank: u8) -> Square {
    Square::new(file, rank).unwrap()
}

fn single_pawn(lion: bool) -> Position {
    let mut builder = PositionBuilder::new(Color::White);
    builder
        .put(
            sq(8, 1),
            PieceCode::new(Color::White, PieceKind::Pawn).unwrap(),
        )
        .unwrap();
    let mut position = builder.finish().unwrap();
    if lion {
        position.set_lion_capture(Some(sq(9, 4))).unwrap();
    }
    position
}

fn sample(position: &Position, game: u32, ply: u16, score: i16, outcome: Outcome) -> Record {
    Record::from_position(position, score, outcome, game, ply)
}

#[test]
fn mmap_reads_multiple_files_and_checks_global_bounds() {
    let fixture = Fixture::new();
    let first_records = [
        sample(&Position::initial(), 1, 0, 10, Outcome::Win),
        sample(&single_pawn(true), 1, 1, -20, Outcome::Loss),
    ];
    let first = fixture.write("first.bin", 11, 1, 200, &first_records);
    let empty = fixture.write("empty.bin", 33, 1, 200, &[]);
    let last_record = sample(&Position::initial(), 2, 0, 30, Outcome::Draw);
    let last = fixture.write("last.bin", 22, 2, 200, std::slice::from_ref(&last_record));
    // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
    let dataset = unsafe { Dataset::open(&[first, empty, last], None) }.unwrap();
    assert_eq!(dataset.record_count(), 3);
    assert_eq!(dataset.headers().count(), 3);
    for (index, file, offset, expected) in [
        (0, 0, HEADER_LEN, &first_records[0]),
        (1, 0, HEADER_LEN + RECORD_LEN, &first_records[1]),
        (2, 2, HEADER_LEN, &last_record),
    ] {
        assert_eq!(dataset.location(index).unwrap().file, file);
        assert_eq!(dataset.location(index).unwrap().byte_offset, offset);
        assert_eq!(&dataset.record(index).unwrap(), expected);
    }
    for index in [3, u64::MAX] {
        assert!(matches!(
            dataset.record(index),
            Err(Error::RecordIndex { .. })
        ));
        assert!(dataset.teacher_score(index).is_err());
        assert!(dataset.teacher_class(index).is_err());
    }
    assert_eq!(dataset.teacher_classes().len(), 2);
    assert_eq!(dataset.teacher_class(0).unwrap(), 0);
    assert_eq!(dataset.teacher_class(2).unwrap(), 1);
    // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
    assert!(unsafe { Dataset::open(&[], None) }.is_err());
}

#[test]
fn mmap_rejects_invalid_headers_lengths_and_records() {
    let fixture = Fixture::new();
    let records = [sample(&Position::initial(), 1, 0, 1, Outcome::Win)];
    let path = fixture.write("data.bin", 11, 1, 200, &records);
    let original = fs::read(&path).unwrap();
    let mut cases = Vec::new();
    cases.push(original[..135].to_vec());
    cases.push(original[..original.len() - 1].to_vec());
    let mut extra = original.clone();
    extra.push(0);
    cases.push(extra);
    let mut bad_magic = original.clone();
    bad_magic[0] = b'?';
    cases.push(bad_magic);
    let mut overflow = original.clone();
    overflow[128..136].copy_from_slice(&u64::MAX.to_le_bytes());
    cases.push(overflow);
    for bytes in cases {
        fs::write(&path, bytes).unwrap();
        // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
        assert!(unsafe { Dataset::open(std::slice::from_ref(&path), None) }.is_err());
    }
    // 正しいSHAを持っていてもRecord::decodeが予約欄の違反を拒否する。
    let mut reserved = original;
    reserved[HEADER_LEN + 159] = 1;
    fs::write(&path, reserved).unwrap();
    refresh_checksum(&path);
    assert!(matches!(
        // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
        unsafe { Dataset::open(&[path], None) },
        Err(Error::Mnsd(
            minase::training::records::Error::NonzeroReservedBytes
        ))
    ));
}

#[test]
fn mmap_requires_valid_provenance_and_unique_seeds() {
    let fixture = Fixture::new();
    let records = [sample(&Position::initial(), 1, 0, 1, Outcome::Win)];
    let path = fixture.write("data.bin", 11, 1, 200, &records);
    let provenance = fs::read(sidecar(&path)).unwrap();
    fs::remove_file(sidecar(&path)).unwrap();
    assert!(matches!(
        // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
        unsafe { Dataset::open(std::slice::from_ref(&path), None) },
        Err(Error::Io(_))
    ));
    fs::write(sidecar(&path), b"{}").unwrap();
    assert!(matches!(
        // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
        unsafe { Dataset::open(std::slice::from_ref(&path), None) },
        Err(Error::Provenance(_))
    ));
    fs::write(sidecar(&path), &provenance).unwrap();
    update_provenance(&path, |value| value["mnsd_sha256"] = "0".repeat(64).into());
    // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
    assert!(unsafe { Dataset::open(std::slice::from_ref(&path), None) }.is_err());
    fs::write(sidecar(&path), &provenance).unwrap();
    update_provenance(&path, |value| value["teacher"]["nodes"] = 300.into());
    // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
    assert!(unsafe { Dataset::open(std::slice::from_ref(&path), None) }.is_err());
    fs::write(sidecar(&path), &provenance).unwrap();
    // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
    assert!(unsafe { Dataset::open(&[path.clone(), path.clone()], None) }.is_err());
    let other = fixture.write("other.bin", 11, 2, 200, &records);
    // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
    assert!(unsafe { Dataset::open(&[path, other], None) }.is_err());
}

#[test]
fn mmap_rejects_both_kinds_of_game_id_split() {
    let fixture = Fixture::new();
    for human_result in [false, true] {
        let path = fixture.write(
            "data.bin",
            11,
            1,
            200,
            &[sample(&Position::initial(), 1, 0, 1, Outcome::Win)],
        );
        update_provenance(&path, |value| {
            value["start_origin"] = "human-game".into();
            value["games"] = serde_json::json!([{"game": 1, "id": "fixture-game", "ply": 0}]);
            if human_result {
                value["result_origin"] = "human".into();
                value["lambda"] = 0.0.into();
            }
        });
        assert!(matches!(
            // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
            unsafe { Dataset::open(&[path], None) },
            Err(Error::Dataset { .. })
        ));
    }
}

#[test]
fn lookahead_is_file_local_and_validates_record_order() {
    let fixture = Fixture::new();
    let pos = Position::initial();
    let first = fixture.write(
        "first.bin",
        11,
        1,
        200,
        &[
            sample(&pos, 1, 0, 7, Outcome::Win),
            sample(&pos, 1, 1, 2, Outcome::Loss),
            sample(&pos, 1, 3, 1, Outcome::Loss),
        ],
    );
    let second = fixture.write(
        "second.bin",
        22,
        1,
        200,
        &[sample(&pos, 1, 4, 999, Outcome::Win)],
    );
    let options = Lookahead::new(0.5, 3).unwrap();
    // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
    let dataset = unsafe { Dataset::open(&[first, second], Some(options)) }.unwrap();
    let scores: Vec<_> = (0..4).map(|i| dataset.teacher_score(i).unwrap()).collect();
    assert_eq!(scores, [-1.8_f32, 1.0, 1.0, 999.0]);
    assert_eq!(dataset.record(0).unwrap().score(), 7);
    assert_eq!(dataset.teacher_classes()[0].lookahead(), Some(options));
    for (games, plies) in [([2, 1], [0, 1]), ([1, 1], [1, 1]), ([1, 1], [2, 1])] {
        let path = fixture.write(
            "unordered.bin",
            33,
            1,
            200,
            &[
                sample(&pos, games[0], plies[0], 0, Outcome::Draw),
                sample(&pos, games[1], plies[1], 0, Outcome::Draw),
            ],
        );
        assert!(matches!(
            // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
            unsafe { Dataset::open(&[path], Some(options)) },
            Err(Error::RecordOrder { index: 1 })
        ));
    }
}

#[test]
fn teacher_classes_use_header_keys_and_k_uses_only_training() {
    let fixture = Fixture::new();
    let pos = Position::initial();
    // seed=11のgame=4は検証。符号と結果が逆の検証標本をKの校正に混ぜない。
    let first = fixture.write(
        "first.bin",
        11,
        1,
        200,
        &[
            sample(&pos, 1, 0, 1000, Outcome::Win),
            sample(&pos, 4, 0, 32767, Outcome::Loss),
        ],
    );
    let second = fixture.write(
        "second.bin",
        22,
        1,
        200,
        &[sample(&pos, 1, 0, -1000, Outcome::Loss)],
    );
    let third = fixture.write(
        "third.bin",
        33,
        1,
        300,
        &[sample(&pos, 1, 0, 1000, Outcome::Loss)],
    );
    let fourth = fixture.write(
        "fourth.bin",
        44,
        2,
        200,
        &[sample(&pos, 1, 0, 1000, Outcome::Loss)],
    );
    let paths = [first, second, third, fourth];
    // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
    let dataset = unsafe { Dataset::open(&paths, None) }.unwrap();
    assert_eq!(dataset.teacher_classes().len(), 3);
    assert_eq!(dataset.teacher_class(2).unwrap(), 0);
    assert_eq!(dataset.teacher_class(3).unwrap(), 1);
    assert_eq!(dataset.teacher_class(4).unwrap(), 2);
    assert_eq!(dataset.validation_indices(), [1]);
    let ks = dataset.estimate_teacher_ks().unwrap();
    assert!((ks[0] - 50.0).abs() < 1e-8);
    assert!((ks[1] - 4000.0).abs() < 1e-8);
    assert!((ks[2] - 4000.0).abs() < 1e-8);
    // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
    let future = unsafe { Dataset::open(&paths, Some(Lookahead::new(0.9, 40).unwrap())) }.unwrap();
    assert_ne!(dataset.teacher_classes(), future.teacher_classes());
    let only_validation = fixture.write(
        "validation.bin",
        11,
        1,
        200,
        &[sample(&pos, 4, 0, 1, Outcome::Win)],
    );
    assert!(matches!(
        // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
        unsafe { Dataset::open(&[only_validation], None) }
            .unwrap()
            .estimate_teacher_ks(),
        Err(Error::EmptyTeacherClass { class: 0 })
    ));
}

#[test]
fn observation_counts_match_python_canonical_half_without_doubling() {
    let fixture = Fixture::new();
    let pawn = single_pawn(false);
    let path = fixture.write(
        "data.bin",
        11,
        1,
        200,
        &[
            sample(&pawn, 1, 0, 1, Outcome::Win),
            sample(&pawn, 1, 1, 2, Outcome::Win),
            sample(&Position::initial(), 4, 0, 3, Outcome::Draw),
        ],
    );
    // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
    let dataset = unsafe { Dataset::open(&[path], None) }.unwrap();
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let observed = Observations::compute(&dataset, set).unwrap();
        assert_eq!(observed.canonical_counts().len(), 6841);
        // 白歩の手番視点は(29*144 + 10*12 + 8)=4304、正準行は2151。
        // Pythonの鏡映込みの左半分の回数は2であり、4にはしない。
        assert_eq!(observed.canonical_counts()[2151], 2);
        assert_eq!(observed.canonical_counts().iter().sum::<u64>(), 2);
        assert_eq!(observed.input_counts()[4304], 2);
        assert_eq!(observed.input_counts()[10964], 2);
        assert_eq!(observed.input_counts().iter().sum::<u64>(), 4);
        assert_eq!(observed.canonical_mask().iter().filter(|&&v| v).count(), 1);
        assert!(!observed.canonical_mask()[6840]);
        assert!(!observed.input_mask()[set.dimensions()]);
        if let Some(mask) = observed.virtual_mask() {
            assert!(mask[4304] && mask[10964]);
            assert_eq!(mask.iter().filter(|&&v| v).count(), 2);
            assert!(!mask[FEATURE_COUNT]);
        } else {
            assert_eq!(set, InputFeatureSet::P);
        }
    }
}

#[test]
fn halfkp_virtual_observations_are_the_union_of_buckets() {
    let fixture = Fixture::new();
    let mut records = Vec::new();
    for rank in [0, 1] {
        let mut builder = PositionBuilder::new(Color::Black);
        builder
            .put(
                sq(8, rank),
                PieceCode::new(Color::Black, PieceKind::King).unwrap(),
            )
            .unwrap();
        builder
            .put(
                sq(3, 4),
                PieceCode::new(Color::Black, PieceKind::Pawn).unwrap(),
            )
            .unwrap();
        records.push(sample(
            &builder.finish().unwrap(),
            1,
            u16::from(rank),
            1,
            Outcome::Win,
        ));
    }
    let path = fixture.write("data.bin", 11, 1, 200, &records);
    let observed = Observations::compute(
        // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
        &unsafe { Dataset::open(&[path], None) }.unwrap(),
        InputFeatureSet::HalfKpHmB8,
    )
    .unwrap();
    let base = 29 * 144 + 4 * 12 + 3;
    assert_eq!(observed.input_counts()[3 * FEATURE_COUNT + base], 1);
    assert_eq!(observed.input_counts()[6 * FEATURE_COUNT + base], 1);
    let mask = observed.virtual_mask().unwrap();
    assert!(mask[base]);
    for (base, &seen) in mask[..FEATURE_COUNT].iter().enumerate() {
        assert_eq!(
            seen,
            (0..8).any(|bucket| observed.input_counts()[bucket * FEATURE_COUNT + base] != 0)
        );
    }
}

fn verify_inverse(slot: &BatchSlot, inverse: &ReverseIndex, linear: bool) {
    let (features, views, rows) = if linear {
        (slot.linear_features(), 1, CANONICAL_ROWS)
    } else {
        (slot.features(), 2, slot.feature_set().first_layer_rows())
    };
    let p_encoded = !linear && slot.feature_set() == InputFeatureSet::P;
    assert_eq!(inverse.starts().len(), rows + 1);
    assert_eq!(inverse.starts()[0], 0);
    assert_eq!(inverse.starts()[rows] as usize, inverse.entries().len());
    assert_eq!(inverse.segment_starts().len(), rows + 1);
    assert_eq!(inverse.segment_starts()[0], 0);
    assert_eq!(
        inverse.segment_starts()[rows] as usize,
        inverse.segment_count()
    );
    assert_eq!(inverse.segment_offsets().len(), inverse.segment_count() + 1);
    assert_eq!(inverse.segment_offsets()[0], 0);
    assert_eq!(
        inverse.segment_offsets()[inverse.segment_count()] as usize,
        inverse.entries().len()
    );
    assert_eq!(
        inverse.entries().len(),
        features.iter().filter(|&&v| v != SENTINEL).count()
    );
    let mut visited = vec![false; features.len()];
    for row in 0..rows {
        // 固定長区分は行境界をまたがず、全要素を元の順にちょうど1回覆う。
        let begin = inverse.segment_starts()[row] as usize;
        let end = inverse.segment_starts()[row + 1] as usize;
        assert_eq!(inverse.segment_offsets()[begin], inverse.starts()[row]);
        assert_eq!(inverse.segment_offsets()[end], inverse.starts()[row + 1]);
        for segment in begin..end {
            let len = (inverse.segment_offsets()[segment + 1] - inverse.segment_offsets()[segment])
                as usize;
            assert!((1..=REVERSE_INDEX_SEGMENT_LEN).contains(&len));
            if segment + 1 < end {
                assert_eq!(len, REVERSE_INDEX_SEGMENT_LEN);
            }
        }
        let mut previous = None;
        for &word in
            &inverse.entries()[inverse.starts()[row] as usize..inverse.starts()[row + 1] as usize]
        {
            let entry = PackedOccurrence::from_word(word).unwrap();
            let key = (entry.position(), entry.perspective(), entry.column());
            if let Some(previous) = previous {
                assert!(previous < key);
            }
            previous = Some(key);
            assert!((entry.perspective() as usize) < views);
            let source = (entry.position() * views + entry.perspective() as usize)
                * FEATURE_COLUMNS
                + entry.column() as usize;
            assert!(!visited[source]);
            visited[source] = true;
            let feature = features[source];
            assert_ne!(feature, SENTINEL);
            assert_eq!(row as u32, if p_encoded { feature / 2 } else { feature });
            assert_eq!(entry.rotated(), p_encoded && feature & 1 != 0);
        }
    }
    for (seen, &feature) in visited.iter().zip(features) {
        assert_eq!(*seen, feature != SENTINEL);
    }
}

#[test]
fn inverse_segments_preserve_hot_rows_across_fixed_length_boundaries() {
    let fixture = Fixture::new();
    let path = fixture.write(
        "data.bin",
        11,
        1,
        200,
        &[sample(&single_pawn(false), 1, 0, 1, Outcome::Win)],
    );
    // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
    let dataset = unsafe { Dataset::open(&[path], None) }.unwrap();
    // 指示書の128要素分割に対し、直前・一致・直後と複数区分を検査する。
    assert_eq!(REVERSE_INDEX_SEGMENT_LEN, 128);
    for count in [127, 128, 129, 256, 257] {
        let indices = vec![0; count];
        for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
            let batches = build_batches(&dataset, set, &[&indices]).unwrap();
            assert_eq!(batches, build_batches(&dataset, set, &[&indices]).unwrap());
            let batch = &batches[0];
            verify_inverse(batch, batch.first_layer_inverse(), false);
            verify_inverse(batch, batch.linear_inverse(), true);
            // 歩1枚なので線形は1行、第1層は色の異なる2視点で2行が発火する。
            assert_eq!(batch.linear_inverse().segment_count(), count.div_ceil(128));
            assert_eq!(
                batch.first_layer_inverse().segment_count(),
                2 * count.div_ceil(128)
            );
        }
    }
}

#[test]
fn batches_preserve_order_metadata_and_inverse_occurrences() {
    let fixture = Fixture::new();
    let empty = PositionBuilder::new(Color::Black).finish().unwrap();
    let records = [
        sample(&Position::initial(), 1, 0, 101, Outcome::Win),
        sample(&single_pawn(true), 1, 1, -12, Outcome::Loss),
        sample(&empty, 2, 0, 0, Outcome::Draw),
    ];
    let path = fixture.write("data.bin", 11, 1, 200, &records);
    // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
    let dataset = unsafe { Dataset::open(&[path], None) }.unwrap();
    let batches: &[&[u64]] = &[&[2, 0, 1, 0], &[1, 0], &[2]];
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let slots = build_batches(&dataset, set, batches).unwrap();
        assert_eq!(slots, build_batches(&dataset, set, batches).unwrap());
        for (slot, &indices) in slots.iter().zip(batches) {
            assert_eq!(slot.record_indices(), indices);
            assert_eq!(slot.features().len(), slot.len() * 2 * 145);
            assert_eq!(slot.linear_features().len(), slot.len() * 145);
            verify_inverse(slot, slot.first_layer_inverse(), false);
            verify_inverse(slot, slot.linear_inverse(), true);
        }
        assert_eq!(slots[0].piece_counts(), [0, 92, 1, 92]);
        assert_eq!(slots[0].teacher_scores(), [0.0, 101.0, -12.0, 101.0]);
        assert_eq!(slots[0].outcomes(), [1, 2, 0, 2]);
        assert_eq!(slots[0].teacher_classes(), [0, 0, 0, 0]);
        assert!(slots[2].features().iter().all(|&v| v == SENTINEL));
        assert!(
            slots[2]
                .first_layer_inverse()
                .starts()
                .iter()
                .all(|&v| v == 0)
        );
        assert!(slots[2].first_layer_inverse().entries().is_empty());
        assert!(slots[2].linear_inverse().entries().is_empty());
    }
    assert!(matches!(
        build_batches(&dataset, InputFeatureSet::P, &[&[]]),
        Err(Error::BatchSize { size: 0 })
    ));
    assert!(matches!(
        build_batches(&dataset, InputFeatureSet::P, &[&[3], &[0]]),
        Err(Error::RecordIndex { .. })
    ));
}

#[test]
fn p_batch_columns_match_known_python_numbers_and_lion_slot() {
    let fixture = Fixture::new();
    let path = fixture.write(
        "data.bin",
        11,
        1,
        200,
        &[sample(&single_pawn(true), 1, 0, 1, Outcome::Win)],
    );
    // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
    let dataset = unsafe { Dataset::open(&[path], None) }.unwrap();
    let slots = build_batches(&dataset, InputFeatureSet::P, &[&[0]]).unwrap();
    let slot = &slots[0];
    assert_eq!(slot.features()[20], 4303);
    assert_eq!(slot.features()[145 + 20], 10963);
    assert_eq!(slot.features()[144], 13625);
    assert_eq!(slot.features()[145 + 144], 13589);
    assert_eq!(slot.linear_features()[20], 2151);
    assert_eq!(slot.linear_features()[144], 6812);
    for (offset, expected) in [(20, 4304), (165, 10964), (144, 13629), (289, 13593)] {
        assert_eq!(decode_p_feature(slot.features()[offset]).unwrap(), expected);
    }
    for column in 0..144 {
        if column != 20 {
            assert_eq!(slot.features()[column], SENTINEL);
            assert_eq!(slot.features()[145 + column], SENTINEL);
            assert_eq!(slot.linear_features()[column], SENTINEL);
        }
    }
    assert_eq!(slot.piece_counts(), [1]);
}

#[test]
fn halfkp_batch_indices_match_feature_set_for_both_perspectives() {
    let fixture = Fixture::new();
    let mut positions = vec![single_pawn(true)];
    // 全区分・左右両側・両手番と、王不在時の太子を通る。
    for side in Color::ALL {
        for royal in [PieceKind::King, PieceKind::CrownPrince] {
            for rank in 0..3 {
                for file in 0..12 {
                    let mut builder = PositionBuilder::new(side);
                    let piece = if royal == PieceKind::CrownPrince {
                        PieceCode::new_promoted(side, royal).unwrap()
                    } else {
                        PieceCode::new(side, royal).unwrap()
                    };
                    let absolute_rank = if side == Color::Black {
                        rank
                    } else {
                        11 - rank
                    };
                    builder.put(sq(file, absolute_rank), piece).unwrap();
                    builder
                        .put(
                            sq(0, 6),
                            PieceCode::new(side.opposite(), PieceKind::Pawn).unwrap(),
                        )
                        .unwrap();
                    let mut position = builder.finish().unwrap();
                    position.set_lion_capture(Some(sq(11, 6))).unwrap();
                    positions.push(position);
                }
            }
        }
    }
    let records: Vec<_> = positions
        .iter()
        .enumerate()
        .map(|(i, position)| sample(position, 1, i as u16, 0, Outcome::Draw))
        .collect();
    let path = fixture.write("data.bin", 11, 1, 200, &records);
    // SAFETY: このテスト専用のファイルは、呼び出し中とDatasetの生存中に変更・切り詰めしない。
    let dataset = unsafe { Dataset::open(&[path], None) }.unwrap();
    let indices: Vec<_> = (0..positions.len() as u64).collect();
    let slots = build_batches(&dataset, InputFeatureSet::HalfKpHmB8, &[&indices]).unwrap();
    for (batch, position) in positions.iter().enumerate() {
        for (view, side) in [position.side_to_move(), position.side_to_move().opposite()]
            .into_iter()
            .enumerate()
        {
            let context = HalfKpHmB8::context(position, side);
            let offset = (batch * 2 + view) * FEATURE_COLUMNS;
            for square in Square::all() {
                let expected = match position.piece_at(square) {
                    Some(piece) => {
                        HalfKpHmB8::index(context, feature_index(side, piece, square)) as u32
                    }
                    None => SENTINEL,
                };
                assert_eq!(slots[0].features()[offset + square.dense_index()], expected);
            }
            let expected = HalfKpHmB8::index(
                context,
                lion_feature_index(side, position.lion_capture_square().unwrap()),
            );
            assert_eq!(slots[0].features()[offset + 144], expected as u32);
        }
    }
    verify_inverse(&slots[0], slots[0].first_layer_inverse(), false);
}

#[test]
fn dump_batch_writes_complete_json_without_gpu() {
    let fixture = Fixture::new();
    let path = fixture.write(
        "data.bin",
        11,
        1,
        200,
        &[
            sample(&single_pawn(true), 1, 0, 7, Outcome::Win),
            sample(&single_pawn(false), 1, 1, 2, Outcome::Loss),
        ],
    );
    for set in ["P", "HalfKP-hm-b8"] {
        let output = fixture.0.join(set);
        let status = std::process::Command::new(env!("CARGO_BIN_EXE_nnue-trainer"))
            .args(["dump-batch", "--feature-set", set, "--data"])
            .arg(&path)
            .args([
                "--indices",
                "1,0,1",
                "--lookahead-gamma",
                "0.5",
                "--lookahead-plies",
                "3",
                "--output",
            ])
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "{}",
            String::from_utf8_lossy(&status.stderr)
        );
        let dump: serde_json::Value =
            serde_json::from_slice(&fs::read(output.join("batch.json")).unwrap()).unwrap();
        assert_eq!(dump["feature_set"], set);
        assert_eq!(dump["record_indices"], serde_json::json!([1, 0, 1]));
        assert_eq!(dump["piece_counts"], serde_json::json!([1, 1, 1]));
        assert_eq!(dump["teacher_scores"], serde_json::json!([2.0, -2.0, 2.0]));
        assert_eq!(dump["input_features"].as_array().unwrap().len(), 3);
        assert_eq!(dump["input_features"][0].as_array().unwrap().len(), 2);
        assert_eq!(dump["input_features"][0][0].as_array().unwrap().len(), 145);
        assert_eq!(dump["linear_features"][0][20], 2151);
        assert_eq!(dump["linear_features"][0][144], SENTINEL);
        if set == "P" {
            assert_eq!(dump["base_features"][0][0][20], 4304);
            assert_eq!(dump["base_features"][0][1][20], 10964);
            assert_eq!(dump["base_features"][1][0][144], 13629);
        }
    }
    // 先読みは両引数を指定する契約。
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nnue-trainer"))
        .args(["dump-batch", "--feature-set", "P", "--data"])
        .arg(&path)
        .args(["--indices", "0", "--lookahead-gamma", "0.9", "--output"])
        .arg(&fixture.0)
        .output()
        .unwrap();
    assert!(!output.status.success());
}
