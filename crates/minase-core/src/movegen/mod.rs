//! 合法手の生成。

#[cfg(test)]
pub(crate) mod tests;

mod checked;
mod control;
mod expand;
mod generate;
mod gives_check;
mod lion;
mod lion_capture;
mod lion_like;
mod search_captures;
mod virtual_board;

pub use checked::IllegalMove;
pub use search_captures::{CaptureCache, CaptureCandidate, OrdinaryCapturer};

use crate::attacks::{AttackTables, attack_tables};
use crate::mv::Move;
use crate::position::Position;
use crate::rules::MoveRules;

/// 局面合法手を列挙する生成器。
///
/// 駒の動き、捕獲、成り、獅子の捕獲制限（RULES.md第13条から第16条）、
/// および[`MoveRules`]を満たす着手を生成する。
/// 王駒の安全は合法性に含めないため、疑似合法手を適用して王の安全を確かめる段階はない
/// （第8条第3項から第5項）。
/// 反復規則R2とR3による禁止は[`Game::legal_moves`](crate::Game::legal_moves)が扱う。
/// 履歴を分離する理由は`docs/adr/keep-game-history-out-of-position-and-movegen.md`にある。
///
/// 共有する`'static`の利き表と`Copy`の[`MoveRules`]だけを持ち、`Send + Sync`である。
#[derive(Clone)]
pub struct MoveGenerator {
    /// 利きの前計算テーブル。
    tables: &'static AttackTables,
    /// 採用しているローカルルールの集合。
    rules: MoveRules,
}

impl MoveGenerator {
    /// 標準規則の生成器を作る。
    pub fn standard() -> Self {
        Self::new(MoveRules::standard())
    }

    /// 指定ルールの生成器を作る。
    pub fn new(rules: MoveRules) -> Self {
        Self {
            tables: attack_tables(),
            rules,
        }
    }

    /// `position`の手番側の全合法手を`output`へ追加する。
    ///
    /// 駒種は[`PieceKind::ALL`](crate::PieceKind::ALL)の順、同じ駒種の駒は升の生値の昇順に走査する。
    /// 各駒では、固定利き、走り、および特殊移動の第1段階で止まる手の到達升をまとめ、昇順に出す。
    /// 次に獅子では、経由升で取る2段階移動を経由升、その中で到達升の昇順に出し、
    /// 直接の跳びを到達升の昇順に出して、最後にじっとを1手だけ出す。
    /// 角鷹と飛鷲では動きの定義にある許可方向の順に居喰い、跳び、2段階捕獲を出し、
    /// 全方向の後にじっとを1手だけ出す。
    /// 各着手は不成を先、成りを後に展開し、強制成りでは成りだけを出す。
    ///
    /// 出力は[`Move`]の正準形であり、移動元へ戻る場合を除いて到達升に自駒はなく、重複を含まない。
    /// 捕獲生成と非捕獲生成は、この列から該当する手を抜き出した順序を保つ。
    pub fn generate_moves(&self, position: &Position, output: &mut Vec<Move>) {
        generate::generate_moves::<false, false>(self, position, &mut Vec::new(), output);
    }

    /// `position`の手番側の捕獲を伴う合法手を`output`へ追加する。
    ///
    /// 追加する列は、[`Self::generate_moves`]の出力を[`Position::captured_squares`]が1升以上を返す手に絞った列と、要素、重複、順序まで一致する。
    /// 正準形では`mid`を持つ手は経由升で捕獲するので、居喰いを含めて候補に残す。
    /// `mid`のない跳びと移動は到達升に相手駒がある場合だけ残し、捕獲のないじっとは除く。
    /// 獅子捕獲規則による合法性検査と成りの展開は全手生成と共通である。
    /// 同じ整列値の手は安定ソートで生成順を保つため、生成順の違いで探索の総ノード数が変わり得る。
    pub fn generate_captures(&self, position: &Position, output: &mut Vec<Move>) {
        self.generate_captures_with_scratch(position, &mut Vec::new(), output);
    }

    /// [`Self::generate_captures`]と同じ列を、呼び出し側の作業領域を使って追加する。
    ///
    /// `base_moves`は各駒の生成開始時に消去する。
    /// 手番側に駒がなければ触れないため、呼び出し後の内容を結果として使わない。
    /// 十分な容量を確保して使い回すことで、ノードごとの作業領域の確保を省く。
    pub fn generate_captures_with_scratch(
        &self,
        position: &Position,
        base_moves: &mut Vec<Move>,
        output: &mut Vec<Move>,
    ) {
        generate::generate_moves::<true, false>(self, position, base_moves, output);
    }

    /// [`Self::generate_moves`]の非捕獲手だけを、同じ順序で`output`へ追加する。
    ///
    /// 非捕獲手は、空升への1段階の移動、空升への直接の跳び、およびじっとである。
    /// 跳び越す升の駒は取らず、経由升で取る手は居喰いを含めて除く。
    /// `base_moves`は呼び出し側の作業領域であり、
    /// 消去と再利用の契約は[`Self::generate_captures_with_scratch`]と同じである。
    pub fn generate_quiets(
        &self,
        position: &Position,
        base_moves: &mut Vec<Move>,
        output: &mut Vec<Move>,
    ) {
        generate::generate_moves::<false, true>(self, position, base_moves, output);
    }

    /// 候補手が`position`で手番側の合法手かを返す。
    pub fn is_legal_move(
        &self,
        position: &Position,
        candidate: Move,
        base_moves: &mut Vec<Move>,
        moves: &mut Vec<Move>,
    ) -> bool {
        moves.clear();
        let Some(piece) = position.piece_at(candidate.from) else {
            return false;
        };
        if piece.color() != Some(position.side_to_move()) {
            return false;
        }
        let kind = piece.kind().expect("board piece must have a valid kind");
        generate::generate_piece_moves::<false, false>(
            self,
            position,
            position.side_to_move(),
            kind,
            candidate.from,
            base_moves,
            moves,
        );
        moves.contains(&candidate)
    }

    /// 利きの前計算テーブルを返す。
    pub(crate) const fn tables(&self) -> &AttackTables {
        self.tables
    }

    /// 採用しているルールの集合を返す。
    pub const fn rules(&self) -> MoveRules {
        self.rules
    }
}
