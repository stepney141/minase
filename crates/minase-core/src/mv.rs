//! 着手の表現と、その巻き戻しに必要な記録。

use crate::board::Square;

/// 1回の着手を表す正準形（RULES.md第3条）。
///
/// 正準形の不変条件は、`mid == Some(m)`なら着手前の`m`に相手駒があり、
/// 第1段階でその駒を取ることである。
/// 空升を経由する2段階移動は、同じ到達升への直接の跳びとして`mid: None`で表す。
/// じっとは`mid: None`かつ`to == from`、居喰いは`mid: Some(m)`かつ`to == from`である。
/// 1段階の移動、走り、麒麟と鳳凰の固定跳び、および獅子力の直接跳びは、
/// いずれも`mid: None`かつ`to != from`で表し、区別しない。
///
/// 捕獲する駒は保持せず、適用前の局面と経由升、および`to != from`のときの到達升から導く。
/// この型自体は経路の幾何を検査しない。
/// [`MoveGenerator`](crate::MoveGenerator)が正準形と局面合法性を保証する。
/// この表現を採る理由は`docs/adr/represent-moves-in-canonical-form.md`にある。
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
