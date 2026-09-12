現行の `src/notation/**` と `src/protocol/**` にある120テストの本文をすべて読んだ。本番コードとテストは変更していない。実行結果は[全体の報告](../test-audit-2026-09-12.md)に記載した。以下の行番号は監査時点のソースによる。削除可能とは、指定した残存テストと統合条件を満たした場合を指す。同じ残存テストを別の候補でも消す場合は、契約の引継ぎが必要である。

USIとCECPは対局プログラムの通信方式、SFENは局面の文字列表現を指す。

根拠は [docs/protocols/usi-lishogi.md](/home/stepney141/board-games/minase/docs/protocols/usi-lishogi.md)、[docs/protocols/cecp.md](/home/stepney141/board-games/minase/docs/protocols/cecp.md)、[docs/protocols/hachu.md](/home/stepney141/board-games/minase/docs/protocols/hachu.md)、[docs/plans/protocol-layer.md](/home/stepney141/board-games/minase/docs/plans/protocol-layer.md)、D5・D6の挙動マトリクスと追跡台帳、および後発の [docs/plans/engine-connectivity.md](/home/stepney141/board-games/minase/docs/plans/engine-connectivity.md)、[docs/plans/strength-stage2.md](/home/stepney141/board-games/minase/docs/plans/strength-stage2.md)、[docs/plans/lazy-smp.md](/home/stepney141/board-games/minase/docs/plans/lazy-smp.md) とした。マトリクスの1項目につき1テストという分割は、保持理由にしていない。

以下は、重複する探索を減らす候補である。

P01では、USIの同一探索を3回から2回へ減らす。

- 対象は [src/protocol/usi.rs:2092](/home/stepney141/board-games/minase/src/protocol/usi.rs:2092) の `go_depth_returns_a_deterministic_legal_bestmove_without_applying_it`、特に2110〜2113行の `again` / `again2` である。
- `again2` のセッションを無条件に削除し、最初の `output` の bestmove と `again` の bestmove を比較する。最初も `position startpos; go depth 1` を実行しており、追加の `moves` / `state` は読み取り専用である。
- 消すことで失う現実的な検出力はない。合法性、go後の局面不変、同一条件での決定性が同じテストに残る。3回目だけで出るランダム障害のために残す価値はない。
- 1回の探索と既定置換表の確保を減らせる。既定置換表は256 MiB（[src/search/tt.rs:16](/home/stepney141/board-games/minase/src/search/tt.rs:16)）で、USIは探索開始時に確保する（[src/protocol/usi.rs:221](/home/stepney141/board-games/minase/src/protocol/usi.rs:221)）。所要時間は未計測である。

P02では、USIのHash正常系とオーバーフロー回復を1セッションへ統合する。

- 対象は [src/protocol/usi.rs:2383](/home/stepney141/board-games/minase/src/protocol/usi.rs:2383) の `usi_hash_is_accepted_and_leaves_the_game_unchanged` と2404行の `oversized_usi_hash_is_rejected_and_the_session_continues` である。
- 条件付き統合とする。初期局面とstateを記録し、0拒否、極大値拒否、正当な設定、state不変、1回の探索成功を1台本で観測する。元の2本は削除できる。
- 極大値でのパニック、無効設定の握りつぶし、拒否後に探索不能となる不具合は残す。元の2本で別々に行う `position startpos; go depth 1` に独自の検出価値はない。
- 正常サイズ64 MiBも境界ではない。64という値に依存する不具合の根拠はなく、1〜2 MiBの異なる正当値で設定・リサイズを行えば足りる。2本目は1 MiBですでに成功させている。
- 探索開始1回と初期化の重複、64 MiBの不要な確保を除ける。極大値のケース自体は外部入力の既知回帰なので削除しない。

P03では、CECPのmemory正常系とオーバーフロー回復を統合する。

- 対象は [src/protocol/cecp.rs:1620](/home/stepney141/board-games/minase/src/protocol/cecp.rs:1620) の `memory_is_accepted_while_idle_and_search_still_works` と1630行の `oversized_memory_is_rejected_and_the_session_continues` である。
- 条件付き統合とする。極大値のエラー、1〜2 MiBの正当な設定・リサイズ、最後に1回だけ探索して1手を返すことを同じ台本で確認する。
- 元の正常系は `memory 64; memory 1` の後に1回探索するだけなので、64 MiBの確保そのものには保証の価値がない。極大値を受けた後も生存する保証と、正当なmemoryを処理できる保証は統合先に残る。
- 探索開始を2回から1回へ、64 MiBの一時確保を小容量へ減らせる。

P04では、USIのinfo書式専用探索を既存の探索へ載せる。

- 対象は [src/protocol/usi.rs:2337](/home/stepney141/board-games/minase/src/protocol/usi.rs:2337) の `info_lines_follow_the_contract_token_order`、2274行の `multi_worker_search_places_info_immediately_before_one_bestmove`、2092行の `go_depth_returns_a_deterministic_legal_bestmove_without_applying_it` である。
- 条件付き統合とする。infoのトークン検証を、すでに探索する後2本の出力へ適用する。1ワーカーのdepth終了語と順序、複数ワーカーの最終info・stop・bestmoveの並びを保持し、2337行の専用 `go depth 2` を削除する。
- 書式破損と複数ワーカーでの重複bestmoveを見逃さない。1ワーカーと複数ワーカーの経路を片方だけに潰す提案ではない。
- 深さ2探索と256 MiBの置換表初期化を1回除ける。単なるテスト関数のパラメータ化ではなく、実際の探索回数が減る。

P05では、USIのThreads変更台本を通常のキュー順序テストへ統合する。

- 対象は [src/protocol/usi.rs:2254](/home/stepney141/board-games/minase/src/protocol/usi.rs:2254) の `threads_arriving_during_search_applies_before_the_next_search` と2229行の `commands_arriving_during_search_apply_after_bestmove` である。
- 条件付き統合とする。1回目の探索中に Threads変更、position、isready、stateを送り、stop後の出力順・局面更新を観測して2回目の探索まで進める。別々の1探索＋2探索を2探索へ減らす。
- 現在のThreads専用テストはエラーなしとbestmove 2件だけを検査しており、設定が完全に無視されても通る。現在持つ「設定コマンドを含む待機列が詰まらず次探索まで完走する」という保証は統合先で維持できる。
- ワーカー数が実際に2になる保証は現状にもない。その欠落を、本テストを残す理由にしない。統合では特定の並列探索結果値を追加で固定しない。
- 探索開始と既定256 MiB置換表の重複を1回減らせる。

P06では、CECPの「?」「pong」「cores」を1つの対話系列に統合する。

- 対象は [src/protocol/cecp.rs:1531](/home/stepney141/board-games/minase/src/protocol/cecp.rs:1531) の `move_now_plays_immediately_or_is_ignored`、1571行の `pong_echoes_after_prior_commands_complete`、1586行の `cores_arriving_during_search_applies_before_the_next_search` である。
- 条件付き統合とする。待機中の `?` と `ping 42`、長い探索の開始、探索中の `cores 2` / `ping 7`、`?`、次局の短い探索を1台本にする。待機時の無応答、数値反響、最初のmoveより後のpong、論理的なmove 2件、エラーなしを残す。
- 現在は合計4探索している。2探索で各コマンドの観測できる契約を満たせる。cores専用テストはUSIと同様、ワーカー数の適用そのものを検証していない。
- stopとquit/result/force/newの「結果を捨てる」ケースは意味が異なるので `stop_class_commands_discard_the_running_search` は残す。

P07では、USI時計引数の後手側スモーク探索を削除する。

- 対象は [src/protocol/usi.rs:2006](/home/stepney141/board-games/minase/src/protocol/usi.rs:2006) の `go_time_arguments_are_accepted_and_normalized_per_side`、特に2010〜2016行の後手側 `white` セッションとそのbestmove件数である。
- 後手側の追加セッションは無条件削除候補である。前後手で別の時計値になることは同テスト後半の `parse_go_config` の2色の厳密比較が担う。先手側のwire経路の受理・探索開始は残す。
- 現在の後手側セッションは `nodes 1` に到達して1手返ることしか見ず、btimeとwtimeを取り違えても通る。時計側の取り違えについて追加の検出価値がない。
- 1回の探索開始と256 MiBの置換表確保を除ける。後半の時計選択とplyの検証は、時間予算に直結するので残す。

P08では、CECP時計コマンドのスモーク探索を既存台本へ移す。

- 対象は [src/protocol/cecp.rs:1599](/home/stepney141/board-games/minase/src/protocol/cecp.rs:1599) の `time_commands_are_accepted_and_normalized_to_milliseconds` と1017行の `time_and_otim_normalize_centiseconds_to_milliseconds` である。
- 条件付き統合とする。前者の `time 6000; otim 6000` は後者の換算検証へ集約する。`level`・`st`・`sd`を受信した際の受理確認は残し、後続の `new; sd 1; go` は既存のCECP探索台本で行う。専用探索1回は不要である。
- 元のスモークは設定後にnewで設定を変え、sd 1で探索を打ち切るため、levelやstの正規化・適用が誤っていても通り得る。意味のあるミリ秒換算assertを残す方が保証は明確である。
- `parse_level_base_milliseconds` の分のみ・秒のみ・分秒混合、小数秒の1桁/2桁は独立した換算不具合を検出するので、その数値ケースは一括削除しない。

以下は、内部構造への依存を除く候補である。

P09では、USIのキャッシュ消去そのものを検査する1本を削除する。

- 対象は [src/protocol/usi.rs:1826](/home/stepney141/board-games/minase/src/protocol/usi.rs:1826) の `position_history_is_cleared_at_boundaries_and_on_rule_changes` 全体である。
- 無条件削除候補である。1834、1836、1854〜1855、1866〜1867行は privateな `accepted_position` / `position_synchronized` の値を断定する。正常なキャッシュ保持＋適用時判定でも同じ外部動作を実現できる。
- 対局境界で旧手順が再利用される不具合は `pending_rules_commit_with_or_without_usinewgame`（1581）、`ruleset_changes_latch_until_the_next_game`（1551）、`royal_capture_finishes_silently_and_state_reports_the_verdict`（2420）、`state_lifecycle_spans_finished_and_next_game_rules`（2585）等の後続コマンドで観測する。
- 対局中のRuleSet変更は次局用のpendingだけを変える契約であり、この時点でキャッシュを必ず消すこと自体は利用者の保証ではない。キャッシュ構造を変更するたびに修正が必要となる44行の保守負担を取り除ける。

P10では、増分適用判定のprivate helper分岐表は、外部挙動へ引き継いで削除する。

- 対象は [src/protocol/usi.rs:1673](/home/stepney141/board-games/minase/src/protocol/usi.rs:1673) の `position_extension_requires_ingame_matching_setup_and_move_prefix` 全体である。
- 条件付き削除とする。戻り値 `Some(2)` / `None` の8ケースは `AcceptedPosition` の形と `position_extension_start` の分岐条件を再掲しており、同じ挙動を別の判定方法で実現すると壊れる。
- ただし無条件削除は勧めない。現状このhelperテストだけが明瞭に分けている「長さは同じで既存の着手prefixが違う」「同じ手順の再送」「開始局面だけが異なる」を、`shorter_moves_and_different_setup_use_full_replay`（1795）と `incremental_position_matches_full_replay_for_ongoing_and_finished_games`（1737）へ外部入出力として引き継ぐ。
- 現在のdifferent_setupケースは開始局面を変えると同時に着手列も短くしている。これではsetup比較だけの破損が隠れるため、同数または長い手順で開始局面だけを変える比較が必要である。
- 現実的な見逃しは「異なる棋譜を前の棋譜の続きと誤認して盤面を破損する」ことであり、この条件付き統合なら残る。AwaitingStart/Finishedでの誤再利用は既存ライフサイクル台本で確認する。
- 差分適用の性能保証は別に残す。`docs/plans/strength-stage2.md:56,167` は2,860手で約65 msかかった既知の時間切れを根拠とし、2,000手以上の処理時間測定を要求する。外部出力同値だけで、全再生への性能退行を保証できるとは主張しない。

P11では、不正な差分positionのprivate同期assertを、実際のgo拒否へ統合する。

- 対象は [src/protocol/usi.rs:1871](/home/stepney141/board-games/minase/src/protocol/usi.rs:1871) の `invalid_incremental_position_preserves_state_and_clears_synchronization` と1920行の `failed_position_blocks_go_until_a_valid_position_recovers` である。
- 条件付き統合とする。後者の初期入力を `position startpos moves 6i6h`、失敗入力を `position startpos moves 6i6h 1a1b` にして、前者と同じ差分失敗経路で state保持、go拒否、正当なposition後の探索回復を観測する。1871行のテストは削除できる。
- `accepted_position.is_none()` と `!position_synchronized` は削除する。実際に古い局面で探索してしまう不具合は後続goで検出する。plyの原子性は [src/protocol/engine.rs:585](/home/stepney141/board-games/minase/src/protocol/engine.rs:585) の `extend_position_is_atomic_and_preserves_base_ply` と後発設計の手数検証を保持する。
- 同じ不正suffixは `position_applies_atomically_or_not_at_all`（1646）にも既にある。盤面とstatusの3重検証、private状態への依存を除ける。

P12では、同値なstate出力の直後にstatusをもう一度比較しない。

- 対象は [src/protocol/usi.rs:1737](/home/stepney141/board-games/minase/src/protocol/usi.rs:1737) の `incremental_position_matches_full_replay_for_ongoing_and_finished_games` 内の1768行と1791行である。
- `engine.status()` の2比較は無条件削除候補である。直前に終端の `state` 行を含む出力全体の一致を検査しており、ongoing/finishedと裁定理由も同じ文字列へ出ている。
- ただし `engine.ply()` はstateへ出ないため残す。`engine.game().position()` も成り保留・先獅子などstateの基本2欄に出ない情報を将来含むため、statusとの同列で丸ごと落とさない。
- 実行時間の効果は小さいが、同じ観測の重複を取り除ける。

P13では、USI statusの到達不能な防御分岐を固定しない。

- 対象は [src/protocol/usi.rs:2525](/home/stepney141/board-games/minase/src/protocol/usi.rs:2525) の `state_status_vocabulary_matches_the_contract`、2569〜2581行の Resignation / Agreement エラーassertである。
- 無条件削除候補である。コメント自身が現行USIに到達経路がないと明記している。private helperへ人工的に渡した到達不能な結果の処理変更は、利用者に生じる不具合ではない。
- 外部へ出る6種類の勝因と3種類の引分理由の文字列対応は残す。これらは実装の写経ではなく、`browser-gui` の公開status語彙である。
- 同じテストの `Ongoing` 直接検査（2565〜2568）も `state_line_matches_the_exact_contract`（2516）の完全一致で覆われるので削除できる。
- 6勝因×2色の直積は「全勝因を片方の色で＋反対色1件」へ縮約できる。色と理由の対応は独立であり、色ごとに別の語彙を使う仕様はない。黒のroyal-captureは2420行の実台本でも固定されている。

P14では、CECPのsmp宣言をdoneの直前に固定するassertを削除する。

- 対象は [src/protocol/cecp.rs:1088](/home/stepney141/board-games/minase/src/protocol/cecp.rs:1088) の `protover_declares_the_feature_set_with_done_last`、1116〜1120行の `smp + 1 == lines.len() - 1` である。
- 無条件削除候補である。[docs/protocols/cecp.md](/home/stepney141/board-games/minase/docs/protocols/cecp.md) はfeatureの行分割・順序を任意とする。[docs/plans/lazy-smp.md:187](/home/stepney141/board-games/minase/docs/plans/lazy-smp.md:187) は `feature smp=1` の宣言を要求するが、doneと隣接させる要求は書いていない。
- D6マトリクスの D6-CECP-01 が「smp=1の次がdone=1」を追加しているため、こちらは根拠文書より強くなった期待値として 規範文書に根拠がない期待 とする。テストを残して実装順を固定する理由にはしない。
- 同テストにあるsmp宣言の存在、全featureの集合、done=1が最後という保証はそのまま残る。将来featureを1つ間へ追加しただけで壊れる修正負担を取り除ける。

以下は、表記テストの重複を除く候補である。

N01では、USI・CECPの代表局面生成と空振り検査を1回にする。

- 対象は [src/notation/usi.rs:625](/home/stepney141/board-games/minase/src/notation/usi.rs:625) の `all_legal_moves_round_trip_in_representative_positions` と [src/notation/cecp.rs:486](/home/stepney141/board-games/minase/src/notation/cecp.rs:486) の `all_legal_moves_round_trip_via_concatenated_legs` である。
- 条件付き統合とする。ほぼ同一の8局面を2回構築し、同じMoveGeneratorで同じ合法手と出自別の存在検査を2回生成している。共通のテストで1回生成した `(position, moves)` から両表記の往復を行い、非空・経路捕獲・居喰い・成/不成の存在検査を1回にする。
- 2つの表記器は実装もwire契約も異なるため、USIまたはCECPの往復assertそのものはどちらも残す。共有してよいのは局面構築、合法手生成、fixtureが必要な類型を含むことの検査である。
- CECPではさらに、`legs` が駒種を参照せず正準じっとを一律 `@@@@` にするため、単独獅子・角鷹2色・飛鷲2色の5局面を削って初期局面・特殊捕獲局面・成り局面の3つに絞ることもできる。`legs_output_splits_two_stage_moves_and_writes_jitto_as_null_move`（336）がじっと出力を固定する。USIの方向依存のじっと代表升検査は残す。
- 独立の往復保証を落とさず、代表局面の大きなfixture定義、少なくとも1組分の合法手生成と再生成を除ける。単に同じケースをパラメータ化する提案ではない。

N02では、USI往復成立直後の再往復を削除する。

- 対象は [src/notation/usi.rs:625](/home/stepney141/board-games/minase/src/notation/usi.rs:625) の同テスト、679〜687行の非じっと向け `text_generated(pos, parse(pos, &rendered).unwrap()) == rendered` である。
- 無条件削除候補である。直前に `rendered = text_generated(pos, mv)` と `parse(pos, &rendered) == mv` が成立しており、同じ関数を同じ値に再適用しているだけである。
- parseと出力の整合が崩れる不具合は直前のMove往復で残る。通常手の出力が同一入力の2回目だけ変わるという仮定に、全合法手の追加parse/formatを費やす必要はない。じっとの決定性は別の専用テスト（503）で保持する。

N03では、表記往復テストから成り規則の再検証を削除する。

- 対象は [src/notation/usi.rs:690](/home/stepney141/board-games/minase/src/notation/usi.rs:690)、`all_legal_moves_round_trip_in_representative_positions` 内の `!(mv.promote && (mv.mid.is_some() || mv.to == mv.from))` である。
- 無条件削除候補である。このassertは表記の出力を一切見ず、生成器の成り規則を再検証している。
- [src/core/movegen/tests/promotion.rs:308](/home/stepney141/board-games/minase/src/core/movegen/tests/promotion.rs:308) の `article_18_6_7_two_stage_moves_never_carry_promotion`、83行の `article_17_1_king_lion_and_free_king_never_promote`、117行の `article_17_4_promoted_pieces_never_promote_again`、および [src/core/movegen/tests/lion_moves.rs:310](/home/stepney141/board-games/minase/src/core/movegen/tests/lion_moves.rs:310) の `article_11_10_falcon_and_eagle_moves_never_promote` が責務を担う。これらを削除する監査案と同時採用する場合は、成り規則側に代表検証を残す。
- 表記の往復保証はN01/N02の残存assertにあり、規則変更時に2領域を修正する負担を減らせる。

N04では、USIの不正入力リストを既存の構文テストへ吸収する。

- 対象は [src/notation/usi.rs:588](/home/stepney141/board-games/minase/src/notation/usi.rs:588) の `invalid_inputs_are_rejected_without_panicking` と276行の `move_strings_concatenate_square_names_without_separators`、310行の `two_square_form_maps_to_mid_none_and_rejects_zero_distance` である。
- 条件付き統合とする。588行の `6f6f`、`P*3d`、`13a1a`、`0a1a`、`7g7d7e7f` は既存と同一入力、`7m1a` も既存 `7m7d` と同じ盤外の始点である。これら6ケースは削除できる。
- 残る空文字、`resign`、`@@@@`、不明suffix `7g7d#` を既存の構文拒否リストへ移し、588行のテスト関数を削除する。単なる並べ替えではなく重複6ケースと同一fixtureの再作成が減る。
- 非ASCIIの既知パニック回帰 `non_ascii_move_text_is_rejected_as_a_parse_error`（265）は別に残す。

N05では、USI成り出力の追加生成ループを削除する。

- 対象は [src/notation/usi.rs:391](/home/stepney141/board-games/minase/src/notation/usi.rs:391) の `promotion_suffix_writes_plus_and_reads_equals_and_question_as_non_promotion`、420〜432行である。
- 無条件削除候補である。前半408〜409行ですでに無印と `+` の出力完全一致を確認し、代表局面往復テストも成/不成の両方を生成する。後半は同じ成り有無に対して `=` / `?` がないことを弱い包含検査で重ねている。
- suffixの取り違えは前半の固定オラクルで検出でき、fixtureの成/不成が実在する保証はN01の共有検査へ残る。合法手生成と全手の再文字列化、約13行を除ける。

N06では、USI正規化をもう一度示す末尾の同型例を削除する。

- 対象は [src/notation/usi.rs:731](/home/stepney141/board-games/minase/src/notation/usi.rs:731) 以降の `all_legal_moves_round_trip_in_representative_positions` 末尾の `7g7f7e -> 7g7e` である。
- 無条件削除候補である。`three_square_input_normalizes_by_intermediate_occupancy`（439）の空中間升の `6f5e6d -> 6f6d` と493〜497行の出力・再解析が同じ契約を検証する。
- 両例は空中間升の冗長経路をmidなしへ潰すもので、直線方向か斜め経路かに表記層の合法性判断はない。この入力の違いに追加の現実的な検出価値はない。
- 中間升が敵駒/自駒/空の3分岐と、往復が居喰い/じっとになる相違は439行のテストに残す。

N07では、CECP出力完全一致に含まれる部分文字列assertを削除する。

- 対象は [src/notation/cecp.rs:375](/home/stepney141/board-games/minase/src/notation/cecp.rs:375) の `promotion_suffix_attaches_to_the_final_leg_only`、411〜414行の `legs(plain).chain(legs(two_leg))` に `=` がないことのループである。
- 無条件削除候補である。前半の `legs(plain) == ["c4c5"]` と `legs(two_leg) == ["e6d6,", "d6c6+"]` が同じ入力の完全一致を検証している。
- `=`が出る不具合は前半ですでに失敗する。後半が独自に見つける現実的な不具合はない。

N08では、CECP正規化テスト末尾のUSI再解析を削除する。

- 対象は [src/notation/cecp.rs:431](/home/stepney141/board-games/minase/src/notation/cecp.rs:431) の `intermediate_normalization_matches_the_usi_contract`、470〜480行のUSI/CECP同値2比較である。
- 無条件削除候補である。前半がCECPの敵駒・空・自駒に対して固定Moveを期待し、[src/notation/usi.rs:439](/home/stepney141/board-games/minase/src/notation/usi.rs:439) が同じ正準化契約を固定Moveで検証する。2つの実装どうしを追加で比較しても、それぞれの独立オラクルより検出力は増えない。
- 片方だけ正規化が変わる不具合は各表記の固定Move期待で落ちる。座標系の相互対応は `cecp_squares_cover_all_144_squares_and_correspond_to_usi_names`（233）の固定 `7g7d == f6f9` を残す。

N09では、SFENのテスト用配列長を自己検査しない。

- 対象は [src/notation/sfen.rs:656](/home/stepney141/board-games/minase/src/notation/sfen.rs:656) の `all_21_base_piece_letters_parse_and_round_trip_in_both_cases` 内680行の `letters.len() == 21` と712行の `promoted_prefix_covers_18_kinds_and_unpromotable_pieces_reject_it` 内733行の `promoted.len() == 18` である。
- 2assertは無条件削除候補である。製品コードを呼ばず、テスト自身の固定配列の長さを数えている。
- 21種類/18種類の独立した文字割当は保持する。対応の1文字が誤る不具合は重要だが、配列長assertはその誤りを見つけない。配列の1要素をテスト側で誤削除する懸念を、製品の保証と混同しない。

N10では、SFEN成駒の出自区別を同じ入力で再検証しない。

- 対象は [src/notation/sfen.rs:712](/home/stepney141/board-games/minase/src/notation/sfen.rs:712) の `promoted_prefix_covers_18_kinds_and_unpromotable_pieces_reject_it` 内751〜758行の `+f11` / `b11` 再解析と2つの `assert_ne!` である。
- 無条件削除候補である。`+f` は直前の成駒36通りループ、`b` は656行の基底42通りループで同じ配置を検査し、各々のPieceCodeと出力文字列を完全一致させている。
- 成駒と生駒の取り違えはそこに残る。PieceCodeの値がそもそも区別されない不具合は [src/core/piece.rs:497](/home/stepney141/board-games/minase/src/core/piece.rs:497) の `piece_codes_are_injective_over_owner_kind_and_promotion` に残す。
- 2つの追加局面解析と再出力、重複8行を除ける。

N11では、SFEN初期局面を完全一致した後に玉だけ再確認しない。

- 対象は [src/notation/sfen.rs:870](/home/stepney141/board-games/minase/src/notation/sfen.rs:870) の `lishogi_initial_four_field_sfen_matches_the_initial_position` 内876〜886行の先後のKing 2個の `piece_at` assertである。
- 無条件削除候補である。直前に固定lishogi文字列の解析が `Position::initial()` と一致し、初期配置は [src/core/position.rs:1013](/home/stepney141/board-games/minase/src/core/position.rs:1013) の `article_5_initial_position_matches_the_full_144_square_table` が独立表で保証する。
- 王の位置・先後取り違えはこれら2本で検出する。文字列自体の固定期待とPosition全体の比較は削除しない。

N12では、SFEN初期形式の同じ受理・出力を重ねない。

- 対象は [src/notation/sfen.rs:1191](/home/stepney141/board-games/minase/src/notation/sfen.rs:1191) の `extended_parser_accepts_only_four_or_five_fields` 内1197〜1198行の初期SFEN受理assertと1217行の `extended_output_always_writes_five_fields_and_round_trips_all_state` 内1219〜1222行のplain初期局面出力検査である。
- どちらも無条件削除候補である。870行の `lishogi_initial_four_field_sfen_matches_the_initial_position` が同じ4欄入力の解析と `to_extended_sfen == 初期4欄 + " -"` の完全一致をすでに持つ。
- 4欄を受け付けない/5欄を書かない不具合は残存テストで検出する。1191行の4欄と5欄の同値および不正欄数、1217行の捕獲升・手数・保留升が同時に存在するrichケースは残す。

N13では、SFENのrichな出力完全一致後の再解析を削除する。

- 対象は [src/notation/sfen.rs:1217](/home/stepney141/board-games/minase/src/notation/sfen.rs:1217) の `extended_output_always_writes_five_fields_and_round_trips_all_state` 内1236〜1239行である。
- 無条件削除候補である。1234行で `rich = parse(rich_text)`、1235行で `to_extended_sfen(rich) == rich_text` を確認した直後、同じ `rich_text` をもう1回parseして `rich` と比較することになる。
- 最初の解析と出力の固定文字列オラクルが残れば、情報の欠落・書式違反は残存assertで検出できる。後半は純関数の再実行だけである。

N14では、SFENの巨大入力ケースと他テスト済み拒否を削る。

- 対象は [src/notation/sfen.rs:930](/home/stepney141/board-games/minase/src/notation/sfen.rs:930) の `malformed_inputs_are_rejected_without_panicking`、特に947行の `"1".repeat(65536)`、935行の手番zである。
- この2ケースは無条件削除候補である。巨大入力は段区切りを1つも含まないため、12段でないという既存の入力区分で終わる。945行の「12段を保った1段に30桁の9を置く」ケースは実際の桁あふれ検証なので残す。手番zは `side_to_move_letter_maps_b_to_black_and_w_to_white`（842）の不正手番xで覆われる。
- SFEN共通盤面パーサの全不正リストを基本形と拡張形の両方で回す949〜954行も縮約候補である。基本形に不正リストを集約し、拡張形では1件の不正盤面を統合検証として残す。ただし拡張固有の欄数・捕獲升・手数・保留欄の拒否はそれぞれ残す。
- UTF-8境界、連続/末尾の段区切り、桁あふれを同じ「不正」で一括削除しない。これらは異なる実際のパニック経路・入力境界を持つ。

N15では、SFEN先獅子テストの重複した通常駒と値の不等号を削る。

- 対象は [src/notation/sfen.rs:1008](/home/stepney141/board-games/minase/src/notation/sfen.rs:1008) の `lion_capture_square_preserves_kirin_promoted_lion_identity_for_l2`、1015行の配列中 `"5p6"` と1031行の `assert_ne!(pieces[0], pieces[1])` である。
- 2箇所は無条件削除候補である。通常の相手歩兵と捕獲升の受理は961行の `lion_capture_field_accepts_dash_empty_and_non_mover_squares_only` と全く同じ盤面・手番・捕獲升で検証済みである。値の区別は直前の2つの厳密PieceCode期待とPieceCode単射性検査で覆われる。
- さらに、1008行の `+o` / `n` の受理2ケースを961行の非手番側駒の受理へ引き継ぐなら、1008行のテスト全体を統合できる。成麒麟と生獅子を同じ駒として扱う不具合は、SFEN文字表の出自保存検査に加えてこの受理2ケースを残す。
- このテストは実際のL2裁定を行わない。L2の有効性を理由に同じ盤面解析を重ねない。

N16では、SFEN保留升の単独正常系をP1/P2/P5テストへ集約する。

- 対象は [src/notation/sfen.rs:1079](/home/stepney141/board-games/minase/src/notation/sfen.rs:1079) の `deferred_field_is_a_strictly_ordered_comma_list_that_round_trips` 内1090〜1093行の単一8c受理・保存・再出力である。
- 無条件削除候補である。1126行の `deferred_field_requires_p1_p2_or_p5_and_round_trips_for_each` がP1、P2、P5それぞれの単独8c受理・保存・再出力を検査する。
- 削除後も単独欄のパース漏れを検出でき、1079行には2升の順序・重複・空要素・不正升名の検証が残る。P1/P2/P5は独立した規則スイッチなので3つの正常ケースは削除しない。

以下は、追加の重複と過剰に分割された台本を整理する候補である。

P15では、Threads/coresの不正値をhelperでもう一度検査しない。

- 対象は [src/protocol/usi.rs:1417](/home/stepney141/board-games/minase/src/protocol/usi.rs:1417) の `threads_accepts_boundaries_and_rejects_invalid_values` 内1443〜1445行と、[src/protocol/cecp.rs:1134](/home/stepney141/board-games/minase/src/protocol/cecp.rs:1134) の `cores_accepts_boundaries_and_rejects_invalid_values` 内1145〜1147行である。
- 0、257、nopeに対するprivate helperの6assertは無条件削除候補である。直前のwire台本が同じ入力の拒否を厳密なエラー行で固定する。
- 有効値1/256の `get()` assertも標準整数パーサとNonZero変換の再確認に近い。ただしwire台本は受理した値自体を見ないため、「受理値の取り違え」を残す必要があるなら、受信後に探索開始引数へ渡る値を確認する1箇所へ移した条件付き削除とする。
- 費用の中心は実行時間ではなくprivate関数への依存である。境界の1、0、256、257をwire側から減らす提案ではない。

P16では、stop理由の既にwireで固定する3語を削除する。

- 対象は [src/protocol/usi.rs:2374](/home/stepney141/board-games/minase/src/protocol/usi.rs:2374) の `stop_reasons_have_distinct_protocol_words` 内2375、2376、2379行のdepth/nodes/externalである。
- 無条件削除候補である。depthは `info_lines_follow_the_contract_token_order`（2337、P04統合後はその引継ぎ先）、nodesは `go_depth_256_at_the_upper_bound_is_accepted`（2078）、externalは `go_infinite_withholds_bestmove_until_stop`（2161）で出力行を固定する。
- soft/hardは同じ実行経路のwire検証がなく、時間対局で厳密に引くと不安定になる。2語の軽い直接対応検査は残す。5語を丸ごと実装コピーとして捨てない。

P17では、CECPのresult書き出しを表と台本の両方で重ねない。

- 対象は [src/protocol/cecp.rs:1769](/home/stepney141/board-games/minase/src/protocol/cecp.rs:1769) の `result_reason_table_matches_the_documented_mapping`、1794〜1822行の `write_result` による先後勝ちと引分3行の生成である。
- 無条件削除候補である。1235行の `result_lines_take_the_white_viewpoint_and_appear_once` が実セッションで1-0、0-1、1/2-1/2と波括弧を完全一致で検査し、1769行の前半の理由表がcheckmate/bare kingsを含む全語彙を検査する。
- 勝者の反転、結果コード誤り、理由文字列の誤りはこれらに残る。「Mate理由だけ結果コードが違う」といった仕様に存在しない相互依存を仮定して直積を増やす必要はない。
- 前半の外部仕様から引いた理由表は残す。内部helperだからという理由だけで削除しない。

P18では、Engineの終局遷移とライフサイクル周回の重複を統合する。

- 対象は [src/protocol/engine.rs:531](/home/stepney141/board-games/minase/src/protocol/engine.rs:531) の `newly_finished_marks_only_the_finishing_transition` と783行の `lifecycle_cycles_through_start_game_finish_and_back` である。
- 条件付き統合とする。AwaitingStartのApplyMove拒否、SetPosition成功、王捕獲、FinishedでのApplyMove/SetPosition拒否、EndGame後の再構成、NewGameでの復帰を、newly_finishedの厳密な期待を保って同じライフサイクル列へまとめる。
- 後者の開始・王捕獲・EndGame・SetPositionの部分は前者と同じ状態遷移を弱いmatches!で再現している。FinishedでのSetPosition拒否とNewGame復帰は独自なので引き継ぐ。
- `extend_position_reports_the_finishing_transition_once`（638）は別の公開コマンド境界を検証するため、同じ王捕獲fixtureという理由だけで削除しない。
- 重複するEngineと棋譜fixtureの構築、および同じ遷移への期待修正を減らせる。

P19では、Engineの拒否理由の列挙を、既存の状態保持シナリオへ吸収する。

- 対象は [src/protocol/engine.rs:718](/home/stepney141/board-games/minase/src/protocol/engine.rs:718) の `each_reject_reason_is_idempotent_and_state_preserving` である。
- 条件付き統合とする。GameNotStartedとGameAlreadyOverの拒否は531/783行のライフサイクル列へ、InvalidRulesのpending保持は669行の `set_rules_validates_on_receipt_and_keeps_pending_on_failure` へ集約する。
- 718行のうち「ApplyMoveが不正手を拒否した後、元の局面の合法手probeを受理する」は独立に価値がある。これを残存シナリオに移してから元のテストを削除する。SetPosition/ExtendPositionの原子性だけでApplyMoveの原子性を代替しない。
- 各エラーを同じ入力でもう一度送り、同じエラーであることだけを確認するassertは減らせる。同じエラーを再返却しても盤面を壊す実装は通るため、拒否後の合法手が通る観測の方を残す。

P20では、USI次局規則の同じ成功系列を集約する。

- 対象は [src/protocol/usi.rs:1581](/home/stepney141/board-games/minase/src/protocol/usi.rs:1581) の `pending_rules_commit_with_or_without_usinewgame`、1551行の `ruleset_changes_latch_until_the_next_game`、1479行の `ruleset_values_are_case_insensitive_and_duplicates_are_rejected` である。
- 1581行のテスト全体は削除候補である。usinewgameありでpendingが反映される経路は1479行、usinewgameなしでgameover後のpendingが反映される経路は1551行にすでにあり、いずれも固定期待RuleSetを確認する。
- 残存2本の最終状態を同じ規則期待へそろえれば「2経路の同値」も個別の固定期待から従う。比較のためだけに2つの新しいセッションを作る必要はない。
- usinewgameあり/なしのどちらかのcommitが壊れる不具合は対応する残存台本で検出する。1つの経路しか残さない提案ではない。

P21では、CECPの不正手番入力を色対応テストでもう一度試さない。

- 対象は [src/protocol/cecp.rs:1194](/home/stepney141/board-games/minase/src/protocol/cecp.rs:1194) の `cecp_white_matches_the_usi_first_player` 内1226〜1230行の `setboard INITIAL_BOARD x` である。
- 無条件削除候補である。1643行の `setboard_takes_two_fields_ignores_extras_and_fails_atomically` が同じ不正手番xを実セッションで拒否し、拒否後の局面不変まで検査する。
- 色反転そのものの前後手2ケースは残す。CECPとUSIは先手のwire表現が逆なので、表記層一般の色テストで省略できない。

P22では、CECPの表記エラー全分類をwireで再列挙しない。

- 対象は [src/protocol/cecp.rs:1266](/home/stepney141/board-games/minase/src/protocol/cecp.rs:1266) の `usermove_legs_must_be_continuous` 内1283〜1290行の3レグ拒否と1301行の `promotion_suffixes_are_interpreted_on_the_final_leg_only` 内1324〜1331行の非最終レグ+拒否である。
- これら2wireケースは削除候補である。[src/notation/cecp.rs:278](/home/stepney141/board-games/minase/src/notation/cecp.rs:278) の `comma_separated_legs_parse_with_continuity` が3レグ拒否を、375行の `promotion_suffix_attaches_to_the_final_leg_only` が非最終+拒否を固定する。
- wireの表記エラーから `Illegal move: 元入力` への変換は1266行の不連続レグを残して確認する。2段捕獲・居喰いが実際に受理されること、成り後に金の横移動が可能になること、成れない手の+拒否は別の意味なので残す。
- 削除によって各エラーの拒否や入力反響が消えることはない。失うのは、同じエラールーティングを別のパーサエラーで再通過させる冗長な2セッションである。

次の検証には固有の検出力があり、保持する。

SFENの全21基底文字と18成駒の出自対応は、独立した外部表の検証である。単に数が多い、match表に似ているという理由では削除しない。全144升の座標ループも仕様に変換式があり、ライブラリ保証ではない。座標軸の反転と1桁/2桁境界を短く固定しており、実行費用は小さい。

SFENの手数0/1/9999/10000、先頭+の既知回帰、保留欄の重複/順序/空要素、P1/P2/P5の独立スイッチ、空の先獅子捕獲升と自軍占有拒否は残す。`setup_position_constructor_rejects_values_the_parser_cannot_produce`（sfen.rs:1058）も公開コンストラクタが別の入力境界であり、パーサ経由だけでは未検証なので残す。

USIの未検証Moveを公開textが拒否する回帰、UTF-8入力、じっとの方向と候補一意の升、CECPの@@@@受理と表記層拒否の責務差は残す。USIとCECPの同じ棋譜であっても異なるパーサ・通信方式を横断する部分は、共通生成fixture以外まで消さない。

探索中のgameover/quitとCECPのforce/result/new/quitは停止後の動作が違う。これらは実在のスレッド所有・結果破棄契約であり、単なるコマンド名の過剰ケースとして削除しない。`go_infinite_withholds_bestmove_until_stop` の5秒タイムアウトは待機中にbestmoveを出さないことを順序で観測するためにあり、他の即時stop台本だけへ置換しない。

EngineのSetPosition、ExtendPosition、ApplyMoveは別の公開コマンドである。それぞれの失敗後の状態保持と終局遷移を、1つの成功例や規則生成器のテストで代替しない。増分positionによる超長手数での時間切れ修正は、同値性検査と性能測定の両方を根拠に保持する。

全件を確認したファイルと件数を以下に示す。

| ファイル | テスト数 | 本文を確認した範囲 |
|---|---:|---|
| [src/notation/sfen.rs](/home/stepney141/board-games/minase/src/notation/sfen.rs) | 17 | 609〜1241行のテストモジュール全体。 |
| [src/notation/usi.rs](/home/stepney141/board-games/minase/src/notation/usi.rs) | 10 | 230〜738行のテストモジュール全体。 |
| [src/notation/cecp.rs](/home/stepney141/board-games/minase/src/notation/cecp.rs) | 7 | 196〜572行のテストモジュール全体。 |
| [src/notation/mod.rs](/home/stepney141/board-games/minase/src/notation/mod.rs) | 0 | 全5行。 |
| [src/protocol/engine.rs](/home/stepney141/board-games/minase/src/protocol/engine.rs) | 7 | 396〜867行のテストモジュール全体。 |
| [src/protocol/usi.rs](/home/stepney141/board-games/minase/src/protocol/usi.rs) | 47 | 1282〜2636行のテストモジュール全体。 |
| [src/protocol/cecp.rs](/home/stepney141/board-games/minase/src/protocol/cecp.rs) | 32 | 945〜1824行のテストモジュール全体。 |
| [src/protocol/mod.rs](/home/stepney141/board-games/minase/src/protocol/mod.rs) | 0 | 全25行。 |
| 合計 | 120 | 全件の本文を確認した。 |

他領域の残存テスト名は参照先を実際に読んで確認したが、そのファイル全体の監査件数には含めていない。実装変異の実行、削除後のテスト実行、各候補の壁時計時間計測は行っていない。したがって速度の改善幅や削除後の生存変異数は未確認である。
