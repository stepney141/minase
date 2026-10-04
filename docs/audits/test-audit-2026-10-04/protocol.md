対象は [src/protocol/usi.rs](../../../src/protocol/usi.rs) の77テスト、[src/protocol/usi/tests/history.rs](../../../src/protocol/usi/tests/history.rs) の4テスト、[src/protocol/cecp.rs](../../../src/protocol/cecp.rs) の29テスト、[src/protocol/engine.rs](../../../src/protocol/engine.rs) の6テスト、[src/notation/usi.rs](../../../src/notation/usi.rs) の9テスト、[src/notation/cecp.rs](../../../src/notation/cecp.rs) の6テスト、[tests/debugging_tools.rs](../../../tests/debugging_tools.rs) の3テスト、[tests/io_log.rs](../../../tests/io_log.rs) の5テスト、[src/bin/minase/](../../../src/bin/minase/) の7テスト、[src/bin/usi_random.rs](../../../src/bin/usi_random.rs) の5テスト、計151テストである。全テスト本文と、対応する実装を確認した。本番コードとテストは変更していない。実行結果は[全体の報告](../test-audit-2026-10-04.md)に記載した。行番号は監査時点（master 9027662）のものである。この範囲で時間を計測できた142件の合計は8.92秒、最長は0.65秒であり、候補は実行時間ではなく重複と保守負担を根拠にした。1秒未満の費用の多くは、新しいセッションの`go`または`isready`が256 MiBの既定置換表を確保することから生じる。これは削除ではなく試験側の置換表容量の指定で下げられるので、候補には含めていない。

根拠は RULES.md 第15版、[docs/plans/protocol-layer.md](../../plans/protocol-layer.md)、[ponder.md](../../plans/ponder.md)、[usi-resignation.md](../../plans/usi-resignation.md)、[debugging-tools.md](../../plans/debugging-tools.md)、[time-management-efficiency.md](../../plans/time-management-efficiency.md)、[docs/plans/spec-first-tests/matrices/d5-notation.md](../../plans/spec-first-tests/matrices/d5-notation.md)、[d6-protocol.md](../../plans/spec-first-tests/matrices/d6-protocol.md) と台帳 [d5.md](../../plans/spec-first-tests/ledgers/d5.md)、[d6.md](../../plans/spec-first-tests/ledgers/d6.md) である。マトリクスの行数や過去のテスト件数は保持理由にしない。「削除」は現在の他テストを残せば実行でき、「統合」は記載した観測の移植を先に要する。

USIとCECPの各テストが [protocol/engine.rs](../../../src/protocol/engine.rs) の共通処理を二重に検査していないかも照合した。重なっていたのはP06と、P16のCECP規則テストの解析エラー2件だけである。engine.rs のライフサイクル、局面延長の原子性、規則設定、末尾の手の置換の各テストはコマンドの境界を見ており、プロトコル側は通信上の経路選択を見ているので、両方を残す。表記の2ファイル（N）は前回監査の適用後で十分に絞られており、候補はない。

以下は、結果がスケジューリングに依存するテストを含み、優先して整理できる候補である。

P01では、実行中の先読みの的中と外れを、スケジューリングに依存する形で検査している。

削除対象は [src/protocol/usi.rs:4034](../../../src/protocol/usi.rs:4034) `ponder_channel_hit_continues_search_and_miss_recovers`である。

先読みに的中しても探索が続き（`stop nodes`）、`stop`では`stop external`になり、その後の`go`が通ることを見る。stopの周回は、50,000ノードの探索がstop処理より先に終わると、保留結果が`stop nodes`で解放されて失敗する。結果が構造上スケジューリングに依存する。監査時の全体実行と、負荷平均約23の下での単独10回の再実行では失敗しなかったが、失敗し得る条件は解消されていない。

保持先は3件ある。的中後も実行中が続くことは [usi.rs:2473](../../../src/protocol/usi.rs:2473) `resignation_running_ponderhit_continues_until_the_adopted_result` が決定的に検査する。実行中のstopが`stop external`になることは [usi.rs:2437](../../../src/protocol/usi.rs:2437) `resignation_running_stop_returns_resign_for_normal_infinite_and_ponder_searches` が、stop後の`go`は [usi.rs:3286](../../../src/protocol/usi.rs:3286) `commands_arriving_during_search_apply_after_bestmove` が見る。失う現実的な検出力はない。P10で usi.rs:2437 を1種類へ減らす場合も、先読み中のstopの周回は残すことが条件になる。

推奨: 適用する。

P02では、先読みの予想手の検査に、他のテストと重複する周回が含まれる。

一部削除の対象は [src/protocol/usi.rs:4233](../../../src/protocol/usi.rs:4233) `ponder_prediction_uses_adopted_pv_and_rejects_illegal_or_finishing_moves` のうち、threads=1の周回とdepth 1のセッションである。

threads=1の`bestmove X ponder pv[1]`は [usi.rs:3985](../../../src/protocol/usi.rs:3985) `ponder_channel_withholds_completed_result_until_hit_or_stop` の4013行付近と同じ観測である。1手しかない読み筋で予想手が付かないことは、同テストの単体assert（usi.rs:4286付近）が構成局面で固定している。Threads=2の周回は採用した読み筋の扱いを検査する固有の観測なので残す。

このテストは、共有置換表のカットオフでThreads=2のPVが1手になると失敗することがあったが、2026-09-22のコミット6c5c559で1手のPVを許す形に直されている。監査時の負荷下での単独10回の再実行でも失敗しなかった。失う現実的な検出力はない。

推奨: 適用する。

以下は、同じ保証を持つテストを削除・統合する候補である。

P03では、探索中の重複goと破棄コマンドを、先読みのテストと同じ分岐で再検査している。

削除対象は [src/protocol/usi.rs:3266](../../../src/protocol/usi.rs:3266) `duplicate_go_is_rejected_and_bestmove_stays_unique` と [src/protocol/usi.rs:3333](../../../src/protocol/usi.rs:3333) `gameover_and_quit_during_search_discard_the_result` である。

探索中の`go`の分岐（usi.rs:413付近）と、探索中の`gameover`・`quit`が`discard_search`へ進む分岐（usi.rs:409-412付近）は、探索の種類で分かれない。前者は [usi.rs:3985](../../../src/protocol/usi.rs:3985) `ponder_channel_withholds_completed_result_until_hit_or_stop` が4000-4002行で重複goのエラーを、3974行で余分なbestmoveがないことを観測する。後者は [usi.rs:4125](../../../src/protocol/usi.rs:4125) `ponder_invalid_hits_and_discard_commands_preserve_lifecycle_contracts` の4138-4147行が、入力切断を加えた上位集合として検査する。失う現実的な検出力はない。P08で usi.rs:3985 のtriggerを1周に減らしても、重複goの観測は残る。

推奨: 適用する。

P04では、投了の設定が破棄の結果に影響しないことを、投了と無関係な経路で確かめている。

削除対象は [src/protocol/usi.rs:2617](../../../src/protocol/usi.rs:2617) `resignation_discarded_running_and_held_results_emit_no_bestmove` である。

`discard_search`（usi.rs:697-708付近）は投了判定もbestmoveの出力も呼ばないので、ResignValueは結果を変えられない。保留中の破棄3経路は [usi.rs:4081](../../../src/protocol/usi.rs:4081) `ponder_held_results_release_once_or_are_discarded` が、実行中の破棄3経路は usi.rs:4138-4147 が固定している。失う現実的な検出力はない。D6マトリクスの対応欄を保持先へ移す。

推奨: 適用する。

P05では、規則セットの正準表示と上限深さの受理を、既存の観測で重ねて検査している。

削除対象は [src/protocol/usi.rs:2698](../../../src/protocol/usi.rs:2698) `ruleset_default_is_the_canonical_form_of_the_startup_rules` と [src/protocol/usi.rs:3135](../../../src/protocol/usi.rs:3135) `go_depth_256_at_the_upper_bound_is_accepted` である。

正準化は`canonical_rule_codes`（engine.rs:428付近）にある。順不同の入力からの正準default表示は usi.rs:1972 付近、群内の番号順は [usi.rs:3807](../../../src/protocol/usi.rs:3807) `state_line_matches_the_exact_contract` と usi.rs:2741 付近が固定している。プリセット名が出ないことは、`Engine::new`が展開後のコードを受け取る構造で保証される。

通信上のdepth 256の受理は usi.rs:4134 と usi.rs:4141 にあり、どちらもP08で残す部分である。`stop nodes`の語は [usi.rs:2130](../../../src/protocol/usi.rs:2130) `resignation_depth_zero_returns_a_legal_move_without_search_info` の2139行付近が固定している。前回監査P16は nodes の語の保持先に usi.rs:3135 を指定していたので、その契約を usi.rs:2139 へ移す扱いになる。失う現実的な検出力はない。

推奨: 適用する。

P06では、開始前の失敗commitを、共通層のテストと同じ手順で通信越しに再検査している。

削除対象は [src/protocol/usi.rs:3889](../../../src/protocol/usi.rs:3889) `failed_awaiting_start_commit_changes_nothing` である。

保留中の規則をR2にし、失敗commitの後に対局未開始のエラーを返し、次のcommitでR2を反映する手順を、[src/protocol/engine.rs:537](../../../src/protocol/engine.rs:537) `failed_commit_is_atomic_and_keeps_active_pending_and_lifecycle` が直接固定している。USI層は転送するだけである。position失敗時のエラー出力は [usi.rs:2853](../../../src/protocol/usi.rs:2853) `position_applies_atomically_or_not_at_all` が見る。失う現実的な検出力はない。

推奨: 適用する。

P07では、clapが保証する引数の性質を再確認している。

削除対象は [src/bin/minase/main.rs:148](../../../src/bin/minase/main.rs:148) `cli_io_log_is_optional_and_accepts_a_path` である。一部削除の対象は [src/bin/minase/main.rs:171](../../../src/bin/minase/main.rs:171) `rules_argument_shares_the_wire_value_grammar` のうち、`lishogi,P1`と、群を欠く列の`Engine::new`による拒否である。

`Option<PathBuf>`の省略可能性と値の欠落エラーはclapの保証であり、プロトコルの値は解析に影響しない。パスの受理は [tests/io_log.rs](../../../tests/io_log.rs) の全プロセス試験が実際に使っている。規則の併記の拒否は [src/core/rules/tests.rs](../../../src/core/rules/tests.rs) の127行付近、群の欠如は93行付近が固定しており、起動引数が共通文法へ接続されていることの確認はLISHOGIの1件で足りる。失う現実的な検出力はない。

推奨: 適用する。

P08では、先読みの保留と破棄のテストに、同じ分岐を通る周回が重なっている。

一部削除の対象は次の3つである。

- [src/protocol/usi.rs:3911](../../../src/protocol/usi.rs:3911) `ponder_accepts_finite_limits_and_ignores_usi_ponder_option` の、通信上の`ponder`と`ponder infinite depth 1`のケース、拒否ごとの`go depth 1`、およびUSI_Ponderの`true`の周回。
- [src/protocol/usi.rs:3985](../../../src/protocol/usi.rs:3985) `ponder_channel_withholds_completed_result_until_hit_or_stop` のtriggerを`ponderhit`の1周にし、末尾の待機中ponderhitのエラーも削る。
- [src/protocol/usi.rs:4125](../../../src/protocol/usi.rs:4125) `ponder_invalid_hits_and_discard_commands_preserve_lifecycle_contracts` の末尾の「go ponder中のposition、stop、state」のブロック。4138-4147行の破棄の周回はP03とP04の保持先なので残す。

`go ponder`だけの拒否は [usi.rs:3184](../../../src/protocol/usi.rs:3184) `ponder_without_limits_and_idle_ponderhit_are_rejected` と同じであり、2つのinfinite併用ケースは同じ分岐（usi.rs:1260-1264付近）を通る。未知オプションの無視は usi.rs:851-853 付近、宣言しないことは usi.rs:1988 付近、bestmoveへの予想手の付与は usi.rs:4013 付近が固定している。保留中のstopとponderhitはどちらも`finish_search`の停止待ち分岐へ進み、停止要求を見ない（usi.rs:574-576、605-619付近）。両triggerは [usi.rs:4081](../../../src/protocol/usi.rs:4081) `ponder_held_results_release_once_or_are_discarded` が決定的に検査する。探索中のpositionは種類によらず待機列へ積まれ、遅延適用は [usi.rs:3286](../../../src/protocol/usi.rs:3286) が同じ観測で固定している。[ponder.md](../../plans/ponder.md) の「検証」が求める1行ずつの観測は1周で保たれる。失う現実的な検出力はない。

推奨: 適用する。

P09では、同じ出力を観測する2つのテストが並んでいる。

統合対象は次の4組である。

- [src/protocol/usi.rs:2683](../../../src/protocol/usi.rs:2683) `isready_returns_readyok_without_changing_state` を [usi.rs:2671](../../../src/protocol/usi.rs:2671) `isready_allocates_the_default_transposition_table_before_the_first_go` へ。isreadyの実装は置換表にしか触れないので、state不変の1assertだけを移す。実測0.43秒の大半は256 MiBの確保2回による。台帳はD6-USI-02である。
- [src/protocol/usi.rs:2064](../../../src/protocol/usi.rs:2064) `resignation_option_declares_default_and_bounds_once` を [usi.rs:1961](../../../src/protocol/usi.rs:1961) `handshake_declares_ruleset_before_variant_and_ends_with_usiok` へ。後者が同じ握手出力の各option行と末尾のusiokを検査しているので、ResignValue宣言の1行一致assertを移す。台帳はD6-USI-47である。
- [src/protocol/usi.rs:3318](../../../src/protocol/usi.rs:3318) `multi_worker_search_places_info_immediately_before_one_bestmove` を [usi.rs:2145](../../../src/protocol/usi.rs:2145) `resignation_default_emits_mate_info_then_stop_then_resign_once` のThreads=2のセッションへ。`assert_search_info`は投了出力にも適用できる。usi.rs:4233 はThreads=2のPV長に結果が左右される経緯があるので統合先にしない。depth 4は採用したinfoの経路を出やすくする選択なので、統合先でその経路が出ることを確認する。
- [src/protocol/usi/tests/history.rs:109](../../../src/protocol/usi/tests/history.rs:109) `history_carry_usi_keeps_history_between_go_commands` を [history.rs:135](../../../src/protocol/usi/tests/history.rs:135) `history_carry_usi_returns_ponder_history_on_hit_and_miss` へ。後者の的中かつ非保留の場合が同じ`finish_search`の実行中分岐を通り、回収値が種値と減衰の積であることから、保存済みの表がgoへ渡ることも示す。goごとに1回の減衰は [src/search/alphabeta/tests/history.rs](../../../src/search/alphabeta/tests/history.rs) の45行付近がハンドルの往復で固定している。探索中の`histories.is_none()`は内部状態への依存であり、期待値は実装（team.rs:180-185付近）と同じ式で計算している。期待値は0e5be87で1回書き換えた。

いずれも統合すれば失う現実的な検出力はない。

推奨: 適用する。usi.rs:3318 は、統合先のThreads=2のセッションで採用infoの経路が出ることを確認したうえで適用する。

P10では、投了の判定を同じ比較と同じ分岐で重ねて検査している。

投了の判定`should_resign`（usi.rs:692-694付近）は整数比較1つで、詰み帯の分岐を持たない。`finish_search`の実行中分岐と停止待ち分岐は、探索の種類（通常、無限、先読み）で分かれない。次の候補は、[usi-resignation.md](../../plans/usi-resignation.md) の145行付近とD6-USI-47〜60がケースを名指ししている。

- 削除: [src/protocol/usi.rs:2325](../../../src/protocol/usi.rs:2325) `resignation_infinite_completed_result_waits_for_stop`。停止待ち分岐は無限か先読みかを区別しない。保留中のstopによる投了は usi.rs:2233 と usi.rs:2516 が、無限探索がbestmoveを保留することは [usi.rs:3218](../../../src/protocol/usi.rs:3218) `go_infinite_withholds_bestmove_until_stop` が見る。台帳はD6-USI-55である。
- 一部削除: [src/protocol/usi.rs:2078](../../../src/protocol/usi.rs:2078) `resignation_option_accepts_bounds_and_rejects_invalid_values_without_changing_it` の`value`、`nope`、`-1`、`1.5`、`1e3`、`１２`のケースと、不正値ごとの`go depth 1`を最後の1回へ。値の欠落は空文字列と同じ分岐を、非数字はすべて`is_ascii_digit`の検査（usi.rs:840-845付近）を通る。`i32::parse`が受理してしまう`+1`、0、100000、桁あふれは残す。台帳はD6-USI-48である。
- 一部削除: [src/protocol/usi.rs:2207](../../../src/protocol/usi.rs:2207) `resignation_mate_thresholds_and_winning_scores_use_signed_comparison` の閾値29744と30000のケース。29744は29998と、30000は29999と同じ結果になる。29998と29999の等号境界と、ハーネスが無効化に使う99999は残す。台帳はD6-USI-53である。
- 一部削除: [src/protocol/usi.rs:2233](../../../src/protocol/usi.rs:2233) `resignation_uses_adopted_score_at_mate_boundary_without_repeating_info` の解放方法`ponderhit`と、評価値の組`(-29744, 29744/29745)`。15セッションが6になる。
- 一部削除: [src/protocol/usi.rs:2437](../../../src/protocol/usi.rs:2437) `resignation_running_stop_returns_resign_for_normal_infinite_and_ponder_searches` の3種類の制限を1つへ。残す1種類は先読みとする。P01の保持先だからである。台帳はD6-USI-54である。
- 一部削除: [src/protocol/usi.rs:2516](../../../src/protocol/usi.rs:2516) `resignation_held_stop_and_ponderhit_preserve_score_and_completed_depth` のdepth 2で投了するケース。保留した深さ0で投了しない境界（D6-USI-55、D6-USI-57）はこのテストにしかないので残す。
- 一部削除: [src/protocol/usi.rs:2563](../../../src/protocol/usi.rs:2563) `resignation_option_changes_wait_until_running_or_held_result_is_released` の保留中と実行中の片方。setoptionは探索の状態によらず待機列へ積まれ、両分岐とも解放時点の閾値を読む。台帳はD6-USI-49である。

失う現実的な検出力はない。判定を結果の種類や探索の種類ごとに書き分ける実装へ変えた場合に、片側だけの誤りを見逃す可能性は残るが、その書き換えは現行の設計にない。

推奨: usi-resignation.md:22 の「14挙動を15テストで固定」という状態の記述と、D6-USI-47〜60の対応欄を書き換えることを条件に適用する。

P11では、規則セットの値の拒否を、core側の文法テストと同じ分岐で重ねて検査している。

一部削除の対象は次のとおりである。

- [src/protocol/usi.rs:2732](../../../src/protocol/usi.rs:2732) `presets_expand_to_code_lists_and_reject_any_combination` の2本目のセッションの`lishogi,P1`と`lishogi,engine-default`。併記拒否の文法は [src/core/rules/tests.rs](../../../src/core/rules/tests.rs) の111行付近と127行付近が固定し、拒否後も保留が残ることは usi.rs:2767 が見る。`standard`の拒否は [protocol-layer.md](../../plans/protocol-layer.md) の167行付近とD6-USI-05が要求し、core側にテストがないので残す。
- [src/protocol/usi.rs:2767](../../../src/protocol/usi.rs:2767) `invalid_ruleset_values_are_rejected_on_receipt_and_pending_survives` の`R0`のケース。`R0`は`XX9`と同じ未知コードの分岐を通り、R0が非提供であることは src/core/rules/tests.rs の13行付近が固定している。
- [src/protocol/cecp.rs:1670](../../../src/protocol/cecp.rs:1670) `ruleset_option_latches_until_new_and_rejects_invalid_values` の`lishogi,P1`のケースと、末尾のLISHOGIとengine-defaultの2セッション。`XX9`と`lishogi,P1`はCECPでは同じ解析エラーの分岐を通り、プリセットの文法は src/core/rules/tests.rs の100行付近が固定している。

失う現実的な検出力はない。

推奨: 適用する。

P12では、lishogiのBotの対局列を、ほぼ全セッションと同じ経路で検査している。

削除対象は [src/protocol/usi.rs:3019](../../../src/protocol/usi.rs:3019) `lishogi_bot_series_completes_without_usinewgame` である。

本モジュールのほぼ全セッションが、usinewgameなしの`position startpos`からgoする（例 [usi.rs:3149](../../../src/protocol/usi.rs:3149) `go_depth_returns_a_deterministic_legal_bestmove_without_applying_it`、usi.rs:3286）。全列の再送による延長は [usi.rs:2880](../../../src/protocol/usi.rs:2880) `incremental_position_matches_full_replay_for_ongoing_and_finished_games` が、USI_Variantは [usi.rs:2815](../../../src/protocol/usi.rs:2815) `usi_variant_accepts_only_chushogi` が見る。失う現実的な検出力はない。台帳D6-USI-13の対応欄を保持先へ移す。

推奨: 適用する。

P13では、診断コマンドの表示検査に、評価関数のテストと重なる局面と直積が含まれる。

一部削除の対象は [src/protocol/usi.rs:3654](../../../src/protocol/usi.rs:3654) `diagnostic_evaluation_matches_full_evaluation_with_and_without_clamping` の密な盤の2局面と上下限制限のassert、および [src/protocol/usi.rs:3731](../../../src/protocol/usi.rs:3731) `diagnostic_hit_formats_all_fields` の3種類のbound×bestmove有無の6通り（Someの3通りとNoneの1通りへ）である。

評価の内訳と全評価の一致は、上下限制限と両手番を含めて [src/eval/pst/tests.rs](../../../src/eval/pst/tests.rs) の610行付近と653行付近が固定している。通信固有の行構成（12行×15語、注記、獅子の行）は1局面で足りる。boundとbestmoveは独立に書式化される（usi.rs:1720-1736付近）。usi.rs:3731 の費用はほぼ0なので優先度は低い。失う現実的な検出力はない。

推奨: 適用する。

P14では、制限のないgoの拒否の後に、他テストと同じ探索を走らせている。

一部削除の対象は [src/protocol/cecp.rs:1466](../../../src/protocol/cecp.rs:1466) `go_without_search_limits_is_a_tellusererror` の末尾の`sd 1\ngo`による探索である。

sd 1の探索で応手が出ることは [cecp.rs:1375](../../../src/protocol/cecp.rs:1375) `engine_moves_are_emitted_as_move_lines_with_a_legal_payload` と [cecp.rs:1424](../../../src/protocol/cecp.rs:1424) `usermove_triggers_exactly_one_automatic_reply` が見る。実測0.12秒の大半はこの探索と256 MiBの確保による。失う現実的な検出力はない。

推奨: 適用する。

P15では、プロトコルに依存しない処理を、規則セットやプロトコルごとに繰り返している。

一部削除の対象は次のとおりである。

- [tests/debugging_tools.rs:11](../../../tests/debugging_tools.rs:11) `perft_accepts_d_output_with_lion_and_deferred_promotion` の3規則セットを`L1,L2,P1,R1,E2`の1つへ。[debugging-tools.md](../../plans/debugging-tools.md) の220行付近が求めるのは、成り権の保留と先獅子を含む1局面だけである。P1、P2、P5ごとの保留欄の受理は [src/notation/sfen.rs](../../../src/notation/sfen.rs) の規則別テストが、dのSFEN往復は [usi.rs:3554](../../../src/protocol/usi.rs:3554) `diagnostic_sfen_round_trips_all_position_state` が見る。
- [tests/io_log.rs:171](../../../tests/io_log.rs:171) `existing_log_is_preserved_and_startup_fails` のcecpの周回。ログはプロトコルを分岐する前に開く（main.rs:79-88付近）。
- [tests/io_log.rs:198](../../../tests/io_log.rs:198) `cecp_session_records_both_directions` の、ログなし実行との比較`run("cecp", None, input)`。ログ出力はプロトコルに依存せず、標準出力のバイト一致は [tests/io_log.rs:186](../../../tests/io_log.rs:186) `logging_preserves_stdout_byte_for_byte` が見る。CECP分岐の配線（main.rs:113-116付近）の確認には、記録の比較だけで足りる。

失う現実的な検出力はない。

推奨: 適用する。

## 保持と判断した主なテスト

- [usi.rs:3530](../../../src/protocol/usi.rs:3530) `diagnostics_queued_during_search_follow_bestmove` は残す。探索後のttコマンドは、置換表が探索から返却されたことを観測できる唯一の手段である。返却されないと`start_go`が黙って新しい表を確保するため、他のテストでは検出できない。
- [usi.rs:2671](../../../src/protocol/usi.rs:2671) `isready_allocates_the_default_transposition_table_before_the_first_go` は内部状態を参照するが残す。[time-management-efficiency.md](../../plans/time-management-efficiency.md) がこの挙動を要求しており、所要時間以外に外から観測する方法がない。
- 先読みの保留解放と破棄（[usi.rs:4081](../../../src/protocol/usi.rs:4081)）、逐次経路（[usi.rs:4166](../../../src/protocol/usi.rs:4166) `ponder_sequential_hit_finishes_before_reading_another_command`）、末尾の手の置換（[usi.rs:4330](../../../src/protocol/usi.rs:4330) `ponder_last_move_replacement_matches_replay_and_is_atomic`）、4,000手の固定費を測る`#[ignore]`の [usi.rs:4422](../../../src/protocol/usi.rs:4422) `ponder_long_game_output_and_replacement_stay_within_three_ms` は残す。いずれも [ponder.md](../../plans/ponder.md) の「検証」が要求しており、P01、P02、P08の削減後もこれらの要求は満たされる。
- 投了のうち、実際の探索値による等号境界（[usi.rs:2182](../../../src/protocol/usi.rs:2182) `resignation_material_threshold_includes_equality`）、Threads 1と2の出力順（usi.rs:2145）、深さ0（usi.rs:2130）、的中後も実行中が続くことの決定的な確認（usi.rs:2473）、`Protocol::run`の逐次経路（[usi.rs:2367](../../../src/protocol/usi.rs:2367) `resignation_sequential_normal_and_ponderhit_return_resign`）は、それぞれ固有の経路または境界を持つので残す。
- 履歴表の設定拒否時の保存と新規対局での消去を見る [history.rs](../../../src/protocol/usi/tests/history.rs) の2件は内部状態に依存するが、履歴の消去を外から決定的に観測する方法がないので残す。
- 前回監査で保持と決めたもの（CECPのstop系、`go_infinite_withholds_bestmove_until_stop`、`clock_stop_reasons_have_distinct_protocol_words`、Hashとmemoryの桁あふれ、Threadsとcoresの境界、CECPの理由表）は、状況が変わっていないので再提案していない。
- 表記の2ファイルは残す。USIとCECPの指し手の解析は別実装（notation/cecp.rs:70付近、notation/usi.rs:104付近）なので、両方の正規化テストが要る。[src/notation/usi.rs:426](../../../src/notation/usi.rs:426) `three_square_input_normalizes_by_intermediate_occupancy` の相手駒を経由するassert1つは、[usi.rs:338](../../../src/notation/usi.rs:338) `lishogi_documented_three_square_examples_are_legal_moves` と同じ局面・同じ入力の重複だが、費用がほぼ0なので候補にしない。
- [src/bin/usi_random.rs](../../../src/bin/usi_random.rs) の5件は安価で、この実行ファイル固有の契約を見ているので残す。[tests/io_log.rs](../../../tests/io_log.rs) の`usi_session_records_handshake_search_and_exact_stdout`、`input_preserves_whitespace_and_removes_only_line_endings`、`logging_preserves_stdout_byte_for_byte`、および [src/bin/minase/io_log.rs](../../../src/bin/minase/io_log.rs) の単体テスト4件も残す。記録の失敗の伝播は統合試験では観測できないからである。
