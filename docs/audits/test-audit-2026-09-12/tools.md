監査範囲は `src/bin/**`、[tests/match_runner.rs](../../../tests/match_runner.rs)、`tools/train/**`、`scripts/**` である。
テスト本文121件を全件読んだ。内訳はRust 95件、Python 26件であり、実行ログ上の `match_runner` 59件は本体47件とstorage 12件の合計である。
今回の実測では、Rust全体はビルド込み6.97秒、ハーネス統合4件は2.76秒、bench 5件は0.15秒、selfplay 11件は0.03秒、Python 26件はテスト部分3.023秒で全件成功した。
したがって大幅な高速化を約束する根拠はなく、主な効果は重複実行と変更時の保守負担の削減である。

USIは対局プログラムの通信方式、SFENは局面の文字列表現を指す。
CLIはコマンドラインからの操作系、PSTは駒種と升目による評価表、SPRTは逐次確率比検定を指す。

判断の根拠は [docs/guides/sprt.md](../../guides/sprt.md)、[docs/guides/pst-training.md](../../guides/pst-training.md)、[docs/plans/spec-first-tests/matrices/d8-stats-harness.md](../../plans/spec-first-tests/matrices/d8-stats-harness.md) と同ledgers/d8、[docs/plans/match-harness-efficiency.md](../../plans/match-harness-efficiency.md)、[docs/plans/random-play.md](../../plans/random-play.md)、[docs/plans/evaluation-gen1.md](../../plans/evaluation-gen1.md)、[docs/plans/tapered-pst.md](../../plans/tapered-pst.md)、教訓索引である。
D8マトリクスは古いH1=5やキャッシュ方式の記述も含むため、現行sprt.mdと完成済み設計書を優先した。
以下は削除・統合候補であり、テストも本番コードも変更していない。

以下は、重複や実装依存が大きく、優先して整理できる候補である。

T01では、手書きワーカーを動かす並行テスト1件を削除する。

対象は [src/bin/match_runner.rs:3533](../../../src/bin/match_runner.rs:3533) の `delayed_head_pair_does_not_leave_a_worker_idle` 全体である。
テストは `run_worker_loop` を使わず、ロック、recv、待機、sendからなる独自のワーカーループを3550行付近に作っている。
本番に対する観測は `accept_completed_pair` が後続番号を1件返すことだけで、`out_of_order_completions_replenish_one_slot_each`（3482行）が同じ「2が先に完了したら3を補充」を直接検査する。
削除して新たに見逃す本番の現実的な不具合はない。実ワーカーの停止配線は `decision_stop_discards_an_unfinished_worker_result`（3614行）が実関数を通して検査し、プロセス対局の順序は統合テストが残る。
スレッド2本、AtomicBoolによるbusy wait、2秒の待機期限、約80行のfixtureを除去できる。これは無条件削除候補である。

T02では、ペア得点をテスト自身が加算するassert 3件を削除する。

対象は [src/bin/match_runner.rs:4838](../../../src/bin/match_runner.rs:4838) の `game_outcomes_map_to_candidate_pair_score_categories` 内、4885行以降の「2局とも勝ち」「1勝1敗」「2局とも反則負け」に対する `half_points(...) + half_points(...)` のassertである。
実際の `run_pair` の分類結果を呼ばず、テスト自身が加算している。加算を本番から削除しても通るため、ペア分類の検査にはなっていない。
直前の勝ち2、負け0、引分け1、反則・投了の両視点と、投了を異常件数へ算入しないassertは残す。
3件を除去して失うのは同じ関数を再実行して整数加算する検査だけであり、現実的な新規見逃しはない。実際のペア集計に対する不足は別の課題で、これを理由に自己加算を残す価値はない。

T03では、benchの本番処理コピーを実関数の観測へ置き換え、局面検査も統合する。

対象は [src/bin/bench.rs:388](../../../src/bin/bench.rs:388) の `total_node_counts_are_reproducible_across_runs` と364行の `bench_positions_are_the_fifteen_documented_valid_games` である。
388行のテストは `run_bench`（194行）を呼ばず、SFEN解析、Game生成、置換表clear、search、ノード加算のループをテスト内へ複製している。
本番のclearや集計を壊してもコピー側は正しいまま通るため、現状はbench接続ではなく探索の再現性を再検査している。
条件付き統合として `run_bench(..., print_positions=false)` を2回呼び、そのノード数を比較する1件へ置き換える。局面数15という明文契約のassertも同じテストへ移し、実際のbenchが全局面を解析・探索することを使って独立した局面走査をなくす。
単純に両テストを消すと、benchの入力破損や再現性を見逃すので無条件削除ではない。統合後はコピー約30行と全局面の余分な走査1回を除去し、本番に対する検出力を上げられる。

T04では、同一シードのプロセス対局2回を、並列度比較へ吸収する。

対象は [tests/match_runner.rs:98](../../../tests/match_runner.rs:98) の `random_usi_match_is_reproducible_for_the_same_seed` と108行の `random_usi_match_output_is_independent_of_concurrency` である。
後者は同じシード・同じ4ペアを2回実行して出力を比較し、並列度も変えるため、前者の「同じシードで同じ結果」の保証を含む。
前者の正常終了、有効ペア数、異常0件の出力assertを後者の4ペアの結果へ移したうえで、前者を削除できる。helperが既に終了コードを検査している。
追加の現実的な見逃しはない。同じ並列度でだけ起こる不決定性という独立要件はなく、再現性の試行を増やすだけになっている。
2回のrunner起動と2ペア、すなわち4局分を省ける。統合テスト全体2.76秒の一部に効くが、個別時間の内訳は未測定である。

T05では、同一自己対局の3回目をなくす。

対象は [src/bin/selfplay_gen.rs:1197](../../../src/bin/selfplay_gen.rs:1197) の `play_game_is_deterministic_for_same_seed_and_arguments` と1273行の `records_start_at_or_after_planned_injection_boundary` である。
双方が同じ規則、基本シード7、対局番号1、注入上限4、探索100ノード、最大600手を使い、前者2回＋後者1回で同じ対局を3回走らせている。
後者の注入境界、記録対象数、除外数保存のassertを前者の `first` へ移し、後者を削除する。
再現性と注入後だけを記録する保証をすべて残して3回を2回へ減らすため、現実的な見逃しは増えない。1対局分と重複fixtureを省ける。

T06では、内部モデルに3列を手渡すテストを削除する。

対象は [tools/train/pst/test_train_pst.py:506](../../../tools/train/pst/test_train_pst.py:506) の `TaperedFormatTest.test_make_model_rejects_unexpected_columns` 全体である。
`make_model` の唯一の本番呼出元は `command_train` であり、入力列は検査済みMNPTから `single` の1列か `tapered` の2列として構築される（train_pst.py 538〜583行）。
CLIや保存ファイルから3列を渡す経路はなく、未公開helperへ不可能な形を直接作って防御分岐だけを固定している。
外部入力の形・重み形式の破損は `test_mnpt_corruption_and_inconsistent_piece_values_are_rejected` とsingle/taperedの学習テストが残す。
削除で見逃すのは現状の入口から生成できない内部引数を受理する変更であり、利用者の現実的な不具合ではない。小さいが無条件削除候補である。

T07では、最良エポックの比較helperを直接検査する3ケースを削除する。

対象は [tools/train/pst/test_train_pst.py:218](../../../tools/train/pst/test_train_pst.py:218) の `TrainingPathTest.test_best_epoch_replacement_requires_strict_improvement` 全体である。
`should_replace_best_epoch` は浮動小数点数の `<` 1行であり、改善・同値・悪化の3値を当ててその演算を再確認している。
より重要な「最終エポックで悪化したとき初期重みを保存する」は `test_training_path_uses_documented_data_membership`（223行）が実学習と出力重みで検査し、「改善した端点を更新する」は `test_single_model_requires_identical_endpoints_and_duplicates_output`（468行）のtaperedケースが検査する。
残る唯一の差は同値の損失で最初の重みを優先することだが、評価gen1設計書193行とpst-training.mdが定めるのは最小損失の重みであり、同値候補の選択順は明文化されていない。したがって同値時の保持方針は規範文書に根拠がない期待であり、現行のhelperを規範にして固定しない。
3ケースを削除しても、仕様で認められない劣る重みの保存は実学習テストで検出できる。

以下は、固有の保証を移した後に統合できる候補である。

T08では、全CLI入口で繰り返す規則文法の網羅を共通parserへ集約する。

対象は [src/bin/minase.rs:155](../../../src/bin/minase.rs:155) の `rules_argument_shares_the_wire_value_grammar`、[src/bin/random_play.rs:625](../../../src/bin/random_play.rs:625) の `rules_argument_resolves_presets_case_insensitively_and_rejects_combinations`、[src/bin/match_runner.rs:4499](../../../src/bin/match_runner.rs:4499) の `rules_presets_resolve_per_article_33` である。
共通側の保持先は [src/core/rules.rs:2211](../../../src/core/rules.rs:2211) の `article_33_5_and_33_6_presets_expand_from_rule_constants`、2228行の `rule_set_parse_errors_preserve_the_failure_kind`、2243行の `rule_set_display_includes_all_four_base_groups` である。
共通側が大小文字、両プリセット、プリセットとコード／プリセットとの併記、不正コードを検査しているため、各CLIに同じ全文法の表を持つ必要はない。
各入口には、LISHOGIの正常な解決1件と `lishogi,P1` の拒否1件を残す。match_runnerでは省略時のengine-defaultと原文保持、minaseでは不完全な4群をEngine構築で拒否する固有経路も残す。
削れるのは各CLIの追加大小文字表現、正規コード列との重ねた比較、R0/R0,R1や未知名の反復である。共通parserの配線を切る不具合は入口の正常・異常ケースで検出し、文法の仕様変更は共通テストで検出する。
この統合はプリセット仕様を変えたときに3つのCLI表まで連鎖修正する費用を下げる。CLI接続のテスト全件削除ではない。

T09では、random_playの検証付き短局を再現性比較へ統合する。

対象は [src/bin/random_play.rs:683](../../../src/bin/random_play.rs:683) の `same_seed_and_game_number_reproduce_the_same_game` と698行の `verify_all_verification_completes_on_a_short_game` である。
同一条件での2回の `run_game` の片方を `verify_all=true` にし、手順・手数・Cutoffの一致を検査すれば、別の短局1回を省ける。
検証を有効にしても対局結果が変わらないことと、全合法手適用の検査でパニックしないことを残せる。
現テストは `verify_all` が無視されても通るため、有効化の配線それ自体を保証しているとは扱わない。統合して失うのは別シードの短い補助試行だけであり、独立した挙動分類はない。

T10では、自己対局記録の重複カウントをマージのfixtureに統合する。

対象は [src/bin/selfplay_gen.rs:1394](../../../src/bin/selfplay_gen.rs:1394) の `recorded_statistics_counts_every_repeated_search_key` と1476行の `completed_games_are_merged_independently_of_arrival_order` である。
後者はすでにsearch_key重複の集計をマージ経路で検査するが、現fixtureには1回の重複しかなく、前者は2回目以降すべてを数えることを検査する。
条件付きで、マージfixtureの4レコード中3レコードに同じsearch_keyを与え、duplicate_positions=2と記録数4を要求する。そのうえで1394行の単独テストを削除する。
これにより「初回の再登場だけ数え、それ以降を落とす」不具合の検出を残し、重複した局面・レコードfixtureを1つ減らせる。

T11では、単一モデルをもう1回学習する正常ケースを削除する。

対象は [tools/train/pst/test_train_pst.py:468](../../../tools/train/pst/test_train_pst.py:468) の `test_single_model_requires_identical_endpoints_and_duplicates_output` 内、491行の `for model in ("single", "tapered")` のsingle正常学習だけである。
singleの両端点一致と出力駒価値は、同ファイル223行の実学習テストと [test_pst_workflow.py:205](../../../tools/train/pst/test_pst_workflow.py:205) の `test_cpu_training_and_diagnostics_preserve_scale_and_outputs` が確認する。
このテストには、両端点が異なる初期重みをsingleが拒否する前半と、taperedの終盤側の勾配が0になり中盤側だけ更新されるケースを残す。
single正常ケースの追加実行でしか捕まらない現実的な不具合はない。モデルの内部列数をログ文字列で再確認するassertも除去し、公開される重みの内容へ絞る。
CPU学習1回とMNPT/NPZ出力の重複を省ける。

T12では、手番時計の更新をgo出力テストへ統合する。

対象は [src/bin/match_runner.rs:4380](../../../src/bin/match_runner.rs:4380) の `clock_update_adds_the_increment_after_each_move` と4397行の `go_time_arguments_reflect_current_clocks_and_cross_colors` である。
後者の更新後出力用fixtureをbase=1000、increment=100、消費250の条件にし、goの残時間850と加算100を要求すれば、前者の会計保証を公開コマンドの観測へ移せる。
前者を単純削除すると加算し忘れを見逃すが、この移管後なら検出力を失わず、Clockを単独構築するテスト1件をなくせる。
失権境界と秒読みは別契約なので4362行のテストは残す。

T13では、規則数・正常解決の繰返しだけを縮小する。

対象は [src/bin/match_runner.rs:4242](../../../src/bin/match_runner.rs:4242) の `hash_options_resolve_independently_for_each_engine` 内の4表行である。
同時にcandidate=300、baseline=512とした1行で両側の別値保持とUSI/CECPの解決を検査し、省略時Noneは `documented_defaults_match_sprt_md` へ移す。
片側だけ指定した2行は `manifest_hash_sizes_prefer_each_players_explicit_value`（4329行）が候補側指定・基準側指定をそれぞれ実際の記録まで通しているため削れる。
入れ替わりや両側の取り違えを同時指定ケースで、省略側の既定値扱いをmanifestテストで検出する。単なる表形式化ではなく4組を2組相当へ減らす提案である。

以下は、assertや過剰なケースを削除できる候補である。

T14では、usi_randomの期待着手コピーを、シード配線の独立な観測へ置き換える。

対象は [src/bin/usi_random.rs:221](../../../src/bin/usi_random.rs:221) の `identical_seeds_reproduce_identical_outputs` 内、238〜252行のGame生成、`legal_moves`、同じ `XorShift64::index`、`usi::text` による期待値生成と最初のbestmove照合である。
本番 `handle_go`（143行）と同じAPI・同じ選択式を再実行しており、乱数indexや表記の共通バグは両側へ伝播する。内部合法手の列挙順を変えれば外部仕様を保つ変更でも期待値生成が結びついたままである。
同一入力の出力一致、goごと1応答はその場に残す。合法手を返す契約は実際の審判を通る [tests/match_runner.rs](../../../tests/match_runner.rs) のrandom対局が残す。
ただしこの部分を単に削除すると、setoption Seedを無視して常に既定シードを使う現実的な不具合を見逃す。したがって、複数シードから異なる応答列を得る性質、または仕様の乱数式と盤面から独立に求めた固定の応答fixtureへ移管することが条件である。一様性の統計検査には元からなっていない。シードの反映保証を残しながら、本番と同じ選択コード約15行を期待値側から除く提案である。

T15では、非公開注入計画の座標2件と過剰な0ケースを削る。

対象は [src/bin/selfplay_gen.rs:1306](../../../src/bin/selfplay_gen.rs:1306) の `injection_plan_has_unique_offsets_and_planned_boundary` 内の `[68,72,82]` とrecord_from=83の固定assert、および1333行の `zero_random_moves_produces_empty_plan_at_opening_boundary` の開始手数8/12/16の3行である。
評価gen1設計書134〜148行は上限、範囲、重複禁止、最後に予定した注入の次から記録という性質を定め、特定シードの具体的な3座標は定めない。
32シードの範囲・一意性・境界の性質と、実対局の再現性および記録境界テストを残し、固定座標を削る。0注入は非零の開始手数1例へ絞る。
ただし序盤の乱数方式や既存データを保つ制約は同設計書165行にあり、`XorShift64` の参照値そのものは削除対象にしない。提案は注入内部の未規定の具体座標だけである。
3座標の実装を変更しても仕様上同じ性質を保つ場合に不要な修正を要求する負担を下げる。32シードをさらに機械的に1件へ削る提案はしていない。

T16では、8局の注入範囲検査を必要な中断ケースへ絞る。

対象は [src/bin/selfplay_gen.rs:1214](../../../src/bin/selfplay_gen.rs:1214) の `injection_counts_and_offsets_stay_within_configured_bounds` の対局番号1..=8である。
8件とも上限80、最大32手という同じ条件で、記録本体より予定と途中まで実行した回数の会計を検査している。乱数計画の範囲は1306行の32シードの安価な純粋テストが別にある。
「途中まで注入したが、最後に予定した注入には届かず打ち切られる」fixture 1件を特定したうえで、実行回数、offsetヒストグラム、探索数の保存をその1件へ集約する。0件の計画は1333行、記録可能領域は1273行を統合した1197行で残す。
これはfixtureの条件確認後に行う条件付き削減である。8件の任意シードから得る散発的な回帰検出機会は減るが、別々の仕様分類を8件が担っている根拠はない。selfplay全体0.03秒なので主効果はfixture構造の単純化である。

T17では、ペアシードの100件の再現・一意性検査を減らす。

対象は [src/bin/match_runner.rs:4922](../../../src/bin/match_runner.rs:4922) の `pair_seed_derivation_is_deterministic_nonzero_and_distinct` 内、100個を2回生成しsort/dedupする部分である。
実ペア配線が同じシードを再利用してしまう不具合は [tests/match_runner.rs:108](../../../tests/match_runner.rs:108) が出力された4個のpair_seedの相異で検査し、再現性は同じプロセス対局比較で検査する。
このテストに残すのは出力0の原像に対する仕様上の非零置換1件である。[src/rng.rs](../../../src/rng.rs) の `derive_seed` には専用テストがなく、random_playの `game_seed` は別実装なので、この境界まで重複として削除してはいけない。
100件の有限列挙は一般的な単射性の証明にならず、出力が常に同じになる代表的故障は統合側で残る。

T18では、開始局面を二重に作る再現性比較を統合テストへ任せる。

対象は [src/bin/match_runner.rs:4942](../../../src/bin/match_runner.rs:4942) の `openings_stay_within_8_to_12_plies_and_derive_deterministically` 内、同じpair_seedでreplayを作って `moves` と `usi_moves` を比較する2assertと4回の再生成である。
8〜12手の境界、movesと送信手順の個数一致、および異なるペアの開始手順が全部同じにならないことはこのテストに残す。
同じ開始手順の再現は統合の同一シード・並列度比較が全結果行で検査する。したがって生成部だけでの2回目の比較を削っても、利用者が観測する再現性の代表的故障を見逃さない。

T19では、テストfixtureと標準ライブラリしか検査しないassertを削る。

対象は [tests/match_runner.rs:17](../../../tests/match_runner.rs:17) の `run_random_match` 冒頭にある `CARGO_BIN_EXE_match_runner` の隣のusi_randomパスと `CARGO_BIN_EXE_usi_random` を比較するassertである。
これはCargoの配置とテスト自身の `with_file_name` を比較し、本番の `resolve_player` を呼んでいない。実際のrunner起動が続くので、random解決の破損は後続の正常終了assertで検出できる。
また [src/bin/match_runner.rs:4008](../../../src/bin/match_runner.rs:4008) の `cecp_resolution_rejects_unsupported_search_limits` の全ケースに付いた `!error.to_string().is_empty()` は、`InvalidInput` の検査に対して仕様上の追加保証がないので削れる。
いずれも本番の現実的な検出力を失わない部分削除である。

T20では、外部形式の検査を残し、同じ値のserde往復を減らす。

対象は [src/bin/match_runner/storage.rs:958](../../../src/bin/match_runner/storage.rs:958) の `stop_reason_record_uses_fixed_snake_case_words` 内、5列挙値それぞれの `from_value(to_value(reason)) == reason` と、928行の `turn_record_round_trip_preserves_stop_data_and_nulls` のwith_values側往復である。
出力文字列depth/nodes/soft/hard/externalは保存形式の契約なので5件を残す。各serde派生の逆変換まで同じ生成値で繰り返す価値は低く、完全なTurnRecordのwithValuesを保存・再開する `create_save_and_resume_round_trip`（912行）が残る。
nullの出力、nullを明示した読み戻し、および必須キーが欠けた場合の拒否は別の外部契約であり、928行の後半は残す。`unknown_json_field_is_rejected` も厳密な保存形式の契約なので残す。
削除で失うのは自動派生した変換の対称性の再確認であり、外部形式とアプリ側のrequired/nullの指定ミスの検出は維持する。

T21では、エラー文言の固定を拒否の観測へ絞る。

対象は [src/bin/match_runner/storage.rs:976](../../../src/bin/match_runner/storage.rs:976) の `resume_rejects_format_version_two` の `unsupported manifest format version 2` 完全文字列assert、[src/bin/match_report.rs:783](../../../src/bin/match_report.rs:783) の `report_rejects_format_version_two` の同様の全文assert、[src/bin/selfplay_gen.rs:1488](../../../src/bin/selfplay_gen.rs:1488) の `completed_game_merge_rejects_a_missing_game_number` の `worker channel closed after 1 of 4 games` 全文assertである。
いずれも誤形式・欠番の拒否と `InvalidData` は残す。文言の全文は利用者との機械的インターフェースとして明記されておらず、例外メッセージを改善しただけでテストを修正する負担になっている。
特に最後の「1 of 4」は受信済み4件中の3件ではなく番号順に統合済み1件という内部手順に依存する。
また [src/bin/usi_random.rs:204](../../../src/bin/usi_random.rs:204) の `handshake_declares_ruleset_and_seed_defaults` はid authorの具体名や行順まで含む全stdout一致をやめ、必要なid行、option宣言、usiok/readyokを検査する。Seed既定の具体値と説明文はD8のSU-08/09が明文外と認めているので、これを独立仕様の保証とは数えない。
例外分類や必須wire tokenを消す提案ではなく、自由な文言と手順の固定だけを削る。

T22では、診断中の本番評価による期待値再計算を、独立な駒得へ置き換える。

対象は [tools/train/pst/test_pst_diagnostics.py:141](../../../tools/train/pst/test_pst_diagnostics.py:141) の `test_differing_endpoints_change_removal_deltas_with_the_phase` 内、149〜156行の `feature_indices`、`phase_numerators`、`integer_evaluate` による期待差の再計算である。
本番 `_removal_report` も同じ関数群で評価するため、補間式や駒数の共通バグが両側へ入る。このfixtureの自駒1枚は中盤200cp、終盤100cpなので、初期局面から除いたときq=89となり、期待差は独立な算術で-198cp、敵駒なら+198cpと求められる。
この定数の由来を設計書の補間式に接地して置き換え、内部評価APIによる期待値生成を削る。`test_integer_evaluate_interpolates_truncates_and_clips` は補間・切捨ての単体保証として残す。
テスト全体を削除すると「駒を除いても元の係数を使い続ける」不具合を見逃すので、独立期待値への置換が条件である。

T23では、Python製probeでRust一致を数える重複assertを削る。

対象は [tools/train/pst/test_pst_workflow.py:205](../../../tools/train/pst/test_pst_workflow.py:205) の `test_cpu_training_and_diagnostics_preserve_scale_and_outputs` 末尾、`report["rust_agreement"] == {"base": validation, "candidate": validation}` のassertである。
`diagnose_probe` は `python_probe` に置換され、probeは検査対象と同じ `Weights.evaluate` を呼ぶ。このassertはRustとの等価性を保証しておらず、Python評価を2回計算して集計件数が出ることだけを重複検査する。
[test_pst_diagnostics.py:75](../../../tools/train/pst/test_pst_diagnostics.py:75) の保存済み標本・件数の検査を1箇所に残し、同162行の `test_undefined_correlations_and_mismatched_probe_are_reported` のwrong_probe拒否も残す。後者は返値を故意にずらして比較配線の故障を検出するので、モックだから削る対象ではない。
workflow側は実際のdiagnosticsファイル、標本ファイル、入力の同一性を確認すればよい。正常probeの同じ件数を両層で固定する保守負担を減らせる。

T24では、学習ログの期待値を本番の特徴抽出・bincountで組み立てる部分を削る。

対象は [tools/train/pst/test_train_pst.py:223](../../../tools/train/pst/test_train_pst.py:223) の `test_training_path_uses_documented_data_membership` 内、276〜312行の特徴抽出、`np.bincount`、未観測数・最大観測数、mirrorで同じカウントを作るfixture検査、および350行付近のログからその内部集計値を比較する部分である。
このfixtureは全訓練レコードが同じ非対称盤面なので、保持すべき意味は「学習データに一度だけ現れた特徴を1回と数え、鏡映拡張で二重に数えない」ことである。現行の特徴番号やpadding配列と同じカウント式を複製する必要はない。
条件付きで、仕様から読める既知の特徴数と訓練レコード数を固定fixtureとして要求し、本番のfeature_indices/mirror/bincountを期待値側から外す。学習重み、最良epoch=0、検証標本数のassertは残す。
単純に観測数の保証を消すと特徴頻度ログの二重計上を見逃すため、これはテスト全削除ではない。実装コピー約35行と内部ログへの過剰な依存を減らす提案である。

T25では、検証用ハッシュの第二実装を固定した分割fixtureへ置き換える。

対象は [tools/train/pst/test_train_pst.py:68](../../../tools/train/pst/test_train_pst.py:68) の `reference_hash64`、93行の `test_hash_split_is_stable_when_file_order_changes` の80個の期待ハッシュ生成、および223行テストの `labels` である。
参照関数はmnsd.pyのハッシュを別の数値型でほぼ同じ手順として書き直しており、「指示書」とのコメントに反して、現行設計書はシード・対局番号のハッシュを20で割る分割規則までしか明文化していない。
一方、既存分割を保つ価値は高く、ここを根拠なく削除してはいけない。教訓 [validation-split-change-invalidates-old-baseline.md](../../lessons/validation-split-change-invalidates-old-baseline.md) が分割変更による比較破綻を明記している。
条件付きで、現在の入力ファイルに対する検証対局IDの既知集合と、u64桁あふれ・シード差を含む少数の独立参照ベクトルをfixtureへ固定する。その上で80回の同型ハッシュ計算と第二実装を取り除き、ファイル順序を入れ替えてもIDの所属が変わらない検査を残す。
これは参照関数の見かけの独立性を除く統合候補であり、分割方式を変更する許可ではない。固定ハッシュの厳密式の規範は規範文書に根拠がない期待として明記する。

以下は、固有の不具合を検出するため削除候補に含めなかったテストである。

[src/bin/match_runner.rs:3429](../../../src/bin/match_runner.rs:3429) の `linux_resource_probe_reports_current_process_cpu_and_memory` は実環境依存で、特にphysical_core_count/physical_memoryが必ずSomeというassertは隔離環境で不安定になり得る。
ただし独自の資源取得の配線を試す唯一のテストでもあり、削除すると全資源が欠測になる退行を検出できなくなる。現時点の実行は成功しており、不安定さの実測もないので「Linux依存」という理由だけで削除候補にはしなかった。将来失敗が確認されたら決定的な入力fixtureへの置換を検討する対象である。

`unknown_commit_revision_fails_resolution`（4100行）はgit自体を呼ぶが、非零終了コードをアプリが見落として空ハッシュを正常値にする不具合を検査する。単にgitの保証を再確認しているとは言えず、残す。
CLIの必須引数、排他的引数、境界値検査はclapを使っていても、アプリ側のattributeとvalidatorの設定ミスを検出するため、丸ごと削除しない。
storageのversion拒否とmatch_reportのversion拒否は別のreaderであり、同じversion=2でも重複ではない。
Pythonの生成失敗mockは `run_command` だけを置換して本物の完了記録管理を通している。中断出力の再利用禁止、完了記録を作らないこと、変更済み出力の拒否には現実的な検出力があるので残す。
PythonのMNPT readerとRustのMNPT readerは独立実装であり、両言語に破損・版・チェックサムのテストがあることだけで重複とは判断しない。

調査済みファイルと保持する保証は次のとおりである。

| ファイル | テスト数 | 保持すべき保証 |
|---|---:|---|
| src/bin/match_runner.rs | 47 | 再開記録の矛盾拒否、停止通知の実worker経路、番号順統計、CLI独自配線、先後の時計交換、失権境界、CECP拒否・投了・分割着手・pong、USI情報の解析、異常分類を残す。 |
| src/bin/match_runner/storage.rs | 12 | 排他ロック、上書き禁止、クラッシュ中の一時ファイル回収、累積時間、マニフェスト同一性、保存番号・カテゴリ・形式検証、nullと欠損キーの区別を残す。 |
| src/bin/match_report.rs | 4 | 独立な保存記録から統計・資源・指標を再計算し、欠測を0補完せず、条件不一致と欠番を拒否する保証を残す。 |
| src/bin/selfplay_gen.rs | 11 | 注入前に記録しないこと、計画と実施の区別、除外数保存、対局番号順の出力、欠番拒否、重複カウント、値分布と分母0表示を残す。 |
| src/bin/bench.rs | 5 | 実bench関数での再現性、固定15局面、CLIの範囲、中央値を残す。 |
| src/bin/random_play.rs | 5 | シードの独立な参照値、桁あふれと0置換、CLI固有契約、再現性、verify-allの実行を残す。 |
| src/bin/usi_random.rs | 5 | handshakeの必要項目、シード指定、合法な応答、ライフサイクルの拒否、共通規則parserへの接続を残す。 |
| src/bin/minase.rs | 2 | protocolとrulesの明示必須、受理するprotocol、規則parser接続と不完全規則の起動拒否を残す。 |
| src/bin/perft.rs | 0 | cfg(test)、test関数を持たないことを確認した。 |
| src/bin/pst_probe.rs | 0 | cfg(test)、test関数を持たないことを確認した。 |
| tests/match_runner.rs | 4 | 実プロセスの並列度非依存、USIとCECPの整合、最小欠番の再実行と後続記録の保存を残す。 |
| tools/train/pst/test_train_pst.py | 11 | データ来歴と検証分割、任意順のgather、世代別教師K、実学習の最良重み、整数補間・切捨て・上限、MNPT破損拒否、単一/2端点学習を残す。 |
| tools/train/pst/test_pst_workflow.py | 10 | 明示設定、seed区間の境界、完成と中断の区別、変更検知、再利用、実CPU学習から保存・診断までの接続を残す。 |
| tools/train/pst/test_pst_diagnostics.py | 5 | 保存標本の再現性と世代所属、空帯、相関未定義、量子化誤差の拒否、probe不一致の拒否、駒除去の評価、固定駒価値、丸めを残す。 |

残る [tools/train/pst/features.py](../../../tools/train/pst/features.py)、[mnsd.py](../../../tools/train/pst/mnsd.py)、[pst_diagnostics.py](../../../tools/train/pst/pst_diagnostics.py)、[pst_workflow.py](../../../tools/train/pst/pst_workflow.py)、[taper.py](../../../tools/train/pst/taper.py)、[taper_report.py](../../../tools/train/pst/taper_report.py)、[train_pst.py](../../../tools/train/pst/train_pst.py) と設定ファイルには別個のtest関数はない。
[scripts/clock_profile.py](../../../scripts/clock_profile.py)、[fetch_lishogi_replays.py](../../../scripts/fetch_lishogi_replays.py)、[hachu_replay.py](../../../scripts/hachu_replay.py)、[hachu_selfplay.py](../../../scripts/hachu_selfplay.py)、[match_cost_profile.py](../../../scripts/match_cost_profile.py) にもtest関数やunittest/pytest/doctestはない。
本監査では変異を実行していない。「本番関数を呼ばない」「テスト内の加算」等の検出不能性は本文と呼出先の読解に基づく。
