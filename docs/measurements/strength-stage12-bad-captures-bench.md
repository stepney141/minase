# 段階12項目2の除外件数と探索bench

## 目的

[段階12の項目2](../plans/strength-stage12.md)で負の捕獲手を後回しにした実装について、規則ごとの除外件数と固定深さでの探索量、毎秒探索ノード数（NPS）を確認する。

## コマンドライン

計数には[一時的な差分](strength-stage12-bad-captures-bench/counters.patch)を適用し、次のコマンドで深さ6を1回測った。
差分は今回の実装に対するものであり、置換表の手を除いた捕獲手の生成時に数える。
各手選択器の計数は`reset`で消さず、破棄時に出力して合算した。

```console
cargo build --release --bin bench
taskset -c 3 target/release/bench --depth 6
```

計数コードを除去してから、次の検査と計測を実行した。

```console
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release --bin bench
taskset -c 3 target/release/bench --depth 6 --repetitions 3
```

## エンジン

候補は`strength-stage12`の`3311ee86041dc8a61918ced30d2f843dd438814f`に今回の未コミット差分を適用した構成である。
比較値は指示書が示した段階開始版`df0c75e`の1,677,944ノード、NPS約2,050,000を使い、基準版の再測定は行っていない。
規則はbench既定の`engine-default`、評価重みは埋め込みの`nets/pst.bin`を使った。

## 環境

2026年9月27日、Intel Core Ultra 7 265KF、20物理コア、20論理コアの環境で測定した。
Rustは`rustc 1.98.0 (88d9e12ae 2026-08-18)`を使い、releaseの設定でビルドした。
CPU 3へ固定し、探索ワーカーは1、置換表は256 MiBとした。
開始前に`match_runner`と`spsa_runner`のプロセスがないことを確認した。

## 結果

### 除外件数

捕獲手の段を生成した時点の候補は延べ1,453,230手であり、そのうち604,942手をSEE（静的交換評価）が負と判定した。
以下の「負かつ除外」は、負と判定されたのに当該規則により前段に残った手を表す。
各規則は独立に数え、同じ手が両方に該当する場合は両方へ加算する。

| 規則 | 負かつ除外 | SEEの符号を問わない該当数 |
|---|---:|---:|
| 最後の王駒の捕獲 | 0 | 23,538 |
| 獅子の2枚取り | 0 | 63,588 |

したがって、負と判定した604,942手はすべて後段へ移り、候補全体の41.63%を占めた。
獅子の2枚取りは経由升を持つため、現行SEEの契約では判定不能となり、`see_prunes`は偽を返す。
王駒の捕獲でも今回の測定では負の判定がなかった。
指示書が示したフェーズ1の8,430件と45,951件は符号を問わない該当数であり、今回の「負かつ除外」とは定義が異なる。
探索順序も変わっているため、符号を問わない件数同士も同一の探索木の比較にはならない。

[計数の生出力](strength-stage12-bad-captures-bench/counters.txt)の配列は、候補総数、負の手、負かつ王駒の除外、負かつ2枚取りの除外、王駒の該当数、2枚取りの該当数の順である。
[計数時のbench出力](strength-stage12-bad-captures-bench/counters-bench.txt)も保存した。
計数コードは実装から除去済みである。

### 計数コード除去後のbench

ウォームアップ1回を計測外で行い、続く3回を測った。
[生出力](strength-stage12-bad-captures-bench/bench.txt)は次のとおりである。

| 反復 | 総ノード数 | 探索時間 | NPS |
|---|---:|---:|---:|
| 1 | 1,618,614 | 0.874724秒 | 1,850,428 |
| 2 | 1,618,614 | 0.876775秒 | 1,846,100 |
| 3 | 1,618,614 | 0.872894秒 | 1,854,308 |
| 中央値 | 1,618,614 | 0.874724秒 | 1,850,428 |

指示書の基準値に対し、総ノード数は3.54%減り、NPSは約9.74%低下した。
基準のNPSは概数であり、同時期に測り直した値ではない。

### 検証

`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、releaseのbenchビルドがすべて成功した。
`cargo test`は合計774件成功、16件が既存の無視設定で、失敗は0件だった。
[全テストの出力](strength-stage12-bad-captures-bench/tests.txt)を保存した。

| テスト | 検証する契約 |
|---|---|
| `staged_picker_defers_negative_captures_in_stable_mvv_lva_order` | 負の捕獲を静かな手の後へ置き、捕獲価値順と同点時の生成順を保つ。捕獲の真偽、置換表とkillerへの重複指定、消費途中の`reset`も確認する。 |
| `staged_picker_keeps_royal_double_lion_and_unknown_captures_before_quiets` | 最後の王駒、獅子の2枚取り、判定不能の居喰いを捕獲手の段に残す。 |
| `staged_picker_yields_every_legal_move_exactly_once` | 助言手の合法性と重複を処理し、各合法手を1回だけ返す。 |
| `staged_picker_matches_reference_sequence_with_changing_history` | 複数規則と局面で全出力列を照合し、履歴更新後も合法手の重複と欠落がないことを確認する。 |
| `main_picker_captures_match_stable_reference_for_all_rules` | 前段と後段の両方を含む捕獲列が各規則で安定なMVV-LVA順になることを確認する。 |
| `staged_picker_respects_advisory_precedence` | 置換表とkillerの優先順位を保つ。 |
| `staged_picker_classifies_capture_and_quiet_tt_moves` | 置換表の手の捕獲分類を保つ。 |
| `staged_picker_classifies_all_generated_captures` | 返した手の捕獲分類が合法手生成と一致する。 |
| `see_pruning_searches_safe_capture_after_losing_tt_move` | 負ける置換表の手の後でも、王を守る負の捕獲を探索する。 |
| `human_opening_is_exact_and_records_start_after_its_original_ply` | 開始局面を正確に引き継ぎ、その次の局面から元の手数を加えた記録を作る。 |

負の捕獲を振り分ける処理だけを一時的に外すと、追加した順序テストが失敗した。
実装を戻してから全体検査を通した。
[失敗時の出力](strength-stage12-bad-captures-bench/regression.txt)を保存した。

判断を要した点は、SEEが負の場合の除外と既存テストの前提である。
2種類の明示的な除外は指示書どおり実装したが、獅子の2枚取りは現行SEEでは負にならず、除外テストは前段に残るという出力の契約を検査している。
規則は既存の`MoveGenerator::rules()`から取得できるため、`MovePicker::next`の引数は増やしていない。

既存の枝刈りテストには王を守れる静かな手があり、今回の順序変更後はその手が先に探索された。
そこで、指定の捕獲以外は王駒の捕獲を許す局面へ修正し、その条件自体も合法手生成で検査した。
自己対局テストは200ノードの予算で4,000手の上限に達して破棄されたため、テスト内の予算を1,000ノードへ変更した。
破棄0件、記録が空でないこと、記録手数が60より大きいことなどの既存の条件は維持したが、終局までの対局経過への依存は残る。

## 結論

項目2の実装は要求された順序と合法手の一意性を満たし、深さ6ではノード数が減る一方でNPSが低下したため、棋力の採否は後続の自己対局測定で判定する。
