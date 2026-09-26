//! 着手の表現と、その巻き戻しに必要な記録。

use crate::core::board::Square;

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
