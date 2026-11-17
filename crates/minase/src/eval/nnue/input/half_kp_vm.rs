use super::{AnchorError, FeatureSet, Indexer};
use crate::eval::features::{BOARD_FEATURE_COUNT, FEATURE_COUNT, PIECE_STATE_COUNT};
use minase_core::{Color, PieceKind, Position};

/// 王将を除く特徴を、基準の王駒による72区分へ写す番号付け。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HalfKpVm;

/// 基準の王駒で決まる区分と、盤全体の左右鏡映の有無。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HalfKpVmContext {
    bucket: u8,
    mirrored: bool,
}

impl HalfKpVmContext {
    /// 0以上72未満の区分の番号を返す。
    pub const fn bucket(self) -> usize {
        self.bucket as usize
    }
    /// 筋を`11 − 筋`へ写すかを返す。
    pub const fn mirrored(self) -> bool {
        self.mirrored
    }
}

impl Indexer for HalfKpVm {
    const FEATURE_SET: FeatureSet = FeatureSet::HalfKpVm;
    type Context = HalfKpVmContext;

    /// 自陣の王将1枚を優先し、なければ太子1枚を基準にする。
    /// 王将の重複、王将がない場合の太子の重複、王駒の欠落はそれぞれ拒否する。
    /// 後手では段だけを反転し、基準の筋が6未満なら左右鏡映する。
    /// 鏡映後の筋をf、視点から見た段をrとして、区分は`6r + (11 − f)`となる。
    fn context(position: &Position, perspective: Color) -> Result<Self::Context, AnchorError> {
        let kings = position.pieces_of_kind(perspective, PieceKind::King);
        let anchors = match kings.popcount() {
            0 => {
                let princes = position.pieces_of_kind(perspective, PieceKind::CrownPrince);
                match princes.popcount() {
                    0 => return Err(AnchorError::MissingRoyal { color: perspective }),
                    1 => princes,
                    _ => return Err(AnchorError::MultiplePrinces { color: perspective }),
                }
            }
            1 => kings,
            _ => return Err(AnchorError::MultipleKings { color: perspective }),
        };
        let anchor = anchors
            .into_iter()
            .next()
            .expect("exactly one anchor was checked");
        let mirrored = anchor.file() < 6;
        let file = if mirrored {
            11 - anchor.file()
        } else {
            anchor.file()
        };
        let rank = match perspective {
            Color::Black => anchor.rank(),
            Color::White => 11 - anchor.rank(),
        };
        Ok(HalfKpVmContext {
            bucket: 6 * rank + (11 - file),
            mirrored,
        })
    }

    /// 状態11の両王将を除き、状態sを`s < 11`ならs、それ以外は`s − 1`へ詰める。
    /// 盤面の行は`区分×13,392 + (相対陣営×46 + 詰めた状態)×144 + 相対段×12 + 鏡映後の筋`。
    /// 先獅子の行は`区分×13,392 + 13,248 + 相対段×12 + 鏡映後の筋`となる。
    ///
    /// # Panics
    /// `base`が13,680以上ならパニックする。
    fn row(context: Self::Context, base: usize) -> Option<usize> {
        assert!(base < FEATURE_COUNT, "base feature out of range");
        let square = base % 144;
        let file = square % 12;
        let square = square / 12 * 12 + if context.mirrored { 11 - file } else { file };
        let piece_offset = if base < BOARD_FEATURE_COUNT {
            let state = base / 144 % PIECE_STATE_COUNT;
            if state == 11 {
                return None;
            }
            let relative_color = base / (PIECE_STATE_COUNT * 144);
            let state = if state < 11 { state } else { state - 1 };
            (relative_color * 46 + state) * 144
        } else {
            13_248
        };
        Some(context.bucket() * Self::FEATURE_SET.spec().piece_inputs() + piece_offset + square)
    }

    /// 両陣営について王将が1枚以下、太子と未成の醉象の合計が1枚以下、王駒が1枚以上か検査する。
    /// 中将棋には持ち駒がなく、王将は成りで増えず、太子は未成の醉象の成りでしか増えない。
    /// 仲人が成った醉象は再び成れないので数えない。この条件により探索中の基準王駒は重複しない。
    /// 王駒をすべて失った側は、評価の前に負けとして処理される（規則第20条、第21条）。
    fn check_search_root(position: &Position) -> Result<(), AnchorError> {
        for color in Color::ALL {
            let kings = position.pieces_of_kind(color, PieceKind::King).popcount();
            if kings > 1 {
                return Err(AnchorError::MultipleKings { color });
            }
            let princes = position
                .pieces_of_kind(color, PieceKind::CrownPrince)
                .popcount();
            let elephants = position
                .pieces_of_kind(color, PieceKind::DrunkElephant)
                .into_iter()
                .filter(|&square| {
                    !position
                        .piece_at(square)
                        .expect("piece set contains a piece")
                        .is_promoted()
                })
                .count();
            if princes as usize + elephants > 1 {
                return Err(AnchorError::PrinceCandidates { color });
            }
            if kings + princes == 0 {
                return Err(AnchorError::MissingRoyal { color });
            }
        }
        Ok(())
    }
}
