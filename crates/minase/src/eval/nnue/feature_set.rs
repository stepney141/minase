//! 基底特徴を、視点ごとの文脈に応じて第1層の行へ写す。

use crate::eval::features::{FEATURE_COUNT, mirror_feature};
use minase_core::position::Undo;
use minase_core::{Color, PieceKind, Position};

/// 左右鏡映に対する非線形の経路の扱い。
/// 線形の経路はどちらの方式でも鏡映対で重みを共有する。
/// 理由は`docs/adr/share-p-type-mirror-weights-by-half-width-rotation.md`にある。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mirror {
    /// 鏡映対の特徴はユニットjとj+H/2を交換して第1層の行を共有する。
    /// バイアスと出力重みも半幅で共有するため、鏡映で累算値の両半分が入れ替わっても評価は変わらない。
    Shared,
    /// 文脈に応じて盤面を鏡映して番号を付け、非線形の重みには共有を課さない。
    Normalized,
}

/// 入力特徴集合の名前、文脈、および基底特徴から第1層の行への写像。
/// 特徴番号は`文脈の区分番号×13,680 + 文脈に応じて写した基底特徴番号`とする。
/// 1視点では144升の駒と先獅子1升の最大145個が発火する。
/// [`Self::context_after_move`]で文脈が変わった視点は、第1層を差分更新せずバイアスから全再計算する。
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
/// 文脈は1種類、特徴番号は基底特徴の番号そのもので、特徴数は13,680、鏡映は[`Mirror::Shared`]である。
/// 理由は`docs/adr/use-p-type-input-features-for-the-nnue.md`にある。
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

/// 自陣の王駒の位置を8区分に分け、左右鏡映で正規化する109,440特徴の入力集合。
/// 基準は自陣の王将、なければ太子であり、同種が複数なら`dense_index`が最小の升を選ぶ。
/// 王駒がなければ区分0で鏡映しない。
/// 視点から見た段rは先手なら元の段、後手なら`11 − 段`である。
/// 基準の王駒の筋fが6未満なら、全駒と先獅子の筋を`11 − f`へ鏡映する。
/// 鏡映後の王駒の筋f′に対し、r=0の区分は`11−f′`、r=1は6、r≥2は7とする。
/// 最下段の区分0は端、5は中央寄りである。
/// 両陣営で基準の王駒が一意の局面に限って、左右鏡映に対する評価の不変性を保証する。
/// P型との比較では採用されなかったが、再比較のために残す。
/// 理由は`docs/adr/use-p-type-input-features-for-the-nnue.md`にある。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HalfKpHmB8;

/// 基準の王駒による区分と左右鏡映。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct HalfKpContext {
    pub(super) bucket: usize,
    pub(super) mirrored: bool,
}

impl HalfKpContext {
    /// 基準の王駒から定めた区分番号を0以上8未満で返す。
    pub const fn bucket(&self) -> usize {
        self.bucket
    }

    /// 特徴番号の計算に左右鏡映を適用するかどうかを返す。
    pub const fn mirrored(&self) -> bool {
        self.mirrored
    }
}

impl FeatureSet for HalfKpHmB8 {
    const NAME: &'static str = "HalfKP-hm-b8";
    const DIMENSIONS: usize = 8 * FEATURE_COUNT;
    const MIRROR: Mirror = Mirror::Normalized;
    type Context = HalfKpContext;

    /// [`HalfKpHmB8`]の基準の王駒から区分と鏡映方向を求める。
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

    /// 王将または太子の移動、醉象の成り、王駒の捕獲だけで文脈を求め直す。
    /// それ以外の着手では着手前の文脈を返す。
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
