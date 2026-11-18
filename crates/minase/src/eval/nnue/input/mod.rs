//! 入力特徴集合の仕様と、基底特徴から第1層の行への番号付け。

mod half_kp_vm;
mod p;
pub use half_kp_vm::{HalfKpVm, HalfKpVmContext};
pub use p::P;

use crate::eval::features::active_features_for;
use minase_core::{Color, Position};
use std::fmt;

/// 対応する入力特徴集合。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureSet {
    /// 王将を含む駒と先獅子の特徴。
    P,
    /// 王将を除く駒と先獅子を、自陣の基準王駒の72区分に分ける特徴。
    HalfKpVm,
}

impl FeatureSet {
    /// 対応する入力特徴集合の全種類。
    pub const ALL: [FeatureSet; 2] = [Self::P, Self::HalfKpVm];

    /// 入力特徴集合の仕様を構築する唯一の関数。
    pub const fn spec(self) -> FeatureSetSpec {
        let (arch_name, king_buckets, piece_inputs, mirror, factorized) = match self {
            Self::P => ("P", 1, 13_680, Mirror::HalfWidthRotation, false),
            Self::HalfKpVm => ("HalfKPVm", 72, 13_392, Mirror::KingFile, true),
        };
        FeatureSetSpec {
            feature_set: self,
            arch_name,
            king_buckets,
            piece_inputs,
            max_active: 145,
            feature_hash: fnv1a32(arch_name.as_bytes()),
            mirror,
            factorized,
        }
    }

    /// 構造記述での名前から引く。大文字と小文字を区別し、未知の名前は`None`を返す。
    pub fn from_arch_name(name: &str) -> Option<FeatureSet> {
        Self::ALL
            .into_iter()
            .find(|set| set.spec().arch_name() == name)
    }
}

/// 入力の次元、発火数、鏡映、および学習時の特徴分解の仕様。
/// フィールドは非公開であり、[`FeatureSet::spec`]だけが構築する。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeatureSetSpec {
    feature_set: FeatureSet,
    arch_name: &'static str,
    king_buckets: usize,
    piece_inputs: usize,
    max_active: usize,
    feature_hash: u32,
    mirror: Mirror,
    factorized: bool,
}

impl FeatureSetSpec {
    /// 対応する入力特徴集合を返す。
    pub const fn feature_set(&self) -> FeatureSet {
        self.feature_set
    }
    /// 構造記述での名前を返す。
    pub const fn arch_name(&self) -> &'static str {
        self.arch_name
    }
    /// 基準の王駒の升をまとめる区分の数を返す。
    pub const fn king_buckets(&self) -> usize {
        self.king_buckets
    }
    /// 区分1つあたりの基底特徴の数を返す。
    pub const fn piece_inputs(&self) -> usize {
        self.piece_inputs
    }
    /// `king_buckets × piece_inputs`で求めた第1層の行数を返す。
    pub const fn dimensions(&self) -> usize {
        self.king_buckets * self.piece_inputs
    }
    /// 1局面の1視点で値が1になる特徴の数の上限を返す。
    pub const fn max_active(&self) -> usize {
        self.max_active
    }
    /// 構造記述での名前のFNV-1a 32ビットハッシュを返す。
    pub const fn feature_hash(&self) -> u32 {
        self.feature_hash
    }
    /// 左右鏡映の扱いを返す。
    pub const fn mirror(&self) -> Mirror {
        self.mirror
    }
    /// 学習時だけ王の位置によらない仮想行を加えるかを返す。
    pub const fn factorized(&self) -> bool {
        self.factorized
    }
}

/// 入力特徴集合の左右鏡映の扱い。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mirror {
    /// P型では左右鏡映の対の特徴の第1層の行を半幅回して共有する。
    /// 設計の根拠は`docs/adr/share-p-type-mirror-weights-in-the-first-layer-and-the-psqt.md`にある。
    HalfWidthRotation,
    /// HalfKP_vmでは基準の王駒の筋で盤を左右鏡映して番号を付ける。
    KingFile,
}

const fn fnv1a32(bytes: &[u8]) -> u32 {
    let mut hash = 0x811C_9DC5_u32;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u32;
        hash = hash.wrapping_mul(0x0100_0193);
        index += 1;
    }
    hash
}

/// 入力特徴集合ごとの第1層の行の番号付け。
pub trait Indexer {
    /// 対応する公開の入力特徴集合。
    const FEATURE_SET: FeatureSet;
    /// 1視点の番号付けを決める値。
    type Context: Copy + PartialEq + Eq + fmt::Debug;
    /// 局面から指定視点の文脈を求める。
    fn context(position: &Position, perspective: Color) -> Result<Self::Context, AnchorError>;
    /// `eval::features`の基底特徴番号を第1層の行へ写す。含めない特徴は`None`。
    ///
    /// # Panics
    /// `base`が13,680以上ならパニックする。
    fn row(context: Self::Context, base: usize) -> Option<usize>;
    /// 根からの探索中、終局以外の全局面で両視点の文脈を一意に決められるかを検査する。
    fn check_search_root(position: &Position) -> Result<(), AnchorError>;
}

/// 指定視点で有効な基底特徴を列挙し、入力に含める第1層の行だけを渡す。
pub fn for_each_active_row<I: Indexer>(
    context: I::Context,
    perspective: Color,
    position: &Position,
    mut f: impl FnMut(usize),
) {
    active_features_for(perspective, position, |base| {
        if let Some(row) = I::row(context, base) {
            f(row);
        }
    });
}

/// 基準の王駒を一意に決められない原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorError {
    /// 王将も太子もない。
    MissingRoyal {
        /// 対象の陣営。
        color: Color,
    },
    /// 王将が2枚以上ある。
    MultipleKings {
        /// 対象の陣営。
        color: Color,
    },
    /// 王将がなく、太子が2枚以上ある。
    MultiplePrinces {
        /// 対象の陣営。
        color: Color,
    },
    /// 太子と成っていない醉象の合計が2枚以上ある。探索の根の検査だけが返す。
    PrinceCandidates {
        /// 対象の陣営。
        color: Color,
    },
}

impl fmt::Display for AnchorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRoyal { color } => write!(f, "{color:?} has no king or crown prince"),
            Self::MultipleKings { color } => write!(f, "{color:?} has multiple kings"),
            Self::MultiplePrinces { color } => {
                write!(f, "{color:?} has multiple crown princes without a king")
            }
            Self::PrinceCandidates { color } => {
                write!(f, "{color:?} has multiple crown prince candidates")
            }
        }
    }
}
impl std::error::Error for AnchorError {}

#[cfg(test)]
mod tests;
