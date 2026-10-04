[監査報告](../test-audit-2026-10-04.md)の90候補群を、報告の推奨に従って適用した。
固有の保証は既存の実行へ移し、テスト専用の写しを観測していた検証は本番処理の観測へ置き換え、SPSA係数に依存するリテラルの期待値は `params::*` からの計算へ改めた。
製品の動作は変更していない。
本番コードの変更は、未使用の定数 `FILE_MASKS` とその再エクスポートの削除、およびテスト専用の `mod` 宣言の削除だけである。

| 監査範囲 | 整理の結果 |
|---|---|
| C01〜C38 | C22とC29を推奨どおり保持し、C30を適用せず、その他を適用した。テスト関数は328件から278件になった。 |
| P01〜P15 | P09のうち usi.rs:3318 の統合を見送り、その他を適用した。テスト関数は151件から138件になった。 |
| S01〜S17 | S10の ponder.rs:276 を削除せず許容幅を緩め、S06の ordering.rs:103 を保持し、その他を適用した。テスト関数は250件から223件になった。 |
| T01〜T20 | T20を適用せず、T02の simulation.rs:573 を保持し、その他を適用した。テスト関数は435件から296件になった。 |

追跡ファイルのテスト関数は、全体で1,164件から935件になった。
内訳はRustが889件から793件、Pythonが275件から142件である。

## 監査の推奨と異なった3点

C30は適用しなかった。
監査は tests/lishogi_import.rs:270 の `lambda == 0.75` を、src/bin/selfplay_gen/provenance.rs:53 の単体テストと重複するとした。
しかし後者は provenance サブコマンドへ `--lambda 0.75` を明示して保存値を確かめるだけであり、generate サブコマンドが来歴へ書き込むリテラル（src/bin/selfplay_gen/generate.rs:94）を固定するのは前者だけだった。

P09のうち、usi.rs:3318 `multi_worker_search_places_info_immediately_before_one_bestmove` の統合は見送った。
監査は、統合先の投了テストのThreads=2のセッションで同じ出力構造を検査できるとした。
しかし統合先を10回実行しても採用深さと最終進捗深さがともに2で、採用したinfoを追加で出力する経路を一度も通らなかった。

T20は適用しなかった。
監査は、KingFeatures の読み込みコードをRust側に書き出し処理がない未使用コードと判断し、テストとともに削除することを推奨した。
しかし保持を推奨した診断 tools/train/src/minase_train/diagnostics/lookahead_teacher.py が17行で KingFeatures をimportし、176行で読み込み、177行で列数118と定義2を検証している。
この診断は既存の標本に対して実行でき、未使用という前提は成り立たなかった。

## 適用時に加えた改善

監査の候補に加えて、整理の過程で次の検査を強めた。
いずれも削除した検証の保証を失わずに、変わり得る値への依存を減らすか、本番処理の観測を増やすものである。

- 時間予算の参照式（tuning.rs）は、係数だけでなく格子の境界点も `params::*` から導出した。境界点が既定係数の値に固定されていると、SPSAで係数が変わったときに境界の検査が黙って外れるからである。
- SEEの手計算テストは、余裕値0と200に加えて、負の期待値 v について余裕値 −v と −v−1 でも本番の `see_prunes` を照合し、値を厳密に固定した。
- ponder.rs:276 の待機の閾値を `params::iteration_ratio()` から導出した。的中時に反復を当て直す判定は比の値に依存し、宣言範囲の下限では固定の閾値では成り立たないからである。
- isready の統合先は、position の後に置換表が未確保であることと、isready の後に確保されることを2段で確かめる形にした。
- 削除したテストが担っていたマトリクスID（D6-USI-03、13、23、60、D6-CLI-02、D7-SRCH-10、D7-LIM-04）を、保持先のテストのコメントへ移した。

## 統合と置換の確認

assertを移した統合と、期待値の計算方法を置き換えた箇所では、実装の1行を一時的に壊し、残したテストが失敗することを確かめてから戻した。
領域ごとの代表例は次のとおりである。

| 範囲 | 誤変更 | 失敗したテスト |
|---|---|---|
| C07 | P2の群割当てを省略する。 | `article_33_4_from_codes_reports_duplicate_conflict_and_missing_in_contract_order` |
| C15 | 詰み判定の仮想着手の取消しを省略する。 | `article_21_3_b_royal_counter_capture_rescues_mate_and_restores_position` |
| C36 | 拡張SFENの手数を1増やす。 | `predecessor_profile`（計測の前に停止する） |
| S01 | 時間予算の丸め順序を変える。 | `tuning_default_clock_budget_matches_reference_grid` |
| S10 | 的中時に進行中の反復を当て直す配線を外す。 | `ponder_long_iteration_stops_on_hit_without_spending_hard_budget` |
| P09 | ResignValueの宣言上限を99998にする。 | `handshake_declares_ruleset_before_variant_and_ends_with_usiok` |
| P09 | position で置換表を確保する。 | `isready_allocates_the_default_transposition_table_before_the_first_go` |
| T04 | 版不一致のエラー種別を変える。 | storage と report の版3の拒否テスト |
| T17 | λ=0 の転送を真偽判定で落とす。 | `test_training_forwards_and_records_mirrored_penalty_and_explicit_k` と学習器側の実学習テスト |

実施した誤変更はすべて検出した。
全コードを対象とする変異検証は行っていない。

## 不安定さと壊れたテストの修理

ponder.rs:276 は、修理前に負荷下でモジュール15回中7回失敗していた。
許容幅を緩めた後は、同じ負荷の下で15回、追加修正後に5回、別の確認で5回実行し、いずれも失敗しなかった。

`#[ignore]` の synthetic_objective、gain_simulation、gain_decay_diagnostic は、測定時の22係数の範囲をテスト内の表に固定し、params.rs の係数表に依存しないようにした。
synthetic_objective は20シードすべてで改善し、平均正規化距離は0.164〜0.181だった。
gain_simulation は最初の検算段階までパニックせずに進むことを確かめて止めた。

## 検証

次のコマンドがすべて成功した。

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --features tuning -- -D warnings
cargo clippy --all-targets --features search-stats -- -D warnings
cargo test --all-targets
cargo test --features tuning --lib search::
cargo test --features search-stats --lib search::
cargo test --doc
(cd tools/train && .venv/bin/python -m unittest discover -s tests -p 'test_*.py')
PYTHONPATH=. python3 scripts/test_fetch_lishogi_games.py
```

| 実行対象 | 結果 |
|---|---|
| `cargo test --all-targets` | 762件が成功し、14件を `#[ignore]` で除外した。監査時は855件と16件だった。 |
| `tuning` フィーチャの探索テスト | 180件が成功した。 |
| `search-stats` フィーチャの探索テスト | 184件が成功した。 |
| Python（tools/train/tests） | 119件が成功した。監査時は132件だった。 |
| scripts/test_fetch_lishogi_games.py | 13件が成功した。 |

実行時間の比較測定は行っておらず、速度改善率は主張しない。
