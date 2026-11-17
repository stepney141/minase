//! P型の第1層、要素積、16→30→32→1の後段、およびPSQT。

use super::*;
use crate::eval::features::{self, FEATURE_COUNT, mirror_feature};
use crate::eval::nnue::input::P;
use crate::eval::nnue::transformer::FtAccumulator;
use minase_core::{Move, Square, position::Undo};

/// LayerStackの量子化済みパラメータ。配列は詰め物のない行優先である。
/// フィールドは学習器の書き出し用に公開し、全ての不変条件は`new`で検査する。
pub struct LayerStackParameters {
    /// 第1層の幅。2〜1,024の偶数。
    pub width: usize,
    /// 評価値へ換算する正の除数。
    pub fv_scale: i32,
    /// 第1層のバイアスH個。
    pub ft_biases: Vec<i16>,
    /// 第1層の重み13,680×H個。
    pub ft_weights: Vec<i16>,
    /// PSQTのバイアス。0だけを受理する。
    pub psqt_bias: i32,
    /// 基底特徴ごとのPSQTの重み13,680個。
    pub psqt_weights: Vec<i32>,
    /// L1のバイアス16個。
    pub l1_biases: Vec<i32>,
    /// L1の重み16×H個。
    pub l1_weights: Vec<i8>,
    /// L2のバイアス32個。
    pub l2_biases: Vec<i32>,
    /// L2の重み32×30個。
    pub l2_weights: Vec<i8>,
    /// 出力のバイアス。
    pub output_bias: i32,
    /// 出力の重み32個。
    pub output_weights: Vec<i8>,
}

/// 値域と鏡映共有を検査済みのP型LayerStack。
pub struct LayerStackNetwork {
    ft: FeatureTransformer<P>,
    fv_scale: i32,
    psqt: Box<[i32]>,
    dense: Dense,
    pub(super) accumulator_stats: super::super::stats::SharedStats,
}

/// P型の第1層と、視点ごとのPSQTの和。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LayerStackAccumulator {
    ft: FtAccumulator<P>,
    psqt: [i32; 2],
}

/// 手番側の評価値と、整数の尺度を保った各経路の寄与。
pub(crate) struct LayerStackBreakdown {
    /// センチポーンでの評価値。
    pub(crate) value: i32,
    /// 出力の除数。
    pub(crate) fv_scale: i32,
    /// 後段とskipの和。
    pub(crate) dense: i32,
    /// PSQTの両視点の差を2で割った値。
    pub(crate) psqt: i32,
    /// 升ごとのQ[手番側]−Q[相手側]。dense_index順で、空升は0。
    pub(crate) squares: [i32; 144],
    /// 先獅子のQ[手番側]−Q[相手側]。
    pub(crate) lion: i32,
}

impl LayerStackNetwork {
    /// 寸法、累算値の上限、後段とPSQTの値域、P型の鏡映共有を検査して構築する。
    /// 第1層はVecの所有権をBoxへ移し、重みの複製を作らない。
    /// 鏡映対は第1層の半幅回転、バイアスの前後半の一致、PSQTの一致を要求する。
    pub fn new(p: LayerStackParameters) -> Result<Self, FormatError> {
        check_scale(p.fv_scale)?;
        validate_ft(FeatureSet::P, p.width, &p.ft_biases, &p.ft_weights)?;
        check_len("psqt_weights", FEATURE_COUNT, p.psqt_weights.len())?;
        if p.psqt_bias != 0 {
            return Err(FormatError::PsqtBias { value: p.psqt_bias });
        }
        for (row, &value) in p.psqt_weights.iter().enumerate() {
            if value.unsigned_abs() > 1 << 22 {
                return Err(FormatError::PsqtRange { row, value });
            }
        }
        for component in 0..p.width / 2 {
            if p.ft_biases[component] != p.ft_biases[component + p.width / 2] {
                return Err(FormatError::MirrorBias { component });
            }
        }
        for row in 0..FEATURE_COUNT {
            let mirror = mirror_feature(row);
            for component in 0..p.width {
                if p.ft_weights[mirror * p.width + component]
                    != p.ft_weights[row * p.width + (component + p.width / 2) % p.width]
                {
                    return Err(FormatError::MirrorWeight { row, component });
                }
            }
            if p.psqt_weights[row] != p.psqt_weights[mirror] {
                return Err(FormatError::MirrorPsqt { row });
            }
        }
        let dense = Dense {
            l1_biases: p.l1_biases,
            l1_weights: p.l1_weights,
            l2_biases: p.l2_biases,
            l2_weights: p.l2_weights,
            output_bias: p.output_bias,
            output_weights: p.output_weights,
        };
        dense.validate(p.width, 16, 30)?;
        Ok(Self {
            ft: transformer(p.width, p.ft_biases, p.ft_weights),
            fv_scale: p.fv_scale,
            psqt: p.psqt_weights.into_boxed_slice(),
            dense,
            accumulator_stats: Default::default(),
        })
    }
    /// 第1層の幅を返す。
    pub fn width(&self) -> usize {
        self.ft.width()
    }
    /// 評価値へ換算する正の除数を返す。
    pub fn fv_scale(&self) -> i32 {
        self.fv_scale
    }
    /// tataraのLayerStack形式で、区分数1、PSQTありとして書き出す。
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = header(FeatureSet::P, self.width(), self.fv_scale);
        bytes.extend(1_u32.to_le_bytes());
        bytes.extend(ft_hash(FeatureSet::P, self.width()).to_le_bytes());
        leb128::encode(&mut bytes, self.ft.biases());
        leb128::encode(&mut bytes, self.ft.weights());
        write_i32(&mut bytes, &[0]);
        write_i32(&mut bytes, &self.psqt);
        bytes.extend(fc_hash(FeatureSet::P, self.width()).to_le_bytes());
        self.dense.encode(&mut bytes, self.width(), 30);
        bytes
    }
    /// tataraのLayerStack形式を読み、`new`と同じ値域・鏡映共有条件を検査する。
    pub fn decode(bytes: &[u8]) -> Result<Self, FormatError> {
        let mut r = Reader::new(bytes);
        let (width, fv_scale) = read_header(&mut r, FeatureSet::P)?;
        let buckets = r.u32("buckets")?;
        if buckets != 1 {
            return Err(FormatError::Buckets { actual: buckets });
        }
        check_hash("ft_hash", ft_hash(FeatureSet::P, width), r.u32("ft_hash")?)?;
        let ft_biases = leb128::decode(&mut r, width, "FT biases")?;
        let ft_weights = leb128::decode(&mut r, FEATURE_COUNT * width, "FT weights")?;
        let psqt_bias = r.i32s(1, "PSQT bias")?[0];
        let psqt_weights = r.i32s(FEATURE_COUNT, "PSQT weights")?;
        check_hash("fc_hash", fc_hash(FeatureSet::P, width), r.u32("fc_hash")?)?;
        let dense = Dense::decode(&mut r, width, 16, 30)?;
        r.finish()?;
        Self::new(LayerStackParameters {
            width,
            fv_scale,
            ft_biases,
            ft_weights,
            psqt_bias,
            psqt_weights,
            l1_biases: dense.l1_biases,
            l1_weights: dense.l1_weights,
            l2_biases: dense.l2_biases,
            l2_weights: dense.l2_weights,
            output_bias: dense.output_bias,
            output_weights: dense.output_weights,
        })
    }
    /// 手番側の視点で全再計算する。P型では王駒の文脈を必要としない。
    pub fn evaluate(&self, position: &Position) -> i32 {
        self.evaluate_accumulator(&self.refresh(position), position.side_to_move())
    }
    /// 探索の差分更新先として、空の累算器を確保する。
    pub(crate) fn new_accumulator(&self) -> LayerStackAccumulator {
        LayerStackAccumulator {
            ft: FtAccumulator::new(self.width()),
            psqt: [0; 2],
        }
    }
    /// 第1層とPSQTの両視点の和を全再計算する。
    pub(crate) fn refresh(&self, position: &Position) -> LayerStackAccumulator {
        let mut psqt = [0; 2];
        for side in Color::ALL {
            features::active_features_for(side, position, |row| {
                psqt[side.index()] += self.psqt[row]
            });
        }
        LayerStackAccumulator {
            ft: self.ft.refresh(position),
            psqt,
        }
    }
    /// 着手による第1層とPSQTの差分を更新する。P型の文脈は変わらないので偽を返す。
    pub(crate) fn update_after_move(
        &self,
        before: &LayerStackAccumulator,
        after: &mut LayerStackAccumulator,
        position_after: &Position,
        undo: &Undo,
    ) -> bool {
        let refreshed = self
            .ft
            .update_after_move(&before.ft, &mut after.ft, position_after, undo);
        after.psqt = before.psqt;
        for side in Color::ALL {
            features::for_each_move_change(side, position_after, undo, |row, sign| {
                after.psqt[side.index()] += sign * self.psqt[row]
            });
        }
        refreshed
    }
    /// 親の累算値を写し、null moveで消える先獅子だけを引く。
    pub(crate) fn update_after_null(
        &self,
        before: &LayerStackAccumulator,
        after: &mut LayerStackAccumulator,
        lion_before: Option<Square>,
    ) {
        self.ft
            .update_after_null(&before.ft, &mut after.ft, lion_before);
        after.psqt = before.psqt;
        for side in Color::ALL {
            features::for_each_null_change(side, lion_before, |row, sign| {
                after.psqt[side.index()] += sign * self.psqt[row]
            });
        }
    }
    /// 各視点の前後半を0〜127へ制限し、その積を7ビット右シフトして連結する。
    /// L1の15番目はskipとし、残りは`clamp(y² >> 19, 0, 127)`と
    /// `clamp(y >> 6, 0, 127)`の順に30成分へ展開する。2乗はi64で計算する。
    /// L2の32成分を6ビット右シフトして制限し、出力層の積和とskipを足す。
    fn dense(&self, acc: &LayerStackAccumulator, side: Color) -> i32 {
        let width = self.width();
        let mut storage = [0; 1024];
        let input = &mut storage[..width];
        for (part, perspective) in [side, side.opposite()].into_iter().enumerate() {
            let row = acc.ft.row(perspective).expect("P always has a context");
            for i in 0..width / 2 {
                input[part * width / 2 + i] = pair(row[i], row[i + width / 2]);
            }
        }
        let d = &self.dense;
        let mut z = [0; 30];
        for i in 0..15 {
            let y = affine(
                d.l1_biases[i],
                &d.l1_weights[i * width..(i + 1) * width],
                input,
            );
            z[i] = square_activation(y);
            z[15 + i] = activation(y);
        }
        let skip = affine(
            d.l1_biases[15],
            &d.l1_weights[15 * width..16 * width],
            input,
        );
        let h: [u8; 32] = std::array::from_fn(|k| {
            activation(affine(
                d.l2_biases[k],
                &d.l2_weights[k * 30..(k + 1) * 30],
                &z,
            ))
        });
        affine(d.output_bias, &d.output_weights, &h) + skip
    }
    /// PSQTの両視点の差を0方向に2で割り、後段に足してから`fv_scale`で割る。
    /// 最後の加算と除算はi64とし、評価値を±28,999に制限する。
    pub(crate) fn evaluate_accumulator(
        &self,
        acc: &LayerStackAccumulator,
        side_to_move: Color,
    ) -> i32 {
        score(
            i64::from(self.dense(acc, side_to_move))
                + i64::from(psqt_half(
                    acc.psqt[side_to_move.index()],
                    acc.psqt[side_to_move.opposite().index()],
                )),
            self.fv_scale,
        )
    }
    /// 静かな手の駒移動と成りによる`ΔS_stm−ΔS_nstm`を整数のまま返す。
    /// 先獅子の変化は含めない。探索から合法な静かな手だけを渡す。
    pub(crate) fn quiet_move_psqt_delta(&self, position: &Position, mv: Move) -> i64 {
        let piece = position
            .piece_at(mv.from)
            .expect("legal quiet move has a source piece");
        let moved = if mv.promote {
            piece.promote().expect("legal promotion")
        } else {
            piece
        };
        let delta = |side| {
            i64::from(self.psqt[features::feature_index(side, moved, mv.to)])
                - i64::from(self.psqt[features::feature_index(side, piece, mv.from)])
        };
        delta(position.side_to_move()) - delta(position.side_to_move().opposite())
    }
    /// 手番側の評価値、後段、PSQT、および各升と先獅子の両視点の差を返す。
    pub(crate) fn breakdown(&self, position: &Position) -> LayerStackBreakdown {
        let acc = self.refresh(position);
        let side = position.side_to_move();
        let dense = self.dense(&acc, side);
        let psqt = psqt_half(acc.psqt[side.index()], acc.psqt[side.opposite().index()]);
        let mut squares = [0; 144];
        for square in position.occupied() {
            let piece = position
                .piece_at(square)
                .expect("occupied square contains a piece");
            squares[square.dense_index()] = self.psqt[features::feature_index(side, piece, square)]
                - self.psqt[features::feature_index(side.opposite(), piece, square)];
        }
        let lion = match position.lion_taken_by_non_lion() {
            Some(trigger) => {
                self.psqt[features::lion_feature_index(side, trigger.square)]
                    - self.psqt[features::lion_feature_index(side.opposite(), trigger.square)]
            }
            None => 0,
        };
        LayerStackBreakdown {
            value: score(i64::from(dense) + i64::from(psqt), self.fv_scale),
            fv_scale: self.fv_scale,
            dense,
            psqt,
            squares,
            lion,
        }
    }
}
