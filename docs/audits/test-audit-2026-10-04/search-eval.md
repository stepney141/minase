対象は `src/search/**`、`src/eval/**`、[src/stats.rs](../../../src/stats.rs)、[src/rng.rs](../../../src/rng.rs) の32ファイルにある250テストである。`search-stats` フィーチャでだけ走る [src/search/alphabeta/tests/stats.rs](../../../src/search/alphabeta/tests/stats.rs) の10件と、`tuning` フィーチャでだけ走る7件を含む。全テスト本文と補助関数を確認し、関係する実装と並べて読んだ。本番コードとテストは変更していない。実行結果と不安定さの再実行は[全体の報告](../test-audit-2026-10-04.md)に記載した。行番号は監査時点（master 9027662）のものである。

根拠は RULES.md 第15版、[docs/plans/spsa.md](../../plans/spsa.md)、[docs/plans/ponder.md](../../plans/ponder.md)、[docs/plans/search.md](../../plans/search.md)、[docs/plans/movegen-speedup.md](../../plans/movegen-speedup.md)、[docs/plans/movegen-speedup-2.md](../../plans/movegen-speedup-2.md)、[docs/plans/debugging-tools.md](../../plans/debugging-tools.md)、[docs/guides/sprt.md](../../guides/sprt.md)、[docs/guides/pst-training.md](../../guides/pst-training.md)、[docs/plans/spec-first-tests/matrices/d7-search-eval.md](../../plans/spec-first-tests/matrices/d7-search-eval.md)、[d8-stats-harness.md](../../plans/spec-first-tests/matrices/d8-stats-harness.md) と台帳 [d7.md](../../plans/spec-first-tests/ledgers/d7.md)、[d8.md](../../plans/spec-first-tests/ledgers/d8.md) である。範囲内のテストは最長でも1.83秒で、合計実行時間は判断材料にならない。以下の候補は、期待値の書き換え頻度、重複、実時間への依存を根拠にしている。

以下ではTTを置換表、PSTを駒種と升目による評価表、SEEを静的交換評価、SPSAを同時摂動確率近似とする。

S01では、時間予算の式を2か所で互いに重複先として持っている。

削除対象は [src/search/alphabeta/tests/time.rs:12](../../../src/search/alphabeta/tests/time.rs:12) `clock_budget_matches_the_normative_formula`、[:55](../../../src/search/alphabeta/tests/time.rs:55) `opening_coefficient_scales_only_the_byoyomi_term`、[:82](../../../src/search/alphabeta/tests/time.rs:82) `moves_to_go_decreases_monotonically_to_the_documented_floor` の3件である。いずれも手計算したミリ秒値や下限88、ply 432を期待値に書いている。これらはSPSAの調整対象である時間係数（params.rs の ExpectedPlies、MinMoves、IncrementShare、HardSoftRatio、HardRemainingShare）から決まる値であり、96aa2ef、2945d7e、90d1446、d491c37 で計4回書き換えた。90d1446とd491c37は係数値の変更だけによる書き換えである。

保持先は [src/search/alphabeta/tests/tuning.rs:52](../../../src/search/alphabeta/tests/tuning.rs:52) `tuning_default_clock_budget_matches_reference_grid` と [time.rs:94](../../../src/search/alphabeta/tests/time.rs:94) `clock_budget_preserves_bounds_over_a_deterministic_grid` である。前者は既定機能でも走り、残り時間・加算・秒読み・ply の格子（ply 0、35〜37、431〜433、u32::MAX を含む）で式全体を照合する。後者は soft≤hard、hard≥1、安全余裕30 ms という係数に依存しない不変条件を検査する。

tuning.rs:52 自体も係数を88、432、76、451、27 のリテラルで書き写しており、そのままでは同じ書き換え負担を持つ。そこで、参照関数の係数を `params::*` の読み出しに改めたうえで残す。tuning.rs:52 を残し S02 の tuning.rs:16 を消すのは、前者の参照式が [docs/plans/spsa.md](../../plans/spsa.md)「整数表現」に書かれた式を典拠とする仕様側の照合であるのに対し、後者は SPSA フェーズ1の書き換え前後の一致を示す一度きりの照合だったからである。両方を消すと、丸めの順序や切り詰めの順序を守るのが time.rs:94 の不変条件だけになり、係数を変えずに式の構造を壊す変更を検出できなくなる。

失う現実的な検出力はない。time.rs の3件が固有に持っていたのは「係数がこの値である」という主張だけであり、その値はSPSAの採用のたびに変わる。

推奨: tuning.rs:52 の係数を `params::*` の読み出しに改めることを条件に適用する。

S02では、SPSAフェーズ1の一度きりの照合が通常のテストとして残っている。

削除対象は [src/search/alphabeta/tests/tuning.rs:16](../../../src/search/alphabeta/tests/tuning.rs:16) `tuning_default_null_move_and_aspiration_match_reference` と [:106](../../../src/search/alphabeta/tests/tuning.rs:106) `tuning_default_iteration_prediction_matches_reference_grid` である。前者は null move の減深量（深さ0〜256）、窓の拡大、delta 余裕値を、pruning.rs:104、root.rs:70〜72、searcher.rs:72 と同じ式で再計算して照合する。後者は次の反復の予測判定を、比263をリテラルにした同じ式で照合する。どちらも実装のコピーであり、[docs/plans/spsa.md](../../plans/spsa.md):240〜241 がフェーズ1の完了条件として求めた、アクセサへの書き換え前後で値が一致することの確認だった。フェーズ1は完了している。

保持先は、null move が [pruning.rs:729](../../../src/search/alphabeta/tests/pruning.rs:729) `null_move_reduction_scales_with_eval_surplus_and_depth` と [tuning.rs:715](../../../src/search/alphabeta/tests/tuning.rs:715) `zero_null_move_eval_scale_matches_base_reduction_search`、窓の拡大と飽和が [root.rs:29](../../../src/search/alphabeta/tests/root.rs:29) `aspiration_widens_only_the_failed_side`、delta 余裕値が [quiesce.rs:310](../../../src/search/alphabeta/tests/quiesce.rs:310) と [qsearch_move_limit.rs:463](../../../src/search/alphabeta/tests/qsearch_move_limit.rs:463)、反復予測の等号境界が [ponder.rs:8](../../../src/search/alphabeta/tests/ponder.rs:8) `ponder_iteration_predictions_obey_hit_offset_and_exact_boundaries` である。

失う現実的な検出力はない。予測判定は u128 で計算するので、Duration::MAX でも溢れない。

推奨: 適用する。

S03では、35係数の宣言表をテスト内にリテラルで写している。

一部削除の対象は [src/search/alphabeta/tests/tuning.rs:161](../../../src/search/alphabeta/tests/tuning.rs:161) `tuning_parameters_and_usi_contract_in_isolated_process` のうち、443〜480行の `expected` 表である。この表は params.rs のマクロ宣言の写しで、既定値を90d1446、d491c37、2659bfb、c3ca7c0、a2e35d7、0e5be87、85f6e87、cf188f3 の8回書き換えた。`zip` に使う名前は `params::PARAMETERS` から取る。

残すのは、USI の `usi` 応答に出る宣言行と `PARAMETERS` の比較、35係数の反映検査、範囲外の値の拒否である。宣言行の整形はマクロとは別のコードなので、この比較は同語反復にならない。反映検査は [docs/plans/spsa.md](../../plans/spsa.md):246 が、入力の拒否は同:283 が要求している。

失う検出力は、係数の宣言範囲を誤って書き換える変更の検出である。ただし範囲は計画上も変わる値であり（85f6e87）、変更のたびに表を書き換える負担がこの検出力を上回る。

推奨: 適用する。

S04では、次の反復を始める判定の境界検査が、後から追加されたテストに包含されている。

統合対象は [src/search/alphabeta/tests/time.rs:158](../../../src/search/alphabeta/tests/time.rs:158) `next_iteration_requires_both_time_conditions` と [:210](../../../src/search/alphabeta/tests/time.rs:210) `next_iteration_stable_requires_the_prediction_within_soft` で、統合先は [ponder.rs:8](../../../src/search/alphabeta/tests/ponder.rs:8) `ponder_iteration_predictions_obey_hit_offset_and_exact_boundaries` である。前回の監査のS02はこの2件の保持を決めたが、その後に ponder.rs:8（817963a）と tuning.rs:106 が追加されて状況が変わった。ponder.rs:8 は soft の未満境界、hard の予測の等号境界、安定時の予測完了時刻の soft 境界を、的中時刻を含めて検査している。

time.rs の2件が持つ境界値38/39 msは IterationRatio=263 の帰結で、d491c37 で40/41から書き換わった。統合先の境界値も同じ係数に依存するので、統合の際には境界を `params::iteration_ratio()` から計算し、書き換えを不要にする。:210 の340〜358行が扱う hard<soft の組合せは本番で到達しない。`clock_budget` が soft を hard で切り詰め、`time_budget` が soft と hard をそれぞれ小さい方で採るため、常に soft≤hard になるからである。

失う現実的な検出力はない。

推奨: 境界値を `params::iteration_ratio()` から計算することを条件に適用する。

S05では、SEE の手計算テストが本番の関数ではなくテスト専用の写しを観測している。

統合対象は [src/search/alphabeta/see.rs:734](../../../src/search/alphabeta/see.rs:734) `see_values_match_hand_calculated_exchange_sequences`、[:823](../../../src/search/alphabeta/see.rs:823) `see_values_lion_captures_of_non_lions`、[:866](../../../src/search/alphabeta/see.rs:866) `see_values_lion_like_special_recaptures`、[:906](../../../src/search/alphabeta/see.rs:906) `see_accounts_for_promotions_in_initial_and_recapture_moves`、[:953](../../../src/search/alphabeta/see.rs:953) `see_respects_p5_deferred_pawn_on_recapture`、[:979](../../../src/search/alphabeta/see.rs:979) `see_accepts_single_capture_lion_like_moves_without_mid` の6件である。888bd10（2026-09-15）で本番の SEE は `see_prunes` に置き換わり、元の計算は `#[cfg(test)] see_reference` として残った。前回の監査のS04は `see` が本番コードだった時点の判断なので、前提が変わっている。

各テストの `see_reference` による照合を、[see.rs:311](../../../src/search/alphabeta/see.rs:311) の補助関数 `assert_prune_contract(.., Some(expected))` の呼出しへ置き換える。手計算の期待値を保ったまま、余裕値0と200で本番の `see_prunes` を検査できるようになる。`see_reference` と `see_prunes` は攻撃駒の選択や成りの扱いを共有しており、写しになっているのはループ本体だけである。差分試験 [see.rs:340](../../../src/search/alphabeta/see.rs:340) `see_prunes_matches_reference_on_bench_and_random_captures` は bench 局面とランダム局面だけを扱うので、P5 の保留歩兵のような配置での本番ループの誤りには出会わない。

削除対象は [see.rs:1029](../../../src/search/alphabeta/see.rs:1029) `see_none_never_prunes_quiescence_captures` である。`capture_is_pruned_by_see` は `see_prunes(..,0)` そのものであり（pruning.rs:125〜131）、[see.rs:679](../../../src/search/alphabeta/see.rs:679) `see_returns_none_for_rule_dependent_recaptures` と [:624](../../../src/search/alphabeta/see.rs:624) `see_returns_none_for_rule_dependent_initial_moves` が、余裕値0と200で `see_prunes` が false になることを同じ型の獅子の取り返しで検査している。

統合は検出力を増やし、削除は失う検出力がない。

推奨: 適用する。

S06では、指し手選択器の個別性質テストが、参照ピッカーとの全列照合に包含されている。

削除対象は [src/search/alphabeta/tests/ordering.rs:8](../../../src/search/alphabeta/tests/ordering.rs:8) `staged_picker_yields_every_legal_move_exactly_once`、[:61](../../../src/search/alphabeta/tests/ordering.rs:61) `staged_picker_respects_advisory_precedence`、[:136](../../../src/search/alphabeta/tests/ordering.rs:136) `staged_picker_classifies_all_generated_captures` の3件である。

保持先は [ordering.rs:303](../../../src/search/alphabeta/tests/ordering.rs:303) `staged_picker_matches_reference_sequence_with_changing_history` である。c6e2539（2026-09-16）で追加されたこのテストは、不合法な置換表手、killer、重複した killer を含む組合せで、全出力列を捕獲フラグ付きで `ReferenceMovePicker` と完全照合する。対象はランダム局面と固定局面、規則は全種類である。全合法手が1回ずつ返ること、段階の順序、捕獲フラグの誤りは、いずれも列の不一致として現れる。捕獲の先頭部分は [captures.rs:395](../../../src/search/alphabeta/tests/captures.rs:395) `main_picker_captures_match_stable_reference_for_all_rules` も照合する。前回の監査のS41とS42はこれらのテストに頼ったが、それは参照ピッカーが入る前の判断である。

[ordering.rs:103](../../../src/search/alphabeta/tests/ordering.rs:103) `staged_picker_classifies_capture_and_quiet_tt_moves` も同じ包含関係にある。ただし ordering.rs:303 の置換表手は `first` の静かな手とランダム局面の `all[len/2]` であり、捕獲の置換表手を必ず含むかどうかは局面次第である。置換表手が捕獲である場合のフラグの誤りは、これを確認するまでは :103 だけが確実に捕まえる。

失う現実的な検出力はない。

推奨: :8、:61、:136 は適用する。:103 は ordering.rs:303 が捕獲の置換表手を必ず含むと確認できるまで保持する。

S07では、静止探索の stand-pat テストが同型の局面で二重になっている。

統合対象は [src/search/alphabeta/tests/quiesce.rs:83](../../../src/search/alphabeta/tests/quiesce.rs:83) `quiescence_stand_pat_declines_a_losing_capture` で、統合先は [quiesce.rs:347](../../../src/search/alphabeta/tests/quiesce.rs:347) `quiescence_see_pruned_candidates_still_store` である。どちらも守られた歩兵を飛車で取ると飛車を取り返される局面で、捕獲は SEE で枝刈りされる同じ経路を通る。統合先は stand-pat の値に加えて置換表への記録も検査する。

[docs/plans/search.md](../../plans/search.md):425 と [d7-search-eval.md](../../plans/spec-first-tests/matrices/d7-search-eval.md):112 は D7-SRCH-10 をこのテストに対応させているので、ラベルを統合先へ移す。

失う現実的な検出力はない。

推奨: D7-SRCH-10 の対応を統合先へ移すことを条件に適用する。

S08では、実装と同じ式による照合、内部状態への依存、同じ分岐の重複が残っている。

削除対象は次の3件である。

[src/search/alphabeta/tests/improving.rs:9](../../../src/search/alphabeta/tests/improving.rs:9) `futility_margins_follow_improving_thresholds` は、futility 余裕値を pruning.rs:48〜59 と同じ式 `pawn*percent/100*scale/100` で計算して照合する。失う検出力は丸め順序の変更（先に掛けて最後に割る形）の検出だけで、余裕値の+1方向の誤りは [improving.rs:103](../../../src/search/alphabeta/tests/improving.rs:103) `futility_prunes_quiets_only_when_not_improving` の境界検査が、尺度100での丸め順序は tuning.rs:789〜795（tuning 専用）が捕まえる。

[captures.rs:193](../../../src/search/alphabeta/tests/captures.rs:193) `tt_capture_is_returned_before_generating_groups` は、置換表の捕獲が先頭に出ることに加え、そのとき内部の `group` が空で `special` が空でないことを確かめる。先頭に出ることは [captures.rs:40](../../../src/search/alphabeta/tests/captures.rs:40) `staged_captures_match_reference_for_rules_tt_moves_and_thresholds` が置換表手5種で照合済みで、残りは内部状態への依存である。遅延生成そのものは [captures.rs:105](../../../src/search/alphabeta/tests/captures.rs:105) `stopping_in_first_group_leaves_later_groups_ungenerated` が守り、これは [docs/plans/movegen-speedup.md](../../plans/movegen-speedup.md):313 が要求する。

[qsearch_move_limit.rs:356](../../../src/search/alphabeta/tests/qsearch_move_limit.rs:356) `qsearch_limit_recaptures_igui_after_normal_search`（tuning 専用、子プロセス起動）は、通常探索で獅子が居喰いした直後の取り返しが手数制限の例外になることを確かめる。通常探索側の記録は negamax.rs:373 の `mv.to` の1行で、居喰いでも経由捕獲でも同じ分岐を通る。[qsearch_move_limit.rs:364](../../../src/search/alphabeta/tests/qsearch_move_limit.rs:364) `qsearch_limit_recaptures_mid_capture_after_normal_search` は `mv.from` と `mv.to` の取り違えも検出するが、居喰いでは from と to が一致するので区別できない。静止探索側の居喰い（:372）は、獅子の居喰いが `quiesce` を通る唯一の型なので保持する。

失う現実的な検出力は、improving.rs:9 の丸め順序を除いてない。

推奨: 適用する。

S09では、補正表の内部値を直接見ている。

一部削除の対象は [src/search/alphabeta/correction.rs:92](../../../src/search/alphabeta/correction.rs:92) `correction_fixed_point_weights_rounding_caps_and_sides` のうち、private な `table.values[..]` を直接見る103、106、109、112、126、127行である。期待値の `first` と `second` は実装の更新式（correction.rs:35〜38）の写しで、d491c37 と cf188f3 で2回書き換えた。

残すのは公開の `read` による0、1、−1、上限、手番、鍵のマスクの検査であり、内部値の誤りはこれらの値に現れる。

失う現実的な検出力はない。

推奨: 適用する。

S10では、先読みの的中後に反復を当て直す配線のテストが、負荷下で約半数失敗する。

対象は [src/search/alphabeta/tests/ponder.rs:276](../../../src/search/alphabeta/tests/ponder.rs:276) `ponder_long_iteration_stops_on_hit_without_spending_hard_budget` である。1スレッドと2スレッドで700 ms以上の先読みの後に的中させ、70 ms未満に SoftLimit で止まることを実時計で確かめる。20コアで負荷平均が約28の状態で、ponder モジュールを15回実行すると7回、単独では10回中4回失敗した。失敗はすべて ponder.rs:312 で、停止理由が HardLimit だった。範囲内で最も遅いテスト（1.83秒）でもある。

当て直し、hard の優先、1回性という判定ロジックは、決定的なテスト [ponder.rs:178](../../../src/search/alphabeta/tests/ponder.rs:178) `ponder_rechecks_pre_hit_iterations_once_and_checks_hard_first`、[:244](../../../src/search/alphabeta/tests/ponder.rs:244) `ponder_aspiration_research_preserves_iteration_start_and_stability`、[:343](../../../src/search/alphabeta/tests/ponder.rs:343) `ponder_team_adopts_completed_auxiliary_result_after_main_recheck` が固定している。一方、deepening.rs:80 が反復ごとに `PonderIteration` を設置する配線を検査するのはこのテストだけである。この配線が欠けると、的中時に進行中の反復が当て直されず、hard（70 ms）まで探索が続く。開始判定と当て直しは同じ入力を見るため、現行の継ぎ目では配線を決定的に検査できない。

推奨: 保持する。削除せず、movetime を300 msへ上げ、的中後の上限を300 ms未満へ緩めて不安定さを直す。待機時間は変わらず、HardLimit へ落ちる条件が「70 ms以上の停止」から「300 ms以上の停止」に変わる。

S11では、先読みのテストが決定的な検査で足りる部分を実時間で検査している。

一部削除の対象は次の3件である。

[ponder.rs:76](../../../src/search/alphabeta/tests/ponder.rs:76) `ponder_ignores_time_until_hit_but_obeys_depth_nodes_and_stop` の77〜85行は、先読み中に70 msの hard を超えても350 ms止まらないことと外部停止を実時間で確かめる。これを、[ponder.rs:436](../../../src/search/alphabeta/tests/ponder.rs:436) `ponder_hit_before_worker_start_obeys_the_iteration_start_budget` と同じ `run_search_team` に、`started = Instant::now() - 10s`、`hit_ns = u64::MAX`、`clock(0,0,100)` と depth 3、`ponder = true` を与える決定的な呼出しへ置き換え、DepthCompleted と depth 3 を確かめる。開始判定（deepening.rs:69）と `check_time`（searcher.rs:184）が hit=MAX で時間を見ないことを、待ち時間0で固定できる。外部停止は [handle.rs:193](../../../src/search/alphabeta/tests/handle.rs:193) `infinite_limits_stop_only_on_external_request` と同じ機構である。86〜97行の深さ・ノード上限は残す。

[ponder.rs:128](../../../src/search/alphabeta/tests/ponder.rs:128) `ponderhit_records_only_the_first_notification_even_before_worker_start` の142〜171行は、実探索を20回×2行う。これを ponder=false の1回で `first == 0` を確かめる形へ縮める。1回性は前半（129〜141行）が、Finished が1回でチャンネルが切れることは [team.rs:65](../../../src/search/alphabeta/tests/team.rs:65) などが検査する。非先読みのハンドルへの ponderhit が何もしないこと（時間予算が誤って延びる不具合）を捕まえるのはこの行だけなので、縮小後も残す。[docs/lessons/repeat-concurrency-regression-tests.md](../../lessons/repeat-concurrency-regression-tests.md) は修正時に10回以上連続実行することを求める規則で、テスト内の反復は要求していない。

[ponder.rs:103](../../../src/search/alphabeta/tests/ponder.rs:103) `ponder_hit_finishes_within_the_post_hit_budget_tolerance` の threads=2 の回は、スケジューリングの標本を1つ増やすだけで、350 msの待機の約半分を占める。`handle.ponderhit` から共有の `hit_ns` を経て探索が止まる統合経路を検査する唯一のテストなので、1スレッドの回は残す。補助ワーカーの停止は team.rs:65 が時間停止で検査する。

削除対象は [ponder.rs:326](../../../src/search/alphabeta/tests/ponder.rs:326) `ponder_hit_before_first_completed_depth_returns_the_root_fallback` である。時間予算がないので、的中は結果に影響しない。根の先頭手と depth 0 は ponder.rs:436 が決定的に、nodes=1 の合法手は [handle.rs:116](../../../src/search/alphabeta/tests/handle.rs:116) `node_limit_stops_the_search_with_a_legal_best_move` が、チームのノード予算は [team.rs:104](../../../src/search/alphabeta/tests/team.rs:104) `four_worker_node_limit_never_exceeds_the_team_budget` が検査する。台帳 [d7.md](../../plans/spec-first-tests/ledgers/d7.md):51 の対応を ponder.rs:436 へ移す。

失う現実的な検出力はない。

推奨: ponder.rs:76 の置換を条件に適用する。ponder.rs:326 は台帳 d7.md:51 の対応を移すことを条件に適用する。

S12では、不採用になった機構の検証テストが残っている。

削除対象は [src/search/alphabeta/tests/contracts.rs:8](../../../src/search/alphabeta/tests/contracts.rs:8) `stage6_long_history_search_contract` と [:104](../../../src/search/alphabeta/tests/contracts.rs:104) `stage6_fixed_node_search_contract` である。[docs/plans/movegen-speedup-2.md](../../plans/movegen-speedup-2.md):21、:285 によると、検証対象だった段階6の「履歴の `HashSet`」と「ノード数の一括反映」は不採用で、この契約の目的は失われた。前者は300手のランダム履歴から深さ5を2回探索し、内部状態を手で組み立てて（320〜350行）検査する。後者はノード上限1、4095、4096、4097、10000で `result.nodes == nodes` を確かめるが、ノードの予約は毎ノード行われ（searcher.rs:167）、STOP_CHECK_INTERVAL は時計の検査周期にすぎない。docs/ からの参照はない。

保持先は、固定ノード探索の再現性が [handle.rs:9](../../../src/search/alphabeta/tests/handle.rs:9) `search_with_node_limit_is_deterministic`、null move 前の履歴が [negamax.rs:155](../../../src/search/alphabeta/tests/negamax.rs:155) `null_move_jitto_does_not_repeat_pre_null_position`（実際の `search_null_move` 経路）、上限との厳密な一致が [root.rs:191](../../../src/search/alphabeta/tests/root.rs:191) `aspiration_interruption_preserves_the_last_completed_iteration`、nodes=1 が handle.rs:116 である。

失う現実的な検出力はない。長い履歴での反復判定の正しさは、もともとこのテストも検査していなかった。

推奨: 適用する。

S13では、同じ条件を通る2つのテストが別々に残っている。

削除対象は [src/search/alphabeta/tests/negamax.rs:33](../../../src/search/alphabeta/tests/negamax.rs:33) `iir_without_tt_entry_searches_and_stores_reduced_depth` である。実装は `depth >= 3 && tt_move.is_none()` の1条件（negamax.rs:89）で、記録なしと手なしの記録は同じ経路を通る。[negamax.rs:78](../../../src/search/alphabeta/tests/negamax.rs:78) `iir_reduces_depth_when_tt_entry_has_no_move` が手なしの記録で保存深さ2を検査し、記録の有無を条件にする誤りも捕まえる。

削除対象は [tt.rs:331](../../../src/search/alphabeta/tests/tt.rs:331) `tt_clear_empties_all_entries_and_search_restarts` である。`clear` は全要素を走査する単純なループで（tt.rs:181〜183）、隣接スロットの3キーは部分消去の誤りも捕まえない。消去の契約は [tt.rs:302](../../../src/search/alphabeta/tests/tt.rs:302) `tt_clear_is_skipped_until_a_search_uses_the_table` が検査し、消去後の探索は空の表で探索する全テストが通る。台帳 [d7.md](../../plans/spec-first-tests/ledgers/d7.md):38 の D7-TT-05 の対応を tt.rs:302 へ移す。

失う現実的な検出力はない。

推奨: tt.rs:331 は台帳 D7-TT-05 の対応を移すことを条件に適用する。negamax.rs:33 は適用する。

S14では、探索ハンドルのテストが実時間の標本や重い入力を重ねている。

一部削除の対象は [src/search/alphabeta/tests/handle.rs:154](../../../src/search/alphabeta/tests/handle.rs:154) `stop_or_tiny_budget_before_depth_one_still_yields_a_legal_best_move` の170〜186行（movetime=1 ms の部分）である。movetime による停止と合法手は [time.rs:331](../../../src/search/alphabeta/tests/time.rs:331) `movetime_search_respects_the_hard_limit_and_returns_a_legal_move` が、深さ1未完了の退避手は前半（157〜168行）と handle.rs:116 が検査する。後半は Soft と Hard の2値を許すだけの実時間標本である。

一部削除の対象は [handle.rs:9](../../../src/search/alphabeta/tests/handle.rs:9) `search_with_node_limit_is_deterministic` のうち、初期局面の100,000ノードの組である。このテストは [docs/guides/sprt.md](../../guides/sprt.md)「ペア対局と再現性」の完全再現契約を担い、0.71秒の大半を初期局面の組が占める。S12で contracts.rs:104 を削除すると、Threads=1 の固定ノード探索の再現性を検査するのはこのテストだけになるので、中盤局面の組は必ず残す。

失う検出力は、局面に依存して現れる非決定性の検出だけであり、現実的な例は特定できない。

推奨: handle.rs:154 は適用する。handle.rs:9 は中盤局面の組を残すことを条件に適用する。

S15では、PST のテストが復号時検証や `invariants` フィーチャと重複している。

削除対象は [src/eval/pst/tests.rs:389](../../../src/eval/pst/tests.rs:389) `embedded_pst_piece_values_satisfy_search_invariants` である。採用中の pst.bin の駒価値が正であり、王駒の値が非王駒の最大値と歩の値の和に等しいことを確かめるが、format.rs の `validate_piece_values` が同じ検査を復号時に行い、失敗した値は復号できない。拒否側は [tests.rs:530](../../../src/eval/pst/tests.rs:530) `decode_rejects_inconsistent_royal_values` と [:583](../../../src/eval/pst/tests.rs:583) `stored_piece_values_are_validated_independently_of_weights` が固定している。

削除対象は [tests.rs:733](../../../src/eval/pst/tests.rs:733) `accumulator_invariants_accept_full_refresh` である。Cargo.toml の自己 dev-dependency により `cargo test` では `invariants` が常に有効になり、探索テスト全般が deepening.rs:45、negamax.rs:105、quiesce.rs:45 などで、整合した累算値に対する同じ検査を通過している。検査を一時的に壊すと失敗することは [tests.rs:742](../../../src/eval/pst/tests.rs:742) `accumulator_invariants_report_mismatch` が確かめ、これは [docs/plans/debugging-tools.md](../../plans/debugging-tools.md):276 の要求に当たる。

失う現実的な検出力はない。

推奨: 適用する。

S16では、評価表の補間と反射のテストが、後から追加された独立期待値のテストに包含されている。

統合対象は [src/eval/pst/tests.rs:486](../../../src/eval/pst/tests.rs:486) `interpolation_clips_both_signs` と [:428](../../../src/eval/pst/tests.rs:428) `distinct_endpoints_follow_phase_boundaries_and_exclude_lion_feature` で、統合先は [tests.rs:611](../../../src/eval/pst/tests.rs:611) `diagnostic_numerators_match_independent_feature_sum_and_clamping` である。統合先は09a3a52より後（c951d57）に追加され、`[i16::MAX;2]` と `[i16::MIN;2]` の重みを両方の手番について駒数100まで検査し、補間係数の境界0、1、2、3、47、92と92超を独立した期待値で検査している。:486 からは駒数144を統合先の駒数リストへ移す。前回の監査のS29が「最大絶対重みと最大枚数」の保持を決めたためである。:428 の期待値は製品と同じ `active_features` で作っており（実装のコピー）、統合先に累算経路の評価と `piece_count` の assert を加えれば足りる。

削除対象は [tests.rs:412](../../../src/eval/pst/tests.rs:412) `identical_endpoints_match_single_table_evaluation` である。両端点が等しいとき q·s+(90−q)·s=90s であり、90s/720 は s/8 に等しいので、0方向への切り捨て結果も一致する。したがって補間式の検査に含まれる。期待値は `active_features` で作っている。

削除対象は [tests.rs:504](../../../src/eval/pst/tests.rs:504) `evaluation_matches_rank_reflection_with_colors_swapped` である。`evaluate` は有効特徴の多重集合だけで決まり（mod.rs:165）、特徴集合の反射一致は先獅子つきの局面で [features.rs:148](../../../src/eval/pst/features.rs:148) `white_features_match_rank_reflection_with_colors_swapped` が固定している。ランダム局面の反射は [tests.rs:689](../../../src/eval/pst/tests.rs:689) `random_legal_positions_preserve_evaluation_under_rank_and_color_reflection` が、後手の累算評価は [tests.rs:126](../../../src/eval/pst/tests.rs:126) `accumulator_updates_match_full_refresh_across_move_shapes` が検査する。前回の監査のS30は縮小した形での保持を決めたが、その後に :689 が加わった。

失う現実的な検出力はない。

推奨: :486 は駒数144を統合先へ移すことを条件に適用する。:428 は累算経路と `piece_count` の assert を統合先へ加えることを条件に適用する。:412 と :504 は適用する。

S17では、LLR の符号の傾向が参照値の照合に包含されている。

削除対象は [src/stats.rs:225](../../../src/stats.rs:225) `mirrored_frequencies_flip_llr_sign_tendency` である。度数を左右反転すると LLR の符号が反転する傾向を確かめる。

保持先は [stats.rs:205](../../../src/stats.rs:205) `public_gsprt_llr_matches_h1_10_fishtest_reference_values_within_1e_6` である。負の LLR（[10,25,45,25,10] で −0.168）と正の LLR の両方を1e-6の精度で照合するので、符号や係数の誤りは必ず検出される。[d8-stats-harness.md](../../plans/spec-first-tests/matrices/d8-stats-harness.md):422 も、参照値を「いかなる変異も1e-6許容で検出する最強のオラクル」と明記している。台帳 [d8.md](../../plans/spec-first-tests/ledgers/d8.md):15 はこのテストを D8-STAT-01 の付随性質としており、STAT-01 は参照値のテスト2件が担う。前回の監査のS39が保持したのは Elo の反転テスト（stats.rs:277）であり、この LLR のテストではない。

失う現実的な検出力はない。

推奨: 適用する。

## 保持と判断した主なテスト

- [src/eval/pst/tests.rs:604](../../../src/eval/pst/tests.rs:604) `embedded_pst_matches_python_initial_position_evaluation` は、期待値を6回書き換えた（12→−8→67→57→33→24→27、af200b4、77fe8c0、960c56b、240fbfd、4bf2507、0fd2824）。それでも保持する。[docs/guides/pst-training.md](../../guides/pst-training.md):355〜358 は、採用のたびにこの値を Python の診断値と学習ログで確認して更新するよう定めており、書き換えそのものが採用時の照合手順になっている。Rust の評価を Python の参照評価と照合するテストはこれだけである。
- [src/search/alphabeta/tests/late_move.rs:37](../../../src/search/alphabeta/tests/late_move.rs:37) `late_move_limits_follow_quadratic_depth_rule` は、期待値を pruning.rs:63〜66 と同じ式で計算している。それでも、深さ2〜3での2乗則の誤り（d*d を d にするなど）を検査する唯一のテストなので保持する。
- [src/search/alphabeta/tests/pruning.rs:9](../../../src/search/alphabeta/tests/pruning.rs:9) `lmr_table_follows_logarithmic_rule` は式の写しを含むが、前回の監査のS16が LMR の直積テストを縮小する根拠としてこのテストに頼っているので保持する。ただし pruning.rs:22 と :45〜46 の assert は LmrDivisor が約222以下でなければ成り立たない。宣言範囲は100〜400で、これまでの値は200、186、156であり、書き換えはまだ起きていない。
- [pruning.rs:343](../../../src/search/alphabeta/tests/pruning.rs:343) `see_pruning_skips_losing_capture_only_at_eligible_nodes` と [pruning.rs:102](../../../src/search/alphabeta/tests/pruning.rs:102) `futility_reduces_quiet_nodes_without_false_mate` は、それぞれ SeeMargin2・SeeMargin3 と PST の値、FutilityMargin3 が300%未満であることに依存する。それでも SPSA の3回の適用を通じて書き換えは0回だったので保持する。
- [src/search/alphabeta/see.rs:340](../../../src/search/alphabeta/see.rs:340) などの SEE の差分試験と逆引き回数の検査は、本番の `see_prunes` を観測しており、[docs/plans/movegen-speedup.md](../../plans/movegen-speedup.md)「SEEの不要な反復を省く」の契約に対応する。凍結した pst-init.bin を使うので、PST の重みが変わっても書き換えは要らない。
- [captures.rs:105](../../../src/search/alphabeta/tests/captures.rs:105) と `warmed_buffers_retain_capacity_across_nodes` は内部実装に依存するが、[docs/plans/movegen-speedup.md](../../plans/movegen-speedup.md):145、:313 が検証を明示的に求めているので保持する。
- [history.rs:81](../../../src/search/alphabeta/tests/history.rs:81) `history_carry_changes_the_second_search_with_a_fresh_transposition_table` は、持ち越した履歴を探索が実際に読むことを示す唯一の証拠である。[capture_history.rs:274](../../../src/search/alphabeta/tests/capture_history.rs:274) と [correction.rs:190](../../../src/search/alphabeta/tests/correction.rs:190) は、butterfly history を持ち越す設計に変わった中で、捕獲履歴と補正表を探索ごとに作り直す方針を守る。
- [negamax.rs:8](../../../src/search/alphabeta/tests/negamax.rs:8) `depth_zero_tt_score_does_not_cut_off_depth_one_negamax` は、照会規則 `hit.depth >= depth` 自体を negamax.rs:113 と :78 が覆うが、[d7-search-eval.md](../../plans/spec-first-tests/matrices/d7-search-eval.md):132 の D7-SRCH-12 がまさにこの境界を挙げているので保持する。
- [ponder.rs:8](../../../src/search/alphabeta/tests/ponder.rs:8)、[:178](../../../src/search/alphabeta/tests/ponder.rs:178)、[:244](../../../src/search/alphabeta/tests/ponder.rs:244)、[:343](../../../src/search/alphabeta/tests/ponder.rs:343)、[:436](../../../src/search/alphabeta/tests/ponder.rs:436) は決定的で、[docs/plans/ponder.md](../../plans/ponder.md):274 が要求する当て直し、hard の優先、1回性、t の不変、補助ワーカー結果の採用、生成前の的中をそれぞれ唯一の場所で固定している。
- [time.rs:133](../../../src/search/alphabeta/tests/time.rs:133) `movetime_and_clock_combine_per_limit_by_taking_the_smaller` の期待値1037/4676はSPSA係数に依存し、d491c37 で書き換わった。それでも soft と hard を独立に比較する `time_budget` の検査はここだけなので保持する。期待値を `clock_budget(base)` から計算すれば書き換えは不要になる。
- [time.rs:357](../../../src/search/alphabeta/tests/time.rs:357) `clock_driven_searches_stop_with_a_time_limit_reason` の「70 ms/70 ms」は、前回の監査のS15が残り0を無期限扱いする配線の回帰を守るために残したもので、固定の安全余裕30 msと秒読みの8/10から決まり、SPSA係数に依存しない。[time.rs:331](../../../src/search/alphabeta/tests/time.rs:331) は hard の上限が実際に効くことを端から端まで確かめる唯一のテストで、許容幅が700 msと広く15回とも通った。
- [root.rs](../../../src/search/alphabeta/tests/root.rs) の7件、team.rs の残り、limits.rs、royal.rs、その他の tt.rs のテスト、scoring.rs の6件（D7-SRCH-01〜06）は、いずれも別の分岐を検査している。停止・panic 時のワーカー回収、前回の監査のS07とS18は状況が変わっていない。
- [src/search/alphabeta/tests/stats.rs](../../../src/search/alphabeta/tests/stats.rs) の10件は search-stats フィーチャでだけ走り、既定の費用は0である。
- [src/stats.rs:277](../../../src/stats.rs:277) `frequency_reflection_and_scaling_preserve_elo_properties`、旧H1の参照値（前回の監査のS35）、`#[ignore]` のモンテカルロ検証 [stats.rs:341](../../../src/stats.rs:341)（同S40）は状況が変わっていない。
- [src/eval/handcrafted.rs](../../../src/eval/handcrafted.rs) の3件は、v0評価関数を比較用に残すという利用者の決定に従い、凍結した駒価値と対称性の回帰検査として保持する。
- [tests.rs:611](../../../src/eval/pst/tests.rs:611)、[:656](../../../src/eval/pst/tests.rs:656)、[:689](../../../src/eval/pst/tests.rs:689) は [docs/plans/debugging-tools.md](../../plans/debugging-tools.md):39、:276 の契約に当たり、S16の統合先にもなる。
- 採用されなかった実験のテストは残っていない。history pruning はコードもテストもなく、improving は a2e35d7 で復活し NonImprovingFutility 係数が現在も使われている。取り消した実験（b4bcd95、a3845f2、5f02104、602f7e1、1b96a3d）は、テストも一緒に取り消されている。
