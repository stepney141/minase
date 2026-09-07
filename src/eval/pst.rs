//! 量子化した学習PSTの復号、埋め込みおよび整数評価。

use core::fmt;
use std::sync::{Arc, OnceLock};

use sha2::{Digest, Sha256};

use super::features::{
    FEATURE_COUNT, PIECE_STATE_COUNT, active_features, active_features_for, feature_index,
    lion_feature_index, piece_state,
};
use crate::core::mv::Undo;
use crate::{Color, PieceCode, PieceKind, Position, Square};

/// MNPTヘッダのバイト数。
const HEADER_LENGTH: usize = 80;
/// 対応するMNPT形式の版。
const FORMAT_VERSION: u32 = 2;
/// 静的評価値の絶対値上限。
const EVALUATION_LIMIT: i32 = 28_999;

/// 学習PSTの量子化重みと勝率尺度。
pub struct Pst {
    /// 序中盤、終盤の順に保持する1/8センチポーン単位の特徴重み。
    weights: [[i16; FEATURE_COUNT]; 2],
    /// 駒状態ごとのセンチポーン単位の駒価値。
    piece_values: [i32; PIECE_STATE_COUNT],
    /// 静止探索で小さな捕獲を残すための余裕値。
    delta_margin: i32,
    /// 学習時に使ったセンチポーンから勝率ロジットへの尺度。
    k: f32,
    /// 重み本体のSHA-256。学習データのヘッダに生成元のネットとして記録する。
    checksum: [u8; 32],
}

/// 先手視点と後手視点で集計したPSTの生重み和。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct PstAccumulator {
    /// 視点、端点の順に保持する生重み和。
    sums: [[i32; 2]; 2],
    /// 係数へ変換する前の盤上総駒数。
    piece_count: u32,
}

impl Pst {
    /// MNPTバイト列を検証し、学習PSTへ復号する。
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let expected_length = HEADER_LENGTH + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4;
        if bytes.len() != expected_length {
            return Err(Error::InvalidLength {
                expected: expected_length,
                actual: bytes.len(),
            });
        }
        let magic = bytes[0..4].try_into().expect("slice length is fixed");
        if magic != *b"MNPT" {
            return Err(Error::InvalidMagic { actual: magic });
        }
        let version = read_u32(bytes, 4);
        if version != FORMAT_VERSION {
            return Err(Error::UnsupportedVersion { actual: version });
        }
        let feature_count = read_u32(bytes, 8);
        if feature_count != FEATURE_COUNT as u32 {
            return Err(Error::UnexpectedFeatureCount {
                actual: feature_count,
            });
        }
        let k = f32::from_le_bytes(bytes[12..16].try_into().expect("slice length is fixed"));
        if !k.is_finite() || k <= 0.0 {
            return Err(Error::InvalidK { actual: k });
        }
        let rule_set = &bytes[16..48];
        let rule_end = rule_set.iter().position(|&byte| byte == 0).unwrap_or(32);
        if rule_set[rule_end..].iter().any(|&byte| byte != 0)
            || std::str::from_utf8(&rule_set[..rule_end]).is_err()
        {
            return Err(Error::InvalidRuleSet);
        }
        let body = &bytes[HEADER_LENGTH..];
        let checksum: [u8; 32] = bytes[48..80].try_into().expect("slice length is fixed");
        let actual_checksum: [u8; 32] = Sha256::digest(body).into();
        if actual_checksum != checksum {
            return Err(Error::ChecksumMismatch);
        }

        let mut weights = [[0_i16; FEATURE_COUNT]; 2];
        for (index, weight) in weights.iter_mut().flatten().enumerate() {
            let offset = index * 2;
            *weight = i16::from_le_bytes(
                body[offset..offset + 2]
                    .try_into()
                    .expect("slice length is fixed"),
            );
        }
        let mut piece_values = [0_i32; PIECE_STATE_COUNT];
        for (state, value) in piece_values.iter_mut().enumerate() {
            let offset = FEATURE_COUNT * 4 + state * 4;
            *value = i32::from_le_bytes(
                body[offset..offset + 4]
                    .try_into()
                    .expect("slice length is fixed"),
            );
        }
        let delta_margin = validate_piece_values(&piece_values)?;
        Ok(Self {
            weights,
            piece_values,
            delta_margin,
            k,
            checksum,
        })
    }

    /// 盤上の駒コードに対応する駒価値をセンチポーンで返す。
    ///
    /// 値はMNPT本体に格納した探索用駒価値を使う。
    ///
    /// # Panics
    ///
    /// `piece`が空升または盤外の番兵を表す場合にパニックする。
    #[inline]
    pub fn piece_value(&self, piece: PieceCode) -> i32 {
        self.piece_values[piece_state(piece)]
    }

    /// 成っていない歩兵の駒価値をセンチポーンで返す。
    ///
    /// `docs/plans/strength-stage4.md`の「採用した余裕値」節で使う単位`p`。
    #[inline]
    pub fn pawn_value(&self) -> i32 {
        let pawn = PieceCode::new(Color::Black, PieceKind::Pawn)
            .expect("pawn must have an unpromoted code");
        self.piece_value(pawn)
    }

    /// 静止探索で小さな捕獲を残すための余裕値を返す。
    #[inline]
    pub const fn delta_margin(&self) -> i32 {
        self.delta_margin
    }

    /// 学習時に使った勝率尺度Kを返す。
    pub const fn k(&self) -> f32 {
        self.k
    }

    /// 重み本体のSHA-256を返す。
    pub const fn checksum(&self) -> &[u8; 32] {
        &self.checksum
    }

    /// 局面のPST累算値を両視点から完全再計算する。
    pub(crate) fn refresh_accumulator(&self, position: &Position) -> PstAccumulator {
        let mut accumulator = PstAccumulator {
            sums: [[0; 2]; 2],
            piece_count: position.occupied().popcount(),
        };
        for perspective in Color::ALL {
            active_features_for(perspective, position, |feature| {
                for (sum, weights) in accumulator.sums[perspective.index()]
                    .iter_mut()
                    .zip(&self.weights)
                {
                    *sum += i32::from(weights[feature]);
                }
            });
        }
        accumulator
    }

    /// 通常着手後の局面について、PST累算値を差分更新する。
    pub(crate) fn update_accumulator_after_move(
        &self,
        before: PstAccumulator,
        position_after: &Position,
        undo: &Undo,
    ) -> PstAccumulator {
        let mut after = before;
        let moved_piece_after = position_after
            .piece_at(undo.mv.to)
            .expect("move destination must contain the moved piece");

        after.piece_count -= undo.captured.iter().flatten().count() as u32;
        for perspective in Color::ALL {
            for (sum, weights) in after.sums[perspective.index()]
                .iter_mut()
                .zip(&self.weights)
            {
                *sum -= i32::from(
                    weights[feature_index(perspective, undo.moved_piece_before, undo.mv.from)],
                );
                for captured in undo.captured.into_iter().flatten() {
                    *sum -= i32::from(
                        weights[feature_index(perspective, captured.piece, captured.square)],
                    );
                }
                if let Some(trigger) = undo.previous_lion_taken {
                    *sum -= i32::from(weights[lion_feature_index(perspective, trigger.square)]);
                }
                *sum +=
                    i32::from(weights[feature_index(perspective, moved_piece_after, undo.mv.to)]);
                if let Some(trigger) = position_after.lion_taken_by_non_lion() {
                    *sum += i32::from(weights[lion_feature_index(perspective, trigger.square)]);
                }
            }
        }
        after
    }

    /// null move後の局面について、PST累算値から直前の先獅子特徴を除く。
    pub(crate) fn update_accumulator_after_null(
        &self,
        before: PstAccumulator,
        lion_before: Option<Square>,
    ) -> PstAccumulator {
        let mut after = before;
        if let Some(square) = lion_before {
            for perspective in Color::ALL {
                for (sum, weights) in after.sums[perspective.index()]
                    .iter_mut()
                    .zip(&self.weights)
                {
                    *sum -= i32::from(weights[lion_feature_index(perspective, square)]);
                }
            }
        }
        after
    }

    /// 指定手番の視点からPST累算値をセンチポーン評価へ変換する。
    pub(crate) fn evaluate_accumulator(
        &self,
        accumulator: PstAccumulator,
        side_to_move: Color,
    ) -> i32 {
        interpolate(
            accumulator.sums[side_to_move.index()],
            accumulator.piece_count,
        )
    }
}

/// MNPTバイト列の検証失敗。
#[derive(Clone, PartialEq, Debug)]
pub enum Error {
    /// ファイル長がヘッダと重み本体の固定長に一致しない。
    InvalidLength {
        /// 期待するファイル長。
        expected: usize,
        /// 実際のファイル長。
        actual: usize,
    },
    /// ファイル識別子がMNPTではない。
    InvalidMagic {
        /// 読み取った識別子。
        actual: [u8; 4],
    },
    /// 形式の版が本実装の版と異なる。
    UnsupportedVersion {
        /// 読み取った版。
        actual: u32,
    },
    /// 特徴数が本実装の特徴数と異なる。
    UnexpectedFeatureCount {
        /// 読み取った特徴数。
        actual: u32,
    },
    /// 規則セット名欄がUTF-8またはNUL埋めの規約を満たさない。
    InvalidRuleSet,
    /// 勝率尺度Kが正の有限値ではない。
    InvalidK {
        /// 読み取った勝率尺度。
        actual: f32,
    },
    /// 重み本体のSHA-256がヘッダの値と一致しない。
    ChecksumMismatch,
    /// 盤上に現れ得る非王駒の格納値が正ではない。
    NonPositivePieceValue {
        /// 格納値が正ではない駒種。
        kind: PieceKind,
        /// 成った状態かどうか。
        promoted: bool,
        /// 格納されたセンチポーン単位の駒価値。
        value: i32,
    },
    /// 王または太子の値が最大非王駒価値と歩価値の和に一致しない。
    InconsistentRoyalValue {
        /// 王駒の種類。
        kind: PieceKind,
        /// 期待する駒価値。
        expected: i32,
        /// 格納された駒価値。
        actual: i32,
    },
    /// 駒価値が探索で扱える範囲の外にある。
    PieceValueOutOfRange {
        /// 駒状態番号。
        state: usize,
        /// 格納された駒価値。
        value: i32,
    },
}

impl fmt::Display for Error {
    /// エラーの説明を整形する。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "invalid MNPT length: expected {expected}, got {actual}"
                )
            }
            Self::InvalidMagic { actual } => write!(formatter, "invalid MNPT magic: {actual:?}"),
            Self::UnsupportedVersion { actual } => {
                write!(formatter, "unsupported MNPT version: {actual}")
            }
            Self::UnexpectedFeatureCount { actual } => write!(
                formatter,
                "invalid MNPT feature count: expected {FEATURE_COUNT}, got {actual}"
            ),
            Self::InvalidRuleSet => formatter.write_str("invalid MNPT rule-set field"),
            Self::InvalidK { actual } => write!(formatter, "invalid MNPT K: {actual}"),
            Self::ChecksumMismatch => formatter.write_str("MNPT weight checksum mismatch"),
            Self::NonPositivePieceValue {
                kind,
                promoted,
                value,
            } => write!(
                formatter,
                "non-positive MNPT piece value: kind={kind:?}, promoted={promoted}, value={value}"
            ),
            Self::InconsistentRoyalValue {
                kind,
                expected,
                actual,
            } => write!(
                formatter,
                "inconsistent MNPT royal value: kind={kind:?}, expected {expected}, got {actual}"
            ),
            Self::PieceValueOutOfRange { state, value } => write!(
                formatter,
                "MNPT piece value out of range: state={state}, value={value}"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// 格納された駒価値を検査し、静止探索の余裕値を返す。
fn validate_piece_values(values: &[i32; PIECE_STATE_COUNT]) -> Result<i32, Error> {
    for (state, &value) in values.iter().enumerate() {
        if !(0..=EVALUATION_LIMIT).contains(&value) {
            return Err(Error::PieceValueOutOfRange { state, value });
        }
    }

    let mut max_non_royal = 0;
    for kind in PieceKind::ALL {
        if kind.can_promote() {
            let piece = PieceCode::new(Color::Black, kind)
                .expect("promotable kind must have an unpromoted code");
            validate_piece_value(values, piece_state(piece), kind, false, &mut max_non_royal)?;
            if kind.unpromoted().is_some() {
                validate_piece_value(values, kind.index(), kind, true, &mut max_non_royal)?;
            }
        } else if !matches!(kind, PieceKind::King | PieceKind::CrownPrince) {
            let promoted = PieceCode::new(Color::Black, kind).is_none();
            validate_piece_value(values, kind.index(), kind, promoted, &mut max_non_royal)?;
        }
    }

    let pawn =
        PieceCode::new(Color::Black, PieceKind::Pawn).expect("pawn must have an unpromoted code");
    let pawn_value = values[piece_state(pawn)];
    let royal_value = max_non_royal + pawn_value;
    for kind in [PieceKind::King, PieceKind::CrownPrince] {
        let actual = values[kind.index()];
        if actual != royal_value {
            return Err(Error::InconsistentRoyalValue {
                kind,
                expected: royal_value,
                actual,
            });
        }
    }
    Ok(2 * pawn_value)
}

/// 盤上に現れ得る非王駒の値を検査し、最大値を更新する。
fn validate_piece_value(
    values: &[i32; PIECE_STATE_COUNT],
    state: usize,
    kind: PieceKind,
    promoted: bool,
    max_non_royal: &mut i32,
) -> Result<(), Error> {
    let value = values[state];
    if value <= 0 {
        return Err(Error::NonPositivePieceValue {
            kind,
            promoted,
            value,
        });
    }
    *max_non_royal = (*max_non_royal).max(value);
    Ok(())
}

/// 盤上総駒数に応じて生重み和を補間し、最後に1回だけ整数除算する。
fn interpolate(sums: [i32; 2], piece_count: u32) -> i32 {
    let q = i64::from(piece_count.saturating_sub(2).min(90));
    let numerator = q * i64::from(sums[0]) + (90 - q) * i64::from(sums[1]);
    (numerator / 720).clamp(-i64::from(EVALUATION_LIMIT), i64::from(EVALUATION_LIMIT)) as i32
}

/// 学習PSTで局面を手番側の視点からセンチポーン評価する。
pub fn evaluate(pst: &Pst, position: &Position) -> i32 {
    let mut sums = [0_i32; 2];
    active_features(position, |feature| {
        for (sum, weights) in sums.iter_mut().zip(&pst.weights) {
            *sum += i32::from(weights[feature]);
        }
    });
    interpolate(sums, position.occupied().popcount())
}

/// リトルエンディアンのu32を指定位置から読む。
fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("slice length is fixed"),
    )
}

/// 実行バイナリへ埋め込むMNPTバイト列。
static EMBEDDED: &[u8] = include_bytes!("../../nets/pst.bin");
/// 埋め込み重みの復号結果を保持する領域。
static WEIGHTS: OnceLock<Result<Arc<Pst>, Error>> = OnceLock::new();

/// 検証済みの埋め込み学習PSTを返す。
pub fn weights() -> Result<Arc<Pst>, Error> {
    match WEIGHTS.get_or_init(|| Pst::decode(EMBEDDED).map(Arc::new)) {
        Ok(pst) => Ok(Arc::clone(pst)),
        Err(error) => Err(error.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Move;
    use crate::eval::features::{PIECE_STATE_COUNT, piece_state};
    use crate::eval::handcrafted::piece_value;
    use crate::test_util::{position_from_codes, sq};
    use crate::{Color, MoveRules, PieceCode, PieceKind, Square};

    /// 検査用の正しいMNPTバイト列を返す。
    fn valid_bytes() -> Vec<u8> {
        include_bytes!("../../nets/pst-init.bin").to_vec()
    }

    /// 盤上に現れ得る駒状態を代表する先手の駒コードを返す。
    fn reachable_piece_states() -> Vec<PieceCode> {
        let mut pieces = Vec::new();
        for kind in PieceKind::ALL {
            if kind.can_promote() {
                pieces.push(PieceCode::new(Color::Black, kind).unwrap());
                if kind.unpromoted().is_some() {
                    pieces.push(PieceCode::new_promoted(Color::Black, kind).unwrap());
                }
            } else {
                pieces.push(
                    PieceCode::new(Color::Black, kind)
                        .or_else(|| PieceCode::new_promoted(Color::Black, kind))
                        .unwrap(),
                );
            }
        }
        assert_eq!(pieces.len(), PIECE_STATE_COUNT - 10);
        pieces
    }

    /// MNPT本体の指定特徴の重みを書き換える。
    fn set_weight(bytes: &mut [u8], endpoint: usize, feature: usize, weight: i16) {
        let offset = HEADER_LENGTH + (endpoint * FEATURE_COUNT + feature) * 2;
        bytes[offset..offset + 2].copy_from_slice(&weight.to_le_bytes());
    }

    /// 探索用駒価値の指定状態を書き換える。
    fn set_piece_value(bytes: &mut [u8], state: usize, value: i32) {
        let offset = HEADER_LENGTH + FEATURE_COUNT * 4 + state * 4;
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// 全特徴で端点が異なり、升と駒状態の差分も検出できる重みを作る。
    fn distinct_pst() -> Pst {
        let mut bytes = valid_bytes();
        for feature in 0..FEATURE_COUNT {
            set_weight(&mut bytes, 1, feature, (feature % 997) as i16 - 498);
        }
        refresh_checksum(&mut bytes);
        Pst::decode(&bytes).unwrap()
    }

    /// 盤面全体を指定数の駒で埋め、最初の2枚を王にする。
    fn position_with_count(count: usize, side: Color) -> Position {
        let pieces: Vec<_> = Square::all()
            .take(count)
            .enumerate()
            .map(|(index, square)| {
                let (color, kind) = match index {
                    0 => (Color::Black, PieceKind::King),
                    1 => (Color::White, PieceKind::King),
                    _ => (Color::Black, PieceKind::Pawn),
                };
                (square, PieceCode::new(color, kind).unwrap())
            })
            .collect();
        position_from_codes(side, &pieces)
    }

    /// 公開評価と探索用累算評価を、仕様から求めた期待値と照合する。
    fn assert_evaluation(pst: &Pst, position: &Position, expected: i32) {
        assert_eq!(evaluate(pst, position), expected);
        assert_eq!(
            pst.evaluate_accumulator(pst.refresh_accumulator(position), position.side_to_move()),
            expected
        );
    }

    /// MNPT本体を変更した後のSHA-256をヘッダへ反映する。
    fn refresh_checksum(bytes: &mut [u8]) {
        let checksum: [u8; 32] = Sha256::digest(&bytes[HEADER_LENGTH..]).into();
        bytes[48..80].copy_from_slice(&checksum);
    }

    /// 指定局面の手番側駒価値差を計算する。
    fn material_score(position: &Position) -> i32 {
        let perspective = position.side_to_move();
        Square::all()
            .filter_map(|square| position.piece_at(square))
            .map(|piece| {
                let sign = if piece.color() == Some(perspective) {
                    1
                } else {
                    -1
                };
                sign * piece_value(piece.kind().unwrap())
            })
            .sum()
    }

    /// 着手前後の差分累算値が完全再計算と一致し、親累算値を変更しないことを検査する。
    fn assert_move_accumulator(pst: &Pst, position: &mut Position, mv: Move) {
        let before = pst.refresh_accumulator(position);
        let undo = position.make_move_unchecked(mv, MoveRules::standard());
        let after = pst.update_accumulator_after_move(before, position, &undo);
        assert_eq!(after, pst.refresh_accumulator(position));
        assert_eq!(
            pst.evaluate_accumulator(after, position.side_to_move()),
            evaluate(pst, position)
        );
        position.unmake_move(undo);
        assert_eq!(before, pst.refresh_accumulator(position));
    }

    /// 差分累算値が通常手、特殊移動、先獅子状態、およびnull moveで完全再計算と一致する。
    #[test]
    fn accumulator_updates_match_full_refresh_across_move_shapes() {
        check_move_shapes(&weights().unwrap());
        check_move_shapes(&distinct_pst());
    }

    /// 指定した両端点で各着手形状の差分更新を検査する。
    fn check_move_shapes(pst: &Pst) {
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
            pst,
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
            pst,
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
            pst,
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
            pst,
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
        let before = pst.refresh_accumulator(&lion_capture);
        let undo = lion_capture.make_move_unchecked(
            Move {
                from: sq(5, 5),
                mid: None,
                to: sq(5, 3),
                promote: false,
            },
            MoveRules::standard(),
        );
        let after = pst.update_accumulator_after_move(before, &lion_capture, &undo);
        assert_eq!(after, pst.refresh_accumulator(&lion_capture));
        assert_eq!(after.piece_count, 3);
        assert!(lion_capture.lion_taken_by_non_lion().is_some());
        assert_eq!(
            pst.evaluate_accumulator(after, lion_capture.side_to_move()),
            evaluate(pst, &lion_capture)
        );

        let mut normal_response = lion_capture.clone();
        assert_move_accumulator(
            pst,
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
        let after_null = pst.update_accumulator_after_null(after, lion_before);
        assert_eq!(after_null, pst.refresh_accumulator(&lion_capture));
        assert_eq!(after_null.piece_count, 3);
        assert_eq!(
            pst.evaluate_accumulator(after_null, lion_capture.side_to_move()),
            evaluate(pst, &lion_capture)
        );
        lion_capture.unmake_null_move(null_undo);
        assert_eq!(after, pst.refresh_accumulator(&lion_capture));
        assert_eq!(after.piece_count, 3);
        assert!(lion_capture.lion_taken_by_non_lion().is_some());
        assert_eq!(
            pst.evaluate_accumulator(after, lion_capture.side_to_move()),
            evaluate(pst, &lion_capture)
        );
        lion_capture.unmake_move(undo);
        assert_eq!(before, pst.refresh_accumulator(&lion_capture));
    }

    /// 固定長と異なるMNPTが拒否されることを検査する。
    #[test]
    fn decode_rejects_invalid_length() {
        for length in [0, 79, 54_987, 54_989] {
            let mut bytes = valid_bytes();
            bytes.resize(length, 0);
            assert!(
                matches!(Pst::decode(&bytes), Err(Error::InvalidLength { expected: 54_988, actual }) if actual == length)
            );
        }
    }

    /// MNPT識別子の不一致が拒否されることを検査する。
    #[test]
    fn decode_rejects_invalid_magic() {
        let mut bytes = valid_bytes();
        bytes[0] ^= 1;
        assert!(matches!(
            Pst::decode(&bytes),
            Err(Error::InvalidMagic { .. })
        ));
    }

    /// 未対応のMNPT版が拒否されることを検査する。
    #[test]
    fn decode_rejects_unsupported_version() {
        let mut bytes = valid_bytes();
        bytes[4..8].copy_from_slice(&1_u32.to_le_bytes());
        assert!(matches!(
            Pst::decode(&bytes),
            Err(Error::UnsupportedVersion { actual: 1 })
        ));
    }

    /// MNPT特徴数の不一致が拒否されることを検査する。
    #[test]
    fn decode_rejects_unexpected_feature_count() {
        let mut bytes = valid_bytes();
        bytes[8] ^= 1;
        assert!(matches!(
            Pst::decode(&bytes),
            Err(Error::UnexpectedFeatureCount { .. })
        ));
    }

    /// 規則セット名欄のNUL埋め違反が拒否されることを検査する。
    #[test]
    fn decode_rejects_invalid_rule_set_field() {
        let mut bytes = valid_bytes();
        bytes[47] = b'x';
        assert!(matches!(Pst::decode(&bytes), Err(Error::InvalidRuleSet)));
    }

    /// MNPT重み本体の改変がSHA-256で拒否されることを検査する。
    #[test]
    fn decode_rejects_checksum_mismatch() {
        let mut bytes = valid_bytes();
        bytes[HEADER_LENGTH] ^= 1;
        assert!(matches!(Pst::decode(&bytes), Err(Error::ChecksumMismatch)));
    }

    /// v0初期重みが先獅子のない局面の駒価値差と一致することを検査する。
    #[test]
    fn initialized_pst_matches_material_evaluation() {
        let pst = Pst::decode(include_bytes!("../../nets/pst-init.bin")).unwrap();
        let promoted_gold = PieceCode::new_promoted(Color::Black, PieceKind::GoldGeneral).unwrap();
        let promoted_lion = PieceCode::new_promoted(Color::White, PieceKind::Lion).unwrap();
        let positions = [
            Position::initial(),
            position_from_codes(
                Color::Black,
                &[
                    (sq(1, 2), promoted_gold),
                    (sq(7, 8), promoted_lion),
                    (
                        sq(4, 6),
                        PieceCode::new(Color::White, PieceKind::King).unwrap(),
                    ),
                ],
            ),
            position_from_codes(
                Color::White,
                &[
                    (
                        sq(0, 0),
                        PieceCode::new(Color::Black, PieceKind::Pawn).unwrap(),
                    ),
                    (
                        sq(11, 11),
                        PieceCode::new(Color::White, PieceKind::FreeKing).unwrap(),
                    ),
                ],
            ),
        ];
        for position in positions {
            assert_eq!(evaluate(&pst, &position), material_score(&position));
        }
    }

    /// 初期PSTに格納された全駒価値が固定した表と一致することを検査する。
    #[test]
    fn initialized_pst_derives_the_frozen_piece_values() {
        let pst = Pst::decode(include_bytes!("../../nets/pst-init.bin")).unwrap();
        for piece in reachable_piece_states() {
            let kind = piece.kind().unwrap();
            if !matches!(kind, PieceKind::King | PieceKind::CrownPrince) {
                assert_eq!(pst.piece_value(piece), piece_value(kind), "piece={piece:?}");
            }
        }

        let king = PieceCode::new(Color::Black, PieceKind::King).unwrap();
        let prince = PieceCode::new_promoted(Color::Black, PieceKind::CrownPrince).unwrap();
        assert_eq!(pst.piece_value(king), 2_600);
        assert_eq!(pst.piece_value(prince), 2_600);
    }

    /// 埋め込みPSTの盤上駒価値が正で、王駒と余裕値が設計式を満たすことを検査する。
    #[test]
    fn embedded_pst_piece_values_satisfy_search_invariants() {
        let pst = Pst::decode(include_bytes!("../../nets/pst.bin")).unwrap();
        let pawn = PieceCode::new(Color::Black, PieceKind::Pawn).unwrap();
        let king = PieceCode::new(Color::Black, PieceKind::King).unwrap();
        let prince = PieceCode::new_promoted(Color::Black, PieceKind::CrownPrince).unwrap();
        let max_non_royal = reachable_piece_states()
            .into_iter()
            .filter(|piece| !matches!(piece.kind(), Some(PieceKind::King | PieceKind::CrownPrince)))
            .map(|piece| {
                let value = pst.piece_value(piece);
                assert!(value > 0, "piece={piece:?}, value={value}");
                value
            })
            .max()
            .unwrap();

        assert_eq!(pst.piece_value(king), max_non_royal + pst.pawn_value());
        assert_eq!(pst.piece_value(prince), max_non_royal + pst.pawn_value());
        assert_eq!(pst.pawn_value(), pst.piece_value(pawn));
        assert_eq!(pst.delta_margin(), 2 * pst.piece_value(pawn));
    }

    /// 盤上に現れ得る非王駒の格納値が0なら復号を拒否することを検査する。
    #[test]
    fn decode_rejects_non_positive_reachable_piece_value() {
        let mut bytes = valid_bytes();
        let pawn = PieceCode::new(Color::Black, PieceKind::Pawn).unwrap();
        let state = piece_state(pawn);
        set_piece_value(&mut bytes, state, 0);
        refresh_checksum(&mut bytes);

        assert!(matches!(
            Pst::decode(&bytes),
            Err(Error::NonPositivePieceValue {
                kind: PieceKind::Pawn,
                promoted: false,
                value: 0,
            })
        ));
    }

    /// 同一端点では駒数によらず生重み和を8で割った値に一致する。
    #[test]
    fn identical_endpoints_match_single_table_evaluation() {
        {
            let pst = Pst::decode(&valid_bytes()).unwrap();
            assert_eq!(pst.weights[0], pst.weights[1]);
            for count in [0, 1, 2, 3, 47, 92, 93, 144] {
                for side in Color::ALL {
                    let mut position = position_with_count(count, side);
                    let trigger = Square::all()
                        .find(|&square| {
                            position
                                .piece_at(square)
                                .is_none_or(|piece| piece.color() != Some(side))
                        })
                        .unwrap();
                    position.set_lion_capture(Some(trigger)).unwrap();
                    let mut sum = 0_i32;
                    active_features(&position, |feature| {
                        sum += i32::from(pst.weights[0][feature])
                    });
                    assert_evaluation(&pst, &position, (sum / 8).clamp(-28_999, 28_999));
                }
            }
        }
    }

    /// 仕様の係数0、45、90と範囲外の駒数を、異なる端点の生重み和で照合する。
    #[test]
    fn distinct_endpoints_follow_phase_boundaries_and_exclude_lion_feature() {
        let pst = distinct_pst();
        for (count, q) in [
            (0, 0),
            (1, 0),
            (2, 0),
            (3, 1),
            (47, 45),
            (92, 90),
            (93, 90),
            (144, 90),
        ] {
            for side in Color::ALL {
                for with_lion in [false, true] {
                    let mut position = if count == 92 {
                        let initial = Position::initial();
                        let pieces: Vec<_> = Square::all()
                            .filter_map(|square| {
                                initial.piece_at(square).map(|piece| (square, piece))
                            })
                            .collect();
                        position_from_codes(side, &pieces)
                    } else {
                        position_with_count(count, side)
                    };
                    if with_lion {
                        let trigger = Square::all()
                            .find(|&square| {
                                position
                                    .piece_at(square)
                                    .is_none_or(|piece| piece.color() != Some(side))
                            })
                            .unwrap();
                        position.set_lion_capture(Some(trigger)).unwrap();
                    }
                    let mut sums = [0_i64; 2];
                    active_features(&position, |feature| {
                        sums[0] += i64::from(pst.weights[0][feature]);
                        sums[1] += i64::from(pst.weights[1][feature]);
                    });
                    let expected =
                        ((q * sums[0] + (90 - q) * sums[1]) / 720).clamp(-28_999, 28_999) as i32;
                    assert_evaluation(&pst, &position, expected);
                    assert_eq!(pst.refresh_accumulator(&position).piece_count, count as u32);
                }
            }
        }
    }

    /// 分子が負でも0方向へ切り捨て、端点ごとの除算を行わない。
    #[test]
    fn interpolation_divides_once_and_truncates_toward_zero() {
        let position = position_with_count(3, Color::Black);
        let square = position.occupied().into_iter().next().unwrap();
        let feature = feature_index(Color::Black, position.piece_at(square).unwrap(), square);
        // 3枚ならq=1。分子はmg + 89*egであり、±719と±720を境界として検査する。
        for (mg, eg, expected) in [
            (-7, -8, 0),
            (-8, -8, -1),
            (7, 8, 0),
            (8, 8, 1),
            (-9, -7, 0),
            (9, 7, 0),
        ] {
            let mut bytes = valid_bytes();
            bytes[HEADER_LENGTH..HEADER_LENGTH + FEATURE_COUNT * 4].fill(0);
            set_weight(&mut bytes, 0, feature, mg);
            set_weight(&mut bytes, 1, feature, eg);
            refresh_checksum(&mut bytes);
            assert_evaluation(&Pst::decode(&bytes).unwrap(), &position, expected);
        }
    }

    /// 最大絶対値の重みでも中間と序中盤端点で評価上限に収まる。
    #[test]
    fn interpolation_clips_both_signs() {
        for (mg, eg, expected) in [
            (i16::MAX, i16::MAX - 1, 28_999),
            (i16::MIN, i16::MIN + 1, -28_999),
        ] {
            let mut bytes = valid_bytes();
            for feature in 0..FEATURE_COUNT {
                set_weight(&mut bytes, 0, feature, mg);
                set_weight(&mut bytes, 1, feature, eg);
            }
            refresh_checksum(&mut bytes);
            let pst = Pst::decode(&bytes).unwrap();
            for count in [47, 92, 144] {
                assert_evaluation(&pst, &position_with_count(count, Color::Black), expected);
            }
        }
    }

    /// 後手番の段反転と陣営交換は、先獅子特徴も含めて評価を保存する。
    #[test]
    fn evaluation_matches_rank_reflection_with_colors_swapped() {
        let pst = distinct_pst();
        for count in [2, 47, 92, 93] {
            let mut position = position_with_count(count, Color::White);
            position.set_lion_capture(Some(sq(3, 5))).unwrap();
            let pieces: Vec<_> = Square::all()
                .filter_map(|square| {
                    position.piece_at(square).map(|piece| {
                        let color = piece.color().unwrap().opposite();
                        let kind = piece.kind().unwrap();
                        let reflected_piece = if piece.is_promoted() {
                            PieceCode::new_promoted(color, kind).unwrap()
                        } else {
                            PieceCode::new(color, kind).unwrap()
                        };
                        (sq(square.file(), 11 - square.rank()), reflected_piece)
                    })
                })
                .collect();
            let mut reflected = position_from_codes(Color::Black, &pieces);
            reflected.set_lion_capture(Some(sq(3, 6))).unwrap();
            let expected = evaluate(&pst, &position);
            assert_evaluation(&pst, &position, expected);
            assert_evaluation(&pst, &reflected, expected);
        }
    }

    /// 勝率尺度は正かつ有限の値だけを受け入れる。
    #[test]
    fn decode_rejects_invalid_k() {
        for k in [0.0_f32, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut bytes = valid_bytes();
            bytes[12..16].copy_from_slice(&k.to_le_bytes());
            assert!(matches!(Pst::decode(&bytes), Err(Error::InvalidK { .. })));
        }
    }

    /// 王と太子は最大非王駒価値と歩価値の和に厳密に一致する必要がある。
    #[test]
    fn decode_rejects_inconsistent_royal_values() {
        for kind in [PieceKind::King, PieceKind::CrownPrince] {
            for actual in [0, 2_599, 2_601, 28_999] {
                let mut bytes = valid_bytes();
                set_piece_value(&mut bytes, kind.index(), actual);
                refresh_checksum(&mut bytes);
                assert!(
                    matches!(Pst::decode(&bytes), Err(Error::InconsistentRoyalValue { kind: found, expected: 2_600, actual: value }) if found == kind && value == actual)
                );
            }
        }
    }

    /// 未到達状態を含む全47値に探索上の数値範囲を適用する。
    #[test]
    fn decode_rejects_out_of_range_piece_values() {
        for state in 0..PIECE_STATE_COUNT {
            for value in [-1, 29_000, i32::MIN, i32::MAX] {
                let mut bytes = valid_bytes();
                set_piece_value(&mut bytes, state, value);
                refresh_checksum(&mut bytes);
                assert!(
                    matches!(Pst::decode(&bytes), Err(Error::PieceValueOutOfRange { state: found, value: actual }) if found == state && actual == value)
                );
            }
        }
    }

    /// 検査和は両端点と探索用駒価値を含む本体全体を対象にする。
    #[test]
    fn checksum_covers_both_endpoints_and_piece_values() {
        let bytes = valid_bytes();
        assert_eq!(bytes.len(), 54_988);
        let pst = Pst::decode(&bytes).unwrap();
        let expected: [u8; 32] = Sha256::digest(&bytes[80..]).into();
        assert_eq!(pst.checksum(), &expected);
        assert_eq!(
            pst.k(),
            f32::from_le_bytes(bytes[12..16].try_into().unwrap())
        );
        for offset in [80, 80 + 13_680 * 2, 80 + 13_680 * 4, bytes.len() - 1] {
            let mut corrupt = bytes.clone();
            corrupt[offset] ^= 1;
            assert!(matches!(
                Pst::decode(&corrupt),
                Err(Error::ChecksumMismatch)
            ));
        }
    }

    /// 全到達可能な非王駒状態の0を拒否し、端点の変更では探索用駒価値を変えない。
    #[test]
    fn stored_piece_values_are_validated_independently_of_weights() {
        let initial = Pst::decode(&valid_bytes()).unwrap();
        let distinct = distinct_pst();
        assert_eq!(initial.piece_values, distinct.piece_values);
        assert_eq!(initial.delta_margin(), distinct.delta_margin());
        for piece in reachable_piece_states() {
            if matches!(piece.kind(), Some(PieceKind::King | PieceKind::CrownPrince)) {
                continue;
            }
            let mut bytes = valid_bytes();
            set_piece_value(&mut bytes, piece_state(piece), 0);
            refresh_checksum(&mut bytes);
            assert!(matches!(
                Pst::decode(&bytes),
                Err(Error::NonPositivePieceValue { value: 0, .. })
            ));
        }
    }

    /// 埋め込み重みが復号でき、初期局面評価がPython学習器と一致することを検査する。
    #[test]
    fn embedded_pst_matches_python_initial_position_evaluation() {
        assert_eq!(evaluate(&weights().unwrap(), &Position::initial()), 67);
    }
}
