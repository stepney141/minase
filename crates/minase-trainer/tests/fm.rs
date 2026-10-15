//! FMの数式、保存形式とコマンドの契約を、手計算した値から検査する。

#[path = "common/mnsd.rs"]
mod mnsd;
use minase::eval::{
    features::FEATURE_COUNT,
    pst::{FM_RANK, FmWeights, Pst},
};
use minase_trainer::data::{Outcome, Sample};
use minase_trainer::error::ContractError;
use minase_trainer::{
    fm::{self, QuarterArgs, ScaleArgs},
    nn::npz::{self, Array},
    pst::v0_piece_values,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

fn v_with(values: &[f64]) -> Vec<[f64; FM_RANK]> {
    let mut v = vec![[0.0; FM_RANK]; FEATURE_COUNT];
    for (row, &value) in v.iter_mut().zip(values) {
        row[0] = value;
    }
    v
}

#[test]
fn quantization_selects_largest_exponent_and_rounds_ties_even() {
    // K=1、a=1なので吸収後はV。e=1で32766となり、e=0の16383を選ぶ。
    let a = [1.0; FM_RANK];
    let q = fm::quantize(
        &v_with(&[16383.0, 0.5, 1.5, 2.5, -0.5, -1.5, -2.5]),
        &a,
        1.0,
    )
    .unwrap();
    assert_eq!(q.exponent, 0);
    assert_eq!(
        q.embeddings[..7].iter().map(|r| r[0]).collect::<Vec<_>>(),
        [16383, 0, 2, 2, 0, -2, -2]
    );
    // 16383.5のちょうど半分ではe=1の丸めが16384になるのでe=0へ下がる。
    assert_eq!(
        fm::quantize(&v_with(&[8191.75]), &a, 1.0).unwrap().exponent,
        0
    );
    assert_eq!(
        fm::quantize(&v_with(&[8191.5]), &a, 1.0).unwrap().exponent,
        1
    );
    assert!(fm::quantize(&v_with(&[16383.5]), &a, 1.0).is_err());
    assert_eq!(
        fm::quantize(&v_with(&[2.0_f64.powi(-30)]), &a, 1.0)
            .unwrap()
            .exponent,
        30
    );
    // 量子化前の吸収済み重みが非零なら、丸めた全要素が0でも受け入れる。
    let tiny = fm::quantize(&v_with(&[2.0_f64.powi(-40)]), &a, 1.0).unwrap();
    assert_eq!(tiny.exponent, 30);
    assert!(tiny.embeddings.iter().flatten().all(|&u| u == 0));
}

#[test]
fn quantization_checks_inputs_and_preserves_absorption_order_and_signs() {
    let mut a = [1.0; FM_RANK];
    a[0] = -4.0;
    a[1] = -0.0;
    // sqrt(9)*2*sqrt(4)=12。e=10では12288、e=11では24576。
    let q = fm::quantize(&v_with(&[2.0]), &a, 9.0).unwrap();
    assert_eq!((q.exponent, q.embeddings[0][0]), (10, 12288));
    assert_eq!(&q.signs[..2], &[-1, 1]);
    for k in [0.0, -1.0, f64::INFINITY, f64::NAN] {
        assert!(fm::quantize(&v_with(&[1.0]), &a, k).is_err());
    }
    assert!(fm::quantize(&[], &a, 1.0).is_err());
    assert!(fm::quantize(&v_with(&[]), &a, 1.0).is_err());
    assert!(fm::quantize(&v_with(&[1.0]), &[0.0; FM_RANK], 1.0).is_err());
    for value in [f64::NAN, f64::INFINITY] {
        assert!(fm::quantize(&v_with(&[value]), &a, 1.0).is_err());
        let mut invalid_a = a;
        invalid_a[0] = value;
        assert!(fm::quantize(&v_with(&[1.0]), &invalid_a, 1.0).is_err());
    }
    // sqrt(K)*Vが先にoverflowする。sqrt(K)*(V*sqrt(a))なら有限なので順序の検査になる。
    assert!(fm::quantize(&v_with(&[1e200]), &[1e-300; FM_RANK], 1e300).is_err());
}

#[test]
fn zero_report_distinguishes_zero_pairs_from_nonzero_sum() {
    let mut u = [[0; FM_RANK]; 4];
    for row in &mut u {
        row[0] = 1;
    }
    // e=1の対の分母は4。6対の各分子は1だが、総和6の補正は1。
    let report = fm::zero_correction_report(&u, &[1; FM_RANK], 1).unwrap();
    assert!(report.all_pair_corrections_zero);
    assert!(!report.identically_zero);
    assert_eq!(
        (
            report.max_absolute_pair_numerator,
            report.absolute_sum_bound,
            report.pair_denominator
        ),
        (1, 6, 4)
    );
    let smaller = fm::zero_correction_report(&u[..3], &[1; FM_RANK], 1).unwrap();
    assert!(smaller.identically_zero);
    assert_eq!(smaller.absolute_sum_bound, 3);
    let at_boundary = fm::zero_correction_report(&u[..2], &[1; FM_RANK], 0).unwrap();
    assert!(!at_boundary.all_pair_corrections_zero);
    let zeros = fm::zero_correction_report(&[], &[1; FM_RANK], 30).unwrap();
    assert!(zeros.identically_zero);
    assert_eq!(zeros.pair_denominator, 1_i128 << 60);
    let rows = vec![[i16::MIN; FM_RANK]; 146];
    let large = fm::zero_correction_report(&rows, &[1; FM_RANK], 30).unwrap();
    assert_eq!(large.max_absolute_pair_numerator, 32 * (1_i128 << 30));
    assert_eq!(large.absolute_sum_bound, 10440 * 32 * (1_i128 << 30));
    let mut signs = [1; FM_RANK];
    signs[..16].fill(-1);
    let cancelling = fm::zero_correction_report(&rows, &signs, 0).unwrap();
    assert!(cancelling.identically_zero);
    assert_eq!(cancelling.absolute_sum_bound, 0);
}

#[test]
fn v3_constructor_round_trip_and_validation() {
    let weights: Vec<_> = (0..FEATURE_COUNT)
        .map(|i| [i as i16, -(i as i16)])
        .collect();
    let u: Vec<_> = (0..FEATURE_COUNT)
        .map(|i| std::array::from_fn(|f| (i + f) as i16))
        .collect();
    let signs = std::array::from_fn(|f| if f % 2 == 0 { -1 } else { 1 });
    let values = v0_piece_values();
    let fm = FmWeights {
        embeddings: &u,
        signs: &signs,
        exponent: 30,
    };
    let pst = Pst::from_weights_with_fm(&weights, values, 1000.25, "規則", fm).unwrap();
    let bytes = pst.encode();
    assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 3);
    let decoded = Pst::decode(&bytes).unwrap();
    assert_eq!(decoded.weights().as_slice(), weights);
    assert_eq!(decoded.fm_weights(), Some(fm));
    assert_eq!(decoded.k().to_bits(), 1000.25_f32.to_bits());
    assert_eq!(decoded.rule_set(), "規則");
    assert_eq!(
        std::array::from_fn::<_, 47, _>(|i| decoded.piece_value_of_state(i)),
        values
    );
    assert_eq!(decoded.encode(), bytes);
    for invalid in [
        FmWeights { exponent: 31, ..fm },
        FmWeights {
            signs: &[0; FM_RANK],
            ..fm
        },
        FmWeights {
            embeddings: &u[..1],
            ..fm
        },
    ] {
        assert!(Pst::from_weights_with_fm(&weights, values, 1.0, "test", invalid).is_err());
    }
    assert!(Pst::from_weights_with_fm(&weights, values, f32::NAN, "test", fm).is_err());
    assert!(Pst::from_weights_with_fm(&weights[..1], values, 1.0, "test", fm).is_err());
    assert!(Pst::from_weights_with_fm(&weights, [0; 47], 1.0, "test", fm).is_err());
    assert!(Pst::from_weights_with_fm(&weights, values, 1.0, "a\0b", fm).is_err());
}

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "minase-fm-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self { root }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
    fn inputs(&self, exponent: u32) {
        // K=1、Vの先頭3行の第1成分が1、aが1なので、最大指数13、u=8192。
        let mut u = vec![[0; FM_RANK]; FEATURE_COUNT];
        for row in &mut u[..3] {
            row[0] = 8192;
        }
        let weights = vec![[0; 2]; FEATURE_COUNT];
        let fm = FmWeights {
            embeddings: &u,
            signs: &[1; FM_RANK],
            exponent,
        };
        let pst = Pst::from_weights_with_fm(&weights, v0_piece_values(), 1.0, "test", fm).unwrap();
        fs::write(self.path("input.bin"), pst.encode()).unwrap();
        self.write_npz(&v_with(&[1.0; 3]), false);
    }
    fn write_npz(&self, v: &[[f64; FM_RANK]], change_pst: bool) {
        let mut arrays = vec![];
        for name in ["middlegame", "endgame"] {
            let mut values = vec![0.0; FEATURE_COUNT];
            if change_pst {
                values[0] = 1.0;
            }
            arrays.push((name, f64_array(&[FEATURE_COUNT], &values)));
        }
        arrays.push(("V", f64_array(&[FEATURE_COUNT, FM_RANK], v.as_flattened())));
        arrays.push(("a", f64_array(&[FM_RANK], &[1.0; FM_RANK])));
        arrays.push((
            "mask",
            Array {
                dtype: "|b1".into(),
                shape: vec![FEATURE_COUNT],
                data: vec![1; FEATURE_COUNT],
            },
        ));
        fs::write(self.path("float.npz"), npz::encode(&arrays).unwrap()).unwrap();
    }
    fn scale(&self, value: i32) -> ScaleArgs {
        ScaleArgs {
            input: self.path("input.bin"),
            float: self.path("float.npz"),
            fm_scale: value,
            output: self.path("output.bin"),
            data: vec![],
            batch: 4096,
            validation_sample: 10000,
            seed: 1,
        }
    }
    fn quarter(&self) -> QuarterArgs {
        QuarterArgs {
            input: self.path("input.bin"),
            output: self.path("output.bin"),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn f64_array(shape: &[usize], values: &[f64]) -> Array {
    Array {
        dtype: "<f8".into(),
        shape: shape.to_vec(),
        data: values.iter().flat_map(|x| x.to_le_bytes()).collect(),
    }
}

#[test]
fn quarter_changes_only_exponent_and_checksum_and_writes_ordered_report() {
    let fixture = Fixture::new();
    fixture.inputs(13);
    fm::quarter(&fixture.quarter()).unwrap();
    let input = fs::read(fixture.path("input.bin")).unwrap();
    let output = fs::read(fixture.path("output.bin")).unwrap();
    let exponent_offset = 80 + FEATURE_COUNT * 4 + 47 * 4 + 4;
    let mut expected = input.clone();
    expected[exponent_offset..exponent_offset + 4].copy_from_slice(&14_u32.to_le_bytes());
    let hash = Sha256::digest(&expected[80..]);
    expected[48..80].copy_from_slice(&hash);
    assert_eq!(output, expected);
    let json = fs::read_to_string(fixture.path("output.quarter.json")).unwrap();
    let report: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        report["input"]["checksum"],
        format!("{:x}", Sha256::digest(&input[80..]))
    );
    assert_eq!(
        report["output"]["checksum"],
        format!("{:x}", Sha256::digest(&output[80..]))
    );
    let keys = [
        "input",
        "output",
        "exponent_before",
        "exponent_after",
        "zero_check",
    ];
    assert_eq!(json_root_keys(&json), keys);
    assert!(json.ends_with('\n'));
}

#[test]
fn quarter_stdout_is_one_saved_report_line() {
    let fixture = Fixture::new();
    fixture.inputs(13);
    let result = Command::new(env!("CARGO_BIN_EXE_minase-trainer"))
        .args(["fm", "quarter", "--input"])
        .arg(fixture.path("input.bin"))
        .arg("--output")
        .arg(fixture.path("output.bin"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1);
    assert_eq!(
        serde_json::from_str::<Value>(&stdout).unwrap(),
        serde_json::from_slice::<Value>(&fs::read(fixture.path("output.quarter.json")).unwrap())
            .unwrap()
    );
}

#[test]
fn conversions_reject_existing_paths_exponent_limit_and_pst_only_inputs() {
    let f = Fixture::new();
    f.inputs(30);
    assert!(fm::quarter(&f.quarter()).is_err());
    assert!(!f.path("output.bin").exists());
    f.inputs(13);
    for name in ["output.bin", "output.quarter.json", "output.scale.json"] {
        fs::write(f.path(name), b"preserve").unwrap();
        if name != "output.scale.json" {
            assert!(fm::quarter(&f.quarter()).is_err());
        }
        if name != "output.quarter.json" {
            assert!(fm::scale(&f.scale(1024)).is_err());
        }
        assert_eq!(fs::read(f.path(name)).unwrap(), b"preserve");
        fs::remove_file(f.path(name)).unwrap();
    }
    let pst_only =
        Pst::from_weights(&vec![[0; 2]; FEATURE_COUNT], v0_piece_values(), 1.0, "test").unwrap();
    fs::write(f.path("input.bin"), pst_only.encode()).unwrap();
    assert!(fm::quarter(&f.quarter()).is_err());
    assert!(fm::scale(&f.scale(1024)).is_err());
}

#[test]
fn scale_identity_zero_and_mismatch_rejections() {
    let f = Fixture::new();
    f.inputs(13);
    for value in [-1, 1025] {
        assert!(fm::scale(&f.scale(value)).is_err());
    }
    let mut args = f.scale(1024);
    args.batch = 0;
    assert!(fm::scale(&args).is_err());
    args.batch = 1;
    args.validation_sample = 0;
    assert!(fm::scale(&args).is_err());
    f.write_npz(&v_with(&[1.0; 3]), true);
    assert_eq!(
        fm::scale(&f.scale(0))
            .unwrap_err()
            .downcast_ref::<ContractError>(),
        Some(&ContractError::FloatingPstMismatch)
    );
    f.write_npz(&v_with(&[2.0, 1.0, 1.0]), false);
    assert_eq!(
        fm::scale(&f.scale(0))
            .unwrap_err()
            .downcast_ref::<ContractError>(),
        Some(&ContractError::FloatingFmMismatch)
    );
    assert!(!f.path("output.bin").exists());
    f.inputs(13);
    let report = fm::scale(&f.scale(1024)).unwrap();
    assert_eq!(
        fs::read(f.path("input.bin")).unwrap(),
        fs::read(f.path("output.bin")).unwrap()
    );
    assert_eq!(report.exponent_after, Some(13));
    fs::remove_file(f.path("output.bin")).unwrap();
    fs::remove_file(f.path("output.scale.json")).unwrap();
    let report = fm::scale(&f.scale(0)).unwrap();
    let bytes = fs::read(f.path("output.bin")).unwrap();
    assert_eq!(&bytes[4..8], &3_u32.to_le_bytes());
    assert_eq!(&bytes[80 + 13_680 * 4 + 47 * 4..], &[0; 8]);
    let output = Pst::decode(&bytes).unwrap();
    assert!(output.fm_weights().is_none());
    assert_eq!(output.rule_set(), "test");
    assert_eq!(report.format_version, 3);
    assert_eq!(report.exponent_after, None);
    assert_eq!(report.max_absolute_embedding, None);
    assert!(report.validation.is_none());
    let json = fs::read_to_string(f.path("output.scale.json")).unwrap();
    assert!(!json.contains("validation"));
    let keys = [
        "input",
        "float",
        "output",
        "fm_scale",
        "format_version",
        "exponent_before",
        "exponent_after",
        "max_absolute_embedding",
    ];
    assert_eq!(json_root_keys(&json), keys);
}

#[test]
fn npz_rejects_wrong_shapes_types_missing_arrays_and_nonfinite_values() {
    let f = Fixture::new();
    f.inputs(13);
    let valid = npz::decode(&fs::read(f.path("float.npz")).unwrap()).unwrap();
    for case in 0..7 {
        let mut arrays = valid.clone();
        match case {
            0 => arrays.get_mut("V").unwrap().shape = vec![FEATURE_COUNT / 2, FM_RANK * 2],
            1 => arrays.get_mut("a").unwrap().shape = vec![FM_RANK, 1],
            2 => arrays.get_mut("middlegame").unwrap().shape = vec![FEATURE_COUNT, 1],
            3 => arrays.get_mut("V").unwrap().dtype = "<i8".into(),
            4 => arrays.get_mut("a").unwrap().data[..8].copy_from_slice(&f64::NAN.to_le_bytes()),
            5 => arrays.get_mut("endgame").unwrap().data[..8]
                .copy_from_slice(&f64::INFINITY.to_le_bytes()),
            6 => {
                arrays.remove("V");
            }
            _ => unreachable!(),
        }
        let encoded: Vec<_> = arrays
            .iter()
            .map(|(key, array)| (key.as_str(), array.clone()))
            .collect();
        fs::write(f.path("float.npz"), npz::encode(&encoded).unwrap()).unwrap();
        assert!(fm::scale(&f.scale(0)).is_err(), "case {case}");
        assert!(!f.path("output.bin").exists());
        assert!(!f.path("output.scale.json").exists());
    }
}

impl Fixture {
    fn validation_data(&self) -> PathBuf {
        use minase::training::records::Header;
        use minase_core::{Color, PieceCode, PieceKind, PositionBuilder, Square};
        let path = self.path("data.mnsd");
        let header = Header::new(
            "L0,P0,R1,E0".into(),
            "1".repeat(40),
            [2; 32],
            minase::training::records::TeacherKind::AlphaBeta,
            200,
            11,
        )
        .unwrap();
        let mut samples = Vec::new();
        for (ply, (count, game)) in [(2, 1), (2, 4), (3, 4)].into_iter().enumerate() {
            let mut builder = PositionBuilder::new(Color::Black);
            for file in 0..count {
                builder
                    .put(
                        Square::new(file, 0).unwrap(),
                        PieceCode::new(Color::Black, PieceKind::Pawn).unwrap(),
                    )
                    .unwrap();
            }
            samples.push(
                Sample::from_position(
                    &builder.finish().unwrap(),
                    0,
                    Outcome::Draw,
                    game,
                    ply as u16,
                    minase::training::records::Origin::RandomSelfplay,
                )
                .unwrap(),
            );
        }
        fs::write(&path, mnsd::independent(header, &samples)).unwrap();
        let mut u = vec![[0; FM_RANK]; FEATURE_COUNT];
        let mut v = v_with(&[]);
        // 先手歩兵の状態29、段0の3升。V=1とa=1の各対が1cpを与える。
        for f in 29 * 144..29 * 144 + 3 {
            u[f][0] = 8192;
            v[f][0] = 1.0;
        }
        let source = Pst::from_weights_with_fm(
            &vec![[0; 2]; FEATURE_COUNT],
            v0_piece_values(),
            1.0,
            "test",
            FmWeights {
                embeddings: &u,
                signs: &[1; FM_RANK],
                exponent: 13,
            },
        )
        .unwrap();
        fs::write(self.path("input.bin"), source.encode()).unwrap();
        self.write_npz(&v, false);
        path
    }
}

#[test]
fn scale_validation_matches_hand_computed_values() {
    let f = Fixture::new();
    let data = f.validation_data();
    let mut args = f.scale(768);
    args.data = vec![data.clone()];
    args.batch = 1;
    let report = serde_json::to_value(fm::scale(&args).unwrap()).unwrap();
    let saved: Value =
        serde_json::from_slice(&fs::read(f.path("output.scale.json")).unwrap()).unwrap();
    assert_eq!(report, saved);
    // 検証の2局面は1対と3対。実数補正は0.75と2.25、整数補正は0と2。
    assert_eq!(
        report["validation"],
        serde_json::json!({
            "samples":2, "seed":1, "split":"validation",
            "float_vs_integer":{"mean_absolute_cp":0.5,"max_absolute_cp":0.75},
            "tuning_vs_baked":{"mean_absolute_cp":0.0,"max_absolute_cp":0.0},
            "correction_cp":{"output_std":1.0,"tuning_std":1.0}
        })
    );
    let mut args = f.scale(768);
    args.data = vec![data];
    args.output = f.path("sampled.bin");
    args.batch = 2;
    let sampled = fm::scale(&args).unwrap();
    assert_eq!(
        serde_json::to_value(sampled.validation.unwrap()).unwrap(),
        report["validation"]
    );
    args.fm_scale = 0;
    args.output = f.path("zero.bin");
    let zero = fm::scale(&args).unwrap().validation.unwrap();
    assert_eq!(zero.float_vs_integer.max_absolute_cp, 0.0);
    assert_eq!(zero.tuning_vs_baked.max_absolute_cp, 0.0);
    assert_eq!(zero.correction_cp.output_std, 0.0);
    assert_eq!(zero.correction_cp.tuning_std, 0.0);
}

#[test]
fn cli_rejects_scale_range() {
    use clap::Parser;
    #[derive(Parser)]
    struct Arguments {
        #[command(flatten)]
        args: fm::ScaleArgs,
    }
    for scale in ["--fm-scale=-1", "--fm-scale=1025"] {
        let error = Arguments::try_parse_from([
            "scale", "--input", "unused", "--float", "unused", "--output", "unused", scale,
        ])
        .err()
        .unwrap();
        assert_eq!(error.kind(), clap::error::ErrorKind::ValueValidation);
    }
}

fn json_root_keys(json: &str) -> Vec<String> {
    use serde::Deserializer;
    use serde::de::{IgnoredAny, MapAccess, Visitor};
    struct Keys;
    impl<'de> Visitor<'de> for Keys {
        type Value = Vec<String>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a JSON object")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut keys = Vec::new();
            while let Some(key) = map.next_key::<String>()? {
                keys.push(key);
                map.next_value::<IgnoredAny>()?;
            }
            Ok(keys)
        }
    }
    let mut deserializer = serde_json::Deserializer::from_str(json);
    let keys = deserializer.deserialize_map(Keys).unwrap();
    deserializer.end().unwrap();
    keys
}
