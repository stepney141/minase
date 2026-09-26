# 中核モジュールの再編の設計書

## 先に読む要約

中核モジュール`src/core/`は、中将棋の規則をRULES.mdどおりに実装する層であり、盤、駒、局面、合法手生成、規則セット、および対局の審判を持つ。
このうち局面の`position.rs`（2,375行）、規則セットの`rules.rs`（2,143行）、審判の`game.rs`（2,289行）、および終局裁定の`adjudication.rs`（939行）は、それぞれ複数の関心事を1ファイルに抱えている。
たとえば`rules.rs`は、規則コードの文字列処理、規則セットの組み立て、成りの可否、および獅子の捕獲制限（RULES.md第13〜16条）を同居させ、試験の大半は規則セットではなく獅子の捕獲制限を検査している。
本書は、コードの目的ごとにファイルとディレクトリを分け、1ファイルが1つの関心事だけを持つ配置へ移す。
盤の座標系（升、方向、升集合）は`src/core/board/`にまとめ、局面、規則セット、駒、および審判はディレクトリへ昇格させ、成りの判定は新しい`src/core/promotion.rs`へ、獅子の捕獲制限は合法手生成の`src/core/movegen/`へ移す。
終局裁定と反復の判定は、唯一の呼び出し元である審判`game`の子モジュールにする。
再編は処理の中身を変えない移動であり、固定局面を深さ6まで探索するベンチ（`src/bin/bench.rs`）の出力と、全試験の名前と結果が再編の前後で一致することを完了条件とする。

## 状態

起案。
2026年9月27日に起案し、盤の座標系を`board/`へまとめることと、成りの判定を`promotion.rs`へ移す互換性を壊す変更を利用者が決定した。
次の一手は、masterから`core-layout`ブランチを切ってフェーズ1へ着手することである。

## 目的

規則の実装を変更するときに、変更すべきファイルをRULES.mdの条文から一意に決められる状態にする。
あわせて、非公開の欄への参照が許される範囲をディレクトリの親子関係で表し、関心事の境界をコンパイラに検査させる。

## 適用範囲

対象は`src/core/`の全体、`src/core/`の項目を参照する`src/lib.rs`、`src/eval/`、`src/search/`の`use`文、および`docs/`にある`src/core/`のパスの参照である。

関数、型、定数、および試験の名前は変えない。
処理の内容、引数、処理の順序、およびインライン属性（`#[inline]`）も変えない。
モジュールの外へ出る項目の可視性は、移動先から参照を通すのに必要な最小限だけ広げる（「可視性」の節）。

次のファイルとディレクトリは、配置を変えない。

| パス | 変えない理由 |
|---|---|
| `src/core/attacks/` | 駒の動きの定義（`fixed.rs`）、走り駒の射線（`sliding.rs`）、および前計算表（`tables.rs`）に既に分かれている。 |
| `src/core/predecessor/` | 直前局面の生成器であり、駒の在庫、一時状態、および逆向きの着手に既に分かれている。 |
| `src/core/movegen/search_captures.rs` | 静止探索の捕獲生成として既に独立している。ファイルを動かさないので、試験を指す`#[path = "tests/ordinary_capturer.rs"]`もそのまま有効である。 |
| `src/core/movegen/tests/`の既存ファイル | 条文ごとに既に分かれている。`tests/mod.rs`が`pub(crate) mod tests`として探索部の試験から参照される宣言も変えない。 |

`docs/measurements/`の測定記録と`docs/audits/`の監査報告は、ある時点の実行やコードを記録して以後は凍結する文書（docs/README.md）なので、パスの参照を書き換えない。

## 依存関係

[src構成の整理](src-layout.md)が定めたcoreの責務基準（RULES.mdだけから正しさを検証できるコードだけを置く）と、依存の向き（search → eval → core）を保つ。
本書は、同じ設計書の「core内部はフラット構成を維持する」という決定を改める。
同書は、ディレクトリによるグループ化が割に合う条件として、ファイル数による見通しの悪化と、機械検査したい境界の2つを挙げていた。
現在は、局面、規則セット、および審判の各ファイルが2,000行を超え、非公開の欄を参照してよい範囲（局面の欄、終局裁定の文脈の欄、反復の履歴の欄）をファイル単位では表せないので、2つ目の条件が成立している。

`src/core/`を変更する未統合のブランチはない（2026年9月27日に`git diff master...<ブランチ> -- src/core`で確認した）。
兄弟リポジトリの`minase-gui`と`minase-lishogi-bot`は、minaseのライブラリを参照せず実行ファイルだけを使う。

## 設計判断

| 論点 | 採用 | 棄却した代案 |
|---|---|---|
| 局面の分割 | `position/`へ昇格し、`Position`の定義を`mod.rs`に、欄を直接読み書きする処理を子モジュールに置く | 兄弟ファイルへ`impl Position`を分ける |
| 成りの判定の置き場所 | 新しい`src/core/promotion.rs` | `rules/promotion.rs`、または`movegen/`の中 |
| 獅子の捕獲制限の置き場所 | `movegen/lion_capture.rs` | `rules/lion.rs` |
| 着手の取り消し記録`Undo` | `position/make_move.rs`へ移す | `mv.rs`に残す |
| 終局裁定と反復の置き場所 | `game/`の子モジュール`game/adjudication/`と`game/repetition/` | `src/core/`直下に残す |
| 盤の座標系の置き場所 | `square.rs`、`direction.rs`、`bitboard.rs`を中身を変えずに`src/core/board/`へまとめ、`board/mod.rs`から公開する | `src/core/`直下に残す |
| 試験の置き場所 | 複数の関心事を通して検査する試験は`tests/`の下に主題別のファイルで置き、1関数だけを検査する単体試験は定義元のファイルに残す | すべてを定義元のファイルの`mod tests`へ散らす |
| 公開パス | ディレクトリの`mod.rs`は自分の子の項目だけを`pub use`で公開し、別のモジュールへ移した項目は呼び出し側の`use`を書き換える | 旧パスを`pub use`で残す |

局面を`position/`の子モジュールへ分けるのは、Rustの非公開の欄が、定義したモジュールとその子孫からしか見えないからである。
`Position`のメソッドは、`initial`と`royal_pieces`を除くすべてが非公開の欄を直接読み書きし、試験もハッシュ値の欄を書き換えて`validate`の検査を確かめる。
兄弟ファイルへ分けると欄を`pub(crate)`へ広げることになり、局面の不変条件を守る範囲が局面のモジュールからcrate全体へ広がる。
子モジュールなら欄は非公開のまま保たれ、欄を触れるコードが`position/`の中に限られることをコンパイラが検査する。

成りの判定を`promotion.rs`へ移すのは、成りの可否（RULES.md第6章と第30条）が規則セットの選択（第10章と第33条）とは別の関心事だからである。
成りの判定を使うのは、局面の着手の適用、合法手生成、静止探索の捕獲生成、および探索部の静的交換評価（SEE）であり、規則セットの文字列処理や組み立ては使わない。
`rules/promotion.rs`に置く案は、`rules`が「規則セットの選択」と「規則の判定」の2つの関心事を持つ状態を残す。
`movegen/`の中に置く案は、局面の着手の適用（`position`）が合法手生成（`movegen`）の内部を参照することになる。
なお、`promotion_choice`は引数に局面を取り、局面の着手の適用は`promotion_choice_for`を呼ぶので、`promotion`と`position`の相互参照は再編後も残る。
この相互参照は同じcrateの中の関数呼び出しであり、成りの判定がどちらのモジュールの非公開の欄にも触れないので、許容する。

獅子の捕獲制限を`movegen/`へ移すのは、制限を満たすことが合法手の定義の一部（RULES.md第3条第8号）であり、呼び出し元が合法手生成と静止探索の捕獲生成だけだからである。
制限の判定は、合法手生成の仮想盤面`VirtualBoard`と利きの計算`piece_control_with_occupancy`を使うので、現在は`rules`から`movegen`への参照がある。
移動後はこの参照が`movegen`の内部に閉じ、`rules`は規則セットの型だけを持つ。

`Undo`を`position/make_move.rs`へ移すのは、`Undo`が着手の適用と取り消しの記録であり、`mv.rs`が`Undo`のためだけに局面の`LionTrigger`を参照しているからである。
移動後の`mv.rs`は着手の型`Move`だけを持ち、局面に依存しない。

終局裁定と反復を`game/`の子にするのは、両者の呼び出し元が審判`Game`だけであり、両者が審判の結果の型（`GameResult`など）を使うからである。
結果の型を`game/result.rs`に置き、終局裁定と反復を`game/`の子にすると、3者の相互参照は`game/`の中に閉じる。
3つのモジュールは現在も`pub(crate)`であり、この移動はcrateの公開面を変えない。

盤の座標系を`board/`へまとめるのは、`src/core/`の直下を規則の層の順（盤の座標系、駒、着手、局面、駒の動き、合法手生成、成り、規則セット、審判、直前局面）に並べ、1つの目的を1つのディレクトリで表すためである。
升、方向、および升集合は、いずれもRULES.md第4条の12×12の盤の上の位置と位置の集合を表し、駒や局面に依存しない。
3つのファイルは互いの非公開の部分を参照しないので、このまとめは非公開の範囲を表す境界ではなく、読む順序を示す配置である。
3つのファイルはそれぞれ1つの型だけを持つので、中身は分割せずにそのまま移す。
`minase::core::square::Square`などの公開パスは`minase::core::board::Square`へ変わるので、互換性を壊す変更としてコミットする。

一方、`piece.rs`は手番`Color`、駒種`PieceKind`と成りの対応表、および升の内容の符号`PieceCode`という3つの型を持つので、`piece/`へ分ける。

旧パスを再公開しないのは、`src/core/`の項目の利用者がこのリポジトリ内のコードだけであり、書き換えで足りるからである。
ただし、`minase::core::rules::parse_rule_set`のように、ディレクトリへ昇格したモジュールが自分の子の項目を`mod.rs`から公開するものは、旧パスの互換ではなくモジュールの公開面そのものである。
この基準により、`rules`、`position`、`piece`、`game`、および`movegen`の現在の公開パスは、別のモジュールへ移す項目を除いて変わらない。
公開パスが変わるのは次の2つである。
1つ目は、別のモジュールへ移す`PromotionChoice`と`in_promotion_zone`であり、`minase::core::rules`から`minase::core::promotion`へ変わる。
2つ目は、`board/`へまとめる升、方向、および升集合の項目であり、`minase::core::square`、`minase::core::direction`、`minase::core::bitboard`から`minase::core::board`へ変わる。
crateルートの再公開（`minase::PromotionChoice`、`minase::Square`など）は変わらないが、公開パスの変更なので、どちらも互換性を壊す変更としてコミットする（CONTRIBUTING.mdの「コミットとリリース」）。

## 再編後の配置

表に挙げた型の`impl`（`Display`、`Error`、`Hash`、`From`などのトレイト実装を含む）は、表で別のファイルを指定したメソッドを除き、型と同じファイルへ移す。
新しいファイルの先頭には、既存のファイルと同じ体裁の日本語の`//!`コメントを置く。

### 全体

| パス | 置くもの |
|---|---|
| `src/core/mod.rs` | モジュールの宣言。`board`と`promotion`は`pub mod`、`game`は`pub(crate) mod`とし、`square`、`direction`、`bitboard`の宣言は`board/mod.rs`へ、`adjudication`と`repetition`の宣言は`game/mod.rs`へ移す。 |
| `src/core/mv.rs` | `Move`とその`capture_candidates`だけを残す。 |
| `src/core/promotion.rs` | `PromotionChoice`、`in_promotion_zone`、および`impl MoveRules`の`promotion_choice`と`promotion_choice_for`。 |

### 盤の座標系

| パス | 置くもの |
|---|---|
| `src/core/board/mod.rs` | 子モジュールの宣言と、3つの子の`pub`の項目すべての`pub use`（`BOARD_FILES`、`BOARD_RANKS`、`BOARD_SQUARE_COUNT`、`RAW_SQUARE_COUNT`、`Square`、`SquareRange`、`DIRECTION_COUNT`、`Direction`、`step_square`、`Bitboard`、`FILE_MASKS`、`SquareIter`）。 |
| `src/core/board/square.rs` | 現行の`src/core/square.rs`をそのまま移す。 |
| `src/core/board/direction.rs` | 現行の`src/core/direction.rs`をそのまま移す。 |
| `src/core/board/bitboard.rs` | 現行の`src/core/bitboard.rs`をそのまま移す。 |

`src/core/`の内外にある`crate::core::square::`、`crate::core::direction::`、`crate::core::bitboard::`の参照（`src/core/`の外に14行、中に39行）は、`crate::core::board::`へ書き換える。
3つのファイルの間の参照（`bitboard.rs`が使う`Square`など）は、`super::square::Square`のような兄弟の子モジュールのパスで書く。
`FILE_MASKS`は現在どこからも使われていないが、公開APIの項目であり、本書は移動だけを行うので削除しない。

### 駒

| パス | 置くもの |
|---|---|
| `src/core/piece/mod.rs` | 子モジュールの宣言と、`Color`、`PieceKind`、`PieceCode`、`COLOR_COUNT`、`PIECE_KIND_COUNT`の`pub use`。 |
| `src/core/piece/color.rs` | `COLOR_COUNT`と`Color`。 |
| `src/core/piece/kind.rs` | `PIECE_KIND_COUNT`と`PieceKind`（成りの対応表`promoted`と`unpromoted`を含む）。 |
| `src/core/piece/code.rs` | `PieceCode`。 |
| `src/core/piece/tests.rs` | 現行の`piece.rs`の試験4件。 |

### 局面

| パス | 置くもの |
|---|---|
| `src/core/position/mod.rs` | 子モジュールの宣言と`pub use`、`Position`の定義、および欄を読むだけの参照（`piece_at`、`occupied`、`pieces_of`、`pieces_of_kind`、`side_to_move`、`royal_pieces`、`captured_squares`）。 |
| `src/core/position/zobrist.rs` | 局面のハッシュ値。`ZOBRIST_PIECE_CODE_COUNT`、`ZOBRIST_SEED`、`ZobristKeys`、`ZOBRIST_KEYS`、`zobrist_keys`、`impl Hash for Position`、および`zobrist`、`rights_zobrist`、`recompute_zobrist`、`recompute_rights_zobrist`。 |
| `src/core/position/setup.rs` | 局面の作成。`empty`と`initial`（RULES.md第5条）。 |
| `src/core/position/placement.rs` | 駒の配置と除去の基本操作。`put_piece`、`put_piece_without_hash`、`remove_piece`、`remove_piece_without_hash`。 |
| `src/core/position/lion_trigger.rs` | 先獅子の状態（第15条）。`LionTrigger`、`lion_capture_square`、`set_lion_capture`、`lion_taken_by_non_lion`。 |
| `src/core/position/promotion_rights.rs` | 成りを保留した駒の権利（第30条P1、P2、P5）。`promotion_deferred`、`set_promotion_deferred`、`clear_promotion_deferred`、`promotion_deferred_is_valid`。 |
| `src/core/position/make_move.rs` | 着手の適用と取り消し。`CapturedPiece`、`Undo`、`NullUndo`、`flip_side_to_move`、`make_null_move`、`unmake_null_move`、`clone_with_side_to_move`、`make_move_unchecked`、`make_move_with_captures_unchecked`、`unmake_move`。 |
| `src/core/position/validate.rs` | 局面の不変条件の検査。`PositionError`と`validate`。 |
| `src/core/position/builder.rs` | `PositionBuildError`と`PositionBuilder`。 |
| `src/core/position/tests/` | 現行の`position.rs`の試験29件（主題別の表は後述）。 |

### 規則セット

| パス | 置くもの |
|---|---|
| `src/core/rules/mod.rs` | 子モジュールの宣言と、公開する項目の`pub use`。 |
| `src/core/rules/code.rs` | 規則コード。`RuleCode`（`ALL`と`text`を含む）と`RuleCodeParseError`。 |
| `src/core/rules/set.rs` | 規則セットの型。`RepetitionRule`、`LionRule`、`PromotionRule`、`ExhaustionRule`、`RuleGroup`、`MoveRules`（構造体と`standard`）、`Rules`（構造体、`ENGINE_DEFAULT`、`LISHOGI`、`adopts`）、`From<Rules> for Vec<RuleCode>`、および`Display for Rules`。 |
| `src/core/rules/assembly.rs` | 規則コードの列からの組み立てと検査（第33条第2項と第4項）。`RulesError`、`Rules::from_codes`、`assign_group`。 |
| `src/core/rules/parse.rs` | 規則セットの文字列の解析（第33条第5項と第6項）。`RuleSetParseError`、`RULE_SET_PRESETS`、`parse_rule_set`。 |
| `src/core/rules/tests.rs` | 現行の`rules.rs`の試験のうち、規則コードと規則セットを検査する7件（`article_29_30_all_rule_codes_round_trip_through_text`から`rules_error_display_identifies_the_missing_group`まで）と、現行の`game.rs`の試験のうち`Rules::from_codes`だけを検査する2件（`article_25_2_repetition_rule_is_mandatory_and_exclusive`と`article_33_9_e2_and_e3_conflict`）。 |

### 合法手生成

| パス | 置くもの |
|---|---|
| `src/core/movegen/mod.rs` | 子モジュールの宣言と`pub use`、`MoveGenerator`の定義とその`impl`（現行の32〜113行）。 |
| `src/core/movegen/generate.rs` | 合法手生成の駆動。自由関数の`generate_moves`と`generate_piece_moves`。 |
| `src/core/movegen/expand.rs` | 候補手の登録と成りの展開。`promoting_variant`と`push_with_promotion`。 |
| `src/core/movegen/control.rs` | 利き、ある升へ利く駒の逆引き、および特殊な動きの第1段階の到達升。`piece_control_with_occupancy`、`impl Position`の`attackers_to_by`、`ordinary_attackers_to`、`special_attackers_to`、`piece_control_with_tables`、`piece_control_without_special`、`special_step_destinations`。 |
| `src/core/movegen/virtual_board.rs` | 着手後の占有を差分で表す`VirtualBoard`。 |
| `src/core/movegen/lion.rs` | 獅子の2段階移動と跳躍（第12条）。`generate_lion_double_and_jumps`。 |
| `src/core/movegen/lion_like.rs` | 角鷹と飛鷲の2段階移動（第11条）。`generate_lion_like_double_and_jumps`。 |
| `src/core/movegen/lion_capture.rs` | 獅子の捕獲制限（第13〜16条と第29条）。`impl MoveRules`の`special_move_is_legal`、`captured_lions`、`is_tsukegui`、`lion_capture_is_legal`、`lion_has_foot_after_capture`。 |
| `src/core/movegen/checked.rs` | 合法性を検査してから着手を適用する入口。`IllegalMove`と、`impl Position`の`try_make_move`と`try_make_move_with_undo`。 |
| `src/core/movegen/tests/lion_capture.rs` | 現行の`rules.rs`の試験のうち、獅子の捕獲制限を検査する35件（第3条、第13〜16条、および第29条の節）。 |

`movegen/tests/lion_capture.rs`は、`movegen/tests/mod.rs`で他の試験ファイルと同じく`mod lion_capture;`として宣言する。
`movegen/tests/mod.rs`の先頭のコメントにある「獅子の捕獲制限は領域D2（rules.rs側）が検証する」という記述は、移動先の`tests/lion_capture.rs`を指すように改める。

### 審判

| パス | 置くもの |
|---|---|
| `src/core/game/mod.rs` | 子モジュールの宣言と`pub use`。 |
| `src/core/game/result.rs` | 対局の結果。`WinReason`、`DrawReason`、`GameResult`、`GameStatus`。 |
| `src/core/game/error.rs` | 着手を受け付けなかった理由。`IllegalMoveCause`と`GameError`。 |
| `src/core/game/referee.rs` | 審判`Game`とその`impl`。 |
| `src/core/game/adjudication/mod.rs` | 終局裁定の状態と手順。`AdjudicationState`、`AdjudicationContext`（`candidate_is_immediate_win`を含む）、`PostMoveAdjudication`、`adjudicate_after_move`。 |
| `src/core/game/adjudication/mate.rs` | 王駒の捕獲と詰み（第21条と第23条）。`royal_capture_result`、`captures_last_royal`、`can_capture_last_royal`、`has_no_legal_move`、`is_mate`。 |
| `src/core/game/adjudication/exhaustion.rs` | 駒枯れ（第22条）。`PieceExhaustionOutcome`、`PieceExhaustionTransition`、`piece_exhaustion_outcome`、`piece_exhaustion_transition`、`promoted_waiting_square`、`is_last_rank`。 |
| `src/core/game/adjudication/bare_king.rs` | 裸玉の即時裁定（第32条E3）。`bare_king_result`、`bare_king_square`、`effective_pieces`、`dead_pieces`、`pieces_give_check`、`squares_are_adjacent`。 |
| `src/core/game/adjudication/tests.rs` | 現行の`adjudication.rs`の試験のうち、`article_31_r1_irreversible_moves_are_promotions_captures_and_unpromoted_pawn_or_lance_steps`を除く3件。 |
| `src/core/game/repetition/mod.rs` | 反復規則ごとの履歴の切り替え。`RepetitionHistory`とその`new`。 |
| `src/core/game/repetition/r1.rs` | 第31条R1の履歴と裁定。`R1_MIN_REVERSIBLE_PLIES`、`R1Key`、`R1PositionState`、`R1History`、`r1_repetition_result`、`updated_attacking_counters`、`moves_by_color_through`、および現行の`adjudication.rs`にある`move_is_irreversible`と`move_was_attacking`。このファイルの`mod tests`には、現行の`repetition.rs`の試験1件と、`move_is_irreversible`だけを検査する現行の`adjudication.rs`の試験`article_31_r1_irreversible_moves_are_promotions_captures_and_unpromoted_pawn_or_lance_steps`を置く。 |
| `src/core/game/repetition/prohibition.rs` | 第31条R2とR3の禁止。`R2R3Key`、`R2History`、`R3History`、`repetition_is_forbidden`、`retain_repetition_allowed_moves`。 |
| `src/core/game/tests/` | 現行の`game.rs`の試験のうち、`rules/tests.rs`へ移す2件を除く51件（主題別の表は後述）。 |

`move_is_irreversible`と`move_was_attacking`を`r1.rs`へ移すのは、両者がR1の定義する不可逆手と攻撃的着手を判定する関数であり、R1の履歴を更新する入力になるからである。
`repetition_is_forbidden`を`R2History`および`R3History`と同じファイルに置くのは、この関数が両者の非公開の欄`positions`を直接読むからである。
`is_last_rank`は駒枯れと裸玉の両方が使うが、最奥段で移動不能となった駒の扱いは第22条第4項が定めるので`exhaustion.rs`に置き、`bare_king.rs`からは`pub(super)`で参照する。

`Game`を`game/mod.rs`ではなく`referee.rs`に置くのは、`mod.rs`を宣言と公開面だけにする[探索部と評価関数のモジュール再編](search-eval-layout.md)の体裁にそろえるためである。
`game/game.rs`という名前は、clippyの`module_inception`の警告に当たるので使わない。

### 試験の分け方

試験を`tests/`の下へ分けるモジュールでは、複数の試験ファイルが使う補助関数を`tests/mod.rs`に置き、1つのファイルだけが使う補助関数はそのファイルに置く。
試験を定義元の関数の近くへ移すために別のモジュールの試験から取り出す場合（`rules/tests.rs`と`game/repetition/r1.rs`へ移す3件）は、移動先に同じ補助関数がなければ、`piece`や`step`のような数行の補助関数を移動先へ写す。
移動によって`use super::*;`が届かなくなる試験は、必要な項目を明示的に`use`する。
試験の名前は変えない。

局面の試験は、次のファイルに分ける。

| ファイル | 置く試験 |
|---|---|
| `position/tests/zobrist.rs` | ハッシュ値と`Hash`の実装。`equal_positions_have_equal_hashes`、`hash_set_*`、`article_24_1_*`、`article_24_3_*`、`equal_configurations_share_one_key_regardless_of_construction_path`。 |
| `position/tests/validate.rs` | `validate_rejects_corrupted_zobrist_values`、`bench_and_sampled_random_positions_validate`。 |
| `position/tests/builder.rs` | `builder_finish_*`。 |
| `position/tests/setup.rs` | 初期配置（第5条と第20条第1項）。`article_5_*`、`articles_5_and_20_1_*`。 |
| `position/tests/placement.rs` | `placing_then_removing_any_piece_restores_the_empty_position`。 |
| `position/tests/lion_trigger.rs` | `lion_capture_square_*`。 |
| `position/tests/promotion_rights.rs` | `promotion_deferred_returns_the_marked_square_set`、`article_30_p2_*`、`article_30_p5_*`。 |
| `position/tests/make_move.rs` | 着手の適用と取り消し。`article_6_1_*`、`article_4_4_*`、`unmake_restores_every_observable_component`、`d4_imp_10_*`、`seeded_random_playouts_*`、`captured_squares_match_reference_*`。 |

審判の試験は、`rules/tests.rs`へ移す2件を除き、現行の`game.rs`の節見出しのコメントに従って、次のファイルに分ける。

| ファイル | 置く試験 |
|---|---|
| `game/tests/royals.rs` | 第20条の節。 |
| `game/tests/win.rs` | 第21条の節。 |
| `game/tests/exhaustion.rs` | 第22条の節。 |
| `game/tests/no_legal_move.rs` | 第23条の節。 |
| `game/tests/repetition.rs` | 第24条と第25条の節、および第31条の節。 |
| `game/tests/illegal_move.rs` | 第26条と第27条の節。 |
| `game/tests/local_rules.rs` | 第32条の節（第33条第9項の試験を含む）。 |
| `game/tests/self_play.rs` | 横断的な試験とランダムな自己対局の節。 |

## 可視性

移動によって参照元が定義元の子孫でなくなる項目は、参照に必要な最小の可視性へ広げる。
一般の規則は次の3つである。
ディレクトリの子モジュールは非公開の`mod`で宣言する。
子モジュールで定義し、親または兄弟のモジュールから使う項目、欄、およびメソッドは`pub(super)`とする。
現在`pub`または`pub(crate)`の項目は可視性を変えず、ディレクトリの外から参照されるものだけを親の`mod.rs`から同じ可視性の`pub use`または`pub(crate) use`で公開する。
外から参照されない項目を再公開すると未使用の警告が出るので、再公開は実際の参照元がある項目に限る。

推測に任せない項目を次に指定する。

| 項目 | 可視性 | 理由 |
|---|---|---|
| `ZobristKeys`、そのメソッド、欄`side_to_move`、`zobrist_keys` | `pub(super)` | `make_move.rs`の`flip_side_to_move`が欄`side_to_move`を直接読み、`placement.rs`、`lion_trigger.rs`、`promotion_rights.rs`、`setup.rs`、`builder.rs`がハッシュ値を更新する。 |
| `put_piece`、`put_piece_without_hash`、`remove_piece`、`remove_piece_without_hash` | `pub(super)` | `setup.rs`、`builder.rs`、`make_move.rs`が使う。 |
| `set_promotion_deferred`、`clear_promotion_deferred`、`promotion_deferred_is_valid` | `pub(super)` | `builder.rs`と`make_move.rs`が使う。 |
| `validate` | `pub`のまま | 公開APIである。 |
| `Undo` | `pub(crate)`のまま、`position/mod.rs`から`pub(crate) use`で公開する | `eval/pst/accumulator.rs`、`search/alphabeta/correction.rs`、および`movegen/checked.rs`が使う。参照を`crate::core::position::Undo`へ書き換える。 |
| `CapturedPiece`、`NullUndo`、`LionTrigger` | `pub(crate)`のまま、再公開しない | `position/`の外から名前で参照されない。`search/alphabeta/negamax.rs`などは`LionTrigger`を返すメソッドを呼ぶだけで、型の名前を書かない。再公開すると`unused_imports`の警告が出る。`position/`の中の兄弟は`super::lion_trigger::LionTrigger`のパスで参照する。 |
| `movegen`の子へ移す自由関数（`piece_control_without_special`、`special_step_destinations`、`generate_lion_double_and_jumps`、`generate_lion_like_double_and_jumps`、`promoting_variant`など） | `pub(super)` | 兄弟のファイルと`search_captures.rs`が使う。`search_captures.rs`の`use super::*;`は、移動先の子モジュールの項目を明示的に`use`する形へ改める。 |
| `VirtualBoard::move_to` | `pub(super)` | `lion.rs`が使う。 |
| `game/adjudication/`の子で定義し、`adjudication/mod.rs`または兄弟が使う項目 | `pub(super)` | `bare_king.rs`の`is_last_rank`の参照、`mod.rs`の`adjudicate_after_move`と`candidate_is_immediate_win`からの参照など。 |
| `R1History::new`、`R2History::new`、`R3History::new` | `pub(super)` | `repetition/mod.rs`の`RepetitionHistory::new`が呼ぶ。 |
| `PieceKind::can_exist_unpromoted` | `pub(super)` | `piece/tests.rs`が使う。 |

`AdjudicationContext`の欄と`candidate_is_immediate_win`は、定義を`adjudication/mod.rs`に置くので非公開のまま`mate.rs`と`tests.rs`から参照できる。
`Position`の欄も、定義を`position/mod.rs`に置くので非公開のまま子モジュールと`position/tests/`から参照できる。
`pub(crate)`を超える可視性の拡大と、新しい`pub`の追加は行わない。

## 文書のパス参照

`docs/`（`docs/measurements/`と`docs/audits/`を除く）とコードのdocコメントにある`src/core/`のパスの参照を、新しい配置へ書き換える。
対象は約80行であり、`docs/plans/`の`movegen-speedup.md`、`movegen-speedup-2.md`、`movegen-speedup-3.md`、`lishogi-bot.md`、`search-repetition.md`、`predecessor-generator.md`、`spec-first-tests/ledgers/d1.md`から`d4.md`など、および`docs/research/`の各調査に分布する。
`src/core/square.rs`、`src/core/direction.rs`、および`src/core/bitboard.rs`への参照は、`src/core/board/`の下の同名のファイルへ改める。
`src/core/rules.rs`、`src/core/position.rs`、`src/core/game.rs`、`src/core/adjudication.rs`、`src/core/repetition.rs`、`src/core/piece.rs`、および`src/core/movegen/mod.rs`への参照は、同じ文にある関数名、型名、または試験名から「再編後の配置」の表に従って移動先のファイルを決める。
関数名などの手がかりがなく、モジュール全体を指す参照は、ディレクトリ（`src/core/position/`など）へ改める。
現在存在しないファイル（`src/core/movegen/tests/invariants.rs`、`src/core/movegen/tests/articles.rs`など）への参照は、書かれた時点の記録として残す。
行番号の付いた参照（`docs/research/movegen-speedup-ideas.md`の`src/core/movegen/mod.rs#L297`）は、同じ文の関数名から移動先の行を特定できれば新しい行番号へ改め、特定できなければ行番号を削ってパスだけを残す。

[src構成の整理](src-layout.md)の「core内部はフラット構成を維持する」の節は、本書の配置の方針（1関心事1ファイル、非公開の範囲を表す必要がある型はディレクトリへ昇格し、定義を`mod.rs`に置く）に書き換え、配置の詳細は本書を参照させる。

## 実装フェーズ

masterから`core-layout`ブランチを切り、次の順にコミットする。
各コミットは単独でビルドと試験が通る状態にする。

1. `refactor(core)!: group square, direction, and bitboard under board`。盤の座標系を`board/`へまとめ、本文に`BREAKING CHANGE:`フッターを書いて、`minase::core::square`、`minase::core::direction`、`minase::core::bitboard`の項目が`minase::core::board`へ移ったことを記す。
2. `refactor(core)!: move promotion and lion capture rules out of rules`。成りの判定を`promotion.rs`へ、獅子の捕獲制限とその試験35件を`movegen/lion_capture.rs`と`movegen/tests/lion_capture.rs`へ移し、残りを`rules/`へ分ける。本文に`BREAKING CHANGE:`フッターを書き、`minase::core::rules::PromotionChoice`と`minase::core::rules::in_promotion_zone`が`minase::core::promotion`へ移ったことを記す。
3. `refactor(movegen): split the move generator by purpose`。`movegen/mod.rs`を「合法手生成」の表の配置へ分ける。
4. `refactor(core): split the position module by purpose`。`position/`へ分け、`Undo`、`CapturedPiece`を`mv.rs`から移す。
5. `refactor(core): nest adjudication and repetition under game`。`game/`へ分け、終局裁定と反復をその子へ移す。
6. `refactor(core): split the piece module by type`。`piece/`へ分ける。
7. `docs: update source paths after the core module split`。文書のパス参照と[src構成の整理](src-layout.md)を書き換える。

コードの移動（1から6）はcodexへ委任し、文書の書き換え（7）と本書の更新はClaudeが行う。
codexへの指示には、名前の変更、シグネチャの変更、処理の並べ替え、インライン属性の変更、旧パスの再公開、および「可視性」の節を超える可視性の変更を禁じることを含める。
あわせて、試験の名前を変えないこと、`use super::*;`が届かなくなった試験には明示的な`use`を書くこと、および新しいファイルの先頭に日本語の`//!`コメントを置くこと（`missing_docs`の警告を出さないため）を指示する。
移動した項目のdocコメントに「〜から移した」のような経緯の記述を加えない。

## 検証

基点コミットは、`core-layout`ブランチを切ったmasterのコミットとする。
再編の前に、基点コミットで`cargo run --release --bin bench -- --depth 6`の標準出力を保存する。
各コードのコミットの後に同じコマンドを実行し、経過時間とNPSの欄を除く標準出力の全行が基点と一致することを確かめる。
深さ6を使うのは、既定の深さでは発火しない枝刈りも通し、[探索部と評価関数のモジュール再編](search-eval-layout.md)と同じ条件で確かめるためである。

各コミットで、`cargo fmt --check`、`cargo clippy --all-targets`（警告0件）、`cargo test`、および`cargo test --features tuning`を実行し、全件が通ることを確かめる。
`cargo test`の出力にある試験名の一覧を基点と比べ、試験の件数と、名前の末尾（モジュールパスを除いた部分）の集合が一致することも確かめる。
試験を別のファイルへ移すと完全修飾名は変わるので、比べるのは末尾の集合である。

最後のコードのコミットで、benchのNPSを基点と比べる。
計測は、[進行中の測定を確認してから](../lessons/check-running-measurements-before-cpu-load.md)、`taskset`で性能コア1つに固定し、基点と再編後を交互に3回ずつ行う。
再編後の中央値が、基点の3回の最小値から最大値までの範囲に収まることを確かめる。
収まらない場合は、benchのバイナリに含まれるminaseの関数の集合と各関数の機械語のバイト数（`nm --size-sort`の値）を基点と比べ、[呼び出し元の機械語を比較する教訓](../lessons/check-caller-codegen-for-local-optimizations.md)に従ってインライン化が変わった関数を調べる。

文書の書き換えの後に、`docs/`（`docs/measurements/`と`docs/audits/`を除く）とリポジトリ直下の文書に、再編で消えたパス（`src/core/square.rs`、`src/core/direction.rs`、`src/core/bitboard.rs`、`src/core/rules.rs`、`src/core/position.rs`、`src/core/game.rs`、`src/core/adjudication.rs`、`src/core/repetition.rs`、`src/core/piece.rs`）への参照が残っていないことを`grep`で確かめる。
`src/core/movegen/mod.rs`への参照は残り得るので、残った参照が`MoveGenerator`の定義を指していることを目で確かめる。

## 完了条件

7つのコミットが`core-layout`ブランチにあり、「検証」の節の確認がすべて満たされ、ブランチがmasterへ統合されたときに完了とする。
本書は独立したリファクタリングなので、[src構成の整理](src-layout.md)と同じく、ROADMAP.mdのマイルストーン状態表には行を追加しない。

## 参考資料

- [src構成の整理](src-layout.md)。coreの責務基準と依存の向きを定める。
- [探索部と評価関数のモジュール再編](search-eval-layout.md)。移動だけのリファクタリングの手順、可視性の規則、および検証の方法の先例である。
- [挙動マトリクスによる試験の再構築](spec-first-tests.md)。試験を条文ごとに分ける方針を定める。
