//! 着手の表現と、その巻き戻しに必要な記録。

use crate::core::piece::PieceCode;
use crate::core::position::LionTrigger;
use crate::core::square::Square;

/// 1回の着手(第3条)。獅子・角鷹・飛鷲の2段階移動(第11条・第12条)も1つの値で表す。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Move {
    /// 移動元の升。
    pub from: Square,
    /// 2段階移動の経由升(第1段階の到達升)。通常の着手では`None`。
    pub mid: Option<Square>,
    /// 着手終了時の到達升。居喰い・じっとでは移動元と同じ升になる。
    pub to: Square,
    /// 着手終了時に成るかどうか。
    pub promote: bool,
}

impl Move {
    /// 移動元の升を返す。
    #[inline]
    pub(crate) const fn origin(self) -> Square {
        self.from
    }

    /// 着手終了時の到達升を返す。
    #[inline]
    pub(crate) const fn destination(self) -> Square {
        self.to
    }

    /// 成る着手かどうかを返す。
    #[inline]
    pub(crate) const fn is_promoting(self) -> bool {
        self.promote
    }

    /// 捕獲が起こりうる升(経由升と到達升)を返す。移動元へ戻る着手では到達升を候補から除く。
    pub(crate) const fn capture_candidates(self) -> [Option<Square>; 2] {
        [
            self.mid,
            if self.to.raw() == self.from.raw() {
                None
            } else {
                Some(self.to)
            },
        ]
    }
}

/// 着手で取られた駒の記録。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CapturedPiece {
    /// 駒が取られた升。
    pub square: Square,
    /// 取られた駒。
    pub piece: PieceCode,
}

/// [`Position::make_move_unchecked`](crate::Position::make_move_unchecked)の巻き戻しに必要な情報。
#[derive(PartialEq, Eq, Debug)]
pub struct Undo {
    /// 適用した着手。
    pub(crate) mv: Move,
    /// 移動前(成る前)の駒。
    pub(crate) moved_piece_before: PieceCode,
    /// 取られた駒の記録(最大2枚)。
    pub(crate) captured: [Option<CapturedPiece>; 2],
    /// 着手前の先獅子トリガー(獅子捕獲升と麒麟成りフラグ)。
    pub(crate) previous_lion_taken: Option<LionTrigger>,
    /// 着手前のzobristハッシュ。
    pub(crate) previous_zobrist: u64,
}
