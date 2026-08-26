//! 量子化したNNUEの復号、差分更新および整数推論。

use core::fmt;
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

use super::features::{FEATURE_COUNT, active_features_for, feature_index, lion_feature_index};
use crate::{Color, Position, Square, Undo};

/// MNUEヘッダのバイト数。
const HEADER_LENGTH: usize = 96;
/// MNUE本体のバイト数。
const BODY_LENGTH: usize = 7_040_324;
/// 対応するMNUE形式の版。
const FORMAT_VERSION: u32 = 2;
/// 対応する特徴集合の識別子。
const FEATURE_SET: u32 = 1;
/// 第1層の幅。
const HIDDEN1_WIDTH: usize = 256;
/// 第2層の幅。
const HIDDEN2_WIDTH: usize = 32;
/// 第3層の幅。
const HIDDEN3_WIDTH: usize = 32;
/// 第2層の入力幅。
const INPUT2_WIDTH: usize = HIDDEN1_WIDTH * 2;
/// ネットを生成した規則セット名。
const RULE_SET: &[u8; 11] = b"L0,P0,R1,E0";
/// 静的評価値の絶対値上限。
const EVALUATION_LIMIT: i32 = 28_999;
/// 第2層以降の重みの尺度4,096を打ち消す算術右シフト量。
const WEIGHT_SHIFT: u32 = 12;
/// 第2層以降の重みの絶対値上限（実数±2.0に相当）。512項の積和がi32に収まる条件。
const WEIGHT_LIMIT: i16 = 8_192;
/// 第2層以降のバイアスの絶対値上限（実数±8.0に相当）。
const BIAS_LIMIT: i32 = 8 * 127 * 4_096;
/// 第1層の重みとバイアスの絶対値上限（実数±8.0に相当）。146項の積和がi32に収まる条件。
const WEIGHT1_LIMIT: i16 = 8 * 127;
/// 勝率尺度Kの上限。センチポーン変換の積がi64に収まる条件。
const K_LIMIT: f32 = 100_000.0;

/// 量子化済みNNUEの重み、バイアスおよび来歴情報。
pub struct Network {
    /// 第1層の特徴別重み。
    w1: Vec<i16>,
    /// 第1層のバイアス。
    b1: [i32; HIDDEN1_WIDTH],
    /// 第2層の出力別重み。
    w2: Vec<i16>,
    /// 第2層のバイアス。
    b2: [i32; HIDDEN2_WIDTH],
    /// 第3層の出力別重み。
    w3: Vec<i16>,
    /// 第3層のバイアス。
    b3: [i32; HIDDEN3_WIDTH],
    /// 出力層の重み。
    wo: [i16; HIDDEN3_WIDTH],
    /// 出力層のバイアス。
    bo: i32,
    /// 学習時に使ったセンチポーンから勝率ロジットへの尺度。
    k: f32,
    /// 出力をセンチポーンへ整数変換する固定小数点尺度。
    scale: i64,
    /// ネット本体のSHA-256。
    checksum: [u8; 32],
}

impl Network {
    /// MNUEバイト列を完全検証し、量子化済みネットへ復号する。
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let expected_length = HEADER_LENGTH + BODY_LENGTH;
        if bytes.len() != expected_length {
            return Err(Error::InvalidLength {
                expected: expected_length,
                actual: bytes.len(),
            });
        }
        let magic = bytes[0..4].try_into().expect("slice length is fixed");
        if magic != *b"MNUE" {
            return Err(Error::InvalidMagic { actual: magic });
        }
        let version = read_u32(bytes, 4);
        if version != FORMAT_VERSION {
            return Err(Error::UnsupportedVersion { actual: version });
        }
        let feature_set = read_u32(bytes, 8);
        if feature_set != FEATURE_SET {
            return Err(Error::UnexpectedFeatureSet {
                actual: feature_set,
            });
        }
        let widths = [
            read_u32(bytes, 12),
            read_u32(bytes, 16),
            read_u32(bytes, 20),
            read_u32(bytes, 24),
        ];
        let expected_widths = [
            FEATURE_COUNT as u32,
            HIDDEN1_WIDTH as u32,
            HIDDEN2_WIDTH as u32,
            HIDDEN3_WIDTH as u32,
        ];
        if widths != expected_widths {
            return Err(Error::UnexpectedLayerWidths { actual: widths });
        }
        let k = f32::from_le_bytes(bytes[28..32].try_into().expect("slice length is fixed"));
        if !k.is_finite() || k <= 0.0 || k > K_LIMIT {
            return Err(Error::InvalidK { actual: k });
        }
        if bytes[32..64] != rule_set_field() {
            return Err(Error::InvalidRuleSet);
        }
        let body = &bytes[HEADER_LENGTH..];
        let checksum: [u8; 32] = bytes[64..96].try_into().expect("slice length is fixed");
        let actual_checksum: [u8; 32] = Sha256::digest(body).into();
        if checksum != actual_checksum {
            return Err(Error::ChecksumMismatch);
        }

        let mut offset = 0;
        let w1 = read_i16_vec(body, &mut offset, FEATURE_COUNT * HIDDEN1_WIDTH);
        let b1 = read_i32_array(body, &mut offset);
        let w2 = read_i16_vec(body, &mut offset, HIDDEN2_WIDTH * INPUT2_WIDTH);
        let b2 = read_i32_array(body, &mut offset);
        let w3 = read_i16_vec(body, &mut offset, HIDDEN3_WIDTH * HIDDEN2_WIDTH);
        let b3 = read_i32_array(body, &mut offset);
        let wo: [i16; HIDDEN3_WIDTH] = std::array::from_fn(|_| read_i16(body, &mut offset));
        let bo = read_i32(body, &mut offset);
        debug_assert_eq!(offset, BODY_LENGTH);
        // 積和のi32溢れを防ぐため、各層の重みとバイアスの大きさを制限する。
        if w1
            .iter()
            .any(|&w| w.unsigned_abs() > WEIGHT1_LIMIT.unsigned_abs())
            || b1
                .iter()
                .any(|&b| b.unsigned_abs() > u32::from(WEIGHT1_LIMIT.unsigned_abs()))
            || w2
                .iter()
                .chain(&w3)
                .chain(&wo)
                .any(|&w| w.unsigned_abs() > WEIGHT_LIMIT.unsigned_abs())
            || b2
                .iter()
                .chain(&b3)
                .chain([&bo])
                .any(|&b| b.unsigned_abs() > BIAS_LIMIT.unsigned_abs())
        {
            return Err(Error::WeightOutOfRange);
        }
        // Pythonのround()と同じ最近接偶数丸めで固定小数点尺度を作る。
        let scale = (f64::from(k) * 65_536.0 / f64::from(127 * 4_096)).round_ties_even() as i64;
        Ok(Self {
            w1,
            b1,
            w2,
            b2,
            w3,
            b3,
            wo,
            bo,
            k,
            scale,
            checksum,
        })
    }

    /// 学習時に使った勝率尺度Kを返す。
    pub const fn k(&self) -> f32 {
        self.k
    }

    /// ネット本体のSHA-256を返す。
    pub const fn checksum(&self) -> &[u8; 32] {
        &self.checksum
    }
}

/// MNUEバイト列の検証失敗。
#[derive(Clone, PartialEq, Debug)]
pub enum Error {
    /// ファイル長がヘッダとネット本体の固定長に一致しない。
    InvalidLength {
        /// 期待するファイル長。
        expected: usize,
        /// 実際のファイル長。
        actual: usize,
    },
    /// ファイル識別子がMNUEではない。
    InvalidMagic {
        /// 読み取った識別子。
        actual: [u8; 4],
    },
    /// 形式の版が本実装の版と異なる。
    UnsupportedVersion {
        /// 読み取った版。
        actual: u32,
    },
    /// 特徴集合の識別子が本実装と異なる。
    UnexpectedFeatureSet {
        /// 読み取った特徴集合の識別子。
        actual: u32,
    },
    /// 各層の幅が本実装の構成と異なる。
    UnexpectedLayerWidths {
        /// 読み取った入力層から第3層までの幅。
        actual: [u32; 4],
    },
    /// 規則セット名が生成規則と異なる。
    InvalidRuleSet,
    /// 勝率尺度Kが正の有限値ではない。
    InvalidK {
        /// 読み取った勝率尺度。
        actual: f32,
    },
    /// ネット本体のSHA-256がヘッダの値と一致しない。
    ChecksumMismatch,
    /// 重みまたはバイアスが積和の溢れを防ぐ上限を超える。
    WeightOutOfRange,
}

impl fmt::Display for Error {
    /// エラーの説明を整形する。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "invalid MNUE length: expected {expected}, got {actual}"
                )
            }
            Self::InvalidMagic { actual } => write!(formatter, "invalid MNUE magic: {actual:?}"),
            Self::UnsupportedVersion { actual } => {
                write!(formatter, "unsupported MNUE version: {actual}")
            }
            Self::UnexpectedFeatureSet { actual } => write!(
                formatter,
                "invalid MNUE feature set: expected {FEATURE_SET}, got {actual}"
            ),
            Self::UnexpectedLayerWidths { actual } => {
                write!(formatter, "invalid MNUE layer widths: {actual:?}")
            }
            Self::InvalidRuleSet => formatter.write_str("invalid MNUE rule-set field"),
            Self::InvalidK { actual } => write!(formatter, "invalid MNUE K: {actual}"),
            Self::ChecksumMismatch => formatter.write_str("MNUE body checksum mismatch"),
            Self::WeightOutOfRange => formatter.write_str("MNUE weight or bias exceeds its limit"),
        }
    }
}

impl std::error::Error for Error {}

/// 1視点分の第1層積和を保持する。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Accumulator(
    /// 第1層のユニットごとの積和。
    [i32; HIDDEN1_WIDTH],
);

impl Default for Accumulator {
    /// すべての積和が0のアキュムレータを返す。
    fn default() -> Self {
        Self([0; HIDDEN1_WIDTH])
    }
}

/// 先手視点と後手視点のアキュムレータを保持する。
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct AccumulatorPair {
    /// [`Color::index`]で参照する視点別アキュムレータ。
    values: [Accumulator; 2],
}

impl AccumulatorPair {
    /// 指定視点のアキュムレータを返す。
    fn get(&self, perspective: Color) -> &Accumulator {
        &self.values[perspective.index()]
    }

    /// 指定視点のアキュムレータを可変参照で返す。
    fn get_mut(&mut self, perspective: Color) -> &mut Accumulator {
        &mut self.values[perspective.index()]
    }
}

/// 局面の全有効特徴から両視点のアキュムレータを再計算する。
pub fn refresh(network: &Network, position: &Position, out: &mut AccumulatorPair) {
    for perspective in Color::ALL {
        let accumulator = out.get_mut(perspective);
        accumulator.0.copy_from_slice(&network.b1);
        active_features_for(perspective, position, |feature| {
            add_feature(network, accumulator, feature, 1);
        });
    }
}

/// 着手前の値と着手差分から着手後の両視点アキュムレータを作る。
pub fn update_after_move(
    network: &Network,
    before: &AccumulatorPair,
    after: &mut AccumulatorPair,
    position_after: &Position,
    undo: &Undo,
) {
    after.clone_from(before);
    for perspective in Color::ALL {
        let accumulator = after.get_mut(perspective);
        add_feature(
            network,
            accumulator,
            feature_index(perspective, undo.moved_piece_before, undo.mv.from),
            -1,
        );
        for captured in undo.captured.into_iter().flatten() {
            add_feature(
                network,
                accumulator,
                feature_index(perspective, captured.piece, captured.square),
                -1,
            );
        }
        if let Some(trigger) = undo.previous_lion_taken {
            add_feature(
                network,
                accumulator,
                lion_feature_index(perspective, trigger.square),
                -1,
            );
        }
        let moved_piece_after = position_after
            .piece_at(undo.mv.to)
            .expect("move destination must contain the moved piece");
        add_feature(
            network,
            accumulator,
            feature_index(perspective, moved_piece_after, undo.mv.to),
            1,
        );
        if let Some(trigger) = position_after.lion_taken_by_non_lion() {
            add_feature(
                network,
                accumulator,
                lion_feature_index(perspective, trigger.square),
                1,
            );
        }
    }
}

/// null move前の値から先獅子状態を除いた両視点アキュムレータを作る。
pub fn update_after_null_move(
    network: &Network,
    before: &AccumulatorPair,
    after: &mut AccumulatorPair,
    lion_before: Option<Square>,
) {
    after.clone_from(before);
    if let Some(square) = lion_before {
        for perspective in Color::ALL {
            let feature = lion_feature_index(perspective, square);
            add_feature(network, after.get_mut(perspective), feature, -1);
        }
    }
}

/// アキュムレータから手番側視点の静的評価値をセンチポーンで返す。
pub fn evaluate(network: &Network, accumulators: &AccumulatorPair, side_to_move: Color) -> i32 {
    let mut input = [0_u8; INPUT2_WIDTH];
    let perspectives = [side_to_move, side_to_move.opposite()];
    for (half, perspective) in perspectives.into_iter().enumerate() {
        for (index, &value) in accumulators.get(perspective).0.iter().enumerate() {
            input[half * HIDDEN1_WIDTH + index] = value.clamp(0, 127) as u8;
        }
    }

    let mut hidden2 = [0_u8; HIDDEN2_WIDTH];
    for (output, value) in hidden2.iter_mut().enumerate() {
        let mut sum = network.b2[output];
        let weights = &network.w2[output * INPUT2_WIDTH..(output + 1) * INPUT2_WIDTH];
        for (&weight, &input) in weights.iter().zip(&input) {
            sum += i32::from(weight) * i32::from(input);
        }
        *value = (sum >> WEIGHT_SHIFT).clamp(0, 127) as u8;
    }

    let mut hidden3 = [0_u8; HIDDEN3_WIDTH];
    for (output, value) in hidden3.iter_mut().enumerate() {
        let mut sum = network.b3[output];
        let weights = &network.w3[output * HIDDEN2_WIDTH..(output + 1) * HIDDEN2_WIDTH];
        for (&weight, &input) in weights.iter().zip(&hidden2) {
            sum += i32::from(weight) * i32::from(input);
        }
        *value = (sum >> WEIGHT_SHIFT).clamp(0, 127) as u8;
    }

    let mut output = network.bo;
    for (&weight, &input) in network.wo.iter().zip(&hidden3) {
        output += i32::from(weight) * i32::from(input);
    }
    let centipawns = (i64::from(output) * network.scale + 32_768) >> 16;
    centipawns.clamp(-i64::from(EVALUATION_LIMIT), i64::from(EVALUATION_LIMIT)) as i32
}

/// 局面から両視点を完全再計算して静的評価値を返す。
pub fn evaluate_position(network: &Network, position: &Position) -> i32 {
    let mut accumulators = AccumulatorPair::default();
    refresh(network, position, &mut accumulators);
    evaluate(network, &accumulators, position.side_to_move())
}

/// 指定特徴の第1層重みを符号付きでアキュムレータへ加える。
fn add_feature(network: &Network, accumulator: &mut Accumulator, feature: usize, sign: i32) {
    let weights = &network.w1[feature * HIDDEN1_WIDTH..(feature + 1) * HIDDEN1_WIDTH];
    for (value, &weight) in accumulator.0.iter_mut().zip(weights) {
        *value += sign * i32::from(weight);
    }
}

/// リトルエンディアンのu32を指定位置から読む。
fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("slice length is fixed"),
    )
}

/// 本体の現在位置からリトルエンディアンのi32を読む。
fn read_i32(bytes: &[u8], offset: &mut usize) -> i32 {
    let value = i32::from_le_bytes(
        bytes[*offset..*offset + 4]
            .try_into()
            .expect("slice length is fixed"),
    );
    *offset += 4;
    value
}

/// 本体の現在位置から固定長i32配列を読む。
fn read_i32_array<const N: usize>(bytes: &[u8], offset: &mut usize) -> [i32; N] {
    std::array::from_fn(|_| read_i32(bytes, offset))
}

/// 本体の現在位置から指定個数のリトルエンディアンi16を読む。
fn read_i16_vec(bytes: &[u8], offset: &mut usize, length: usize) -> Vec<i16> {
    let mut values = Vec::with_capacity(length);
    for _ in 0..length {
        values.push(i16::from_le_bytes(
            bytes[*offset..*offset + 2]
                .try_into()
                .expect("slice length is fixed"),
        ));
        *offset += 2;
    }
    values
}

/// 本体の現在位置からリトルエンディアンのi16を読む。
fn read_i16(bytes: &[u8], offset: &mut usize) -> i16 {
    let value = i16::from_le_bytes(
        bytes[*offset..*offset + 2]
            .try_into()
            .expect("slice length is fixed"),
    );
    *offset += 2;
    value
}

/// 規則セット名を32バイトのNUL埋め欄として返す。
fn rule_set_field() -> [u8; 32] {
    let mut field = [0_u8; 32];
    field[..RULE_SET.len()].copy_from_slice(RULE_SET);
    field
}

/// 実行バイナリへ埋め込むMNUEバイト列。
static EMBEDDED: &[u8] = include_bytes!("../../nets/nnue.bin");
/// 埋め込みネットの復号結果を保持する領域。
static NETWORK: OnceLock<Result<Network, Error>> = OnceLock::new();

/// 検証済みの埋め込みNNUEネットを返す。
pub fn network() -> Result<&'static Network, &'static Error> {
    match NETWORK.get_or_init(|| Network::decode(EMBEDDED)) {
        Ok(network) => Ok(network),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::XorShift64;
    use crate::test_util::{position, sq};
    use crate::{Game, Move, MoveGenerator, PieceKind, Rules};

    /// 埋め込みネットを改変可能なバイト列として返す。
    fn valid_bytes() -> Vec<u8> {
        include_bytes!("../../nets/nnue.bin").to_vec()
    }

    /// 標準規則の合法手から条件に合う着手を適用して差分更新結果を返す。
    fn apply_matching_move(
        network: &Network,
        position: &mut Position,
        before: &AccumulatorPair,
        predicate: impl Fn(&Position, Move) -> bool,
    ) -> AccumulatorPair {
        let rules = Rules::ENGINE_DEFAULT.moves;
        let mut moves = Vec::new();
        MoveGenerator::new(rules).generate_moves(position, &mut moves);
        let mv = moves
            .into_iter()
            .find(|&mv| predicate(position, mv))
            .expect("fixture must contain the specified legal move");
        let undo = position.make_move_unchecked(mv, rules);
        let mut incremental = AccumulatorPair::default();
        update_after_move(network, before, &mut incremental, position, &undo);
        let mut refreshed = AccumulatorPair::default();
        refresh(network, position, &mut refreshed);
        assert_eq!(incremental, refreshed);
        incremental
    }

    /// 指定局面を完全再計算し、条件に合う着手の差分更新との一致を検査する。
    fn assert_matching_move(mut position: Position, predicate: impl Fn(&Position, Move) -> bool) {
        let network = network().unwrap();
        let mut before = AccumulatorPair::default();
        refresh(network, &position, &mut before);
        apply_matching_move(network, &mut position, &before, predicate);
    }

    /// 固定長と異なるMNUEが拒否されることを検査する。
    #[test]
    fn decode_rejects_invalid_length() {
        let mut bytes = valid_bytes();
        bytes.pop();
        assert!(matches!(
            Network::decode(&bytes),
            Err(Error::InvalidLength { .. })
        ));
    }

    /// MNUE識別子の不一致が拒否されることを検査する。
    #[test]
    fn decode_rejects_invalid_magic() {
        let mut bytes = valid_bytes();
        bytes[0] ^= 1;
        assert!(matches!(
            Network::decode(&bytes),
            Err(Error::InvalidMagic { .. })
        ));
    }

    /// 未対応のMNUE版が拒否されることを検査する。
    #[test]
    fn decode_rejects_unsupported_version() {
        let mut bytes = valid_bytes();
        bytes[4] ^= 1;
        assert!(matches!(
            Network::decode(&bytes),
            Err(Error::UnsupportedVersion { .. })
        ));
    }

    /// MNUE特徴集合の不一致が拒否されることを検査する。
    #[test]
    fn decode_rejects_unexpected_feature_set() {
        let mut bytes = valid_bytes();
        bytes[8] ^= 1;
        assert!(matches!(
            Network::decode(&bytes),
            Err(Error::UnexpectedFeatureSet { .. })
        ));
    }

    /// 入力層から第3層までの幅の不一致が個別に拒否されることを検査する。
    #[test]
    fn decode_rejects_unexpected_layer_widths() {
        for offset in [12, 16, 20, 24] {
            let mut bytes = valid_bytes();
            bytes[offset] ^= 1;
            assert!(matches!(
                Network::decode(&bytes),
                Err(Error::UnexpectedLayerWidths { .. })
            ));
        }
    }

    /// 正で有限ではない勝率尺度Kが拒否されることを検査する。
    #[test]
    fn decode_rejects_invalid_k() {
        let mut bytes = valid_bytes();
        bytes[28..32].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(matches!(
            Network::decode(&bytes),
            Err(Error::InvalidK { .. })
        ));
    }

    /// 規則セット名の不一致が拒否されることを検査する。
    #[test]
    fn decode_rejects_invalid_rule_set() {
        let mut bytes = valid_bytes();
        bytes[32] ^= 1;
        assert!(matches!(
            Network::decode(&bytes),
            Err(Error::InvalidRuleSet)
        ));
    }

    /// MNUE本体の改変がSHA-256で拒否されることを検査する。
    #[test]
    fn decode_rejects_checksum_mismatch() {
        let mut bytes = valid_bytes();
        bytes[HEADER_LENGTH] ^= 1;
        assert!(matches!(
            Network::decode(&bytes),
            Err(Error::ChecksumMismatch)
        ));
    }

    /// ヘッダのSHA-256欄の不一致が拒否されることを検査する。
    #[test]
    fn decode_rejects_header_checksum_mismatch() {
        let mut bytes = valid_bytes();
        bytes[64] ^= 1;
        assert!(matches!(
            Network::decode(&bytes),
            Err(Error::ChecksumMismatch)
        ));
    }

    /// 本体を改変し、SHA-256を再計算したバイト列を返す。
    fn with_body_patch(offset: usize, patch: &[u8]) -> Vec<u8> {
        let mut bytes = valid_bytes();
        let start = HEADER_LENGTH + offset;
        bytes[start..start + patch.len()].copy_from_slice(patch);
        let digest: [u8; 32] = Sha256::digest(&bytes[HEADER_LENGTH..]).into();
        bytes[64..96].copy_from_slice(&digest);
        bytes
    }

    /// 各層の重みとバイアスが上限を超えるネットが拒否され、上限ちょうどは受理されることを検査する。
    #[test]
    fn decode_rejects_weights_and_biases_beyond_their_limits() {
        let w1_end = FEATURE_COUNT * HIDDEN1_WIDTH * 2;
        let b1_end = w1_end + HIDDEN1_WIDTH * 4;
        let w2_end = b1_end + HIDDEN2_WIDTH * INPUT2_WIDTH * 2;
        let cases = [
            (0, (WEIGHT1_LIMIT + 1).to_le_bytes().to_vec()),
            (
                w1_end,
                (i32::from(WEIGHT1_LIMIT) + 1).to_le_bytes().to_vec(),
            ),
            (b1_end, (WEIGHT_LIMIT + 1).to_le_bytes().to_vec()),
            (w2_end, (BIAS_LIMIT + 1).to_le_bytes().to_vec()),
        ];
        for (offset, patch) in cases {
            assert!(matches!(
                Network::decode(&with_body_patch(offset, &patch)),
                Err(Error::WeightOutOfRange)
            ));
        }
        assert!(Network::decode(&with_body_patch(0, &WEIGHT1_LIMIT.to_le_bytes())).is_ok());
    }

    /// 上限を超える勝率尺度Kが拒否されることを検査する。
    #[test]
    fn decode_rejects_k_beyond_the_limit() {
        let mut bytes = valid_bytes();
        bytes[28..32].copy_from_slice(&(K_LIMIT * 2.0).to_le_bytes());
        assert!(matches!(
            Network::decode(&bytes),
            Err(Error::InvalidK { .. })
        ));
    }

    /// 通常の非捕獲手の差分更新が完全再計算と一致することを検査する。
    #[test]
    fn ordinary_move_update_matches_refresh() {
        assert_matching_move(Position::initial(), |position, mv| {
            mv.from != mv.to
                && mv.mid.is_none()
                && !mv.promote
                && position.captured_squares(mv).iter().all(Option::is_none)
        });
    }

    /// 成る手の差分更新が完全再計算と一致することを検査する。
    #[test]
    fn promotion_update_matches_refresh() {
        let fixture = position(Color::Black, &[(sq(4, 7), Color::Black, PieceKind::Pawn)]);
        assert_matching_move(fixture, |_, mv| mv.promote);
    }

    /// 1枚捕獲の差分更新が完全再計算と一致することを検査する。
    #[test]
    fn single_capture_update_matches_refresh() {
        let fixture = position(
            Color::Black,
            &[
                (sq(4, 4), Color::Black, PieceKind::Rook),
                (sq(4, 9), Color::White, PieceKind::Pawn),
            ],
        );
        assert_matching_move(fixture, |position, mv| {
            position.captured_squares(mv).iter().flatten().count() == 1
        });
    }

    /// 獅子の2枚捕獲の差分更新が完全再計算と一致することを検査する。
    #[test]
    fn double_capture_update_matches_refresh() {
        let fixture = position(
            Color::Black,
            &[
                (sq(5, 5), Color::Black, PieceKind::Lion),
                (sq(5, 6), Color::White, PieceKind::Pawn),
                (sq(5, 7), Color::White, PieceKind::Pawn),
            ],
        );
        assert_matching_move(fixture, |position, mv| {
            mv.mid.is_some() && position.captured_squares(mv).iter().flatten().count() == 2
        });
    }

    /// 居喰いの差分更新が完全再計算と一致することを検査する。
    #[test]
    fn igui_update_matches_refresh() {
        let fixture = position(
            Color::Black,
            &[
                (sq(5, 5), Color::Black, PieceKind::Lion),
                (sq(5, 6), Color::White, PieceKind::Pawn),
            ],
        );
        assert_matching_move(fixture, |position, mv| {
            mv.from == mv.to
                && mv.mid.is_some()
                && position.captured_squares(mv).iter().flatten().count() == 1
        });
    }

    /// 獅子のじっとの差分更新が完全再計算と一致することを検査する。
    #[test]
    fn jitto_update_matches_refresh() {
        let fixture = position(Color::Black, &[(sq(5, 5), Color::Black, PieceKind::Lion)]);
        assert_matching_move(fixture, |position, mv| {
            mv.from == mv.to
                && mv.mid.is_none()
                && position.captured_squares(mv).iter().all(Option::is_none)
        });
    }

    /// 非獅子の獅子捕獲による先獅子発生と次の手での消滅を検査する。
    #[test]
    fn lion_trigger_creation_and_expiration_updates_match_refresh() {
        let network = network().unwrap();
        let mut fixture = position(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::Bishop),
                (sq(1, 1), Color::White, PieceKind::Lion),
                (sq(10, 10), Color::White, PieceKind::Pawn),
            ],
        );
        let mut before = AccumulatorPair::default();
        refresh(network, &fixture, &mut before);
        let after_capture = apply_matching_move(network, &mut fixture, &before, |position, mv| {
            position
                .captured_squares(mv)
                .into_iter()
                .flatten()
                .any(|square| {
                    position.piece_at(square).and_then(|piece| piece.kind())
                        == Some(PieceKind::Lion)
                })
        });
        assert!(fixture.lion_taken_by_non_lion().is_some());
        apply_matching_move(network, &mut fixture, &after_capture, |position, mv| {
            mv.from != mv.to
                && mv.mid.is_none()
                && position.captured_squares(mv).iter().all(Option::is_none)
        });
        assert!(fixture.lion_taken_by_non_lion().is_none());
    }

    /// null moveによる先獅子状態の消滅が完全再計算と一致することを検査する。
    #[test]
    fn null_move_lion_expiration_update_matches_refresh() {
        let network = network().unwrap();
        let mut fixture = position(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(11, 11), Color::White, PieceKind::King),
            ],
        );
        fixture.set_lion_capture(Some(sq(5, 5))).unwrap();
        let mut before = AccumulatorPair::default();
        refresh(network, &fixture, &mut before);
        let lion_before = fixture
            .lion_taken_by_non_lion()
            .map(|trigger| trigger.square);
        fixture.make_null_move();
        let mut incremental = AccumulatorPair::default();
        update_after_null_move(network, &before, &mut incremental, lion_before);
        let mut refreshed = AccumulatorPair::default();
        refresh(network, &fixture, &mut refreshed);
        assert_eq!(incremental, refreshed);
    }

    /// 王将を捕獲する手の差分更新が完全再計算と一致することを検査する。
    #[test]
    fn king_capture_update_matches_refresh() {
        let fixture = position(
            Color::Black,
            &[
                (sq(4, 4), Color::Black, PieceKind::Rook),
                (sq(4, 9), Color::White, PieceKind::King),
            ],
        );
        assert_matching_move(fixture, |position, mv| {
            position
                .captured_squares(mv)
                .into_iter()
                .flatten()
                .any(|square| {
                    position.piece_at(square).and_then(|piece| piece.kind())
                        == Some(PieceKind::King)
                })
        });
    }

    /// 醉象が成って太子になる手の差分更新が完全再計算と一致することを検査する。
    #[test]
    fn crown_prince_creation_update_matches_refresh() {
        let fixture = position(
            Color::Black,
            &[(sq(4, 7), Color::Black, PieceKind::DrunkElephant)],
        );
        assert_matching_move(fixture, |position, mv| {
            mv.promote
                && position.piece_at(mv.from).and_then(|piece| piece.kind())
                    == Some(PieceKind::DrunkElephant)
        });
    }

    /// 3局のランダム合法手列で毎手の差分更新が完全再計算と一致することを検査する。
    #[test]
    fn random_games_incremental_updates_match_refresh() {
        let network = network().unwrap();
        for seed in 1..=3 {
            let game = Game::new(Rules::ENGINE_DEFAULT);
            let rules = game.rules().moves;
            let generator = MoveGenerator::new(rules);
            let mut position = game.position().clone();
            let mut current = AccumulatorPair::default();
            refresh(network, &position, &mut current);
            let mut rng = XorShift64::new(seed);
            for _ in 0..200 {
                let mut moves = Vec::new();
                generator.generate_moves(&position, &mut moves);
                if moves.is_empty() {
                    break;
                }
                let mv = moves[rng.index(moves.len())];
                let undo = position.make_move_unchecked(mv, rules);
                let mut incremental = AccumulatorPair::default();
                update_after_move(network, &current, &mut incremental, &position, &undo);
                let mut refreshed = AccumulatorPair::default();
                refresh(network, &position, &mut refreshed);
                assert_eq!(incremental, refreshed, "seed={seed}, move={mv:?}");
                current = incremental;
            }
        }
    }

    /// 埋め込みネットの初期局面評価がPython整数参照実装と一致することを検査する。
    #[test]
    fn embedded_network_matches_python_initial_position_evaluations() {
        let network = network().unwrap();
        let initial = Position::initial();
        assert_eq!(evaluate_position(network, &initial), 386);
        let white_to_move = initial.clone_with_side_to_move(Color::White);
        assert_eq!(evaluate_position(network, &white_to_move), -422);
    }
}
