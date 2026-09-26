# 探索部と評価関数のモジュール再編の設計書

## 先に読む要約

探索部の`src/search/mod.rs`は2,579行あり、探索の呼び出し口の型、並列探索（Lazy SMP。複数のスレッドが置換表を共有して同じ局面を並行に探索する方式）のワーカー管理、時間管理、αβ探索の本体、静止探索の捕獲生成、着手の順序付け、および枝刈りの式が1つのファイルに同居している。
試験も`src/search/tests.rs`の4,653行に集まっており、ある機能の実装と試験を探すには、ファイル内を関数名で検索するしかない。
評価関数の`src/eval/pst.rs`も、重みファイルの復号と検査、差分更新（accumulator）、および補間評価を1ファイルに持ち、評価関数ではない学習データの交換形式3本が`src/eval/`に同居している。
本書は、コードの目的ごとにファイルとディレクトリを分け、1ファイルが1つの関心事だけを持つ配置へ移す。
探索部は、探索方式に依存しない呼び出し口を`src/search/`直下に、既存のαβ探索を`src/search/alphabeta/`に置く。これは[AlphaZero型探索の設計書](alphazero.md)が定めた再編の第1段を兼ねる。
評価関数は、学習PSTを`src/eval/pst/`へ分割し、学習データの交換形式を新しいトップレベルモジュール`src/training/`へ移す。
再編は処理の中身を変えない移動であり、固定局面を深さ6まで探索するベンチ（`src/bin/bench.rs`）で局面ごとのノード数、最善手、および評価値が再編の前後で一致し、既存の試験が全件通ることを完了条件とする。

## 状態

進行中。
2026年9月26日に起案と着手を行い、同日に利用者の決定で、αβ探索を`alphabeta/`へ置くこと、[探索局面を用いた学習](search-aware-evaluation.md)の統合を待たずにmasterで実施すること、および学習データの形式を`src/eval/`の外へ移すことを確定した。
4つのフェーズを`search-eval-layout`ブランチで終え、「検証」の節の確認をすべて満たした。
NPSは、再編後の中央値が基点の3回の計測の揺れの範囲を上側へ約0.07%外れた（速くなった）。このため「検証」の節の手順に従って生成コードを調べたところ、benchのバイナリに含まれるminaseの関数の集合と、各関数の機械語のバイト数（`nm --size-sort`の値）が基点と一致し、インライン化は変わっていなかった。
次の一手は、利用者の判断によるmasterへの統合である。

## 目的

新しい探索改良や評価項目を加えるときに、変更すべきファイルを配置から一意に決められる状態にする。
あわせて、[AlphaZero型探索の設計書](alphazero.md)が予定する`src/search/alphabeta/`と`src/search/mcts/`の並置に向けて、探索方式に依存しない境界とαβ探索の内部を先に分けておく。

## 適用範囲

対象は`src/search/`、`src/eval/`、新設する`src/training/`、これらを参照する`src/lib.rs`、`src/protocol/`、`src/bin/`、`tests/`の`use`文、および`docs/`とリポジトリ直下の文書にあるパスの参照である。

関数、型、定数、および試験の名前は変えず、学習データのモジュール名`training_data`だけを`records`へ改める（「設計判断」の節）。
処理の内容、引数、処理の順序、およびインライン属性は変えない。
ただし、移動に伴って次の3種類の記述は書き換える。
1つ目は、`use`文ではなく式の中に直接書かれたモジュールパス（`src/bin/selfplay_gen.rs`の`minase::eval::training_data::FORMAT_VERSION`など）である。
2つ目は、`include_bytes!`が埋め込むファイルの相対パス（`nets/pst.bin`と`nets/pst-init.bin`）であり、ファイルの階層が深くなった分だけ`../`を増やす。
3つ目は、試験が子プロセスへ渡す試験の完全修飾名（`tuning_parameters_and_usi_contract_in_isolated_process`が`--exact`で指定する`search::tests::…`）であり、移動後の完全修飾名へ改める。

対象外は次のとおりである。
`src/eval/handcrafted.rs`は駒得評価v0の保存場所として現状のまま残す。
`docs/measurements/`の測定記録と`docs/audits/`の監査報告は、ある時点の実行やコードを記録して以後は凍結する文書（docs/README.md）なので、パスや試験名の参照を書き換えない。

## 依存関係

[src構成の整理](src-layout.md)が定めた依存の向き（search → eval → core）を保つ。
新設する`training`は`core`だけに依存し、`search`と`eval`からは参照されない。
`search`の内部では、`search`直下の境界が`alphabeta`の`run_search_team`を呼び、`alphabeta`は境界の型（探索の制限`SearchLimits`、探索開始時の局面の写し`SearchSnapshot`、および呼び出し側へ送る`SearchEvent`）を使う。

ブランチ`search-aware-evaluation`は、`src/eval/features.rs`、`src/eval/pst.rs`、および`src/search/mod.rs`を変更し、`src/search/sampling.rs`を追加している。
[探索局面を用いた学習](search-aware-evaluation.md)は重みを採用せずに完了し、診断用のコードをmasterへ統合するかは利用者の判断を待っている。
本書の再編を先にmasterへ入れるので、同ブランチを統合する場合はmasterを取り込み、採取の処理を`src/search/alphabeta/sampling.rs`へ、静止探索と`search_move`への呼び出しを`quiesce.rs`と`negamax.rs`へ、特徴の変更を`src/eval/pst/features.rs`へ付け替える。
[棋力向上段階12](strength-stage12.md)と[反復負け回避](search-repetition.md)は未着手なので、着手時に新しい配置で実装する。

## 設計判断

| 論点 | 採用 | 棄却した代案 |
|---|---|---|
| αβ探索の置き場所 | `src/search/alphabeta/`へ置き、`src/search/`直下には探索方式に依存しない境界だけを残す | `src/search/`直下で目的別に分けるだけにし、`alphabeta/`の層は[AlphaZero型探索](alphazero.md)の着手時に作る |
| 時間管理の置き場所 | `alphabeta/time.rs` | 境界の`src/search/time.rs` |
| `Searcher`の分割 | 構造体を`alphabeta/searcher.rs`に定義し、欄を`pub(super)`にして、`impl`を目的別のファイルに分ける | `Searcher`を`alphabeta/mod.rs`に置く、または`alphabeta/searcher/`の下に子モジュールとして並べる |
| 試験の置き場所 | `alphabeta/tests/`の下に主題別のファイルを置き、共通の補助関数を`alphabeta/tests/mod.rs`に集める | 各モジュールのファイル内の`mod tests`へ散らす |
| 学習PST | `src/eval/pst/`へ昇格し、特徴、形式、差分更新を子モジュールに分ける | `pst.rs`のまま残す |
| 学習データの交換形式 | トップレベルの`src/training/`へ移し、`training_data`を`records`へ改名する | `src/eval/data/`へまとめる、または現状のまま残す |
| 旧パスの扱い | 再公開を置かず、呼び出し側の`use`をすべて新しいパスへ書き換える | 旧パスを`pub use`で残す |

αβ探索を`alphabeta/`へ置くのは、同じ再編をAlphaZero型探索の着手時に改めて行うと、ファイルの移動と文書の参照の書き換えを2回行うことになるからである。
`alphabeta/`の層は方式が1つしかない現状でも「αβ探索に固有の処理」という境界を示す。境界から`alphabeta`の内部への参照は、探索の実行`run_search_team`と置換表の型に限られ、この限定は後述の可視性`pub(in crate::search)`によってコンパイラが検査する。

時間管理を`alphabeta/`に置くのは、予算式が係数表`params`（探索と時間管理の調整係数の既定値と範囲をまとめた表で、SPSAによる調整の対象）の値を読み、反復深化の開始判定と先読みの的中処理がαβ探索の反復に結びついているからである。
AlphaZero型探索の設計書も、MCTSは探索回数に基づく独自の時間管理を持つと定めている。

`Searcher`の欄を`pub(super)`にするのは、Rustの非公開の欄は定義したモジュールとその子孫からしか見えないためである。
`alphabeta/mod.rs`に構造体を置けば欄を非公開のまま子から使えるが、`mod.rs`が約200行の状態管理を抱えてモジュール宣言が埋もれる。
`alphabeta/searcher/`の下へ子モジュールを並べる案は、置換表やSEEのように`Searcher`の外にある部品との区別を1段深いディレクトリで表すだけで、得るものがない。
`pub(super)`は`alphabeta`の外へ欄を公開しないので、カプセル化の範囲は現状の`search`モジュール内と同じ程度に保たれる。

試験を`alphabeta/tests/`にまとめるのは、既存の試験の多くが制限の構築から探索の実行、置換表の中身の確認までを1つの試験で通して検査しており、1つのモジュールに帰属させられないからである。
主題別のファイルに分けたうえで、試験が参照する非公開の項目は`pub(super)`によって`alphabeta`の子孫から見える。
一方、`see.rs`、`tt.rs`、`correction.rs`、`src/eval/`の各ファイルのように、既にファイル内の`mod tests`で単体試験を持つものは、そのまま移動先のファイルに残す。

学習データの形式を`src/eval/`から出すのは、3つの形式が評価値を計算せず、`core`の局面型だけに依存する交換形式だからである。
3つの形式とは、minaseが定めた二進形式MNSD（自己対局や人間の棋譜から集めた学習局面と教師値の記録）とMNRS（MNSDの局面を深い探索で評価し直した値の記録）、およびMNSDの生成条件を記録する来歴JSONである。
`src/eval/data/`へまとめる案は、評価関数のモジュールに評価関数でないものを残す点で現状と変わらない。
移動先で`training::training_data`と名前が重複するのを避けるため、MNSDのモジュール名を`records`とする。
公開パス`minase::eval::training_data`、`minase::eval::rescore`、および`minase::eval::provenance`がなくなるので、この変更は互換性を壊す変更としてコミットする（CONTRIBUTING.mdの「コミットとリリース」）。

旧パスの再公開を置かないのは、移動するモジュールの利用者がこのリポジトリ内の実行ファイル（`src/bin/`）と試験（`tests/`）だけであり、書き換えで足りるからである。兄弟リポジトリの`minase-gui`と`minase-lishogi-bot`も、移動するモジュールを参照していない。
ただし、`minase::search::TranspositionTable`のように境界の`src/search/mod.rs`が現在すでに公開している名前は、再編後も同じパスで公開する。
これは旧パスの互換ではなく、境界の公開面そのものである。

## 再編後の配置

### 探索部

| パス | 置くもの |
|---|---|
| `src/search/mod.rs` | モジュールの宣言、公開する名前の`pub use`、および`MATE`、`MAX_PLY`、`MATE_THRESHOLD`、`DRAW_SCORE`、`DEFAULT_THREADS`。 |
| `src/search/limits.rs` | `SearchLimits`、`SearchLimitKind`、`FiniteSearchLimits`、`ClockLimits`。 |
| `src/search/snapshot.rs` | `SearchSnapshot`と、探索局面キー（RULES.md第24条第1項が同一局面の条件とする盤面、手番、および一時的な権利と制限をまとめたハッシュ値）を計算する`search_key`。 |
| `src/search/events.rs` | `SearchEvent`、`StopReason`、`SearchResult`。 |
| `src/search/error.rs` | `SearchError`。 |
| `src/search/handle.rs` | `SearchHandle`、`start_search`、`search`。 |
| `src/search/alphabeta/mod.rs` | 子モジュールの宣言と`INFINITY`。 |
| `src/search/alphabeta/team.rs` | 探索チーム（Lazy SMP）の共有状態と実行。`SearchOutcome`、`WorkerOutcome`、`SharedSearch`、`HardLimit`、`stop_reason_priority`、`stop_reason_from_priority`、`run_search_team`、`run_worker_team`、`run_worker_guarded`、`select_worker_outcome`。 |
| `src/search/alphabeta/deepening.rs` | 主ワーカーと補助ワーカーの反復深化。`run_main_worker`、`run_auxiliary_worker`、`auxiliary_depths`。 |
| `src/search/alphabeta/searcher.rs` | ワーカーごとの探索状態。`Searcher`、`PonderIteration`、`HistoryTable`、`KillerTable`、`KILLER_COUNT`、`STOP_CHECK_INTERVAL`、`new_searcher`、`enter_node`、`check_time`、`update_pv`。 |
| `src/search/alphabeta/root.rs` | 根の探索とaspiration windows。`AspirationWindow`、`aspiration_delta`、`grow_aspiration_delta`、`search_iteration`、`search_root`。 |
| `src/search/alphabeta/negamax.rs` | 内部ノードの探索。`negamax`、`search_move`。 |
| `src/search/alphabeta/quiesce.rs` | 静止探索と捕獲の段階生成。`quiesce`、`QsearchCapture`、`CaptureRanks`、`QsearchBuffers`。 |
| `src/search/alphabeta/ordering.rs` | 着手の順序付けと手の成績の表の更新。`MovePicker`、`MovePickerStage`、`OrderedMoveKey`、`MoveOrderKey`、`move_order_key`、`piece_at_for_ordering`、`order_captures`、`order_moves`、`record_quiet_beta_cutoff`。 |
| `src/search/alphabeta/pruning.rs` | 枝刈りと削減の式。`LMR_MAX_REDUCTION`、`LMR_MOVE_COUNT`、`lmr_table`、`lmr_base`、`lmr_reduction`、`futility_margin`、`see_margin`、`null_move_reduction`、`capture_is_pruned_by_see`。 |
| `src/search/alphabeta/royal.rs` | 王駒の捕獲と利きの判定。`royal_under_attack`、`captures_last_royal`、`captured_last_royal`、`captures_all_royals`。 |
| `src/search/alphabeta/time.rs` | 時間管理。`TimeBudget`、`STABLE_ITERATIONS`、`stable_signal`、`should_start_next_iteration`、`iteration_prediction_fits`、`moves_to_go`、`clock_budget`、`time_budget`、`to_u64_ms`。 |
| `src/search/alphabeta/tt.rs` | 現行の`src/search/tt.rs`をそのまま移す。 |
| `src/search/alphabeta/see.rs` | 現行の`src/search/see.rs`をそのまま移す。 |
| `src/search/alphabeta/correction.rs` | 現行の`src/search/correction.rs`をそのまま移す。 |
| `src/search/alphabeta/params.rs` | 現行の`src/search/params.rs`をそのまま移す。 |
| `src/search/alphabeta/tests/` | 現行の`tests.rs`、`correction_tests.rs`、および`search_captures_tests.rs`を主題別に分けて移す。 |

表に挙げた型の`impl`（`Display`、`Error`、`Default`、`Drop`などのトレイト実装を含む）は、表で別のファイルを指定したメソッドを除き、型と同じファイルへ移す。

`royal.rs`を独立させるのは、王駒の判定が`negamax`、`search_move`、`quiesce`、および`QsearchBuffers`の4か所から使われ、どれか1つのファイルに属させると他のファイルがそのファイルの内部を参照することになるからである。
同じ理由で、`piece_at_for_ordering`は`move_order_key`と`QsearchBuffers`が共用するが、着手の整列キーの一部なので`ordering.rs`に置く。

`alphabeta/tests/`は、共通の補助関数（規則、制限、置換表、局面の作成、`run_negamax`、`run_quiesce`、`run_search`、`start`、`event_reports`など）を`mod.rs`に置き、試験を次の主題別のファイルに分ける。
ファイルの名前は、試験の対象となる実装のファイル名にそろえる。

| ファイル | 置く試験 |
|---|---|
| `root.rs` | aspiration windowsと根の窓（`aspiration_*`、`root_*`）。 |
| `pruning.rs` | LMR、futility pruning、およびSEE pruning（`lmr_*`、`futility_*`、`see_pruning_*`）。 |
| `negamax.rs` | IIR、null move、および深さ0の置換表の値の扱い（`iir_*`、`null_move_*`、`depth_zero_*`）。 |
| `quiesce.rs` | 静止探索（`quiescence_*`）。 |
| `ordering.rs` | 段階生成の着手選択と参照実装との照合（`staged_picker_*`）。 |
| `royal.rs` | 王駒への利きの判定（`royal_under_attack_*`）。 |
| `scoring.rs` | 詰み、王駒の捕獲、および反復の評価値（`depth_one_capture_*`、`capture_of_the_first_*`、`without_history_*`、`free_lion_*`、`lion_double_*`、`opponent_with_no_legal_moves_*`）。 |
| `limits.rs` | 制限とスナップショットの構築（`depth_limit_*`、`limits_without_*`、`zero_search_budgets_*`、`snapshot_*`、`synchronous_search_*`）。 |
| `handle.rs` | 探索APIの入口、停止、ハンドルの回収、および決定性（`search_apis_*`、`node_limit_*`、`stop_or_tiny_*`、`infinite_limits_*`、`progress_depths_*`、`join_*`、`dropping_*`、`search_handle_*`、`search_with_node_limit_*`）。 |
| `team.rs` | 複数ワーカーの探索チーム（`panicking_worker_*`、`multi_worker_*`、`four_worker_*`、`external_stop_*`、`auxiliary_depth_*`、`worker_outcome_*`）。 |
| `time.rs` | 時間予算と反復の開始判定（`clock_*`、`opening_coefficient_*`、`moves_to_go_*`、`movetime_*`、`next_iteration_*`、`stable_signal_*`）。 |
| `tt.rs` | 置換表の詰め込み、置換、消去、および容量変更（`packed_moves_*`、`tt_*`、`idle_resize_*`、`atomic_tt_*`、`malformed_atomic_*`、`invalid_table_sizes_*`、`illegal_transposition_table_move_*`、`repetition_draw_values_*`）。 |
| `ponder.rs` | 先読み（`ponder_*`、`ponderhit_*`）。 |
| `contracts.rs` | [棋力向上段階6](strength-stage6.md)で固定した、長い対局履歴と固定ノード数での探索結果の回帰試験（`stage6_*`）。 |
| `tuning.rs` | 係数をUSIから変更できる調整用ビルド（`tuning`機能）での、係数の既定値と設定の試験（`tuning_*`）。 |
| `correction.rs` | 現行の`correction_tests.rs`の全体。 |
| `captures.rs` | 現行の`search_captures_tests.rs`の全体。 |

表に挙げた接頭辞に当てはまらない試験は、検査している実装のファイルに対応する試験ファイルへ置く。
試験の名前と中身は変えない。

### 評価関数

| パス | 置くもの |
|---|---|
| `src/eval/mod.rs` | モジュールの宣言、`evaluate`、および`Pst`と`weights`の再公開（現行どおり）。 |
| `src/eval/handcrafted.rs` | 変更しない。 |
| `src/eval/pst/mod.rs` | `Pst`、`PIECE_STATE_COUNT`、`piece_state_of`、`Pst`の読み出し（`piece_value`、`piece_value_of_state`、`pawn_value`、`max_promotion_gain`、`k`、`checksum`）、`interpolate`、`evaluate`、埋め込み重み（`EMBEDDED`、`WEIGHTS`、`weights`）、および`EVALUATION_LIMIT`。 |
| `src/eval/pst/features.rs` | 現行の`src/eval/features.rs`をそのまま移す。 |
| `src/eval/pst/format.rs` | MNPT形式（学習PSTの重みファイルの二進形式）の復号と検査。`HEADER_LENGTH`、`FORMAT_VERSION`、`Pst::decode`、`Error`、`validate_piece_values`、`validate_piece_value`、`read_u32`。 |
| `src/eval/pst/accumulator.rs` | 差分更新。`PstAccumulator`、`add_feature`、`refresh_accumulator`、`update_accumulator_after_move`、`update_accumulator_after_null`、`evaluate_accumulator`。 |
| `src/eval/pst/tests.rs` | 現行の`pst.rs`の`mod tests`を、補助関数を含めてそのまま移す。 |

`Error`は`format.rs`に定義し、`pst/mod.rs`から`pub use format::Error;`で公開して、公開パス`minase::eval::pst::Error`を保つ。
`Pst`の欄は`pst/mod.rs`に定義するので、子の`format.rs`と`accumulator.rs`から非公開のまま使える。
PSTの試験を1つのファイルにまとめたまま移すのは、`valid_bytes`や`distinct_pst`などの補助関数を復号、差分更新、評価の試験が共用しており、主題別に分けると補助関数の置き場所だけのためにモジュールが増えるからである。

### 学習データの交換形式

| パス | 置くもの |
|---|---|
| `src/training/mod.rs` | モジュールの宣言。 |
| `src/training/records.rs` | 現行の`src/eval/training_data.rs`（MNSD）。 |
| `src/training/rescore.rs` | 現行の`src/eval/rescore.rs`（MNRS）。 |
| `src/training/provenance.rs` | 現行の`src/eval/provenance.rs`（来歴JSON）。 |

`src/lib.rs`に`pub mod training;`を加え、crateのdocコメントのモジュール一覧に`training`を加える。

## 可視性の規則

Rustでは、非公開の項目と欄は、定義したモジュールとその子孫からしか見えない。
移動によって参照元が定義元の子孫でなくなる項目は、参照に必要な最小の可視性へ広げる。
一般の規則は次の2つである。
`alphabeta`の子モジュールで定義し、`alphabeta`とその子孫（`alphabeta/tests/`を含む）から使う項目と欄は`pub(super)`とする。
境界（`src/search/`直下の子モジュール）で定義し、`search`の他の子モジュールや`alphabeta`から使う項目と欄も`pub(super)`とする。`pub(super)`は親の`search`とその子孫すべてから見えるからである。

境界と`alphabeta`をまたぐ項目は、推測に任せず次のとおり指定する。

| 項目 | 可視性 | 理由 |
|---|---|---|
| `alphabeta`モジュール | `pub(crate) mod` | `params`を`src/protocol/usi.rs`から参照する経路になる。 |
| `alphabeta::team`モジュール、`run_search_team`、`SearchOutcome`とその欄`result`、`elapsed`、`pv`、`stop_reason` | `pub(in crate::search)` | `handle.rs`が探索チームを実行し、結果の欄を読む。 |
| `alphabeta::tt`モジュール | `pub(in crate::search)` | 境界の`src/search/mod.rs`が`TranspositionTable`、`TranspositionTableError`、`DEFAULT_SIZE_MB`を`pub use`で公開する。 |
| `alphabeta::params`モジュール | `pub(crate) mod` | `usi.rs`が調整用ビルドの`PARAMETERS`と`set`を使う。参照を`crate::search::alphabeta::params`へ書き換える。 |
| `SearchHandle`の欄、`SearchSnapshot`の欄と`from_parts`、`SearchLimits::finite`、`FiniteSearchLimits`とその欄、`ClockLimits`の欄、`search_key` | `pub(super)` | `handle.rs`、`alphabeta`、および`alphabeta/tests/`が使う。 |

`eval`の側では、`accumulator.rs`の`add_feature`と`PstAccumulator`の欄を`pub(super)`とし、親の`evaluate`と`pst/tests.rs`から使えるようにする。
探索が使う`PstAccumulator`へは、`accumulator`を`pub(crate) mod accumulator`として`crate::eval::pst::accumulator::PstAccumulator`の新しいパスで到達する。
`format.rs`の項目のうち親と`pst/tests.rs`が使うものも`pub(super)`とする。
`pub(crate)`を超える可視性の拡大と、新しい`pub`の追加は行わない。

## 文書のパス参照

`docs/`（`docs/measurements/`と`docs/audits/`を除く）、`AGENTS.md`、`CLAUDE.md`、`CONTRIBUTING.md`、`README.md`、`RULES.md`、およびコードのdocコメントにある`src/search/`と`src/eval/`のパスの参照を、新しい配置へ書き換える。
`src/search/mod.rs`と`src/search/tests.rs`への参照は、同じ文にある関数名、型名、または試験名から「再編後の配置」の表に従って移動先のファイルを決める。
関数名などの手がかりがなく、探索部全体を指す参照は`src/search/`へ改める。
現在存在しないファイル（`src/search/parameters.rs`、`src/eval/fm.rs`、`src/eval/nnue.rs`など）への参照は、書かれた時点の記録として残す。
行番号の付いた参照は、同じ文の関数名などから移動先の行を特定できれば新しい行番号へ改め、特定できなければ行番号を削ってパスだけを残す。
移動先のファイルで古い行番号を残すと、無関係な行を指してしまうからである。

[src構成の整理](src-layout.md)の「探索部・評価関数はcoreの外にトップレベルで並べる」の節に`training`の追加と依存の向きを、[AlphaZero型探索](alphazero.md)の「共存の構造」の節とフェーズ1に、`alphabeta/`への再編が本書で完了していることを反映する。

## 実装フェーズ

masterから`search-eval-layout`ブランチを切り、次の順にコミットする。
各コミットは単独でビルドと試験が通る状態にする。

1. `refactor(search): split the search module by purpose`。探索部を「探索部」の表の配置へ移す。
2. `refactor(eval): split the learned PST into submodules`。学習PSTを`src/eval/pst/`へ分ける。
3. `refactor(training)!: move training data formats out of eval`。学習データの形式を`src/training/`へ移し、本文に`BREAKING CHANGE:`フッターを書く。
4. `docs: update source paths after the module split`。文書のパス参照を書き換える。

コードの移動（1から3）はcodexへ委任し、文書の書き換え（4）はClaudeが行う。
codexへの指示には、名前の変更（`records`を除く）、シグネチャの変更、処理の並べ替え、インライン属性の変更、および旧パスの再公開を禁じることを含める。
あわせて、「適用範囲」の節の3種類の書き換え（完全修飾パス、埋め込みファイルの相対パス、子プロセスへ渡す試験名）を行うことと、新しいファイルの先頭に既存のファイルと同じ体裁の日本語の`//!`コメントを置くことを指示する。

## 検証

基点コミットは、`search-eval-layout`ブランチを切ったmasterのコミットとする。
再編の前に、基点コミットで`cargo run --release --bin bench -- --depth 6`の標準出力を保存する。
benchは固定局面を探索し、局面ごとのノード数、最善手、評価値、経過時間、およびNPSを1行ずつ出力する。
各コードのコミットの後に同じコマンドを実行し、経過時間とNPSの欄を除く標準出力の全行が基点と一致することを確かめる。
深さ6を使うのは、既定の深さ3では発火しない枝刈り（IIRは深さ3以上、null moveとLMRは残り深さに応じて働く）も通し、[AlphaZero型探索](alphazero.md)が再編の確認に定めた条件と同じにするためである。

各コミットで、`cargo fmt --check`、`cargo clippy --all-targets`（警告0件）、`cargo test`、および`cargo test --features tuning`を実行し、試験の件数が基点と同じで全件通ることを確かめる。
`tuning`の機能は係数の読み出しと`PARAMETERS`および`set`を別の形で生成し、USIの係数の契約を検査する試験もこの機能でだけ有効になるので、既定のビルドだけでは移動の誤りを検出できない。
`cargo test`の出力にある試験名の一覧を基点と比べ、名前の末尾（モジュールパスを除いた部分）の集合が一致することも確かめる。
`tuning_parameters_and_usi_contract_in_isolated_process`は自分自身を子プロセスとして`--exact`で起動し、子の終了状態だけを検査するので、完全修飾名を改め忘れると子が0件の試験を実行して成功してしまう。
この試験は`--nocapture`で実行し、各子プロセスの出力に`1 passed`が現れることを確かめる。

最後のコードのコミットで、benchのNPSを基点と比べる。
`lto = true`と`codegen-units = 1`の下ではファイルの分割は生成コードを変えない見込みだが、見込みではなく実測で確かめる。
計測は、[進行中の測定を確認してから](../lessons/check-running-measurements-before-cpu-load.md)、`taskset`で性能コア1つに固定し、基点と再編後を交互に3回ずつ行う。
再編後の中央値が、基点の3回の最小値から最大値までの範囲（同じバイナリの計測の揺れ）に収まることを確かめる。
収まらない場合は、[呼び出し元の機械語を比較する教訓](../lessons/check-caller-codegen-for-local-optimizations.md)に従って、インライン化が変わった関数を調べる。

文書の書き換えの後に、`docs/`（`docs/measurements/`と`docs/audits/`を除く）とリポジトリ直下の文書に、再編で消えたパス（`src/search/tt.rs`、`src/search/see.rs`、`src/search/correction.rs`、`src/search/params.rs`、`src/search/tests.rs`、`src/search/correction_tests.rs`、`src/search/search_captures_tests.rs`、`src/eval/pst.rs`、`src/eval/features.rs`、`src/eval/training_data.rs`、`src/eval/rescore.rs`、`src/eval/provenance.rs`）への参照が残っていないことを`grep`で確かめる。

## 完了条件

4つのコミットが`search-eval-layout`ブランチにあり、「検証」の節の確認がすべて満たされ、ブランチがmasterへ統合されたときに完了とする。
本書は独立したリファクタリングなので、[src構成の整理](src-layout.md)と同じく、ROADMAP.mdのマイルストーン状態表には行を追加しない。

## 参考資料

- [src構成の整理](src-layout.md)。crate構成、coreの責務基準、および依存の向きを定める。
- [AlphaZero型探索と深層強化学習](alphazero.md)。`src/search/`の境界と`alphabeta/`の並置を定める。
- Stockfish（手元の複製、コミット17a6c8f1）の`src/`。探索本体の`search.cpp`から、着手の選択`movepick.cpp`、手の成績の表`history.h`、時間管理`timeman.cpp`、置換表`tt.cpp`、およびスレッド管理`thread.cpp`を分けている。
