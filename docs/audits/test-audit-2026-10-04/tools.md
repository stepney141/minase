対象は、対局ハーネスと補助プログラムのRustテスト160件、学習ツールと取得スクリプトのPythonテスト145件、測定記録と監査に同梱されたPythonテスト130件の計435件である。
Rust側の範囲は `src/harness/**`、`src/bin/match_runner/**`、`src/bin/match_report/**`、[tests/match_runner.rs](../../../tests/match_runner.rs)、`src/bin/spsa_runner/**`、`src/bin/selfplay_gen/**`、`src/bin/random_play/**`、[src/bin/bench.rs](../../../src/bin/bench.rs)、`src/training/**` である。
`src/stats.rs` と `src/eval/**` は[探索・評価・統計](search-eval.md)で扱う。
Python側の範囲は `tools/train/tests/*.py`、[scripts/test_fetch_lishogi_games.py](../../../scripts/test_fetch_lishogi_games.py)、`docs/measurements/**/test_*.py`、および [docs/audits/third-party-code-provenance-2026-09-28/mechanical/](../third-party-code-provenance-2026-09-28/mechanical/) のテストである。
全テスト本文と、対応する実装・補助関数を確認した。
本番コードとテストは変更していない。
実行結果は[全体の報告](../test-audit-2026-10-04.md)に記載した。
行番号は監査時点（master 9027662）のものである。

範囲内のRustテストはどれも1.8秒未満で、ハーネス全体でも約9秒にとどまる。
学習ツールのPython 132件は23.1秒で、その約半分は実学習の子プロセスを起動する4件が占める。
したがって以下の候補の根拠は実行時間ではなく、同じ分岐の重複、外部ライブラリが保証する事項の再確認、および期待値や import の書き換え負担である。

判断の根拠は [docs/guides/sprt.md](../../guides/sprt.md)、[docs/guides/pst-training.md](../../guides/pst-training.md)、[docs/plans/spsa.md](../../plans/spsa.md)、[docs/plans/spsa-apply.md](../../plans/spsa-apply.md)、[docs/plans/spsa-gain-calibration.md](../../plans/spsa-gain-calibration.md)、[docs/plans/ponder.md](../../plans/ponder.md)、[docs/plans/train-tools-layout.md](../../plans/train-tools-layout.md)、[docs/plans/bin-harness-layout.md](../../plans/bin-harness-layout.md)、[docs/plans/spec-first-tests/matrices/d8-stats-harness.md](../../plans/spec-first-tests/matrices/d8-stats-harness.md) と [ledgers/d8.md](../../plans/spec-first-tests/ledgers/d8.md)、および2026-09-12の監査の [tools.md](../test-audit-2026-09-12/tools.md) である。
2026-09-12の監査が保持と決めたものは、その後に重複先が追加されるなど状況が変わった場合を除いて再提案していない。

以下は、壊れているか製品を観測しないため、優先して整理できる候補である。

T01では、合成目的関数と利得較正の模擬比較が、係数の件数を22に固定しているため現在は実行できない。

対象は [src/bin/spsa_runner/tests.rs:701](../../../src/bin/spsa_runner/tests.rs:701) `synthetic_objective_improves_at_least_eighteen_of_twenty_seeds` と、[src/bin/spsa_runner/tests/simulation.rs:136](../../../src/bin/spsa_runner/tests/simulation.rs:136) の補助関数 `ranges()` である。
`ranges()` は [simulation.rs:747](../../../src/bin/spsa_runner/tests/simulation.rs:747) `gain_simulation` と [simulation.rs:770](../../../src/bin/spsa_runner/tests/simulation.rs:770) `gain_decay_diagnostic` が使う。
どちらも `#[ignore]` 付きで、`include_str!` で探索係数の表を読み込み、件数が22であることを表明する（[tests.rs:734](../../../src/bin/spsa_runner/tests.rs:734)、[simulation.rs:168](../../../src/bin/spsa_runner/tests/simulation.rs:168)）。
係数表は2659bfbで26個、c3ca7c0で30個、a2e35d7で35個に増えたため、単独で実行すると `left: 35, right: 22` で即座にpanicする。
さらに、階段状の係数として扱う添字10は、22個の表では NullMoveBase を指していたが、現在は ReverseFutilityMargin を指しており、表の並びにも依存している。

この結果、[spsa.md](../../plans/spsa.md) の完了条件（289行、合成目的関数の試験を通すこと）と、[spsa-gain-simulation.md](../../measurements/spsa-gain-simulation.md) の再現コマンド（49行、143行）は、どちらも満たせない状態にある。
これは削除ではなく修理の候補である。
係数表の読み込みと件数・名前の照合を削り、22個の (名前, 最小, 最大) をテスト内の定数として持たせる。
その表は simulation.rs の20〜43行の `MEASURED` に既にあり、spsa.md:278 も「係数は22個、うち1個は階段状」と事前に定めている。
修理すれば、更新ループの収束不良を見逃さず、C5を選んだ根拠も再現できる。

推奨: 適用する。

T02では、利得較正の所要時間を測るだけの試験が残っている。

削除対象は [src/bin/spsa_runner/tests/simulation.rs:711](../../../src/bin/spsa_runner/tests/simulation.rs:711) `gain_simulation_timing`（ignored）である。
20シードの所要時間を測って本実験の時間を外挿して表示するだけで、assertは `runs.len() == 20` しかない。
[spsa-gain-calibration.md](../../plans/spsa-gain-calibration.md) の94行はこれを本実験の前段の一度きりの手順と位置づけており、計画は2026-09-26に完了している。
現状はT01の `ranges()` で失敗する。
失う現実的な検出力はない。

一方、[simulation.rs:573](../../../src/bin/spsa_runner/tests/simulation.rs:573) `comparison_uses_paired_seeds_and_reference_estimator` はテスト専用の集計関数 `compare` を検査するもので、製品コードを観測しない。
ただし T01 で `gain_simulation` を修理して残すので、その集計の誤りを検出する役割が残る。

推奨: `gain_simulation_timing` は適用し、`comparison_uses_paired_seeds_and_reference_estimator` は保持する。

T03では、完了した計画の測定スクリプトに同梱されたPythonテストが、どこからも実行されずに残っている。

削除対象は次の6ファイル、120件である。

- [docs/measurements/qsearch-output-diagnostics/test_diagnose.py](../../measurements/qsearch-output-diagnostics/test_diagnose.py)（25件）
- [docs/measurements/qsearch-output-leaf-sample/test_leaf_sample.py](../../measurements/qsearch-output-leaf-sample/test_leaf_sample.py)（21件）
- [docs/measurements/qsearch-output-search-trace/test_trace_roots.py](../../measurements/qsearch-output-search-trace/test_trace_roots.py)（35件）
- [docs/measurements/relational-correction-prep/test_phase0.py](../../measurements/relational-correction-prep/test_phase0.py)（28件）
- [docs/measurements/relational-correction-prep/prep/test_phase3.py](../../measurements/relational-correction-prep/prep/test_phase3.py)（8件）
- [docs/measurements/relational-correction-prep/prep/test_prep.py](../../measurements/relational-correction-prep/prep/test_prep.py)（3件）

これらを収集・実行する設定はない。
`conftest.py`、pytestの設定、CIはなく、唯一の標準の実行経路である pst-training.md:22 の `unittest discover -s tools/train/tests` は docs/ を探さない。
後半の3ファイルは現在のmasterで既に動かない。
`test_phase3.py` の対象モジュールは存在しない `mnpt_v3` を import し（train-tools-layout.md:266 もこれを認めている）、`test_prep.py` は存在しない `data/relational-correction/fm-eval-tools/...` を import する。
`test_phase0.py` は、学習ツールの環境にも `uv.lock` にもない pytest を使い、309〜317行では壁時計で2秒未満を表明する。

対象の2計画、[qsearch-output-training.md](../../plans/qsearch-output-training.md) と [relational-correction.md](../../plans/relational-correction.md) は、どちらも不採用で完了している。
結論はスクリプトの正しさに依存せず、現行の計画もこれらのスクリプトを再利用しない。
各テストは `subprocess.Popen` を模擬した状態で測定スクリプトだけを動かし、`src/` を観測しない。
一方で、学習ツールの再編（edfe6cc）ではこのうち3ファイルの import を書き換える作業が生じており、残せば今後のツール変更のたびに同じ作業が要る。

文書はこれらのテストを、測定時点の検証の記録として参照している。
削除する場合は、次の5箇所を、テストが残るコミット（`test_diagnose.py`、`test_leaf_sample.py`、`test_trace_roots.py` は0a0171f、`test_phase0.py` は7988c42、`prep/` の2ファイルはa0b666e）への参照に書き換える。

- [qsearch-output-diagnostics/README.md](../../measurements/qsearch-output-diagnostics/README.md) の188行の検証コマンド
- [relational-correction-prep/prep/README.md](../../measurements/relational-correction-prep/prep/README.md) の72行の検証コマンド
- [qsearch-output-search-trace.md](../../measurements/qsearch-output-search-trace.md) の200行の保存物一覧
- [qsearch-output-leaf-sample.md](../../measurements/qsearch-output-leaf-sample.md) の128行の保存物一覧
- [relational-correction-prep.md](../../measurements/relational-correction-prep.md) の261行の保存物一覧

完了済みの train-tools-layout.md は経緯の記録なので書き換えない。
凍結して残す案は採らない。
プロジェクトはこれまでツールの変更に合わせてテストを追従させてきており、凍結すれば `test_phase3.py` のように動かないテストが再編のたびに増える。
失う現実的な検出力はない。

推奨: 適用する。

以下は、同じ分岐や同じ契約を別のテストが確実に検査しており、削除または統合できる候補である。

T04では、マニフェストの版2の拒否と版3の拒否が、同じ1分岐を検査している。

統合対象は [src/bin/match_runner/storage.rs:602](../../../src/bin/match_runner/storage.rs:602) `resume_rejects_format_version_two` と [src/bin/match_report/report.rs:502](../../../src/bin/match_report/report.rs:502) `report_rejects_format_version_two` である。
統合先はそれぞれ [storage.rs:803](../../../src/bin/match_runner/storage.rs:803) `resume_rejects_version_three_before_decoding_new_fields` と [report.rs:763](../../../src/bin/match_report/report.rs:763) `report_rejects_version_three_before_decoding_new_fields` である。

版の検査は storage.rs:214 の `!= FORMAT_VERSION` と report.rs:144 の `!= Some(FORMAT_VERSION)` の1分岐ずつで、版2も版3も同じ分岐を通る。
版3のテストは新しい欄をデコードする前に拒否することまで確かめるので、版2のテストより強い。
2026-09-12の監査が版2のテストを保持した時点では、版3のテストはまだなかった（b201f39で追加）。
統合先へは `error.kind() == InvalidData` のassertだけを移せばよく、report側は約25行のフィクスチャも消える。

統合先の report.rs:763 は、期待文字列 `"requires format version 4, found 3"` で版数4をリテラルとして固定している。
`FORMAT_VERSION` はこれまでに3回上がっている（3b35793、59846ec、b201f39）ので、統合の際に期待文字列を `FORMAT_VERSION` から組み立てる形へ改め、版が上がるたびの書き換えをなくす。
失う現実的な検出力はない。

推奨: report.rs:763 の期待文字列を `FORMAT_VERSION` から組み立てることを条件に適用する。

T05では、マニフェストの先読み欄の検査に、構造体全体の比較とserdeの往復が重なっている。

一部削除の対象は [src/bin/match_runner/storage.rs:770](../../../src/bin/match_runner/storage.rs:770) `ponder_manifest_and_prediction_fields_are_required_and_round_trip` の771〜785行の先読みフラグ反転ループ、778行の `format_version == 4`、787〜792行の予想手欄の None と Some の往復である。

反転ループは storage.rs:221 の構造体全体の等価比較を検査しており、[storage.rs:672](../../../src/bin/match_runner/storage.rs:672) `resume_rejects_manifest_mismatch_and_saved_number_above_target` と [tests/match_runner.rs:296](../../../tests/match_runner.rs:296) の実プロセスでの不一致検査に重なる。
778行は版数のリテラルで、T04と同じく版が上がるたびに書き換えが要る。
`Option<String>` の往復はserdeが保証する。
793〜798行の、欄が欠けたマニフェストを拒否する検査はこのテストだけが持つので残す。
失う現実的な検出力はない。

推奨: 適用する。

T06では、取り込み順の検査が、構成上その誤りを検出できない入力を使っている。

削除対象は [src/bin/match_runner/scheduler.rs:254](../../../src/bin/match_runner/scheduler.rs:254) `completion_order_does_not_change_gsprt_stopping_result` である。
先頭ペアが遅れて届いても、度数、LLR、判定、停止ペア番号が逐次到着の場合と一致することを確かめる。
しかし合成したペアはすべて同じ得点分類（カテゴリ4）なので、度数とLLRは取り込み順に関係なく一致し、順序の誤りがあっても通る。

番号順の取り込みは、[scheduler.rs:174](../../../src/bin/match_runner/scheduler.rs:174) `out_of_order_completions_replenish_one_slot_each` が `integrated == [1, 2, 3]` で直接確かめ、実プロセスでは [tests/match_runner.rs:92](../../../tests/match_runner.rs:92) `random_usi_match_output_is_independent_of_concurrency` が確かめる。
このテストは2026-09-12の監査より前（3b35793）からあり、同監査の「番号順統計を残す」という判断に含まれていた可能性がある。
ただし上の理由で、番号順統計の保証はこのテストではなく保持先の2件が担っている。
失う現実的な検出力はない。

推奨: 適用する。

T07では、先読みの条件違反と資源数の欠如を、下位のテストと同じ分岐で重ねて検査している。

一部削除の対象は次の3箇所である。

- [src/bin/match_runner/manifest.rs:215](../../../src/bin/match_runner/manifest.rs:215) `ponder_concurrency_reserves_two_engines_per_game` の230〜232行の、資源数が None の3ケース。
- [tests/match_runner.rs:206](../../../tests/match_runner.rs:206) `ponder_invalid_conditions_do_not_create_run_directory` の4ケースのうち3ケース。
- [src/bin/match_runner/cli.rs:234](../../../src/bin/match_runner/cli.rs:234) `each_limit_is_shared_and_per_engine_overrides_are_optional` の275〜289行の「等価表現」ブロック。

manifest.rs の None の検査は、31〜39行の `ok_or_else` で先読みを読むより前の同じ分岐を通り、[manifest.rs:151](../../../src/bin/match_runner/manifest.rs:151) `default_concurrency_requires_resource_counts` が検査済みである。
先読み固有の結果0を確かめる `(Some(2), 1, 1, true)` のケースは残し、D8-HARN-27はそれで保たれる。

tests/match_runner.rs:206 の条件の網羅は、[cli.rs:357](../../../src/bin/match_runner/cli.rs:357) `ponder_arguments_require_two_usi_time_controls` が `validate_ponder`（cli.rs:91）に対して6ケースで済ませている。
このテスト固有の保証は、main.rs:35 の検証が main.rs:124 の実行ディレクトリ作成より前にあることだけで、1ケース残せばその順序の退行は検出できる。
削減できるのはプロセス起動3回である。

cli.rs の275〜289行は、clapが値をフィールドへ入れることだけを確かめている。
意味のある `unwrap_or(each)` の解決は main.rs:77〜78 と cli.rs:94 にあり、このブロックはそれを通らない。
前半に残る非対称ゲートの表現（D8-HARN-09）は保たれる。
失う現実的な検出力はない。

推奨: 適用する。

T08では、先読み付きの再開テストに、既存の再開テストと同じ観測が入っている。

一部削除の対象は [tests/match_runner.rs:235](../../../tests/match_runner.rs:235) `ponder_resume_preserves_pairs_and_replays_all_counts` の267行と274行のバイト不変のassert、および281〜282行の先読み欄の存在と null の検査である。

バイト不変は [tests/match_runner.rs:189](../../../tests/match_runner.rs:189) `resume_fills_the_lowest_gap_and_preserves_later_records` と重なる。
randomエンジンは予想手を出さないので、281〜282行は null しか観測できず、欄の欠落の拒否は T05 で残す storage.rs:793 が担う。
`ponder candidate:` 行を保存記録から再生して数える検査は [sprt.md](../../guides/sprt.md) の「先読みを対局条件に含める測定」が要求するので残す。
先読みフラグの不一致による拒否も、CLIからマニフェストへの配線を守る唯一の検査なので残す。
失う現実的な検出力はない。

推奨: 適用する。

T09では、先読み停止の後始末のテストに、対局経由のテストと同じ経路が含まれている。

一部削除の対象は [src/harness/engine/tests.rs:450](../../../src/harness/engine/tests.rs:450) `ponder_cleanup_keeps_result_when_stop_times_out_and_drop_reaps` の、停止に応答する場合（`responds = true`）の反復である。

応答ありの場合に停止してから記録する経路は、[engine/tests.rs:537](../../../src/harness/engine/tests.rs:537) `ponder_stops_before_resources_on_cutoff_and_opponent_failures` のCutoffのケース（540行）が、`play_game` 経由でCPU時間の下限まで含めて確かめている。
D8-HARN-25の固有部分、すなわち停止がタイムアウトしても結果を保持することと、Dropでプロセスを回収することは残る。
Python製の模擬エンジンの起動1回分（約0.1秒）を削減できる。
失う現実的な検出力はない。

推奨: 適用する。

T10では、利得の既定値の検査が、実装と同じ式で期待値を計算している。

統合対象は [src/bin/spsa_runner/tests.rs:160](../../../src/bin/spsa_runner/tests.rs:160) `calibrated_default_rates_match_c5_endpoints`、統合先は [tests.rs:198](../../../src/bin/spsa_runner/tests.rs:198) `rates_match_independent_fishtest_reference` である。

期待式 `c_end * N^γ` と `a_end * ((A + N) / (A + 1))^α` は、model.rs:70〜77 の `rates` と同じである。
統合先の独立した参照値は k=1 と k=N=1500 を既に含む。
一方で、`ALPHA` と `GAMMA` の定数値を間接的に固定しているのはこのテストだけなので、統合先の設定に `ALPHA` と `GAMMA` を使わせて、その役割を移す。
統合すれば失う現実的な検出力はない。

推奨: 適用する。

T11では、確率的丸めの検査に、実装と同じ式のループと、別テストと同じ再現性の確認が入っている。

一部削除の対象は [src/bin/spsa_runner/tests.rs:263](../../../src/bin/spsa_runner/tests.rs:263) `stochastic_rounding_is_unbiased_coupled_bounded_and_repeatable` の200,000回のループ全体と、`issue` を2回呼ぶ再現性の確認である。

ループ内の `floor(250.35 + u)` は model.rs:97〜103 の `round_pair` と同じ式である。
乱数の消費順と丸め結果は、[tests.rs:947](../../../src/bin/spsa_runner/tests.rs:947) `iteration_stream_draws_all_flips_before_per_pair_rounding` が独立したPythonの参照値で固定しており、最近接丸めへの変更や `uniform` の変更もそこで検出できる。
範囲の端での切り詰めの2件は残す。
失う現実的な検出力はない。

推奨: 適用する。

T12では、異常理由ごとの得点の検査が、ハーネス側のテストと同じ観測を5種類ぶん繰り返している。

一部削除の対象は [src/bin/spsa_runner/tests.rs:875](../../../src/bin/spsa_runner/tests.rs:875) `each_failure_reason_is_scored_as_the_offending_sides_loss` の `half_points` のassert、`r.d == 16` のassert、および5種類の理由のうち4種類である。

`half_points` は [src/harness/pair.rs:287](../../../src/harness/pair.rs:287) 付近のテストが検査し、理由ごとの計数は [src/harness/failure.rs:72](../../../src/harness/failure.rs:72) `failure_reasons_are_counted_separately_and_conserved` が検査する。
`d` はフィクスチャの `pair()` が投了から計算した分類に由来し、製品を観測していない。
このテスト固有の対象は runner.rs:30 と runner.rs:97 の `summary.failures` の集計だけなので、1種類の理由で集計を確かめる形に絞る。
失う現実的な検出力はない。

推奨: 適用する。

T13では、マニフェスト設定の検証と最終値の丸めに、同じ検証関数とstdの保証の再確認が重なっている。

対象は次の3件である。

- [src/bin/spsa_runner/tests.rs:976](../../../src/bin/spsa_runner/tests.rs:976) `session_rejects_counter_overflow_and_nonfinite_schedule` を [tests.rs:1118](../../../src/bin/spsa_runner/tests.rs:1118) `apply_validates_manifest_settings_before_reading_iterations` へ統合する。
- tests.rs:1118 から、係数の重複、start=99、c_end=0.0、r_end=-1.0 の4ケースを削る。
- [tests.rs:1292](../../../src/bin/spsa_runner/tests.rs:1292) `apply_rounds_half_values_away_from_zero_including_negative_values` から (0, −0.5) と (−2, +0.5) の2ケースを削る。

apply.rs:374 は `validate_settings` を呼び、tests.rs:1118 は `iterations = u64::MAX` と `r_end = f64::MAX` を既に含む。
tests.rs:976 に固有なのは `c_end = f64::MIN_POSITIVE` だけで、これは正で有限なので `validate_parameters` を通過し、`validate_settings` の下限側の判定まで届く。
このケースを1件統合先へ足せばよい。

tests.rs:1118 の4ケースは、`parse_parameters` 経由で同じ `validate_parameters`（params.rs:187）を検査済みである（tests.rs:354、365、373、376）。
[spsa-apply.md](../../plans/spsa-apply.md) の179行が要求するのは pairs=0 と最小最大の逆転の2件だけで、`validate_parameters` を経由する配線は min=401 と空の係数表のケースで残る。

tests.rs:1292 の実装は `value.round()`（apply.rs:247）なので、0から遠い側への丸めはstdが保証する。
spsa-apply.md:184 の「小数部0.5の値と負の値」という要求は、2.5→3、−2.5→−3、−2.25→−2 の3ケースで満たせる。
失う現実的な検出力はない。

推奨: 適用する。

T14では、自己対局データの付け直しと来歴の検査に、下位のテストが固定済みの組合せが含まれている。

一部削除の対象は、[src/bin/selfplay_gen/rescore.rs:361](../../../src/bin/selfplay_gen/rescore.rs:361) `rescore_resumes_exactly_and_rejects_condition_changes_and_torn_entries` のヘッダのオフセット8、40、204を改変するループと、[src/training/provenance.rs:216](../../../src/training/provenance.rs:216) `provenance_round_trips_all_supported_origins` で探索条件の2値を4通りの由来すべてで繰り返す部分である。

オフセットと欄の対応は [src/training/rescore.rs:460](../../../src/training/rescore.rs:460) `header_matches_exchange_layout` が、欄とエラーの対応は [rescore.rs:576](../../../src/training/rescore.rs:576) `resume_requires_every_header_field_and_source_identity` が全欄について固定している。
コマンドが `validate_resume` を呼ぶ配線（selfplay_gen/rescore.rs:86）は、nodes=201 のケースで残る。
探索条件の直列化は由来の組合せと独立した列挙型のrenameなので、2値は1つの組合せで検査すれば足りる。
失う現実的な検出力はない。

推奨: 適用する。

T15では、学習設定の検証を、同じ項目集合と同じ数値検査で何度も通している。

対象は次のとおりである。

- [tools/train/tests/test_workflow.py:86](../../../tools/train/tests/test_workflow.py:86) `test_output_k_must_be_explicit` と [:92](../../../tools/train/tests/test_workflow.py:92) `test_removal_penalty_must_be_explicit_for_every_model` を、[:77](../../../tools/train/tests/test_workflow.py:77) `test_missing_and_unknown_fields_are_rejected` の拒否ケース各1件として統合する。
- [:102](../../../tools/train/tests/test_workflow.py:102) `test_zero_removal_penalty_is_accepted_for_every_model` と [:134](../../../tools/train/tests/test_workflow.py:134) `test_mirrored_model_is_an_explicit_choice` を削除する。
- [:125](../../../tools/train/tests/test_workflow.py:125) `test_removal_penalty_rejects_negative_nonfinite_and_nonnumeric_values` を −0.01 の1ケースへ絞る。
- [:275](../../../tools/train/tests/test_workflow.py:275) `test_lookahead_config_domains_and_rescore_rejection` から値域の3ケース（gamma=1.0、plies=4097、plies=1.5）を削る。
- [:363](../../../tools/train/tests/test_workflow.py:363) `test_lambda_override_config_validation` の拒否値7つを1つへ減らす。
- [tools/train/tests/test_train.py:57](../../../tools/train/tests/test_train.py:57) `test_cli_options_reach_dataset_and_reject_invalid_values` の78〜87行の拒否ループを5値から1値へ減らす。
- [tools/train/tests/test_mnsd.py:201](../../../tools/train/tests/test_mnsd.py:201) `test_omission_explicit_zero_and_same_value_have_distinct_classes` の212〜216行のK推定と `build_targets` の検査を削る。

欠落の検査は workflow.py:85〜87 の項目集合の一致だけで、項目ごと・モデルごとの分岐はない。
`k` の省略を拒否するケースは、段階7の出力K固定の契約（pst-training.md）として統合先に1件残す。
係数0の受理は、setUpの設定読み込み（:63）と [:139](../../../tools/train/tests/test_workflow.py:139) `test_example_explicitly_disables_unselected_removal_penalty` が確かめ、mirroredの受理は [:112](../../../tools/train/tests/test_workflow.py:112) も確かめる。
非有限値と非数値の拒否は共有の `number()`（workflow.py:54〜57）が担い、同じ関数を [:147](../../../tools/train/tests/test_workflow.py:147) `test_invalid_numbers_seed_ranges_and_paths_are_rejected` が k=nan/inf/true で検査する。
先読みの値域は `validate_lookahead`（lookahead.py:8〜12）を workflow.py:127 が呼ぶだけで、[test_lookahead.py:37](../../../tools/train/tests/test_lookahead.py:37) `test_invalid_parameters_and_order` が網羅する。
混合比の値域は `validate_lambda_override`（mnsd.py:308）だけが判定し、[test_mnsd.py:226](../../../tools/train/tests/test_mnsd.py:226) `test_rejects_out_of_range_nonfinite_and_nonnumeric_values` が全値域を検査する。
λ=0でKを推定せず教師値を対局結果だけにする挙動は、[test_teacher.py:176](../../../tools/train/tests/test_teacher.py:176) `test_zero_lambda_never_estimates_or_uses_teacher_k` が `estimate_k` の呼び出しを禁止したうえで検査する。
各項目について、検査関数の呼び出しが消える不具合は残す1ケースで捕まる。
失う現実的な検出力はない。

推奨: 適用する。

T16では、準備記録の不一致による拒否を、3つの操作すべてで繰り返し確かめている。

一部削除の対象は、[test_workflow.py:318](../../../tools/train/tests/test_workflow.py:318) `test_lookahead_cpu_training_diagnosis_and_receipt_checks` の358行、[:374](../../../tools/train/tests/test_workflow.py:374) `test_lambda_override_prepare_mismatch_blocks_all_operations` の385行、[:391](../../../tools/train/tests/test_workflow.py:391) `test_lambda_override_training_diagnosis_and_record_mismatches` の432行にある操作ループで、いずれも `load_prepared` だけを呼ぶ形にする。

train と diagnose はどちらも先頭で `load_prepared` を呼ぶ（workflow.py:369、442）。
3操作がこの検査へつながっている配線は、[:715](../../../tools/train/tests/test_workflow.py:715) `test_source_changes_additions_and_deletions_block_all_later_stages` が3操作で確かめる。
拒否そのものは workflow.py:252〜254 と 271〜292 だけで起こり、9通り×3操作の大半は同じ分岐の再実行である。
失う現実的な検出力はない。

推奨: 適用する。

T17では、模擬prepareや学習の引数列の読み取りを、別々のテストが二重に持っている。

対象は次のとおりである。

- [test_workflow.py:289](../../../tools/train/tests/test_workflow.py:289) `test_prepare_records_lookahead_and_rejects_changed_setting` を、[:184](../../../tools/train/tests/test_workflow.py:184) `test_prepare_pins_generator_to_base_and_probe_to_training_tools_commit` へ統合する。
- [:769](../../../tools/train/tests/test_workflow.py:769) `test_training_subprocess_uses_package_module` と [:444](../../../tools/train/tests/test_workflow.py:444) `test_lambda_override_explicit_zero_is_forwarded_and_recorded` を、[:649](../../../tools/train/tests/test_workflow.py:649) `test_training_forwards_and_records_mirrored_penalty_and_explicit_k` へ統合する。
- [:497](../../../tools/train/tests/test_workflow.py:497) `test_generated_provenance_change_blocks_reuse` と [:676](../../../tools/train/tests/test_workflow.py:676) `test_training_forwards_explicit_zero_removal_penalty` を削除する。

:289 と :184 は、gitと `run_command` を同じ形で置換した模擬prepareを丸ごと実行しており、偽コマンドの定義が重複している。
:289 に固有なのは workflow.py:255 の先読みの照合だけで、統合先へ移せば残る。
:769 は、`run_command` を失敗させて引数列を読む点で :649 と同じ形で、違いはassertの1行だけである。

:444 は本体の子プロセスを起動する実学習で、Python全体の実行時間の約1割を占める。
ワークフロー側の固有の検査は workflow.py:418 の `is not None` だけで、模擬実行で確かめられる。
ただし統合先は子プロセスを置換するため、そのままでは学習器の command_train が0を偽として扱って上書きを落とす不具合を見逃す。
λ=0の上書きを train で受けるテストはこれだけなので、[test_train.py:230](../../../tools/train/tests/test_train.py:230) の `TrainHalfTest.arguments` を使う実学習の1件に `--lambda-override 0` を加えることを統合の条件とする。

:497 の来歴の照合は `verify_input`（workflow.py:176〜178）の1関数で、既存データ側の同じ照合は [:468](../../../tools/train/tests/test_workflow.py:468) `test_provenance_change_blocks_generate_train_and_diagnose` が、生成済みの受領記録の経路（workflow.py:264〜267）は [:520](../../../tools/train/tests/test_workflow.py:520) `test_modified_completed_output_is_rejected` が確かめる。
このテストは受領記録を自分で書くので、generate による記録の作成も検査していない。
:676 については、学習器が `--removal-penalty` を `required=True` で受ける（train.py:587）ので、省略されれば係数0の実学習テスト [:596](../../../tools/train/tests/test_workflow.py:596) `test_cpu_training_and_diagnostics_preserve_scale_and_outputs` の子プロセスが終了コード2で失敗する。
統合と条件を満たせば、失う現実的な検出力はない。

推奨: :444 は学習器側の実学習テストに `--lambda-override 0` のケースを1件加えることを条件に適用し、その他は適用する。

T18では、学習器の検査に、変わり得る出力のハッシュ固定、argparseの保証、同じ判定式の値違いが含まれている。

対象は次の3件である。

- [test_train.py:238](../../../tools/train/tests/test_train.py:238) `test_omission_preserves_pre_change_weight_bytes` を削除する。
- [test_train.py:295](../../../tools/train/tests/test_train.py:295) `test_cli_rejects_values_other_than_zero_and_one` を削除する。
- [test_train.py:613](../../../tools/train/tests/test_train.py:613) `test_nonfinite_adam_weights_are_rejected_before_any_projection` の nan、inf、−inf の3値を nan の1値へ減らす。

:238 は、半分割を指定しない学習で出力MNPTのSHA-256が固定値に一致することを確かめる。
本来の目的は半分割を導入したときの移行確認で、固定値は導入（7dd3aab）以降一度も書き換えられていない。
ただしそれは学習器を変えた計画がどれもmasterに統合されなかったためであり、学習ループ、乱数の消費順、損失式を意図して変えれば必ず書き換えが要る。
torchの版上げでも壊れ、README はこれを学習条件の変更として扱う。
学習出力の採否は対局測定で決めるので、重みのバイト列を固定する価値は低い。
残りのassert（train_half、件数、更新回数）は [:252](../../../tools/train/tests/test_train.py:252) の half=None のケースと重複する。
失うのは、学習出力の数値が意図せず変わる変更の自動検出である。

:295 の `type=int, choices=(0, 1)`（train.py:577）はargparseが保証し、choicesを外しても範囲外の値は `training_halves()[n]`（train.py:354）の IndexError で学習前に止まる。
:613 の判定は train.py:256 の `torch.isfinite(parameter).all()` の1式で、値の種類に分岐しない。
15ケースが5ケースになり、このファイルで2番目に長い実行時間の約3分の2を削れる。

推奨: 適用する。

T19では、駒除去損失の勾配と付け直し教師の一致を、より包括的なテストが検査している。

対象は、[tools/train/tests/test_removal.py:189](../../../tools/train/tests/test_removal.py:189) `test_loss_gradient_corrects_reversal_and_same_sign_worsening` の削除と、[tools/train/tests/test_teacher.py:128](../../../tools/train/tests/test_teacher.py:128) `test_direct_saved_scores_reproduce_fitted_k_and_targets` の [test_mnsd.py:399](../../../tools/train/tests/test_mnsd.py:399) `test_rescore_direct_input_equivalence_and_original_feature_rows` への統合である。

勾配の符号と大きさは [test_removal.py:144](../../../tools/train/tests/test_removal.py:144) `test_violation_gradient_is_constant_over_k_and_zero_at_each_boundary` が両方の基準と全領域で検査し、実際の更新による損失の低下は [test_train.py:693](../../../tools/train/tests/test_train.py:693) が検査する。
損失値そのものは test_removal.py:27 と :38 が敵駒を含めて独立計算と照合する。
失うのは、誤った基準側の勾配が MirroredEmbedding の重み共有を通って正しく流れることの、更新後の損失による確認である。
:144 は生のテンソル上の解析勾配で同じ性質を見ているので、この損失は小さい。

test_teacher.py:128 の、訓練と検証の両分割での gather と `build_targets` の一致は、統合先の423〜429行と同じ検査である。
固有のassertは教師Kの一致と `class_metadata` の一致で、統合先へ移せば残る。
統合先は train-tools-layout.md が保持を明記したテストなので、名前と置き場所は統合先に揃える。

推奨: 適用する。

T20では、Rust側に書き出し処理がない追加特徴ファイルの読み込みコードを、テストだけが支えている。

削除対象は [tools/train/tests/test_mnsd.py](../../../tools/train/tests/test_mnsd.py) の `KingFeatureSelectionTests` と `KingFeaturesTests`（計5件）、および [:399](../../../tools/train/tests/test_mnsd.py:399) のうちMNKFを扱う部分である。
あわせて、`minase_train` のMNKF読み込みコードも削除する。

MNKFの書き出しは段階9の完了時にmasterから外れており（[evaluation-terms-relearning.md](../../plans/evaluation-terms-relearning.md) の78行）、Rust側にも書き出すコードはない。
したがって、この読み込みコードに入力を与える経路が現在のmasterに存在せず、テストは未使用コードを検査している。
CLAUDE.md の方針は未使用コードの温存を禁じている。
評価特徴の再学習の計画はMNKFの書き出しと学習器の追加特徴の経路を戻す予定なので、そのときに書き出し側と合わせて git 履歴から復元する。
失う現実的な検出力はない。

推奨: 読み込みコードとともに削除し、評価特徴の再学習の計画で書き出しを戻すときに復元することを条件に適用する。

## 保持と判断した主なテスト

[tools/train/tests/test_lookahead_teacher.py](../../../tools/train/tests/test_lookahead_teacher.py) の2件は保持する。
診断が要求する標本を作る段階9の診断Aのスクリプトは2026-10-03に削除されたが、train-tools-layout.md（84〜87行）は lookahead_diag.py を名指しで残しており、既存の標本に対しては今も実行できる。

[docs/audits/third-party-code-provenance-2026-09-28/mechanical/test_checks.py](../third-party-code-provenance-2026-09-28/mechanical/test_checks.py)（10件）と同じディレクトリの extraction_tests.py は保持する。
監査は再実行を前提に作られており、[README.md](../third-party-code-provenance-2026-09-28/README.md) の119〜120行がこの2ファイルを再実行パイプラインの手順に挙げている。
`mutation_check.py` は `test_checks` を読み込み、`validate.py` は2ファイルを必須成果物として要求する。
期待値は実装ではなく spec.md から作られている。
同ディレクトリの `triage_work/common.py` の `test_ranges` はテストではなく、トリアージで使う補助関数である。

[scripts/test_fetch_lishogi_games.py](../../../scripts/test_fetch_lishogi_games.py) の13件は保持する。
取得スクリプトは実戦棋譜の変換の入力を作り、起案中のAlphaZero計画もlishogiの実戦棋譜を開始局面に使う。
ただし README の標準のテストコマンドは `tools/train/tests` だけを探すので、このファイルは標準の実行手順から外れている。

[engine/tests.rs:537](../../../src/harness/engine/tests.rs:537) `ponder_stops_before_resources_on_cutoff_and_opponent_failures`（1.23秒）の4ケースは、game.rs の別々の終局分岐（130、160、189、240行）に対応し、どれか1か所で記録の受け渡しを誤る配線を個別に検出するので保持する。
応答期限500 msがPythonの起動にも適用されるため、高負荷下では起動遅延で不安定になり得る。
不安定さが観測された場合は、ケースを削るのではなく期限を延ばす。
engine/tests.rs の他の先読みテストは、的中、外れ、期限、送信失敗、終局、先読み無効時の予想手保存、USI設定の送信順という ponder.md の別々の契約（D8-HARN-22〜26）を1つずつ担っている。

[tests/match_runner.rs](../../../tests/match_runner.rs) の元からある3件、すなわち並列度に依存しない出力、CECPとUSIの整合、欠番の再実行は、実プロセスでそれを確かめる唯一の検査なので保持する。
資源取得のテスト（resources.rs と environment.rs）も、統合テストがすべて `--concurrency` を明示するため、物理コア数やメモリ容量の取得が欠測になる退行を検出できるのはこれらだけである。
[failure.rs:72](../../../src/harness/failure.rs:72) は実装の写しに近いが、matchの腕のコピーペーストによる取り違えを検出し、端から端までのテストは異常件数0しか観測しないので保持する。

[cli.rs:294](../../../src/bin/match_runner/cli.rs:294) `hash_options_resolve_independently_for_each_engine` は2026-09-12の監査が縮小のうえ保持を決め、その時点で重複先も存在していたので再提案しない。
`search_limits_reject_malformed_inputs` の10ケースは limit.rs の別々の分岐を通り、`usi_options_are_sent_in_order_before_isready` と `decision_labels_match_the_sprt_md_vocabulary` は SPSAフェーズ2と sprt.md の判定表示語彙という文書上の契約を唯一担っている。

[replay.rs:508](../../../src/bin/match_runner/replay.rs:508) の `" trailing"` と `"trailing "` は同じ条件で拒否され、[harness/engine/cecp.rs:247](../../../src/harness/engine/cecp.rs:247) の `Some(1)` と `Some(512)` も同じ分岐を通る。
ただし費用はほぼ0で、削っても保守負担はほとんど変わらないので、候補にしなかった。

SPSAの [tests.rs:794](../../../src/bin/spsa_runner/tests.rs:794) `command_engine_runs_resumes_and_sets_only_listed_parameters` は、python3の模擬エンジンを起動して setoption の送信範囲と再開を実プロセスで確かめる唯一の端から端までの試験である。
符号、再開、煙試験の各テスト（tests.rs:288、468、562、616、947）は spsa.md の280行と289行が完了条件として要求する。
apply系の各エラー分類、CRLF、ロック、一時ファイルは spsa-apply.md の175〜185行の列挙に対応する。
[tests.rs:1370](../../../src/bin/spsa_runner/tests.rs:1370) `apply_rejects_unknown_manifest_fields_without_writing` は、新しいrunnerが欄を追加したマニフェストを古いapplyが黙って適用する版の食い違いを拒否する。
[tests.rs:924](../../../src/bin/spsa_runner/tests.rs:924) `worker_panics_cancel_other_workers_and_return_an_error` は、2026-09-12の監査の「停止・panic時のワーカー回収」の原則に当たる。

[selfplay_gen/play.rs:441](../../../src/bin/selfplay_gen/play.rs:441) `omitted_max_ply_uses_measured_generation_cap` は既定値4,000の固定に見えるが、match_runner の既定値4096との取り違えを検出できる唯一の検査である。
MNSDのヘッダに max_ply がないため、生成データからは気づけない。
`random_moves_accepts_only_zero_through_eighty` が守る0〜80の上限は、`plan_injections` の固定長配列を保護している（play.rs:244〜249）。

[test_workflow.py:753](../../../tools/train/tests/test_workflow.py:753) `test_root_matches_actual_worktree` は保持する。
import時の Cargo.toml 検査は階層のずれをほぼ防ぐが、起案中のcrate分割で `crates/*/Cargo.toml` ができると、誤った階層でも検査を通る可能性がある。
駒除去の式のテスト（test_removal.py の :189 以外）は、盤面から有理数で直接計算する独立の参照と照合し、切捨て、上限、王駒の除外、先獅子、補間の変化という境界を1つずつ受け持つ。
`test_rescore_rejects_partial_and_invalid_contract_fields`（test_mnsd.py:378）は、Pythonの読み込み側で唯一の交換形式の契約検査である。
test_comparison.py の238行と280行は、診断工程のRustとPythonの照合が実際に発火することを確かめる唯一のテストである。

test_mnsd.py の440〜458行の `TrainHalfTest.setUp` は test_train.py から分割したときの複製で、`self.initial` のMNPTの作成とtorchのスレッド数の設定は test_mnsd.py 側では使われていない。
テストを失わずに削れる補助処理として付記する。
なお、駒除去のRust照合が不一致を拒否する処理（comparison.py:266）と、MNSDのレコード検証（mnsd.py:129〜146）には検査するテストがない。
これは削除によって生じる不足ではなく、既存の未検査箇所である。
