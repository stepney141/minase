//! 先獅子の状態の参照と設定。

use super::zobrist::zobrist_keys;
use super::{Position, PositionError};
use crate::core::board::Square;
use crate::core::piece::PieceKind;

/// 先獅子の発動原因となった直前の獅子捕獲。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct LionTrigger {
    /// 獅子が取られた升。
    pub(crate) square: Square,
    /// 獅子を取った麒麟が同じ着手で成ったかどうか。
    pub(crate) by_kirin_promotion: bool,
}

impl Position {
    /// 直前の着手で獅子以外の駒が相手獅子を取った升を返す。
    #[inline]
    pub fn lion_capture_square(&self) -> Option<Square> {
        self.lion_taken_by_non_lion.map(|trigger| trigger.square)
    }

    /// 拡張SFENの獅子捕獲升から先獅子状態を設定する。
    ///
    /// 空升は、角鷹または飛鷲が経由升で獅子を取った状態として受理する。
    /// 非手番側の麒麟由来の成獅子が指定升にあれば、同一升での取り返しに
    /// 必要な原因も復元する(第15条・第29条L2)。
    ///
    /// # エラー
    ///
    /// 指定升に手番側の駒がある場合は
    /// [`PositionError::InvalidLionCapture`]を返し、局面を変更しない。
    pub fn set_lion_capture(&mut self, lion_capture: Option<Square>) -> Result<(), PositionError> {
        let trigger = match lion_capture {
            None => None,
            Some(square) => {
                let piece = self.piece_at(square);
                if piece.is_some_and(|piece| piece.color() == Some(self.side_to_move)) {
                    return Err(PositionError::InvalidLionCapture { square });
                }
                Some(LionTrigger {
                    square,
                    by_kirin_promotion: piece.is_some_and(|piece| {
                        piece.color() == Some(self.side_to_move.opposite())
                            && piece.kind() == Some(PieceKind::Lion)
                            && piece.is_promoted()
                    }),
                })
            }
        };

        let keys = zobrist_keys();
        self.zobrist ^=
            keys.lion_trigger_state(self.lion_taken_by_non_lion) ^ keys.lion_trigger_state(trigger);
        self.lion_taken_by_non_lion = trigger;
        Ok(())
    }

    /// 直前の着手で獅子以外の駒に取られた獅子の情報を返す。先獅子(第15条)の判定に使う。
    pub(crate) const fn lion_taken_by_non_lion(&self) -> Option<LionTrigger> {
        self.lion_taken_by_non_lion
    }
}
