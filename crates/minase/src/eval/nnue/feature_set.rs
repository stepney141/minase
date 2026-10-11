//! 基底特徴を、視点ごとの文脈に応じて第1層の行へ写す。

use crate::eval::features::{FEATURE_COUNT, mirror_feature};
use minase_core::position::Undo;
use minase_core::{Color, PieceKind, Position};

/// 左右鏡映に対する非線形経路の扱い。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mirror {
    /// 対応する特徴のユニットを半幅だけ交換して重みを共有する。
    Shared,
    /// 文脈で盤面を正規化する。
    Normalized,
}

/// 入力特徴集合の名前、文脈および基底特徴からの写像。
pub trait FeatureSet: Clone + PartialEq + Eq + std::fmt::Debug {
    /// 構造記述文字列での名前。
    const NAME: &'static str;
    /// 第1層の入力行数。
    const DIMENSIONS: usize;
    /// 左右鏡映の扱い。
    const MIRROR: Mirror;
    /// 1視点の入力特徴を定める文脈。
    type Context: Copy + Default + PartialEq + Eq + std::fmt::Debug;

    /// 局面から指定視点の文脈を求める。
    fn context(position: &Position, perspective: Color) -> Self::Context;
    /// 文脈が変わり得る着手だけを調べ、着手後の文脈を返す。
    fn context_after_move(
        before: Self::Context,
        position: &Position,
        perspective: Color,
        undo: &Undo,
    ) -> Self::Context;
    /// 基底特徴を第1層の行番号へ写す。
    fn index(context: Self::Context, base: usize) -> usize;
    /// 特徴分解の仮想行に対応する基底特徴番号を返す。
    fn base_index(feature: usize) -> usize;
}

/// 王の位置で条件付けない入力特徴集合。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct P;

impl FeatureSet for P {
    const NAME: &'static str = "P";
    const DIMENSIONS: usize = FEATURE_COUNT;
    const MIRROR: Mirror = Mirror::Shared;
    type Context = ();

    #[inline]
    fn context(_: &Position, _: Color) {}

    #[inline]
    fn context_after_move(_: (), _: &Position, _: Color, _: &Undo) {}

    #[inline]
    fn index((): (), base: usize) -> usize {
        base
    }

    fn base_index(feature: usize) -> usize {
        feature
    }
}

/// 自玉の位置を8区分に分け、左右鏡映で正規化する入力特徴集合。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HalfKpHmB8;

/// 基準の王駒による区分と左右鏡映。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct HalfKpContext {
    pub(super) bucket: usize,
    pub(super) mirrored: bool,
}

impl FeatureSet for HalfKpHmB8 {
    const NAME: &'static str = "HalfKP-hm-b8";
    const DIMENSIONS: usize = 8 * FEATURE_COUNT;
    const MIRROR: Mirror = Mirror::Normalized;
    type Context = HalfKpContext;

    fn context(position: &Position, perspective: Color) -> Self::Context {
        let anchor = [PieceKind::King, PieceKind::CrownPrince]
            .into_iter()
            .find_map(|kind| {
                position
                    .pieces_of_kind(perspective, kind)
                    .into_iter()
                    .min_by_key(|square| square.dense_index())
            });
        let Some(anchor) = anchor else {
            return HalfKpContext {
                bucket: 0,
                mirrored: false,
            };
        };
        let rank = match perspective {
            Color::Black => anchor.rank(),
            Color::White => 11 - anchor.rank(),
        };
        let mirrored = anchor.file() < 6;
        let file = if mirrored {
            11 - anchor.file()
        } else {
            anchor.file()
        };
        let bucket = match rank {
            0 => usize::from(11 - file),
            1 => 6,
            _ => 7,
        };
        HalfKpContext { bucket, mirrored }
    }

    fn context_after_move(
        before: Self::Context,
        position: &Position,
        perspective: Color,
        undo: &Undo,
    ) -> Self::Context {
        let is_royal = |piece: minase_core::PieceCode| {
            matches!(piece.kind(), Some(PieceKind::King | PieceKind::CrownPrince))
        };
        let changed = is_royal(undo.moved_piece_before)
            || (undo.mv.promote
                && undo.moved_piece_before.kind() == Some(PieceKind::DrunkElephant))
            || undo
                .captured
                .iter()
                .flatten()
                .any(|captured| is_royal(captured.piece));
        if changed {
            Self::context(position, perspective)
        } else {
            before
        }
    }

    #[inline]
    fn index(context: Self::Context, base: usize) -> usize {
        context.bucket * FEATURE_COUNT
            + if context.mirrored {
                mirror_feature(base)
            } else {
                base
            }
    }

    fn base_index(feature: usize) -> usize {
        feature % FEATURE_COUNT
    }
}
