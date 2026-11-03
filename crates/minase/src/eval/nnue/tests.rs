//! 共通土台の形式、整数推論、差分更新および鏡映不変性の契約を検査する。

use super::accumulator::Accumulator;
use super::feature_set::{HalfKpContext, HalfKpHmB8, Mirror, P};
use super::format::HEADER_LENGTH;
use super::*;
use crate::eval::features::{lion_feature_index, mirror_feature};
use minase_core::test_util::{position_from_codes, sq};
use minase_core::{Move, MoveGenerator, MoveRules};
use sha2::{Digest, Sha256};

/// 学習前のP型・幅64を固定し、埋め込みネットの更新から独立させる。
const EPOCH_ZERO: &[u8] = include_bytes!("../../../nets/nnue-epoch0.bin");

/// 同じ仕様の検査を両入力特徴集合へ具体化する。
macro_rules! both_feature_sets {
    ($name:ident) => {
        mod $name {
            #[test]
            fn p() {
                super::$name::<super::P>();
            }
            #[test]
            fn half_kp() {
                super::$name::<super::HalfKpHmB8>();
            }
        }
    };
}

fn set_architecture<F: FeatureSet>(bytes: &mut [u8], width: usize) {
    let text = format!(
        "features={};ft={width};linear=tapered2;head=pairwise-l16-l32",
        F::NAME
    );
    bytes[8..104].fill(0);
    bytes[8..8 + text.len()].copy_from_slice(text.as_bytes());
}

/// P型の各行を各区分の同じ基底特徴へ複写し、出力までのパラメータを保つ。
fn copy_p_rows<F: FeatureSet>(source: &[u8]) -> Vec<u8> {
    let width = u32::from_le_bytes(source[104..108].try_into().unwrap()) as usize;
    let hidden = HEADER_LENGTH + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4;
    let end = hidden + FEATURE_COUNT * width * 2;
    let mut bytes = source[..hidden].to_vec();
    set_architecture::<F>(&mut bytes, width);
    for _ in 0..F::DIMENSIONS / FEATURE_COUNT {
        bytes.extend_from_slice(&source[hidden..end]);
    }
    bytes.extend_from_slice(&source[end..]);
    refresh_checksum(&mut bytes);
    bytes
}

fn refresh_checksum(bytes: &mut [u8]) {
    let checksum = Sha256::digest(&bytes[HEADER_LENGTH..]);
    bytes[144..176].copy_from_slice(&checksum);
}

fn next(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    *state
}

/// 設計書のパラメータと区画順を独立に持つ。推論側の内部フィールドを参照しない。
#[derive(Clone)]
struct HeadParameters {
    output_weights: Vec<i16>,
    output_bias: i32,
    hidden1_weights: Vec<i16>,
    hidden1_biases: [i32; 15],
    hidden2_weights: Vec<i16>,
    hidden2_biases: [i32; 32],
    hidden2_output: [i16; 32],
}

impl HeadParameters {
    fn zero(width: usize) -> Self {
        Self {
            output_weights: vec![0; width],
            output_bias: 0,
            hidden1_weights: vec![0; 15 * width],
            hidden1_biases: [0; 15],
            hidden2_weights: vec![0; 32 * 30],
            hidden2_biases: [0; 32],
            hidden2_output: [0; 32],
        }
    }

    fn random(width: usize, state: &mut u32, shared: bool) -> Self {
        let mut result = Self::zero(width);
        for value in &mut result.output_weights {
            *value = (next(state) % 16_385) as i16 - 8_192;
        }
        result.output_bias = (next(state) % 8_323_073) as i32 - 4_161_536;
        for value in &mut result.hidden1_weights {
            *value = (next(state) % 1_017) as i16 - 508;
        }
        for value in &mut result.hidden1_biases {
            *value = (next(state) % 520_193) as i32 - 260_096;
        }
        for value in &mut result.hidden2_weights {
            *value = (next(state) % 1_017) as i16 - 508;
        }
        for value in &mut result.hidden2_biases {
            *value = (next(state) % 520_193) as i32 - 260_096;
        }
        for value in &mut result.hidden2_output {
            *value = (next(state) % 16_385) as i16 - 8_192;
        }
        if shared {
            for view in 0..2 {
                for j in 0..width / 4 {
                    let i = view * width / 2 + j;
                    result.output_weights[i + width / 4] = result.output_weights[i];
                    for k in 0..15 {
                        result.hidden1_weights[k * width + i + width / 4] =
                            result.hidden1_weights[k * width + i];
                    }
                }
            }
        }
        result
    }

    fn build(&self, shared: bool) -> Result<PairwiseL16L32, Error> {
        self.build_width(self.output_weights.len(), shared)
    }

    fn build_width(&self, width: usize, shared: bool) -> Result<PairwiseL16L32, Error> {
        PairwiseL16L32::new(
            width,
            &self.output_weights,
            self.output_bias,
            &self.hidden1_weights,
            self.hidden1_biases,
            &self.hidden2_weights,
            self.hidden2_biases,
            self.hidden2_output,
            shared,
        )
    }

    /// 設計書の区画表から符号化し、Head::to_bytesを使わない。
    fn bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        for value in &self.output_weights {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(self.output_bias.to_le_bytes());
        for value in &self.hidden1_weights {
            bytes.extend(value.to_le_bytes());
        }
        for value in self.hidden1_biases {
            bytes.extend(value.to_le_bytes());
        }
        for value in &self.hidden2_weights {
            bytes.extend(value.to_le_bytes());
        }
        for value in self.hidden2_biases {
            bytes.extend(value.to_le_bytes());
        }
        for value in self.hidden2_output {
            bytes.extend(value.to_le_bytes());
        }
        bytes
    }
}

/// 中間値も仕様上の境界を検査するために返す。
struct ReferenceOutput {
    s: Vec<i64>,
    z: [i64; 15],
    u: [i64; 30],
    y: [i64; 32],
    out: i64,
    cp: i64,
}

/// 設計書「出力部の定義」の6段階をi64だけで計算する参照実装。
fn reference_head(p: &HeadParameters, us: &[i16], them: &[i16], scale: i64) -> ReferenceOutput {
    let width = p.output_weights.len();
    let mut s = vec![0_i64; width];
    for (view, acc) in [us, them].into_iter().enumerate() {
        for j in 0..width / 4 {
            let c0 = i64::from(acc[j]).clamp(0, 255);
            let c1 = i64::from(acc[j + width / 4]).clamp(0, 255);
            let c2 = i64::from(acc[j + width / 2]).clamp(0, 255);
            let c3 = i64::from(acc[j + 3 * width / 4]).clamp(0, 255);
            s[view * width / 2 + j] = c0 * c1 / 512;
            s[view * width / 2 + j + width / 4] = c2 * c3 / 512;
        }
    }
    let z = std::array::from_fn(|k| {
        i64::from(p.hidden1_biases[k])
            + (0..width)
                .map(|i| i64::from(p.hidden1_weights[k * width + i]) * s[i])
                .sum::<i64>()
    });
    let mut u = [0; 30];
    for k in 0..15 {
        u[k] = (z[k] * z[k] / 8_388_608).min(127);
        u[15 + k] = z[k].div_euclid(256).clamp(0, 127);
    }
    let y = std::array::from_fn(|k| {
        let sum = i64::from(p.hidden2_biases[k])
            + (0..30)
                .map(|i| i64::from(p.hidden2_weights[k * 30 + i]) * u[i])
                .sum::<i64>();
        sum.div_euclid(256).clamp(0, 127)
    });
    let out = i64::from(p.output_bias)
        + (0..width)
            .map(|i| i64::from(p.output_weights[i]) * s[i])
            .sum::<i64>()
        + (0..32)
            .map(|k| i64::from(p.hidden2_output[k]) * y[k])
            .sum::<i64>();
    let cp = (out * scale + 32_768).div_euclid(65_536);
    ReferenceOutput {
        s,
        z,
        u,
        y,
        out,
        cp,
    }
}

/// 線形補正APIの検査には、埋め込みネットによらない幅4の固定重みを使う。
fn linear_correction_base() -> Vec<u8> {
    let mut values = [100; PIECE_STATE_COUNT];
    values[11] = 200;
    values[21] = 200;
    let mut linear = vec![[1, -2]; FEATURE_COUNT];
    linear[12][0] = -1;
    linear[23][0] = -1;
    let mut parameters = HeadParameters::zero(4);
    parameters.output_weights.fill(1);
    let head = parameters.build(true).unwrap();
    encode::<P, _>(&EncodeInput {
        width: 4,
        cp_per_logit: 200.0,
        rule_set: "engine-default",
        piece_values: &values,
        linear: &linear,
        weights: &vec![1; FEATURE_COUNT * 4],
        biases: &[2; 4],
        head: &head,
    })
    .unwrap()
}

#[test]
fn add_to_linear_midgame_rejects_wrong_lengths_and_headers() {
    let bytes = linear_correction_base();
    assert!(matches!(
        add_to_linear_midgame(&bytes, &[0; 3]),
        Err(Error::InvalidParameterLength {
            expected: FEATURE_COUNT,
            actual: 3,
            ..
        })
    ));
    let corrections = vec![0; FEATURE_COUNT];
    for length in [0, 191, 192, 192 + FEATURE_COUNT * 4 - 1] {
        assert!(matches!(
            add_to_linear_midgame(&bytes[..length], &corrections),
            Err(Error::InvalidLength { .. })
        ));
    }
    let mut invalid = bytes;
    invalid[4..8].copy_from_slice(&4_u32.to_le_bytes());
    assert!(matches!(
        add_to_linear_midgame(&invalid, &corrections),
        Err(Error::UnsupportedVersion { actual: 4 })
    ));
}

#[test]
fn add_to_linear_midgame_checks_overflow_and_accepts_exact_i16_limits() {
    let bytes = linear_correction_base();
    let mut corrections = vec![0; FEATURE_COUNT];
    corrections[0] = i16::MAX;
    assert_eq!(
        add_to_linear_midgame(&bytes, &corrections),
        Err(Error::ParameterOutOfRange {
            parameter: "linear",
            index: 0,
            value: 32768,
        })
    );
    corrections[0] = 0;
    corrections[12] = i16::MIN;
    assert_eq!(
        add_to_linear_midgame(&bytes, &corrections),
        Err(Error::ParameterOutOfRange {
            parameter: "linear",
            index: 12,
            value: -32769,
        })
    );
    corrections[0] = i16::MAX - 1;
    corrections[11] = i16::MAX - 1;
    corrections[12] = i16::MIN + 1;
    corrections[23] = i16::MIN + 1;
    let written = add_to_linear_midgame(&bytes, &corrections).unwrap();
    let network = Nnue::decode(&written).unwrap();
    assert_eq!(network.linear[0][0], i16::MAX);
    assert_eq!(network.linear[12][0], i16::MIN);
    assert_eq!(
        &written[144..176],
        Sha256::digest(&written[192..]).as_slice()
    );
    for offset in 0..bytes.len() {
        if (144..176).contains(&offset)
            || (192..192 + FEATURE_COUNT * 4).contains(&offset) && (offset - 192) % 4 < 2
        {
            continue;
        }
        assert_eq!(written[offset], bytes[offset], "offset {offset}");
    }
    assert_eq!(
        add_to_linear_midgame(&bytes, &vec![0; FEATURE_COUNT]).unwrap(),
        bytes
    );
}

/// 任意の許容幅で、鏡映共有された非零の重みを独立に符号化する。
fn random_bytes<F: FeatureSet>(width: usize) -> Vec<u8> {
    let mut state = 17;
    let mut linear = vec![[0_i16; 2]; FEATURE_COUNT];
    let mut hidden = vec![0_i16; F::DIMENSIONS * width];
    for f in 0..FEATURE_COUNT {
        let partner = mirror_feature(f);
        if f > partner {
            continue;
        }
        let pair = std::array::from_fn(|_| (next(&mut state) % 1601) as i16 - 800);
        linear[f] = pair;
        linear[partner] = pair;
        for j in 0..width {
            let value = (next(&mut state) % 25) as i16 - 12;
            hidden[f * width + j] = value;
            hidden[partner * width + (j + width / 2) % width] = value;
        }
    }
    if F::MIRROR == Mirror::Normalized {
        for value in &mut hidden {
            *value = (next(&mut state) % 25) as i16 - 12;
        }
    }
    let mut bytes = EPOCH_ZERO[..HEADER_LENGTH].to_vec();
    set_architecture::<F>(&mut bytes, width);
    bytes[104..108].copy_from_slice(&(width as u32).to_le_bytes());
    for pair in linear {
        for value in pair {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes.extend_from_slice(
        &EPOCH_ZERO[HEADER_LENGTH + FEATURE_COUNT * 4
            ..HEADER_LENGTH + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4],
    );
    for value in hidden {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let mut biases: Vec<i32> = (0..width)
        .map(|_| (next(&mut state) % 101) as i32)
        .collect();
    if F::MIRROR == Mirror::Shared {
        for j in 0..width / 2 {
            biases[j + width / 2] = biases[j];
        }
    }
    for value in biases {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let mut head = HeadParameters::random(width, &mut state, F::MIRROR == Mirror::Shared);
    head.output_bias = -19_713;
    for value in &mut head.hidden2_output {
        *value /= 32;
    }
    bytes.extend(head.bytes());
    refresh_checksum(&mut bytes);
    bytes
}

fn networks<F: FeatureSet>() -> [Network<F, PairwiseL16L32>; 2] {
    let random = Network::<F, PairwiseL16L32>::decode(&random_bytes::<F>(4)).unwrap();
    let accumulator = random.refresh_accumulator(&Position::initial());
    assert_ne!(random.nonlinear_cp(&accumulator, Color::Black), 0);
    [
        Network::decode(&copy_p_rows::<F>(EPOCH_ZERO)).unwrap(),
        random,
    ]
}

/// 着手前後の差分累算値が完全再計算と一致し、親累算値を変更しないことを検査する。
fn assert_move_accumulator<F: FeatureSet, H: Head>(
    nnue: &Network<F, H>,
    position: &mut Position,
    mv: Move,
) {
    let before = nnue.refresh_accumulator(position);
    assert_mirror_when_defined(nnue, position);
    let undo = position.make_move_unchecked(mv, MoveRules::standard());
    assert_mirror_when_defined(nnue, position);
    let mut after = Accumulator::new(nnue.width());
    nnue.update_accumulator_after_move(&before, &mut after, position, &undo);
    assert_eq!(after, nnue.refresh_accumulator(position));
    assert_eq!(
        nnue.evaluate_accumulator(&after, position.side_to_move()),
        evaluate(nnue, position)
    );
    position.unmake_move(undo);
    assert_eq!(before, nnue.refresh_accumulator(position));
}

/// 盤面から行を列挙し、i16の累算処理を使わずi32でバイアスと重みを足す。
fn assert_accumulator_matches_i32<F: FeatureSet>(
    net: &Network<F, PairwiseL16L32>,
    position: &Position,
    accumulator: &Accumulator<F>,
) {
    for side in Color::ALL {
        let context = F::context(position, side);
        let mut expected: Vec<i32> = net.biases.iter().copied().map(i32::from).collect();
        let board_rows = position
            .occupied()
            .into_iter()
            .map(|square| feature_index(side, position.piece_at(square).unwrap(), square));
        let lion_row = position
            .lion_taken_by_non_lion()
            .map(|trigger| lion_feature_index(side, trigger.square));
        for base in board_rows.chain(lion_row) {
            let row = F::index(context, base);
            for (j, value) in expected.iter_mut().enumerate() {
                *value += i32::from(net.weights[row * net.width() + j]);
            }
        }
        for (&actual, expected) in accumulator.hidden[side.index()].iter().zip(expected) {
            assert!((-32_630..=32_630).contains(&expected));
            assert!(i16::try_from(expected).is_ok());
            assert_eq!(i32::from(actual), expected);
        }
    }
    assert_eq!(*accumulator, net.refresh_accumulator(position));
    #[cfg(feature = "invariants")]
    net.assert_accumulator(position, accumulator, 0);
}

both_feature_sets!(dense_accumulators_at_parameter_limits_match_i32);
/// 144升と先獅子を発火させ、境界の重みで移動、捕獲、先獅子、王駒移動、null moveを検査する。
fn dense_accumulators_at_parameter_limits_match_i32<F: FeatureSet>() {
    let pieces: Vec<_> = minase_core::Square::all()
        .map(|square| {
            let (side, kind) = match (square.file(), square.rank()) {
                (5, 0) => (Color::Black, PieceKind::King),
                (11, 11) => (Color::White, PieceKind::King),
                (4, 5) => (Color::Black, PieceKind::Rook),
                (5, 5) => (Color::White, PieceKind::Rook),
                (5, 4) => (Color::Black, PieceKind::Lion),
                _ => (Color::White, PieceKind::Pawn),
            };
            (square, PieceCode::new(side, kind).unwrap())
        })
        .collect();
    let mut initial = position_from_codes(Color::Black, &pieces);
    initial.set_lion_capture(Some(sq(5, 5))).unwrap();
    assert_eq!(initial.occupied().popcount(), 144);
    // 通常の捕獲で先獅子が消え、獅子捕獲で発生し、空升への移動で再び消える。
    // 最後の王駒移動はHalfKPの鏡映文脈を変える。
    let moves = [
        (sq(4, 5), sq(4, 4), 143, None),
        (sq(5, 5), sq(5, 4), 142, Some(sq(5, 4))),
        (sq(4, 4), sq(4, 5), 142, None),
        (sq(5, 4), sq(5, 5), 142, None),
        (sq(5, 0), sq(6, 0), 141, None),
    ];
    for weight in [-220_i16, 220] {
        for bias in [-510_i32, 510] {
            let template = Network::<F, PairwiseL16L32>::decode(&random_bytes::<F>(4)).unwrap();
            let bytes = encode::<F, _>(&EncodeInput {
                weights: &vec![weight; F::DIMENSIONS * 4],
                ..encoding_input(&template, &[bias; 4])
            })
            .unwrap();
            let net = Network::<F, PairwiseL16L32>::decode(&bytes).unwrap();
            let mut position = initial.clone();
            let mut accumulator = net.refresh_accumulator(&position);
            for hidden in &accumulator.hidden {
                assert!(
                    hidden
                        .iter()
                        .all(|&value| i32::from(value) == bias + 145 * i32::from(weight))
                );
            }
            let mut history = Vec::new();
            for step in 0..=moves.len() {
                assert_accumulator_matches_i32(&net, &position, &accumulator);
                let lion_before = position.lion_capture_square();
                let null_undo = position.make_null_move();
                let mut after_null = Accumulator::new(net.width());
                net.update_accumulator_after_null(&accumulator, &mut after_null, lion_before);
                assert_eq!(position.lion_capture_square(), None);
                assert_accumulator_matches_i32(&net, &position, &after_null);
                position.unmake_null_move(null_undo);
                assert_eq!(position.lion_capture_square(), lion_before);
                assert_accumulator_matches_i32(&net, &position, &accumulator);
                let Some(&(from, to, count, lion)) = moves.get(step) else {
                    break;
                };
                let undo = position.make_move_unchecked(
                    Move {
                        from,
                        mid: None,
                        to,
                        promote: false,
                    },
                    MoveRules::standard(),
                );
                assert_eq!(position.occupied().popcount(), count);
                assert_eq!(position.lion_capture_square(), lion);
                let mut after = Accumulator::new(net.width());
                net.update_accumulator_after_move(&accumulator, &mut after, &position, &undo);
                if step == moves.len() - 1 && F::MIRROR == Mirror::Normalized {
                    assert_ne!(accumulator.contexts[0], after.contexts[0]);
                }
                history.push((undo, accumulator));
                accumulator = after;
            }
            for (undo, parent) in history.into_iter().rev() {
                position.unmake_move(undo);
                assert_accumulator_matches_i32(&net, &position, &parent);
            }
            assert_eq!(position, initial);
        }
    }
}

both_feature_sets!(quiet_move_pst_delta_matches_refreshed_linear_numerator);
/// 静かな手の差分は同じ視点の線形分子差に等しく、非線形評価と720での除算を含まない。
fn quiet_move_pst_delta_matches_refreshed_linear_numerator<F: FeatureSet>() {
    let sparse = minase_core::test_util::position(
        Color::Black,
        &[
            (sq(0, 11), Color::Black, PieceKind::King),
            (sq(11, 0), Color::White, PieceKind::King),
            (sq(5, 7), Color::Black, PieceKind::Pawn),
            (sq(5, 6), Color::Black, PieceKind::Lion),
        ],
    );
    let generator = MoveGenerator::new(MoveRules::standard());
    for nnue in networks::<F>() {
        let numerator = |position: &Position, side: Color| {
            let accumulator = nnue.refresh_accumulator(position);
            let q = i64::from(accumulator.piece_count.clamp(2, 92) - 2);
            let [mg, eg] = accumulator.sums[side.index()].map(i64::from);
            q * mg + (90 - q) * eg
        };
        for side in Color::ALL {
            let mut saw_promotion = false;
            let mut saw_jitto = false;
            let mut saw_nonzero_delta = false;
            for mut position in [Position::initial(), sparse.clone()] {
                if side == Color::White {
                    position = minase_core::test_util::reflect_ranks_and_swap_colors(&position);
                }
                assert_eq!(position.side_to_move(), side);
                let before = numerator(&position, side);
                let mut moves = Vec::new();
                generator.generate_moves(&position, &mut moves);
                for mv in moves {
                    if position
                        .captured_squares(mv)
                        .into_iter()
                        .any(|sq| sq.is_some())
                    {
                        continue;
                    }
                    saw_promotion |= mv.promote;
                    saw_jitto |= mv.from == mv.to;
                    let delta = nnue.quiet_move_pst_delta(&position, mv);
                    saw_nonzero_delta |= delta != 0;
                    let undo = position.make_move_unchecked(mv, generator.rules());
                    assert_eq!(
                        delta,
                        numerator(&position, side) - before,
                        "side={side:?}, move={mv:?}"
                    );
                    position.unmake_move(undo);
                }
            }
            assert!(saw_promotion, "side={side:?}");
            assert!(saw_jitto, "side={side:?}");
            assert!(saw_nonzero_delta, "side={side:?}");
        }
    }
}

both_feature_sets!(accumulator_updates_match_full_refresh_across_move_shapes);
/// 差分累算値が通常手、特殊移動、先獅子状態、およびnull moveで完全再計算と一致する。
fn accumulator_updates_match_full_refresh_across_move_shapes<F: FeatureSet>() {
    for nnue in &networks::<F>() {
        let black_king = PieceCode::new(Color::Black, PieceKind::King).unwrap();
        let white_king = PieceCode::new(Color::White, PieceKind::King).unwrap();
        let black_pawn = PieceCode::new(Color::Black, PieceKind::Pawn).unwrap();
        let white_pawn = PieceCode::new(Color::White, PieceKind::Pawn).unwrap();
        let black_lion = PieceCode::new(Color::Black, PieceKind::Lion).unwrap();

        let mut promotion = position_from_codes(
            Color::Black,
            &[
                (sq(0, 11), black_king),
                (sq(11, 0), white_king),
                (sq(5, 3), black_pawn),
                (sq(5, 2), white_pawn),
            ],
        );
        assert_move_accumulator(
            nnue,
            &mut promotion,
            Move {
                from: sq(5, 3),
                mid: None,
                to: sq(5, 2),
                promote: true,
            },
        );

        let mut lion = position_from_codes(
            Color::Black,
            &[
                (sq(0, 11), black_king),
                (sq(11, 0), white_king),
                (sq(5, 5), black_lion),
                (sq(5, 4), white_pawn),
                (sq(5, 3), white_pawn),
            ],
        );
        assert_move_accumulator(
            nnue,
            &mut lion,
            Move {
                from: sq(5, 5),
                mid: Some(sq(5, 4)),
                to: sq(5, 3),
                promote: false,
            },
        );

        let mut igui = position_from_codes(
            Color::Black,
            &[
                (sq(0, 11), black_king),
                (sq(11, 0), white_king),
                (sq(5, 5), black_lion),
                (sq(5, 4), white_pawn),
            ],
        );
        assert_move_accumulator(
            nnue,
            &mut igui,
            Move {
                from: sq(5, 5),
                mid: Some(sq(5, 4)),
                to: sq(5, 5),
                promote: false,
            },
        );

        let mut jitto = position_from_codes(
            Color::Black,
            &[
                (sq(0, 11), black_king),
                (sq(11, 0), white_king),
                (sq(5, 5), black_lion),
            ],
        );
        assert_move_accumulator(
            nnue,
            &mut jitto,
            Move {
                from: sq(5, 5),
                mid: Some(sq(5, 4)),
                to: sq(5, 5),
                promote: false,
            },
        );

        let black_rook = PieceCode::new(Color::Black, PieceKind::Rook).unwrap();
        let white_lion = PieceCode::new(Color::White, PieceKind::Lion).unwrap();
        let mut lion_capture = position_from_codes(
            Color::Black,
            &[
                (sq(0, 11), black_king),
                (sq(11, 0), white_king),
                (sq(5, 5), black_rook),
                (sq(5, 3), white_lion),
            ],
        );
        let before = nnue.refresh_accumulator(&lion_capture);
        let undo = lion_capture.make_move_unchecked(
            Move {
                from: sq(5, 5),
                mid: None,
                to: sq(5, 3),
                promote: false,
            },
            MoveRules::standard(),
        );
        let mut after = Accumulator::new(nnue.width());
        nnue.update_accumulator_after_move(&before, &mut after, &lion_capture, &undo);
        assert_eq!(after, nnue.refresh_accumulator(&lion_capture));
        assert_eq!(after.piece_count, 3);
        assert!(lion_capture.lion_taken_by_non_lion().is_some());
        for side in Color::ALL {
            assert_eq!(
                evaluate_for_diagnostics(nnue, &lion_capture, side),
                evaluate_for_diagnostics(nnue, &mirror(&lion_capture), side)
            );
        }
        assert_eq!(
            nnue.evaluate_accumulator(&after, lion_capture.side_to_move()),
            evaluate(nnue, &lion_capture)
        );

        let mut normal_response = lion_capture.clone();
        assert_move_accumulator(
            nnue,
            &mut normal_response,
            Move {
                from: sq(11, 0),
                mid: None,
                to: sq(10, 0),
                promote: false,
            },
        );

        let lion_before = lion_capture
            .lion_taken_by_non_lion()
            .map(|trigger| trigger.square);
        let null_undo = lion_capture.make_null_move();
        let mut after_null = Accumulator::new(nnue.width());
        nnue.update_accumulator_after_null(&after, &mut after_null, lion_before);
        assert_eq!(after_null, nnue.refresh_accumulator(&lion_capture));
        assert_eq!(after_null.piece_count, 3);
        assert_eq!(
            nnue.evaluate_accumulator(&after_null, lion_capture.side_to_move()),
            evaluate(nnue, &lion_capture)
        );
        lion_capture.unmake_null_move(null_undo);
        assert_eq!(after, nnue.refresh_accumulator(&lion_capture));
        assert_eq!(after.piece_count, 3);
        assert!(lion_capture.lion_taken_by_non_lion().is_some());
        for side in Color::ALL {
            assert_eq!(
                evaluate_for_diagnostics(nnue, &lion_capture, side),
                evaluate_for_diagnostics(nnue, &mirror(&lion_capture), side)
            );
        }
        assert_eq!(
            nnue.evaluate_accumulator(&after, lion_capture.side_to_move()),
            evaluate(nnue, &lion_capture)
        );
        lion_capture.unmake_move(undo);
        assert_eq!(before, nnue.refresh_accumulator(&lion_capture));
    }
}

/// P型は全局面、正規化型は両視点の基準王駒が一意な局面で鏡映不変になる。
fn assert_mirror_when_defined<F: FeatureSet, H: Head>(net: &Network<F, H>, position: &Position) {
    let unique = Color::ALL.into_iter().all(|side| {
        let mut kings = 0;
        let mut princes = 0;
        for square in position.occupied() {
            let piece = position.piece_at(square).unwrap();
            if piece.color() != Some(side) {
                continue;
            }
            match piece.kind() {
                Some(PieceKind::King) => kings += 1,
                Some(PieceKind::CrownPrince) => princes += 1,
                _ => {}
            }
        }
        kings == 1 || (kings == 0 && princes == 1)
    });
    if F::MIRROR == Mirror::Shared || unique {
        for side in Color::ALL {
            assert_eq!(
                evaluate_for_diagnostics(net, position, side),
                evaluate_for_diagnostics(net, &mirror(position), side)
            );
        }
    }
}

fn mirror(position: &Position) -> Position {
    let pieces: Vec<_> = position
        .occupied()
        .into_iter()
        .map(|square| {
            (
                sq(11 - square.file(), square.rank()),
                position.piece_at(square).unwrap(),
            )
        })
        .collect();
    let mut reflected = position_from_codes(position.side_to_move(), &pieces);
    reflected
        .set_lion_capture(
            position
                .lion_taken_by_non_lion()
                .map(|trigger| sq(11 - trigger.square.file(), trigger.square.rank())),
        )
        .unwrap();
    reflected
}

both_feature_sets!(random_games_preserve_accumulators_symmetry_and_epoch_zero_evaluation);
/// 4,096局面以上で差分、null move、取消、左右鏡映とエポック0のPST一致を検査する。
fn random_games_preserve_accumulators_symmetry_and_epoch_zero_evaluation<F: FeatureSet>() {
    let nets = networks::<F>();
    let pst =
        crate::eval::pst::Pst::decode(include_bytes!("../../../nets/nnue-epoch0-pst.bin")).unwrap();
    let generator = MoveGenerator::standard();
    let mut visited = 0;
    for seed in 1..=16 {
        let mut state = seed;
        let mut position = Position::initial();
        let mut accumulators = nets
            .each_ref()
            .map(|net| net.refresh_accumulator(&position));
        let mut children = nets.each_ref().map(|net| Accumulator::new(net.width()));
        let mut nulls = nets.each_ref().map(|net| Accumulator::new(net.width()));
        let mut history = Vec::new();
        for _ in 0..256 {
            for (i, net) in nets.iter().enumerate() {
                assert_eq!(accumulators[i], net.refresh_accumulator(&position));
                for side in Color::ALL {
                    assert_eq!(
                        net.evaluate_accumulator(&accumulators[i], side),
                        evaluate_for_diagnostics(net, &position, side)
                    );
                }
                assert_mirror_when_defined(net, &position);
            }
            assert_eq!(
                evaluate(&nets[0], &position),
                crate::eval::pst::evaluate(&pst, &position)
            );
            visited += 1;
            let mut moves = Vec::new();
            generator.generate_moves(&position, &mut moves);
            // 王駒の消失は専用フィクスチャで検査し、ここでは長い対局列を保つ。
            moves.retain(|&mv| {
                position
                    .captured_squares(mv)
                    .into_iter()
                    .flatten()
                    .all(|square| {
                        !matches!(
                            position.piece_at(square).unwrap().kind(),
                            Some(PieceKind::King | PieceKind::CrownPrince)
                        )
                    })
            });
            assert!(!moves.is_empty());
            let parents = accumulators.clone();
            let undo = position.make_move_unchecked(
                moves[next(&mut state) as usize % moves.len()],
                MoveRules::standard(),
            );
            for (i, net) in nets.iter().enumerate() {
                net.update_accumulator_after_move(
                    &accumulators[i],
                    &mut children[i],
                    &position,
                    &undo,
                );
                assert_eq!(children[i], net.refresh_accumulator(&position));
                std::mem::swap(&mut accumulators[i], &mut children[i]);
            }
            history.push((undo, parents));
            let lion = position
                .lion_taken_by_non_lion()
                .map(|trigger| trigger.square);
            let undo = position.make_null_move();
            for (i, net) in nets.iter().enumerate() {
                net.update_accumulator_after_null(&accumulators[i], &mut nulls[i], lion);
                assert_eq!(nulls[i], net.refresh_accumulator(&position));
                assert_eq!(
                    net.evaluate_accumulator(&nulls[i], position.side_to_move()),
                    evaluate(net, &position)
                );
            }
            position.unmake_null_move(undo);
        }
        for (undo, parents) in history.into_iter().rev() {
            position.unmake_move(undo);
            for (net, parent) in nets.iter().zip(&parents) {
                assert_eq!(*parent, net.refresh_accumulator(&position));
            }
            assert_eq!(
                evaluate(&nets[0], &position),
                crate::eval::pst::evaluate(&pst, &position)
            );
        }
        assert_eq!(position, Position::initial());
    }
    assert_eq!(visited, 4096);
}

#[test]
fn embedded_metadata_and_search_values_match_baseline() {
    let nnue = weights().unwrap();
    assert_eq!(
        nnue.width(),
        u32::from_le_bytes(EMBEDDED[104..108].try_into().unwrap()) as usize
    );
    let baseline = Nnue::decode(EPOCH_ZERO).unwrap();
    assert_eq!(nnue.pawn_value(), baseline.pawn_value());
    assert_eq!(nnue.max_promotion_gain(), baseline.max_promotion_gain());
    assert_eq!(nnue.cp_per_logit(), baseline.cp_per_logit());
    for state in 0..PIECE_STATE_COUNT {
        assert_eq!(
            nnue.piece_value_of_state(state),
            baseline.piece_value_of_state(state)
        );
    }
    assert_eq!(
        nnue.checksum(),
        &<[u8; 32]>::from(Sha256::digest(&EMBEDDED[HEADER_LENGTH..]))
    );
    let codes = minase_core::rules::parse_rule_set(nnue.rule_set()).unwrap();
    assert_eq!(
        minase_core::Rules::from_codes(&codes).unwrap(),
        minase_core::Rules::ENGINE_DEFAULT
    );
}

/// 復号後の重みを、ファイル上のi32バイアスとともに符号化へ渡す。
fn encoding_input<'a, F: FeatureSet, H: Head>(
    net: &'a Network<F, H>,
    biases: &'a [i32],
) -> EncodeInput<'a, H> {
    EncodeInput {
        width: net.width() as u32,
        cp_per_logit: net.cp_per_logit(),
        rule_set: net.rule_set(),
        piece_values: &net.piece_values,
        linear: &net.linear,
        weights: &net.weights,
        biases,
        head: &net.head,
    }
}

/// 配布済みのMNUE版5を、予約領域と検査和も含めて完全に再現する。
#[test]
fn embedded_encode_round_trip_is_byte_identical() {
    let net = Nnue::decode(EMBEDDED).unwrap();
    assert_eq!(
        encode::<ActiveFeatureSet, _>(&encoding_input(
            &net,
            &net.biases
                .iter()
                .copied()
                .map(i32::from)
                .collect::<Vec<_>>()
        ))
        .unwrap(),
        EMBEDDED
    );
}

both_feature_sets!(encode_round_trip_preserves_random_parameters);
fn encode_round_trip_preserves_random_parameters<F: FeatureSet>() {
    for width in [4, 8] {
        // 独立した既存の符号化で作った重みを参照値とする。
        let original = random_bytes::<F>(width);
        let net = Network::<F, PairwiseL16L32>::decode(&original).unwrap();
        let biases: Vec<i32> = net.biases.iter().copied().map(i32::from).collect();
        let input = encoding_input(&net, &biases);
        let bytes = encode::<F, _>(&input).unwrap();
        assert_eq!(bytes, original);
        let decoded = Network::<F, PairwiseL16L32>::decode(&bytes).unwrap();
        let decoded_biases: Vec<i32> = decoded.biases.iter().copied().map(i32::from).collect();
        let actual = encoding_input(&decoded, &decoded_biases);
        assert_eq!(actual.width, input.width);
        assert_eq!(actual.cp_per_logit.to_bits(), input.cp_per_logit.to_bits());
        assert_eq!(actual.rule_set, input.rule_set);
        assert_eq!(actual.piece_values, input.piece_values);
        assert_eq!(actual.linear, input.linear);
        assert_eq!(actual.weights, input.weights);
        assert_eq!(actual.biases, input.biases);
        assert_eq!(actual.head.to_bytes(), input.head.to_bytes());
        assert_eq!(encode::<F, _>(&actual).unwrap(), bytes);
    }
}

/// MNUE版5のヘッダで定めた幅と勝率尺度（1ロジットあたりのcp）の境界を検査する。
#[test]
fn encode_validates_width_and_cp_per_logit() {
    let net = Network::<P, PairwiseL16L32>::decode(&random_bytes::<P>(4)).unwrap();
    let biases: Vec<i32> = net.biases.iter().copied().map(i32::from).collect();
    let input = encoding_input(&net, &biases);
    for width in [0, 1, 3, 513, 514, u32::MAX] {
        assert_eq!(
            encode::<P, _>(&EncodeInput { width, ..input }).unwrap_err(),
            Error::InvalidWidth { actual: width }
        );
    }
    for cp_per_logit in [
        0.0,
        -0.0,
        -1.0,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        100_001.0,
    ] {
        assert!(matches!(
            encode::<P, _>(&EncodeInput {
                cp_per_logit,
                ..input
            }),
            Err(Error::InvalidCpPerLogit { .. })
        ));
    }
    for cp_per_logit in [f32::from_bits(1), f32::MIN_POSITIVE, 100_000.0] {
        let bytes = encode::<P, _>(&EncodeInput {
            cp_per_logit,
            ..input
        })
        .unwrap();
        assert_eq!(
            Network::<P, PairwiseL16L32>::decode(&bytes)
                .unwrap()
                .cp_per_logit()
                .to_bits(),
            cp_per_logit.to_bits()
        );
    }
    let weights = vec![0; FEATURE_COUNT * 512];
    let bytes = encode::<P, _>(&EncodeInput {
        width: 512,
        weights: &weights,
        biases: &[0; 512],
        head: &HeadParameters::zero(512).build(true).unwrap(),
        ..input
    })
    .unwrap();
    assert_eq!(
        Network::<P, PairwiseL16L32>::decode(&bytes)
            .unwrap()
            .width(),
        512
    );
}

both_feature_sets!(encode_rejects_invalid_parameter_lengths);
fn encode_rejects_invalid_parameter_lengths<F: FeatureSet>() {
    let net = Network::<F, PairwiseL16L32>::decode(&random_bytes::<F>(4)).unwrap();
    let biases: Vec<i32> = net.biases.iter().copied().map(i32::from).collect();
    let input = encoding_input(&net, &biases);
    let extra_linear = vec![[0; 2]; input.linear.len() + 1];
    let extra_weights = vec![0; input.weights.len() + 1];
    let extra_biases = vec![0; input.biases.len() + 1];
    for length in [0, input.linear.len() - 1, input.linear.len() + 1] {
        assert!(matches!(
            encode::<F, _>(&EncodeInput {
                linear: &extra_linear[..length],
                ..input
            }),
            Err(Error::InvalidParameterLength {
                parameter: "linear",
                ..
            })
        ));
    }
    for length in [0, input.weights.len() - 1, input.weights.len() + 1] {
        assert!(matches!(
            encode::<F, _>(&EncodeInput {
                weights: &extra_weights[..length],
                ..input
            }),
            Err(Error::InvalidParameterLength {
                parameter: "weights",
                ..
            })
        ));
    }
    for length in [0, input.biases.len() - 1, input.biases.len() + 1] {
        assert!(matches!(
            encode::<F, _>(&EncodeInput {
                biases: &extra_biases[..length],
                ..input
            }),
            Err(Error::InvalidParameterLength {
                parameter: "biases",
                ..
            })
        ));
    }
}

both_feature_sets!(encode_validates_parameter_ranges);
fn encode_validates_parameter_ranges<F: FeatureSet>() {
    let net = Network::<F, PairwiseL16L32>::decode(&random_bytes::<F>(4)).unwrap();
    let biases: Vec<i32> = net.biases.iter().copied().map(i32::from).collect();
    let input = encoding_input(&net, &biases);
    let mut weights = input.weights.to_vec();
    for value in [-221, 221, i16::MIN, i16::MAX] {
        weights[0] = value;
        assert_eq!(
            encode::<F, _>(&EncodeInput {
                weights: &weights,
                ..input
            })
            .unwrap_err(),
            Error::ParameterOutOfRange {
                parameter: "weights",
                index: 0,
                value: i32::from(value)
            }
        );
    }
    let mut biases = input.biases.to_vec();
    for value in [-511, 511, i32::MIN, i32::MAX] {
        biases[0] = value;
        assert_eq!(
            encode::<F, _>(&EncodeInput {
                biases: &biases,
                ..input
            })
            .unwrap_err(),
            Error::ParameterOutOfRange {
                parameter: "biases",
                index: 0,
                value
            }
        );
    }
    // 整数推論の仕様は端点を含み、線形重みにはi16の全範囲を許す。
    let linear = vec![[i16::MIN, i16::MAX]; FEATURE_COUNT];
    for sign in [-1, 1] {
        weights.fill(sign * 220);
        biases.fill(i32::from(sign) * 510);
        let mut parameters = HeadParameters::zero(4);
        parameters.output_weights.fill(sign * 8192);
        parameters.output_bias = i32::from(sign) * 4_161_536;
        let bytes = encode::<F, _>(&EncodeInput {
            linear: &linear,
            weights: &weights,
            biases: &biases,
            head: &parameters.build(F::MIRROR == Mirror::Shared).unwrap(),
            ..input
        })
        .unwrap();
        assert!(Network::<F, PairwiseL16L32>::decode(&bytes).is_ok());
    }
}

both_feature_sets!(encode_validates_mirror_sharing);
fn encode_validates_mirror_sharing<F: FeatureSet>() {
    let net = Network::<F, PairwiseL16L32>::decode(&random_bytes::<F>(4)).unwrap();
    let biases: Vec<i32> = net.biases.iter().copied().map(i32::from).collect();
    let input = encoding_input(&net, &biases);
    let mut linear = input.linear.to_vec();
    linear[0][0] ^= 1;
    assert!(matches!(
        encode::<F, _>(&EncodeInput {
            linear: &linear,
            ..input
        }),
        Err(Error::MirrorMismatch {
            parameter: "linear",
            ..
        })
    ));
    let mut weights = input.weights.to_vec();
    weights[0] ^= 1;
    let mut biases = input.biases.to_vec();
    biases[0] ^= 1;
    let mut parameters = HeadParameters::zero(4);
    parameters.output_weights[0] = 1;
    let us = parameters.build(false).unwrap();
    parameters.output_weights[0] = 0;
    parameters.output_weights[2] = 1;
    let them = parameters.build(false).unwrap();
    for (parameter, altered) in [
        (
            "weights",
            EncodeInput {
                weights: &weights,
                ..input
            },
        ),
        (
            "biases",
            EncodeInput {
                biases: &biases,
                ..input
            },
        ),
        ("output_weights", EncodeInput { head: &us, ..input }),
        (
            "output_weights",
            EncodeInput {
                head: &them,
                ..input
            },
        ),
    ] {
        let result = encode::<F, _>(&altered);
        if F::MIRROR == Mirror::Shared {
            assert!(
                matches!(result, Err(Error::MirrorMismatch { parameter: found, .. }) if found == parameter)
            );
        } else {
            assert!(Network::<F, PairwiseL16L32>::decode(&result.unwrap()).is_ok());
        }
    }
}

/// 規則セット名の上限は文字数ではなくUTF-8のバイト数である。
#[test]
fn encode_validates_rule_set_names() {
    let net = Network::<P, PairwiseL16L32>::decode(&random_bytes::<P>(4)).unwrap();
    let biases: Vec<i32> = net.biases.iter().copied().map(i32::from).collect();
    let input = encoding_input(&net, &biases);
    for rule_set in [
        String::new(),
        "x".repeat(32),
        format!("{}ab", "あ".repeat(10)),
    ] {
        let bytes = encode::<P, _>(&EncodeInput {
            rule_set: &rule_set,
            ..input
        })
        .unwrap();
        assert_eq!(
            Network::<P, PairwiseL16L32>::decode(&bytes)
                .unwrap()
                .rule_set(),
            rule_set
        );
    }
    for rule_set in [
        "x".repeat(33),
        "あ".repeat(11),
        "a\0b".to_owned(),
        "a\0".to_owned(),
    ] {
        assert_eq!(
            encode::<P, _>(&EncodeInput {
                rule_set: &rule_set,
                ..input
            })
            .unwrap_err(),
            Error::InvalidRuleSet
        );
    }
}

/// 公開されたFeatureSetの名前と特徴数も、外部から与えられる値として検査する。
#[test]
fn encode_validates_feature_set_metadata() {
    macro_rules! format_feature_set {
        ($set:ident, $name:expr, $dimensions:expr) => {
            #[derive(Clone, PartialEq, Eq, Debug)]
            struct $set;
            impl FeatureSet for $set {
                const NAME: &'static str = $name;
                const DIMENSIONS: usize = $dimensions;
                const MIRROR: Mirror = Mirror::Shared;
                type Context = ();

                fn context(_: &Position, _: Color) {}
                fn context_after_move(
                    _: (),
                    _: &Position,
                    _: Color,
                    _: &minase_core::position::Undo,
                ) {
                }
                fn index((): (), base: usize) -> usize {
                    base
                }
                fn base_index(feature: usize) -> usize {
                    feature
                }
            }
        };
    }
    // 幅4の構造記述文字列は、名前以外で52バイトを使う。
    format_feature_set!(
        FullName,
        "12345678901234567890123456789012345678901234",
        FEATURE_COUNT
    );
    format_feature_set!(
        LongName,
        "123456789012345678901234567890123456789012345",
        FEATURE_COUNT
    );
    format_feature_set!(NonAsciiName, "あ", FEATURE_COUNT);
    format_feature_set!(NulName, "P\0", FEATURE_COUNT);
    format_feature_set!(OversizedDimensions, "large", usize::MAX);
    format_feature_set!(ShortDimensions, "small", 0);

    let net = Network::<P, PairwiseL16L32>::decode(&random_bytes::<P>(4)).unwrap();
    let biases: Vec<i32> = net.biases.iter().copied().map(i32::from).collect();
    let input = encoding_input(&net, &biases);
    let bytes = encode::<FullName, _>(&input).unwrap();
    assert_eq!(architecture(&bytes).unwrap().len(), 96);
    assert!(Network::<FullName, PairwiseL16L32>::decode(&bytes).is_ok());
    for result in [
        encode::<LongName, _>(&input),
        encode::<NonAsciiName, _>(&input),
        encode::<NulName, _>(&input),
    ] {
        assert_eq!(result.unwrap_err(), Error::InvalidArchitecture);
    }
    assert_eq!(
        encode::<OversizedDimensions, _>(&input).unwrap_err(),
        Error::InvalidDimensions { actual: usize::MAX }
    );
    assert_eq!(
        encode::<ShortDimensions, _>(&EncodeInput {
            weights: &[],
            ..input
        })
        .unwrap_err(),
        Error::InvalidDimensions { actual: 0 }
    );
}

/// 探索用駒価値も復号と同じ契約で検査する。
#[test]
fn encode_rejects_invalid_piece_values() {
    let net = Network::<P, PairwiseL16L32>::decode(&random_bytes::<P>(4)).unwrap();
    let biases: Vec<i32> = net.biases.iter().copied().map(i32::from).collect();
    let input = encoding_input(&net, &biases);
    for value in [-1, 29_000, i32::MAX] {
        let mut piece_values = *input.piece_values;
        piece_values[0] = value;
        assert!(matches!(
            encode::<P, _>(&EncodeInput {
                piece_values: &piece_values,
                ..input
            }),
            Err(Error::PieceValueOutOfRange { .. })
        ));
    }
    for kind in [PieceKind::King, PieceKind::CrownPrince] {
        let mut piece_values = *input.piece_values;
        piece_values[kind.index()] = 0;
        assert!(matches!(
            encode::<P, _>(&EncodeInput {
                piece_values: &piece_values,
                ..input
            }),
            Err(Error::InconsistentRoyalValue { .. })
        ));
    }
    let mut piece_values = *input.piece_values;
    piece_values[29] = 0;
    assert!(matches!(
        encode::<P, _>(&EncodeInput {
            piece_values: &piece_values,
            ..input
        }),
        Err(Error::NonPositivePieceValue { .. })
    ));
}

both_feature_sets!(header_validation_and_runtime_lengths);
fn header_validation_and_runtime_lengths<F: FeatureSet>() {
    let a = random_bytes::<F>(4);
    let b = random_bytes::<F>(8);
    // 幅の増分4ごとに、第1層2D、バイアス4、oの2、W1の15×2バイトが増える。
    assert_eq!(b.len() - a.len(), 4 * (F::DIMENSIONS * 2 + 4 + 2 + 15 * 2));
    for width in [4, 8, 512] {
        let bytes = random_bytes::<F>(width);
        assert_eq!(
            Network::<F, PairwiseL16L32>::decode(&bytes)
                .unwrap()
                .width(),
            width
        );
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(matches!(
            Network::<F, PairwiseL16L32>::decode(&extra),
            Err(Error::InvalidLength { .. })
        ));
        assert!(matches!(
            Network::<F, PairwiseL16L32>::decode(&bytes[..bytes.len() - 1]),
            Err(Error::InvalidLength { .. })
        ));
        let mut bad = bytes.clone();
        bad[HEADER_LENGTH] ^= 1;
        assert!(matches!(
            Network::<F, PairwiseL16L32>::decode(&bad),
            Err(Error::ChecksumMismatch)
        ));
    }
    for length in 0..HEADER_LENGTH {
        assert!(matches!(
            Network::<F, PairwiseL16L32>::decode(&a[..length]),
            Err(Error::InvalidLength { .. })
        ));
    }
    for version in [0, 1, 2, 3, 4, 6, u32::MAX] {
        let mut bad = a.clone();
        bad[4..8].copy_from_slice(&version.to_le_bytes());
        assert!(matches!(
            Network::<F, PairwiseL16L32>::decode(&bad),
            Err(Error::UnsupportedVersion { .. })
        ));
    }
    for width in [0, 1, 3, 513, 514, u32::MAX] {
        let mut bad = a.clone();
        bad[104..108].copy_from_slice(&width.to_le_bytes());
        assert!(matches!(
            Network::<F, PairwiseL16L32>::decode(&bad),
            Err(Error::InvalidWidth { .. })
        ));
    }
    for cp_per_logit in [
        0.0,
        -1.0,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        100_001.0,
    ] {
        let mut bad = a.clone();
        bad[108..112].copy_from_slice(&cp_per_logit.to_le_bytes());
        assert!(matches!(
            Network::<F, PairwiseL16L32>::decode(&bad),
            Err(Error::InvalidCpPerLogit { .. })
        ));
    }
    for cp_per_logit in [f32::MIN_POSITIVE, 100_000.0] {
        let mut bytes = a.clone();
        bytes[108..112].copy_from_slice(&cp_per_logit.to_le_bytes());
        assert!(Network::<F, PairwiseL16L32>::decode(&bytes).is_ok());
    }
    let mut bad = a.clone();
    bad[0] = 0;
    assert!(matches!(
        Network::<F, PairwiseL16L32>::decode(&bad),
        Err(Error::InvalidMagic { .. })
    ));
    bad = a.clone();
    bad[8] = b'X';
    assert!(matches!(
        Network::<F, PairwiseL16L32>::decode(&bad),
        Err(Error::UnexpectedArchitecture { .. })
    ));
    for offset in 176..192 {
        bad = a.clone();
        bad[offset] = 1;
        assert!(matches!(
            Network::<F, PairwiseL16L32>::decode(&bad),
            Err(Error::NonZeroReserved)
        ));
    }
    for (offset, value) in [(112, 255), (143, 65)] {
        bad = a.clone();
        bad[offset] = value;
        assert!(matches!(
            Network::<F, PairwiseL16L32>::decode(&bad),
            Err(Error::InvalidRuleSet)
        ));
    }
}

both_feature_sets!(parameter_ranges_and_mirror_sharing_are_validated);
fn parameter_ranges_and_mirror_sharing_are_validated<F: FeatureSet>() {
    let width = 4;
    let bytes = random_bytes::<F>(width);
    let hidden = HEADER_LENGTH + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4;
    let bias = hidden + F::DIMENSIONS * width * 2;
    let output = bias + width * 4;
    for (offset, values) in [
        (hidden, vec![-221_i16, 221, i16::MIN, i16::MAX]),
        (output, vec![-8193, 8193, i16::MIN]),
    ] {
        for value in values {
            let mut bad = bytes.clone();
            bad[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
            refresh_checksum(&mut bad);
            assert!(matches!(
                Network::<F, PairwiseL16L32>::decode(&bad),
                Err(Error::ParameterOutOfRange { .. })
            ));
        }
    }
    for (offset, limit) in [(bias, 510_i32), (output + width * 2, 4_161_536)] {
        for value in [-limit - 1, limit + 1, i32::MIN, i32::MAX] {
            let mut bad = bytes.clone();
            bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            refresh_checksum(&mut bad);
            assert!(matches!(
                Network::<F, PairwiseL16L32>::decode(&bad),
                Err(Error::ParameterOutOfRange { .. })
            ));
        }
    }
    for (offset, parameter) in [
        (HEADER_LENGTH, "linear"),
        (hidden, "weights"),
        (bias, "biases"),
        (output, "output_weights"),
        (output + width, "output_weights"),
    ] {
        let mut bad = bytes.clone();
        bad[offset] ^= 1;
        refresh_checksum(&mut bad);
        if parameter == "linear" || F::MIRROR == Mirror::Shared {
            assert!(
                matches!(Network::<F, PairwiseL16L32>::decode(&bad), Err(Error::MirrorMismatch { parameter: found, .. }) if found == parameter)
            );
        } else {
            assert!(Network::<F, PairwiseL16L32>::decode(&bad).is_ok());
        }
    }
    for value in [-1_i32, 29_000, i32::MAX] {
        let mut bad = bytes.clone();
        let offset = HEADER_LENGTH + FEATURE_COUNT * 4;
        bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        refresh_checksum(&mut bad);
        assert!(matches!(
            Network::<F, PairwiseL16L32>::decode(&bad),
            Err(Error::PieceValueOutOfRange { .. })
        ));
    }
    for kind in [PieceKind::King, PieceKind::CrownPrince] {
        let mut bad = bytes.clone();
        let offset = HEADER_LENGTH + FEATURE_COUNT * 4 + kind.index() * 4;
        bad[offset..offset + 4].copy_from_slice(&0_i32.to_le_bytes());
        refresh_checksum(&mut bad);
        assert!(matches!(
            Network::<F, PairwiseL16L32>::decode(&bad),
            Err(Error::InconsistentRoyalValue { .. })
        ));
    }
    let mut bad = bytes.clone();
    let offset = HEADER_LENGTH + FEATURE_COUNT * 4 + 29 * 4;
    bad[offset..offset + 4].copy_from_slice(&0_i32.to_le_bytes());
    refresh_checksum(&mut bad);
    assert!(matches!(
        Network::<F, PairwiseL16L32>::decode(&bad),
        Err(Error::NonPositivePieceValue { .. })
    ));
}

/// 数式の境界を手計算できる、一定の非線形入力を持つネット。
fn constant_network<F: FeatureSet>(
    width: usize,
    activation_input: i32,
    output_weight: i16,
    bias: i32,
    cp_per_logit: f32,
    linear: i16,
) -> Network<F, PairwiseL16L32> {
    let mut bytes = random_bytes::<F>(width);
    bytes[108..112].copy_from_slice(&cp_per_logit.to_le_bytes());
    for pair in bytes[HEADER_LENGTH..HEADER_LENGTH + FEATURE_COUNT * 4]
        .as_chunks_mut::<2>()
        .0
    {
        pair.copy_from_slice(&linear.to_le_bytes());
    }
    let hidden = HEADER_LENGTH + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4;
    let biases = hidden + F::DIMENSIONS * width * 2;
    let output = biases + width * 4;
    bytes[hidden..biases].fill(0);
    for chunk in bytes[biases..output].as_chunks_mut::<4>().0 {
        chunk.copy_from_slice(&activation_input.to_le_bytes());
    }
    bytes.truncate(output);
    let mut parameters = HeadParameters::zero(width);
    parameters.output_weights.fill(output_weight);
    parameters.output_bias = bias;
    bytes.extend(parameters.bytes());
    refresh_checksum(&mut bytes);
    Network::<F, PairwiseL16L32>::decode(&bytes).unwrap()
}

both_feature_sets!(integer_activation_and_signed_rounding_follow_the_specification);
fn integer_activation_and_signed_rounding_follow_the_specification<F: FeatureSet>() {
    let position = Position::initial();
    // cp_per_logit=127/16によりscale=1。出力重み4096を4成分足すとa/4 cp。
    for (input, expected) in [
        (i16::MIN, 0),
        (0, 0),
        (1, 0),
        (127, 8),
        (128, 8),
        (254, 32),
        (255, 32),
        (i16::MAX, 32),
    ] {
        let net = constant_network::<F>(4, 0, 4096, 0, 7.9375, 0);
        let mut accumulator = net.refresh_accumulator(&position);
        for hidden in &mut accumulator.hidden {
            hidden.fill(input);
        }
        assert_eq!(
            net.evaluate_accumulator(&accumulator, Color::Black),
            expected
        );
    }
    for (bias, expected) in [(32767, 0), (32768, 1), (-32768, 0), (-32769, -1)] {
        let net = constant_network::<F>(4, 0, 0, bias, 7.9375, 0);
        assert_eq!(evaluate(&net, &position), expected);
    }
    // 0.5と1.5を偶数側に丸め、ヘッダのf32を先に倍精度へ広げる。
    for (cp_per_logit, expected_scale) in [(3.96875, 0), (11.90625, 2)] {
        let net = constant_network::<F>(4, 0, 0, 65536, cp_per_logit, 0);
        assert_eq!(net.scale, expected_scale);
        assert_eq!(evaluate(&net, &position), expected_scale as i32);
    }
}

both_feature_sets!(final_sum_is_clamped_once_and_linear_division_truncates_toward_zero);
fn final_sum_is_clamped_once_and_linear_division_truncates_toward_zero<F: FeatureSet>() {
    let pieces: Vec<_> = minase_core::Square::all()
        .take(10)
        .enumerate()
        .map(|(i, square)| {
            let (side, kind) = match i {
                0 => (Color::Black, PieceKind::King),
                1 => (Color::White, PieceKind::King),
                _ => (Color::Black, PieceKind::Pawn),
            };
            (square, PieceCode::new(side, kind).unwrap())
        })
        .collect();
    let position = position_from_codes(Color::Black, &pieces);
    // 線形は40958cp、非線形は-16128cp。線形を先に制限すると一致しない。
    let net = constant_network::<F>(512, 254, -8192, 0, 15.875, 32767);
    assert_eq!(evaluate(&net, &position), 24_830);
    let net = constant_network::<F>(512, 254, 8192, 0, 15.875, -32768);
    assert_eq!(evaluate(&net, &position), -24_832);
    for (linear, expected) in [(32767, 28_999), (-32768, -28_999), (-1, -1), (1, 1)] {
        let net = constant_network::<F>(4, 0, 0, 0, 7.9375, linear);
        assert_eq!(evaluate(&net, &position), expected);
    }
}

#[cfg(feature = "invariants")]
#[test]
fn accumulator_invariant_reports_a_mismatch() {
    let net = Nnue::decode(EPOCH_ZERO).unwrap();
    let position = Position::initial();
    let mut accumulator = net.refresh_accumulator(&position);
    net.assert_accumulator(&position, &accumulator, 7);
    accumulator.sums[0][0] += 1;
    assert!(
        std::panic::catch_unwind(|| net.assert_accumulator(&position, &accumulator, 7)).is_err()
    );
}

/// 手番による出力行の交換と先獅子の保持を、独立した6cp/8cpの期待値で固定する。
#[test]
fn diagnostic_views_select_both_output_rows_without_removing_lion() {
    use crate::eval::features::lion_feature_index;
    let mut net = constant_network::<P>(4, 0, 0, 0, 7.9375, 0);
    let mut position = position_from_codes(
        Color::Black,
        &[
            (
                sq(0, 11),
                PieceCode::new(Color::Black, PieceKind::King).unwrap(),
            ),
            (
                sq(11, 0),
                PieceCode::new(Color::White, PieceKind::King).unwrap(),
            ),
        ],
    );
    position.set_lion_capture(Some(sq(5, 3))).unwrap();
    // 黒視点の先獅子だけで各ユニット128になり、要素積は128² >> 9 = 32。
    let feature = lion_feature_index(Color::Black, sq(5, 3));
    for f in [feature, mirror_feature(feature)] {
        net.weights[f * 4..f * 4 + 4].fill(128);
    }
    let mut parameters = HeadParameters::zero(4);
    parameters.output_weights = vec![4096, 4096, 8192, 8192];
    net.head = parameters.build(true).unwrap();
    // 黒視点の線形成分は8/16。駒数2なので終盤の16/8=2cp。
    for f in [feature, mirror_feature(feature)] {
        net.linear[f] = [8, 16];
    }
    assert_eq!(evaluate_for_diagnostics(&net, &position, Color::Black), 6);
    assert_eq!(evaluate_for_diagnostics(&net, &position, Color::White), 8);
    assert_eq!(position.lion_taken_by_non_lion().unwrap().square, sq(5, 3));
}

/// 絶対升の番号、基準の王駒の優先順位と例外を仕様の値で固定する。
#[test]
fn half_kp_anchor_selection_and_dense_indices_follow_the_contract() {
    // Squareの密番号は左下を0として、1段12升の順に並ぶ。
    for (square, expected) in [
        (sq(0, 0), 0),
        (sq(11, 0), 11),
        (sq(0, 1), 12),
        (sq(5, 7), 89),
        (sq(0, 11), 132),
        (sq(11, 11), 143),
    ] {
        assert_eq!(square.dense_index(), expected);
    }
    for side in Color::ALL {
        let king = PieceCode::new(side, PieceKind::King).unwrap();
        let prince = PieceCode::new_promoted(side, PieceKind::CrownPrince).unwrap();
        // 後手でも絶対段の小さい王駒が優先される。
        let cases = [
            (
                vec![(sq(10, 0), king), (sq(1, 1), prince)],
                [(1, false), (7, false)],
            ),
            (vec![(sq(1, 1), prince)], [(6, true), (7, true)]),
            (
                vec![(sq(10, 0), king), (sq(1, 1), king)],
                [(1, false), (7, false)],
            ),
            (
                vec![(sq(10, 0), prince), (sq(1, 1), prince)],
                [(1, false), (7, false)],
            ),
            (
                vec![(sq(3, 0), king), (sq(10, 0), king)],
                [(3, true), (7, true)],
            ),
            (vec![], [(0, false), (0, false)]),
        ];
        for (pieces, expected) in cases {
            let position = position_from_codes(side, &pieces);
            let (bucket, mirrored) = expected[side.index()];
            let context = HalfKpHmB8::context(&position, side);
            assert_eq!(context, HalfKpContext { bucket, mirrored });
            assert_eq!(context.bucket(), bucket);
            assert_eq!(context.mirrored(), mirrored);
            let opposite = HalfKpHmB8::context(&position, side.opposite());
            assert_eq!(
                opposite,
                HalfKpContext {
                    bucket: 0,
                    mirrored: false
                }
            );
            assert_eq!(opposite.bucket(), 0);
            assert!(!opposite.mirrored());
        }
    }
    assert_eq!(std::mem::size_of::<<P as FeatureSet>::Context>(), 0);
}

/// 8区分と中央の鏡映境界を、両視点の手作り局面で検査する。
#[test]
fn half_kp_buckets_and_feature_indices_follow_the_contract() {
    let first_rank = [0, 1, 2, 3, 4, 5, 5, 4, 3, 2, 1, 0];
    for side in Color::ALL {
        for (relative_rank, buckets) in [(0, first_rank), (1, [6; 12]), (2, [7; 12]), (11, [7; 12])]
        {
            for (file, bucket) in buckets.into_iter().enumerate() {
                let rank = if side == Color::Black {
                    relative_rank
                } else {
                    11 - relative_rank
                };
                let position = position_from_codes(
                    side,
                    &[(
                        sq(file as u8, rank),
                        PieceCode::new(side, PieceKind::King).unwrap(),
                    )],
                );
                let context = HalfKpHmB8::context(&position, side);
                assert_eq!(
                    context,
                    HalfKpContext {
                        bucket,
                        mirrored: file < 6
                    }
                );
                assert_eq!(context.bucket(), bucket);
                assert_eq!(context.mirrored(), file < 6);
            }
        }
    }
    // 先獅子も同じ鏡映で写す。基底特徴13536は相対升(0,0)。
    for (context, base, expected) in [
        (
            HalfKpContext {
                bucket: 0,
                mirrored: false,
            },
            0,
            0,
        ),
        (
            HalfKpContext {
                bucket: 5,
                mirrored: true,
            },
            0,
            68_411,
        ),
        (
            HalfKpContext {
                bucket: 6,
                mirrored: true,
            },
            13_536,
            95_627,
        ),
        (
            HalfKpContext {
                bucket: 7,
                mirrored: false,
            },
            13_679,
            109_439,
        ),
    ] {
        assert_eq!(HalfKpHmB8::index(context, base), expected);
    }
    for base in 0..13_680 {
        assert_eq!(P::index((), base), base);
        assert_eq!(P::base_index(base), base);
        for bucket in 0..8 {
            assert_eq!(HalfKpHmB8::base_index(bucket * 13_680 + base), base);
        }
    }
}

both_feature_sets!(royal_context_changes_preserve_incremental_evaluation);
fn royal_context_changes_preserve_incremental_evaluation<F: FeatureSet>() {
    for (network, net) in networks::<F>().iter().enumerate() {
        for side in Color::ALL {
            let king = PieceCode::new(side, PieceKind::King).unwrap();
            let enemy_king = PieceCode::new(side.opposite(), PieceKind::King).unwrap();
            let prince = PieceCode::new_promoted(side.opposite(), PieceKind::CrownPrince).unwrap();
            let own_prince = PieceCode::new_promoted(side, PieceKind::CrownPrince).unwrap();
            let rook = PieceCode::new(side, PieceKind::Rook).unwrap();
            let elephant = PieceCode::new(side, PieceKind::DrunkElephant).unwrap();
            let rank = |r| if side == Color::Black { r } else { 11 - r };
            let mut cases = Vec::new();
            for (case, from, to) in [
                ("article_9_king_mirror", sq(5, rank(0)), sq(6, rank(0))),
                ("article_9_king_file_bucket", sq(6, rank(0)), sq(7, rank(0))),
                (
                    "article_9_king_rank_bucket_1",
                    sq(6, rank(0)),
                    sq(6, rank(1)),
                ),
                (
                    "article_9_king_rank_bucket_2",
                    sq(6, rank(1)),
                    sq(6, rank(2)),
                ),
                (
                    "article_9_king_same_context",
                    sq(6, rank(2)),
                    sq(7, rank(3)),
                ),
            ] {
                cases.push((
                    case,
                    vec![(from, king), (sq(11, rank(11)), enemy_king)],
                    Move {
                        from,
                        to,
                        mid: None,
                        promote: false,
                    },
                ));
            }
            let capture = Move {
                from: sq(5, rank(7)),
                to: sq(5, rank(8)),
                mid: None,
                promote: false,
            };
            for (case, victim, survivor) in [
                ("article_20_capture_king_without_prince", enemy_king, None),
                (
                    "article_20_capture_king_with_prince",
                    enemy_king,
                    Some((sq(6, rank(11)), prince)),
                ),
            ] {
                let mut pieces = vec![
                    (sq(0, rank(0)), king),
                    (capture.from, rook),
                    (capture.to, victim),
                ];
                pieces.extend(survivor);
                cases.push((case, pieces, capture));
            }
            cases.push((
                "article_18_elephant_promotion_without_king",
                vec![(capture.from, elephant), (sq(11, rank(11)), enemy_king)],
                Move {
                    promote: true,
                    ..capture
                },
            ));
            // 自王を最上段に置き、敵王または単独の太子を捕獲する。
            for (case, victim) in [
                (
                    "article_20_king_capture_with_own_king_on_top_rank",
                    enemy_king,
                ),
                ("article_20_lone_prince_capture", prince),
            ] {
                cases.push((
                    case,
                    vec![
                        (sq(0, rank(11)), king),
                        (capture.from, rook),
                        (capture.to, victim),
                    ],
                    capture,
                ));
            }
            cases.push((
                "article_18_elephant_promotion_with_king",
                vec![
                    (sq(0, rank(11)), king),
                    (sq(11, rank(0)), enemy_king),
                    (capture.from, elephant),
                ],
                Move {
                    promote: true,
                    ..capture
                },
            ));
            cases.push((
                "article_10_lone_prince_mirror",
                vec![(sq(5, rank(0)), own_prince), (sq(11, rank(11)), enemy_king)],
                Move {
                    from: sq(5, rank(0)),
                    to: sq(6, rank(0)),
                    mid: None,
                    promote: false,
                },
            ));
            for (case, pieces, mv) in cases {
                let mut position = position_from_codes(side, &pieces);
                assert_mirror_when_defined(net, &position);
                let before = net.refresh_accumulator(&position);
                let undo = position.make_move_unchecked(mv, MoveRules::standard());
                let mut after = Accumulator::new(net.width());
                net.update_accumulator_after_move(&before, &mut after, &position, &undo);
                assert_eq!(
                    after,
                    net.refresh_accumulator(&position),
                    "{case}, network={network}, side={side:?}"
                );
                assert_eq!(
                    net.evaluate_accumulator(&after, position.side_to_move()),
                    evaluate(net, &position),
                    "{case}, network={network}, side={side:?}"
                );
                assert_mirror_when_defined(net, &position);
                position.unmake_move(undo);
                assert_eq!(
                    before,
                    net.refresh_accumulator(&position),
                    "{case}, network={network}, side={side:?}"
                );
            }
        }
    }
}

both_feature_sets!(accumulator_counts_include_roots_moves_and_nulls);
/// 根と文脈変更を再計算、null moveを差分更新として数える。
fn accumulator_counts_include_roots_moves_and_nulls<F: FeatureSet>() {
    let net = Network::<F, PairwiseL16L32>::decode(&random_bytes::<F>(4)).unwrap();
    let king = PieceCode::new(Color::Black, PieceKind::King).unwrap();
    let enemy = PieceCode::new(Color::White, PieceKind::King).unwrap();
    // 通常の王移動と、両視点の文脈を同時に変える王の捕獲を検査する。
    for enemy_square in [sq(11, 11), sq(7, 0)] {
        let mut position =
            position_from_codes(Color::Black, &[(sq(6, 0), king), (enemy_square, enemy)]);
        let before = net.refresh_accumulator(&position);
        #[cfg(feature = "invariants")]
        net.assert_accumulator(&position, &before, 0);
        let undo = position.make_move_unchecked(
            Move {
                from: sq(6, 0),
                to: sq(7, 0),
                mid: None,
                promote: false,
            },
            MoveRules::standard(),
        );
        let mut after = Accumulator::new(net.width());
        net.update_accumulator_after_move(&before, &mut after, &position, &undo);
        #[cfg(feature = "invariants")]
        net.assert_accumulator(&position, &after, 1);
        let mut after_null = Accumulator::new(net.width());
        net.update_accumulator_after_null(&after, &mut after_null, None);
        let counts = net.take_accumulator_stats();
        assert_eq!(counts.updates(), 3);
        assert_eq!(counts.refreshes, if F::NAME == "P" { 1 } else { 2 });
        assert_eq!(net.take_accumulator_stats(), AccumulatorStats::default());
    }
}

/// 区分へのP型の行の複写は、鏡映のユニット交換も含めて整数出力を保存する。
#[test]
fn copied_p_rows_produce_identical_half_kp_evaluations() {
    let bytes = random_bytes::<P>(8);
    let p = Network::<P, PairwiseL16L32>::decode(&bytes).unwrap();
    let half_kp =
        Network::<HalfKpHmB8, PairwiseL16L32>::decode(&copy_p_rows::<HalfKpHmB8>(&bytes)).unwrap();
    let mut state = 917;
    for rank in 0..12 {
        for file in 0..12 {
            let mut pieces = vec![
                (
                    sq(file, rank),
                    PieceCode::new(Color::Black, PieceKind::King).unwrap(),
                ),
                (
                    sq(11 - file, 11 - rank),
                    PieceCode::new(Color::White, PieceKind::King).unwrap(),
                ),
            ];
            for square in minase_core::Square::all() {
                if pieces.iter().any(|&(s, _)| s == square) || next(&mut state).is_multiple_of(3) {
                    continue;
                }
                let side = Color::ALL[next(&mut state) as usize % 2];
                let kind = [PieceKind::Pawn, PieceKind::Rook, PieceKind::Lion]
                    [next(&mut state) as usize % 3];
                pieces.push((square, PieceCode::new(side, kind).unwrap()));
            }
            let mut position = position_from_codes(Color::Black, &pieces);
            position
                .set_lion_capture(Some(sq(11 - file, 11 - rank)))
                .unwrap();
            for side in Color::ALL {
                assert_eq!(
                    evaluate_for_diagnostics(&p, &position, side),
                    evaluate_for_diagnostics(&half_kp, &position, side)
                );
            }
        }
    }
}

both_feature_sets!(architecture_must_match_type_and_header_width);
fn architecture_must_match_type_and_header_width<F: FeatureSet>() {
    let bytes = random_bytes::<F>(4);
    for text in [
        format!(
            "features={};ft=2;linear=tapered2;head=pairwise-l16-l32",
            F::NAME
        ),
        format!("features={};ft=4;linear=tapered2;head=unknown", F::NAME),
        format!(
            "features={};ft=4;linear=unknown;head=pairwise-l16-l32",
            F::NAME
        ),
        "features=unknown;ft=4;linear=tapered2;head=pairwise-l16-l32".to_owned(),
        String::new(),
    ] {
        let mut bad = bytes.clone();
        bad[8..104].fill(0);
        bad[8..8 + text.len()].copy_from_slice(text.as_bytes());
        assert!(matches!(
            Network::<F, PairwiseL16L32>::decode(&bad),
            Err(Error::UnexpectedArchitecture { .. })
        ));
    }
    let mut bad = bytes.clone();
    bad[104..108].copy_from_slice(&2_u32.to_le_bytes());
    assert!(matches!(
        Network::<F, PairwiseL16L32>::decode(&bad),
        Err(Error::UnexpectedArchitecture { .. })
    ));
    for offset in [8, 103] {
        let mut bad = bytes.clone();
        bad[offset] = 255;
        assert!(matches!(
            Network::<F, PairwiseL16L32>::decode(&bad),
            Err(Error::InvalidArchitecture)
        ));
    }
    let mut bad = bytes.clone();
    bad[103] = b'x';
    assert!(matches!(
        Network::<F, PairwiseL16L32>::decode(&bad),
        Err(Error::InvalidArchitecture)
    ));
    // 他方の型を明示して復号しても受理しない。
    if F::NAME == "P" {
        assert!(matches!(
            Network::<HalfKpHmB8, PairwiseL16L32>::decode(&bytes),
            Err(Error::UnexpectedArchitecture { .. })
        ));
    } else {
        assert!(matches!(
            Network::<P, PairwiseL16L32>::decode(&bytes),
            Err(Error::UnexpectedArchitecture { .. })
        ));
    }
}

#[cfg(feature = "invariants")]
#[test]
fn accumulator_invariant_includes_contexts() {
    let net =
        Network::<HalfKpHmB8, PairwiseL16L32>::decode(&random_bytes::<HalfKpHmB8>(4)).unwrap();
    let position = Position::initial();
    let mut accumulator = net.refresh_accumulator(&position);
    net.assert_accumulator(&position, &accumulator, 0);
    accumulator.contexts[0].mirrored = !accumulator.contexts[0].mirrored;
    assert!(
        std::panic::catch_unwind(|| net.assert_accumulator(&position, &accumulator, 0)).is_err()
    );
}

both_feature_sets!(encode_rechecks_head_width_and_feature_sharing);
fn encode_rechecks_head_width_and_feature_sharing<F: FeatureSet>() {
    let net = Network::<F, PairwiseL16L32>::decode(&random_bytes::<F>(4)).unwrap();
    let wrong_width = Network::<F, PairwiseL16L32>::decode(&random_bytes::<F>(8)).unwrap();
    assert!(matches!(
        encode::<F, _>(&EncodeInput {
            head: &wrong_width.head,
            ..encoding_input(
                &net,
                &net.biases
                    .iter()
                    .copied()
                    .map(i32::from)
                    .collect::<Vec<_>>()
            )
        }),
        Err(Error::InvalidLength { .. })
    ));
    // 正規化型で作った出力部でも、共有型への符号化時には再検査する。
    let mut bytes = net.head.to_bytes();
    bytes[..2].copy_from_slice(&1_i16.to_le_bytes());
    bytes[2..4].copy_from_slice(&0_i16.to_le_bytes());
    let head = PairwiseL16L32::decode(&bytes, 4, false).unwrap();
    let encoded = encode::<F, _>(&EncodeInput {
        head: &head,
        ..encoding_input(
            &net,
            &net.biases
                .iter()
                .copied()
                .map(i32::from)
                .collect::<Vec<_>>(),
        )
    });
    if F::MIRROR == Mirror::Shared {
        assert!(matches!(
            encoded,
            Err(Error::MirrorMismatch {
                parameter: "output_weights",
                ..
            })
        ));
    } else {
        assert!(encoded.is_ok());
    }
}

#[test]
fn pairwise_output_matches_independent_integer_reference() {
    let mut state = 0x9e37_79b9;
    let mut saw_negative_square = false;
    let mut saw_square_saturation = false;
    let mut saw_y_zero = false;
    let mut saw_y_saturation = false;
    for width in [4, 8, 12, 64, 256, 260, 512] {
        for shared in [false, true] {
            for _ in 0..8 {
                let p = HeadParameters::random(width, &mut state, shared);
                let head = p.build(shared).unwrap();
                let decoded = PairwiseL16L32::decode(&p.bytes(), width, shared).unwrap();
                assert_eq!(head.to_bytes(), p.bytes());
                assert_eq!(decoded.to_bytes(), p.bytes());
                for case in 0..16 {
                    let mut inputs = [vec![0; width], vec![0; width]];
                    for acc in &mut inputs {
                        for value in acc {
                            *value = match case {
                                0 => i16::MIN,
                                1 => 0,
                                2 => 255,
                                3 => i16::MAX,
                                _ => (next(&mut state) % 421) as i16 - 80,
                            };
                        }
                    }
                    let [us, them] = &inputs;
                    let expected = reference_head(&p, us, them, 135);
                    saw_negative_square |= (0..15).any(|k| expected.z[k] < 0 && expected.u[k] > 0);
                    saw_square_saturation |= expected.u[..15].contains(&127);
                    saw_y_zero |= expected.y.contains(&0);
                    saw_y_saturation |= expected.y.contains(&127);
                    assert_eq!(
                        i64::from(head.output(us, them)),
                        expected.out,
                        "new: H={width}, shared_mirror={shared}, case={case}"
                    );
                    assert_eq!(
                        i64::from(decoded.output(us, them)),
                        expected.out,
                        "decode: H={width}, shared_mirror={shared}, case={case}"
                    );
                }
            }
        }
    }
    assert!(saw_negative_square && saw_square_saturation && saw_y_zero && saw_y_saturation);
}

#[test]
fn pairwise_clamping_shift_and_perspective_order_follow_specification() {
    let us = [255, 64, 255, 128, 192, 128, 255, 64];
    let them = [-1, 0, 255, 255, 256, i16::MAX, 255, 255];
    // 255² / 512 = 127。各視点の第1・第2四分区画と、第3・第4四分区画を掛ける。
    let expected = [127, 16, 95, 16, 0, 0, 127, 127];
    for (i, &expected) in expected.iter().enumerate() {
        let mut p = HeadParameters::zero(8);
        p.output_weights[i] = 1;
        let head = p.build(false).unwrap();
        assert_eq!(head.output(&us, &them), expected);
        assert_eq!(reference_head(&p, &us, &them, 1).s[i], i64::from(expected));
    }
}

#[test]
fn dual_activation_squares_negative_inputs_and_preserves_32512_boundary() {
    // W2を尺度256の単位行にすると、最終出力からuの各成分を個別に読める。
    // 2乗の枝は2^23で割る。2,896² < 2^23 ≤ 2,897²、32,512² / 2^23 = 126.0078125。
    for (z, squared, clipped) in [
        (-260_096, 127, 0),
        (-32768, 127, 0),
        (-32512, 126, 0),
        (-4096, 2, 0),
        (-2897, 1, 0),
        (-2896, 0, 0),
        (-1, 0, 0),
        (0, 0, 0),
        (1, 0, 0),
        (255, 0, 0),
        (256, 0, 1),
        (2896, 0, 11),
        (2897, 1, 11),
        (32510, 125, 126),
        (32511, 126, 126),
        (32512, 126, 127),
        (32639, 126, 127),
        (32640, 127, 127),
        (32768, 127, 127),
        (260_096, 127, 127),
    ] {
        let mut p = HeadParameters::zero(4);
        p.hidden1_biases[0] = z;
        p.hidden2_weights[0] = 256;
        p.hidden2_weights[30 + 15] = 256;
        let reference = reference_head(&p, &[0; 4], &[0; 4], 1);
        assert_eq!(reference.u[0], squared);
        assert_eq!(reference.u[15], clipped);
        for (k, expected) in [(0, squared), (1, clipped)] {
            p.hidden2_output.fill(0);
            p.hidden2_output[k] = 1;
            assert_eq!(
                i64::from(p.build(true).unwrap().output(&[0; 4], &[0; 4])),
                expected,
                "z={z}, row={k}"
            );
        }
    }
}

#[test]
fn second_hidden_layer_shift_and_clamping_follow_specification() {
    for (bias, expected) in [
        (-260_096, 0),
        (-257, 0),
        (-1, 0),
        (0, 0),
        (255, 0),
        (256, 1),
        (32511, 126),
        (32512, 127),
        (32768, 127),
        (260_096, 127),
    ] {
        let mut p = HeadParameters::zero(4);
        p.hidden2_biases[31] = bias;
        p.hidden2_output[31] = 1;
        assert_eq!(p.build(true).unwrap().output(&[0; 4], &[0; 4]), expected);
    }
}

#[test]
fn pairwise_output_fits_i32_at_width_512_parameter_limits() {
    let mut p = HeadParameters::zero(512);
    p.output_weights.fill(8192);
    p.output_bias = 4_161_536;
    p.hidden1_weights.fill(508);
    p.hidden1_biases.fill(260_096);
    p.hidden2_weights.fill(508);
    p.hidden2_biases.fill(260_096);
    p.hidden2_output.fill(8192);
    let expected = reference_head(&p, &[i16::MAX; 512], &[i16::MAX; 512], 135);
    // z = 512×127×508+260,096、out = (512+32)×127×8,192+4,161,536。
    assert_eq!(expected.z, [33_292_288; 15]);
    assert_eq!(expected.out, 570_130_432);
    assert!(i32::try_from(expected.z[0]).is_ok());
    assert!(i32::try_from(expected.out).is_ok());
    for shared in [false, true] {
        assert_eq!(
            p.build(shared)
                .unwrap()
                .output(&[i16::MAX; 512], &[i16::MAX; 512]),
            570_130_432
        );
    }
    p.output_weights.fill(-8192);
    p.output_bias = -4_161_536;
    p.hidden2_output.fill(-8192);
    for shared in [false, true] {
        assert_eq!(
            p.build(shared).unwrap().output(&[255; 512], &[255; 512]),
            -570_130_432
        );
    }
    p.hidden1_weights.fill(-508);
    p.hidden1_biases.fill(-260_096);
    p.hidden2_weights.fill(-508);
    p.hidden2_biases.fill(-260_096);
    let expected = reference_head(&p, &[255; 512], &[255; 512], 135);
    assert_eq!(expected.z, [-33_292_288; 15]);
    assert!(i32::try_from(expected.z[0]).is_ok());
    assert!(i32::try_from(expected.out).is_ok());
    for shared in [false, true] {
        assert_eq!(
            i64::from(p.build(shared).unwrap().output(&[255; 512], &[255; 512])),
            expected.out
        );
    }
}

both_feature_sets!(nonlinear_conversion_matches_reference_without_piece_count_phase);
fn nonlinear_conversion_matches_reference_without_piece_count_phase<F: FeatureSet>() {
    let mut state = 0xa179_10d2;
    let p = HeadParameters::random(8, &mut state, F::MIRROR == Mirror::Shared);
    let mut net = constant_network::<F>(8, 0, 0, 0, 7.9375, 0);
    net.head = p.build(F::MIRROR == Mirror::Shared).unwrap();
    let mut acc = net.refresh_accumulator(&Position::initial());
    for values in &mut acc.hidden {
        for value in values.iter_mut() {
            *value = (next(&mut state) % 421) as i16 - 80;
        }
    }
    // cp_per_logit=127/16なのでscale=1。駒数を変えても非線形出力は変わらない。
    for count in [0, 1, 2, 3, 47, 91, 92, 100, 144] {
        acc.piece_count = count;
        for side in Color::ALL {
            let reference = reference_head(
                &p,
                &acc.hidden[side.index()],
                &acc.hidden[side.opposite().index()],
                1,
            );
            assert_eq!(net.nonlinear_cp(&acc, side), reference.cp);
        }
    }
}

#[test]
fn head_constructor_checks_all_lengths_before_ranges() {
    for (parameter, expected) in [
        ("output_weights", 4),
        ("hidden1_weights", 60),
        ("hidden2_weights", 960),
    ] {
        for length in [0, expected - 1, expected + 1] {
            let mut p = HeadParameters::zero(4);
            match parameter {
                "output_weights" => p.output_weights.resize(length, 0),
                "hidden1_weights" => p.hidden1_weights.resize(length, 0),
                "hidden2_weights" => p.hidden2_weights.resize(length, 0),
                _ => unreachable!(),
            }
            p.output_bias = i32::MAX;
            assert_eq!(
                p.build_width(4, true).err().unwrap(),
                Error::InvalidParameterLength {
                    parameter,
                    expected,
                    actual: length,
                }
            );
        }
    }
}

both_feature_sets!(head_sections_reject_invalid_lengths_ranges_and_mirrors);
fn head_sections_reject_invalid_lengths_ranges_and_mirrors<F: FeatureSet>() {
    let width = 8;
    let bytes = random_bytes::<F>(width);
    let offset = HEADER_LENGTH
        + FEATURE_COUNT * 4
        + PIECE_STATE_COUNT * 4
        + F::DIMENSIONS * 2 * width
        + 4 * width;
    // 区画表からの独立な計算。o, b, W1, b1, W2, b2, vの順。
    let section_length = 16 + 4 + 240 + 60 + 1920 + 128 + 64;
    assert_eq!(PairwiseL16L32::byte_len(width), section_length);
    assert_eq!(PairwiseL16L32::byte_len(256), 10_368);
    assert_eq!(bytes.len(), offset + section_length);
    for length in 0..section_length {
        assert!(matches!(
            PairwiseL16L32::decode(&bytes[offset..offset + length], width, false),
            Err(Error::InvalidLength { .. })
        ));
    }
    for length in [0, section_length - 1, section_length + 1] {
        let mut bad = bytes.clone();
        bad.resize(offset + length, 0);
        refresh_checksum(&mut bad);
        assert!(matches!(
            Network::<F, PairwiseL16L32>::decode(&bad),
            Err(Error::InvalidLength { .. })
        ));
        assert!(matches!(
            PairwiseL16L32::decode(&bad[offset..], width, false),
            Err(Error::InvalidLength { .. })
        ));
    }
    let assert_invalid = |p: &HeadParameters, parameter, index, value| {
        let expected = Error::ParameterOutOfRange {
            parameter,
            index,
            value,
        };
        // 値域の違反は、oの鏡映違反より先に返す。
        assert_eq!(p.build(true).err().unwrap(), expected);
        assert_eq!(
            PairwiseL16L32::decode(&p.bytes(), width, true)
                .err()
                .unwrap(),
            expected
        );
        let mut bad = bytes[..offset].to_vec();
        bad.extend(p.bytes());
        refresh_checksum(&mut bad);
        assert_eq!(
            Network::<F, PairwiseL16L32>::decode(&bad).err().unwrap(),
            expected
        );
    };
    let mut p = HeadParameters::zero(width);
    p.output_weights[0] = 1;
    for value in [-8193_i16, 8193, i16::MIN, i16::MAX] {
        let mut bad = p.clone();
        bad.output_weights[width - 1] = value;
        assert_invalid(&bad, "output_weights", width - 1, i32::from(value));
        let mut bad = p.clone();
        bad.hidden2_output[31] = value;
        assert_invalid(&bad, "hidden2_output", 31, i32::from(value));
    }
    for value in [-4_161_537, 4_161_537, i32::MIN, i32::MAX] {
        let mut bad = p.clone();
        bad.output_bias = value;
        assert_invalid(&bad, "output_bias", 0, value);
    }
    for value in [-260_097, 260_097, i32::MIN, i32::MAX] {
        let mut bad = p.clone();
        bad.hidden1_biases[14] = value;
        assert_invalid(&bad, "hidden1_biases", 14, value);
        let mut bad = p.clone();
        bad.hidden2_biases[31] = value;
        assert_invalid(&bad, "hidden2_biases", 31, value);
    }
    for value in [-509_i16, 509, i16::MIN, i16::MAX] {
        for (parameter, index) in [
            ("hidden1_weights", 15 * width - 1),
            ("hidden2_weights", 32 * 30 - 1),
        ] {
            let mut bad = p.clone();
            if parameter == "hidden1_weights" {
                bad.hidden1_weights[index] = value;
            } else {
                bad.hidden2_weights[index] = value;
            }
            assert_invalid(&bad, parameter, index, i32::from(value));
        }
    }
    for sign in [-1, 1] {
        let mut p = HeadParameters::zero(width);
        p.output_weights.fill(sign * 8192);
        p.output_bias = i32::from(sign) * 4_161_536;
        p.hidden1_weights.fill(sign * 508);
        p.hidden1_biases.fill(i32::from(sign) * 260_096);
        p.hidden2_weights.fill(sign * 508);
        p.hidden2_biases.fill(i32::from(sign) * 260_096);
        p.hidden2_output.fill(sign * 8192);
        assert!(p.build(true).is_ok());
        assert!(PairwiseL16L32::decode(&p.bytes(), width, true).is_ok());
    }
    for name in [
        "direct-screlu",
        "direct-screlu-phase2",
        "screlu-hidden16",
        "screlu-hidden16-phase2",
        "unknown",
    ] {
        let mut bad = bytes.clone();
        let text = format!(
            "features={};ft={width};linear=tapered2;head={name}",
            F::NAME
        );
        bad[8..104].fill(0);
        bad[8..8 + text.len()].copy_from_slice(text.as_bytes());
        assert_eq!(
            Network::<F, PairwiseL16L32>::decode(&bad).err().unwrap(),
            Error::UnexpectedArchitecture { actual: text }
        );
    }
}

both_feature_sets!(head_mirror_sharing_checks_each_pair_in_both_views_and_every_w1_row);
fn head_mirror_sharing_checks_each_pair_in_both_views_and_every_w1_row<F: FeatureSet>() {
    let width = 8;
    let mut prefix = random_bytes::<F>(width);
    prefix.truncate(
        HEADER_LENGTH
            + FEATURE_COUNT * 4
            + PIECE_STATE_COUNT * 4
            + F::DIMENSIONS * 2 * width
            + 4 * width,
    );
    let mut state = 0xc063_b901;
    let p = HeadParameters::random(width, &mut state, true);
    // b1、W2、b2、vは乱数のまま共有を課さず、両視点どうしも一致を要求しない。
    assert!(p.build(true).is_ok());
    for row in 0..16 {
        for view in 0..2 {
            for j in 0..width / 4 {
                let mut bad = p.clone();
                let i = view * width / 2 + j;
                let (parameter, index) = if row == 0 {
                    bad.output_weights[i] = if bad.output_weights[i] == 0 { 1 } else { 0 };
                    ("output_weights", i)
                } else {
                    let index = (row - 1) * width + i;
                    bad.hidden1_weights[index] = if bad.hidden1_weights[index] == 0 {
                        1
                    } else {
                        0
                    };
                    ("hidden1_weights", index)
                };
                let expected = Error::MirrorMismatch { parameter, index };
                assert_eq!(bad.build(true).err().unwrap(), expected);
                assert_eq!(
                    PairwiseL16L32::decode(&bad.bytes(), width, true)
                        .err()
                        .unwrap(),
                    expected
                );
                let mut bytes = prefix.clone();
                bytes.extend(bad.bytes());
                refresh_checksum(&mut bytes);
                let decoded = Network::<F, PairwiseL16L32>::decode(&bytes);
                if F::MIRROR == Mirror::Shared {
                    assert_eq!(decoded.err().unwrap(), expected);
                } else {
                    assert!(decoded.is_ok());
                }
                assert!(bad.build(false).is_ok());
                assert!(PairwiseL16L32::decode(&bad.bytes(), width, false).is_ok());
            }
        }
    }
}

#[test]
fn head_rejects_widths_outside_four_to_512_or_not_divisible_by_four() {
    let p = HeadParameters::zero(4);
    for width in [0, 1, 2, 3, 6, 10, 511, 513, 514, 516, u32::MAX as usize] {
        let expected = Error::InvalidWidth {
            actual: width as u32,
        };
        assert_eq!(p.build_width(width, false).err().unwrap(), expected);
        assert_eq!(
            PairwiseL16L32::decode(&[], width, false).err().unwrap(),
            expected
        );
    }
}

both_feature_sets!(network_rejects_widths_accepted_by_the_generic_header);
fn network_rejects_widths_accepted_by_the_generic_header<F: FeatureSet>() {
    for width in [2, 6, 10] {
        let mut bytes = random_bytes::<F>(4);
        let start = HEADER_LENGTH + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4;
        bytes.truncate(start);
        set_architecture::<F>(&mut bytes, width);
        bytes[104..108].copy_from_slice(&(width as u32).to_le_bytes());
        bytes.resize(start + F::DIMENSIONS * 2 * width + 4 * width, 0);
        bytes.extend(HeadParameters::zero(width).bytes());
        refresh_checksum(&mut bytes);
        assert!(architecture(&bytes).is_ok());
        assert_eq!(
            Network::<F, PairwiseL16L32>::decode(&bytes).err().unwrap(),
            Error::InvalidWidth {
                actual: width as u32
            }
        );
    }
}
