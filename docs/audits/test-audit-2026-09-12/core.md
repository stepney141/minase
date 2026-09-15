対象は `src/core/**/*.rs` の249テストと [tests/lishogi_replay.rs](../../../tests/lishogi_replay.rs) の1テスト、計250テストである。全テスト本文と補助関数を確認した。本番コードとテストは変更していない。実行結果は[全体の報告](../test-audit-2026-09-12.md)に記載した。行番号は監査時点のものである。時間削減の数値は計算量・呼び出し回数であり、実測時間ではない。

根拠は RULES.md 第14版、[docs/plans/spec-first-tests/matrices/d1-movement-promotion.md](../../plans/spec-first-tests/matrices/d1-movement-promotion.md)、[d2-lion-local-rules.md](../../plans/spec-first-tests/matrices/d2-lion-local-rules.md)、[d3-game-adjudication.md](../../plans/spec-first-tests/matrices/d3-game-adjudication.md)、[d4-position-foundations.md](../../plans/spec-first-tests/matrices/d4-position-foundations.md) と各領域台帳、[rules-type.md](../../plans/rules-type.md)、[game-referee.md](../../plans/game-referee.md)、[adjudication-refactor.md](../../plans/adjudication-refactor.md)、[move-canonicalization.md](../../plans/move-canonicalization.md) である。マトリクスの行数や過去のテスト件数は保持理由にしない。「削除」は現在の他テストを残せば実行でき、「統合」は記載した観測の移植を先に要する。

以下は、重複や実装依存が大きく、優先して整理できる候補である。

C01では、同じ回転対称性を3層で反復している。

削除対象は [src/core/attacks/tables.rs:209](../../../src/core/attacks/tables.rs:209) `reachable_sets_are_180_degree_rotation_symmetric_across_colors` と [src/core/attacks/fixed.rs:468](../../../src/core/attacks/fixed.rs:468) `white_relative_moves_are_180_degree_rotations_of_black`である。

保持先は [src/core/movegen/tests/properties.rs:47](../../../src/core/movegen/tests/properties.rs:47) `article_9_black_and_white_move_sets_are_180_degree_symmetric`である。こちらは全駒種・全144升の先後対称を、成否・成り選択・経由升を含む `Move` 集合で検査する。tables側は同じ公開生成器から最終到達升だけを抽出しており、保持先の真部分集合である。fixed側の相対方向・変位変換の故障も、利用される変換なら公開生成の対称性で検出される。利用されない変位だけの故障は対局に影響しない。

失う現実的検出力はない。tables側だけで29種×144升×2色＝8,352回の単駒局面構築と手生成を削れ、相対方向型への依存も減る。方向・駒ごとの動きの正解は [pieces.rs](../../../src/core/movegen/tests/pieces.rs) の第9・10条テストを残すので、先後とも同じ誤りになるケースを対称性だけに委ねない。

C02では、動きのプロファイルの共有方法を固定している。

削除対象は [src/core/attacks/fixed.rs:501](../../../src/core/attacks/fixed.rs:501) `movement_profiles_are_shared_only_by_kinds_with_identical_moves`である。

王・太子が同一プロファイルIDであること、他の全駒種対の内部データが相違すること、プロファイル数を固定する。第9・10条が要求するのは到達可能な着手であり、同じ動作を別プロファイルへ分けることも、複数データを合成して表すことも禁止しない。データが異なっていても動きが誤っていれば、このテストは通る。

保持先は [pieces.rs](../../../src/core/movegen/tests/pieces.rs) の全28種の独立期待到達集合、[lion_moves.rs](../../../src/core/movegen/tests/lion_moves.rs) の獅子・角鷹・飛鷲、C01の保持先。駒種を誤った動きへ割り当てる現実的バグはそちらが検出する。全29×29対の内部比較と意味を変えない設計変更時の修正負担を除ける。

C03では、「乱数集合」3個が既存集合と完全に同一である。

削除対象は [src/core/bitboard.rs:291](../../../src/core/bitboard.rs:291) `sample_sets` のLCGによる3集合である。影響するテストは `bitboard_acts_as_the_set_of_its_squares`（:309）、`bitwise_operators_match_elementwise_set_operations`（:347）、`iteration_yields_each_contained_square_exactly_once_in_order`（:385）。

乗数と加数がともに奇数なので、`state & 1` は毎回反転する。初期値は奇数、1集合は偶数の144回更新だから、3集合とも `dense_index() % 2 == 0` の集合になる。これは:287で既に追加されている。失う入力も検出力もない。標本数は13から10に減り、集合演算の直積比較は169組から100組になる。新しい乱数発生器へ置換する必要はなく、重複を削るだけでよい。

C04では、同一実装・同一着手列の再実行を毎手繰り返している。

[src/core/position.rs:2103](../../../src/core/position.rs:2103) `seeded_random_playouts_uphold_conservation_replay_and_undo_invariants` の:2166〜2171「毎手、初期局面から全着手を再適用」と:2193〜2216「同じseedの対局をもう1回実行」。後者だけに使う `recorded_move_lists` と記録処理も削る。

毎手の再生は同じ `make_move_unchecked` を同じ初期局面・同じ入力列へ呼び直している。決定的な駒の消失・成り誤り・キー更新漏れは両経路で同じように再現される。1局72手なら再適用だけで2,628回、8局なら最大21,024回の余分な着手適用となる。最後の同seed再走査も最大576回の生成・適用を追加する。

保持するのは各手のキー再計算との一致、`validate`、駒数保存、手番、全undo、捕獲・成り・二段移動の出現確認。これらと固定シナリオ `incrementally_maintained_state_matches_full_recomputation`（:1987）・`unmake_restores_every_observable_component`（:2006）が状態破壊を検出する。削除で失うのは同一入力が外部可変状態に左右される非決定性の専用検査だけであり、現在の純粋な局面変換の契約に対して2次の再生費用は割に合わない。なお増分値と独立再計算の比較は自己検証ではなく、有用なので残す。

C05では、ランダム対局の終局期限と反復回数が仕様の保証を超える。

[src/core/game.rs:2458](../../../src/core/game.rs:2458) `deterministic_random_default_rules_self_play_reaches_a_terminal_result` を:2246 `representative_rule_sets_random_self_play_reaches_valid_terminations` に既定規則1局として吸収し、既定4局の残り3局を削る。:2386 `deterministic_random_r2_self_play_never_forbids_captures_and_terminates` はR2拒否を観測する別目的があるため残すが、同じ規則セットごとの3局を代表1局へ削減する候補とする。

:2448付近と:2480付近の「必ず1,500手以内に終局」のassertは削る。中将棋の規則はランダム方策がこの上限までに終局することを保証しない。手生成順の正当な変更で同seedの対局が変わり、期限だけが破れる。残すのは進行中には合法手があること、受理すべき手の拒否がないこと、R2が捕獲・成りを拒否しないこと、発生した終局理由が規則と一致すること。

複数seedの多様性を減らすので無条件の同値削除ではない。削る局だけに現れる長い履歴の相互作用を見逃す可能性はあるが、その種類が特定されておらず、上限だけを要求する現行の追加10局は価値が低い。固定規則例と外部棋譜再生は保持する。R2の反復拒否を実際に踏む代表局を選ぶことを条件とし、到達しない場合はランダム局数を戻すのでなく短い既存反復列で保証する。

同じ:2246の末尾:2377〜2382の `Finished || ply_count == PLY_CAP` は**無条件削除**できる。ループは終局以外でbreakせず、拒否はpanicなので、これはテスト自身の制御フローを再確認しているだけである。

C06では、18規則組合せ×4場面の直積が独立挙動を大量に再テストする。

[src/core/game.rs:2082](../../../src/core/game.rs:2082) `plan_referee_7_all_rule_combinations_report_the_firing_adjudication` の全体を削る前に、E3と詰みが同時成立した場合にBareKingを優先する1場面を:2200 `plan_referee_7_piece_exhaustion_evaluates_before_mate` に移す。

このテストは18組合せそれぞれで王捕獲、詰み、駒枯れ、金の往復を実行し、期待側にも `rules.exhaustion`・`rules.e1`・反復規則の分岐を書く。反復履歴がない王捕獲をR1/R2/R3で3度ずつ試しても独立した保証は増えない。

保持先は `article_27_4_r2_r3_reject_before_acceptance_while_r1_adjudicates_after`、`article_21_10_r2_mate_judgement_uses_the_filtered_move_set`、`article_23_3_r2_filtered_out_moves_cause_a_stalemate_loss`、`article_32_e1_*`、`article_32_e2_*`、`article_32_e3_*` とC05の代表規則対局。個別規則の有効/無効、R2と詰み・合法手なしの非自明な相互作用は残る。

減る検出力は選ばれなかった組合せだけに故意・偶発的に追加された相互干渉の検出でありゼロとは言わない。しかし競合条件のない直積72場面を維持するより、文書化された相互作用と実対局を持つ方が保守負担に見合う。D3マトリクスは従来「全18組合せ」としたが、ユーザーの今回の価値判断に従い、行数維持は根拠にしない。

C07では、Gameの手順をテスト側にコピーして同じ裁定関数と比較している。

削除対象は [src/core/adjudication.rs:738](../../../src/core/adjudication.rs:738) `plan_adjudication_shared_functions_match_game_play`である。

低層側で着手適用、反復記録、待機升導出、`adjudicate_after_move` をGameと同じ順に呼び、そこで得た結果をGameの期待値にしている。裁定自体が両経路で誤れば通る。名称は探索との境界を挙げるが、このテストはSearcherを呼ばず、探索が共有関数を呼ばなくなる不具合も検出しない。

固定4場面の正しい結果は [game.rs](../../../src/core/game.rs) の `article_21_2_mate_ends_the_game_at_move_completion`、`article_26_12_no_moves_are_accepted_after_the_game_ends`、`article_22_8_bare_royals_draw_but_prince_pairs_continue`、`article_27_4_*` 等で残る。共有APIの配線変更でGame側だけが壊れれば、それらの公開結果が失敗する。失うのはこの4場面での低層とGameの状態同値観測であり、探索との境界の保証ではない。探索側の有用な裁定テストを別途残すことを前提に、約90行の処理順コピーを撤去できる。

以下は、同じ保証を持つテストを削除・統合する候補である。

C08では、初期配置の対称性は全144升の正解表に含まれる。

削除対象は [src/core/position.rs:1159](../../../src/core/position.rs:1159) `article_5_initial_position_is_symmetric_under_180_degree_rotation`である。

保持先は [article_5_initial_position_matches_the_full_144_square_table](../../../src/core/position.rs:1013) である。盤上の全駒種・所有者・成否・空升を正解表と照合しているため、配置を非対称にする誤りは必ずそちらで失敗する。失うのは正解表自体を非対称へ書き換えた際のテストデータ検算であり、現在の製品の不具合検出ではない。対称性専用ヘルパ `sigma` も除ける。

C09では、初期配置の内訳を別表で再定義している。

[src/core/position.rs:1037](../../../src/core/position.rs:1037) `article_4_2_initial_piece_counts_match_on_every_enumeration_path` を:1013の全144升テストへ統合する。全144升の期待表を1回読む際に所有者別・駒種別の期待集合を作り、`occupied`、`pieces_of`、`pieces_of_kind` とそれぞれ1回比較する。:1060からの21種類の別内訳表、総数を各列挙法で何度も足し直すassert、成駒専用8種の別ループは不要になる。

単なるテスト数圧縮ではない。全144升表を唯一の期待値とし、二重の表とその保守を削る。補助ビットボードだけが壊れて盤面配列は正しい不具合を見逃さないよう、別経路の観測そのものは削らない。

C10では、移動・捕獲の個別テストが、別のテストの観測に含まれる。

次の7テストは**削除**できる。いずれも単純な代表局面が減るだけで、対応する不変条件や固定局面は保持先で残る。

| 削除対象 | 保持先と削除後の検出力 |
|---|---|
| [src/core/movegen/tests/movement.rs:59](../../../src/core/movegen/tests/movement.rs:59) `article_6_1_initial_position_alternates_turns_starting_with_black` | [properties.rs:386](../../../src/core/movegen/tests/properties.rs:386) `mc_canonical_move_invariants_hold_in_seeded_playouts` が全生成手の所有者、:315以降が交替を検査し、[position.rs:1182](../../../src/core/position.rs:1182) が先手開始と2手交替を検査する。相手駒の手を生成するバグ、手番反転漏れは残る。 |
| [movement.rs:85](../../../src/core/movegen/tests/movement.rs:85) `article_6_2_each_move_moves_exactly_one_own_piece` | [properties.rs:315](../../../src/core/movegen/tests/properties.rs:315) `assert_application_effects` が同じ駒数・変更可能升の条件に、着手後の駒の正体と手番も加えて検査する。固定2枚取りは [lion_moves.rs:384](../../../src/core/movegen/tests/lion_moves.rs:384) に残る。ヘルパ `assert_single_mover` 約40行も消せる。 |
| [movement.rs:107](../../../src/core/movegen/tests/movement.rs:107) `article_6_4_lion_jitto_passes_turn_without_board_change` | [lion_moves.rs:490](../../../src/core/movegen/tests/lion_moves.rs:490) `article_12_9_lion_jitto` が同じ生成・盤面不変・手番交替を確認する。C12を実施する場合は統合先へ残す。 |
| [movement.rs:127](../../../src/core/movegen/tests/movement.rs:127) `article_6_5_no_pass_move_exists_for_ordinary_pieces` | [properties.rs:271](../../../src/core/movegen/tests/properties.rs:271) の正準形不変条件がすべての生成手でto=fromの駒種を検査し、裸獅子のじっと存在も別に残る。金と奔王の1局面を追加しても別の保証を持たない。 |
| [movement.rs:171](../../../src/core/movegen/tests/movement.rs:171) `article_7_3_capture_removes_the_enemy_piece_permanently` | [position.rs:1237](../../../src/core/position.rs:1237) `article_4_4_captured_pieces_are_removed_and_never_reused` が公開の検査付き着手から駒数・相手消滅・着手先・直接構築との一致を検査する。 |
| [movement.rs:265](../../../src/core/movegen/tests/movement.rs:265) `article_7_8_second_stage_is_judged_after_first_capture` | [lion_moves.rs:384](../../../src/core/movegen/tests/lion_moves.rs:384) `article_12_4_lion_path_capture_double_move` と同じ獅子6,6・白歩6,5・白銅5,4の局面で、同じ第2段階8升集合と2枚取りを検査している。保持先には最終駒種・不正な経由升の検査もある。完全な重複である。 |
| [movement.rs:298](../../../src/core/movegen/tests/movement.rs:298) `article_7_9_no_move_captures_the_same_piece_twice` | [properties.rs:245](../../../src/core/movegen/tests/properties.rs:245) の全生成手不変条件がmid≠fromとmid≠toを既に検査する。複数隣接駒の居喰いは [lion_moves.rs:453](../../../src/core/movegen/tests/lion_moves.rs:453) に残る。 |

C11では、角鷹の同一2枚取り局面を作り直している。

[src/core/movegen/tests/lion_moves.rs:113](../../../src/core/movegen/tests/lion_moves.rs:113) `article_11_3_two_stage_move_captures_up_to_two_pieces` を:41 `article_11_1_falcon_has_four_forward_actions` に統合する。

同じ角鷹6,6・白歩6,5・白銅6,4の局面と同じ2枚取り手を使っている。生成された2枚取りを同じフィクスチャに適用し、白駒が消え着手先に角鷹がいることを残す。適用効果を単に削ると捕獲取りこぼしを見逃し得るため、assertを移植する。重複する局面構築と独立テストを1つ除ける。

C12では、裸獅子の同じ生成結果を4テストで分割している。

[lion_moves.rs:333](../../../src/core/movegen/tests/lion_moves.rs:333) `article_12_1_3_lion_single_steps`、:346 `article_12_5_7_lion_jumps_two_squares_including_knight_positions`、:490 `article_12_9_lion_jitto`、:545 `article_12_lone_lion_generates_25_canonical_moves` を1テストへ統合する。

同じ6,6の裸獅子について、条文から独立に組み立てた「1升8手＋2升16手＋じっと」の完全Move集合を1回照合し、じっと適用後の盤面と交替だけ追加確認する。現状の各contains、25件、24到達手、midなし、じっと1件はこの集合等価から導けるので削る。

:346後半の中間歩を飛び越す局面は:424 `article_12_4_7_jump_capture_and_path_capture_are_distinct_moves` と同一構成・同一の非捕獲効果なので削る。単純なパラメータ化ではなく、同一盤面生成4回と重なった期待値を1回の完全照合へ変える。漏れた移動、余分な移動、重複手、誤った経由升、じっとの状態破壊を引き続き検出する。完全集合にする際はVec長も1回比較し、HashSet化で重複を隠さない。

C13では、角鷹・飛鷲の小さな局面とassertを重複させている。

- [lion_moves.rs:218](../../../src/core/movegen/tests/lion_moves.rs:218) `article_11_7_falcon_jitto_requires_empty_forward_square` の:228〜235「前が敵歩」の局面。:177 `article_11_5_falcon_igui_captures_and_returns` が同じ局面でじっと不在と居喰い存在・一意性・適用まで検査する。自駒で塞がる場合と盤端は残す。
- [lion_moves.rs:279](../../../src/core/movegen/tests/lion_moves.rs:279) `article_11_9_second_stage_uses_board_after_first_capture` の:297〜306「敵第2駒へ置換した対照」。:41 `article_11_1_falcon_has_four_forward_actions` の2枚取り生成が同じ肯定側を検出する。自駒が残る第2段階の拒否は残す。
- [pieces.rs:335](../../../src/core/movegen/tests/pieces.rs:335) `article_10_7_horned_falcon_moves` と:351 `article_10_8_soaring_eagle_moves` の `jitto_moves(...).len()==1` は:202 `article_11_6_jitto_is_generated_exactly_once` と同一局面・同一観測なので削る。完全到達集合の検査は残す。

これらを削って見逃す製品不具合はない。独立した分岐である自駒・敵駒・盤外の差自体は保持する。

C14では、成り任意の弱い再検査が既存の正確な選択肢検査に含まれる。

削除対象は [src/core/movegen/tests/promotion.rs:281](../../../src/core/movegen/tests/promotion.rs:281) `article_18_5_promotion_is_always_optional`である。

最奥段歩の同一フィクスチャは:331 `article_19_1_pawn_reaching_last_rank_may_promote` が成り・不成各1手と成り適用を確認する。銀の入陣で「成りがあれば不成もある」は:176 `article_18_1_entry_into_the_zone_offers_promotion_choice` の正確な(1,1)確認に含まれ、横断性質にもある。強制成りを誤って導入する不具合は残る。2局面の重複を除ける。

C15では、成り禁止の個別フィクスチャを同じ遷移系列へ吸収する。

[promotion.rs:271](../../../src/core/movegen/tests/promotion.rs:271) `article_18_4_quiet_exit_from_the_zone_cannot_promote` は:239 `article_18_2c_reentry_offers_promotion_again` に統合する。後者で実行している銀6,4→7,5の退出直前に `assert_no_promotion` を追加する。現在はuncheckedで指しているため、assertを移さず削ることはできない。これで「退出では成れず、再入で成れる」の連続した観測になり、専用局面が1つ減る。

削除対象は [promotion.rs:261](../../../src/core/movegen/tests/promotion.rs:261) `article_18_3_quiet_move_inside_the_zone_cannot_promote`である。:200 `article_18_2a_capture_inside_the_zone_offers_promotion` が同じ猛豹の同じ敵陣内で捕獲と非捕獲の選択肢を対照する。違いは非捕獲が前進か前斜めかだけで、成り条件は方向を参照しない。方向依存の誤った成り条件だけは追加の1点で検出できるが、方向自体のテストと捕獲/非捕獲の対照を残す価値に比べて低い。

C16では、成駒専用8種のテストの前半は初期配置検査と完全重複する。

[pieces.rs:368](../../../src/core/movegen/tests/pieces.rs:368) `article_10_promoted_only_pieces_appear_only_by_promotion` の初期92駒が未成である:370〜376のループ。[position.rs:1013](../../../src/core/position.rs:1013) の全144升表が同じis_promoted観測を既に持つ。

後半の成駒8種の「入陣しても成り手なし」は、再成の一般則と重なるが、公開生成で成駒専用種を固定しており残してよい。削るなら [promotion.rs:117](../../../src/core/movegen/tests/promotion.rs:117) の再成テストへ移し、仲人由来の醉象・歩兵由来の金との境界と一緒に守る必要がある。単にパラメータ化するだけでは削減と数えない。

C17では、先獅子の発生・消滅を同一局面から別々に開始している。

[src/core/rules.rs:1462](../../../src/core/rules.rs:1462) `article_15_1_senjishi_blocks_the_immediate_capture_of_the_footed_lion` を:1496 `articles_15_4_and_15_5_senjishi_lasts_only_for_the_very_next_move` へ統合する。同じF9(true)へ同じ飛車9a×9fを適用し同じ直後禁止を確認している。禁止と無関係な飛車6i→6dが生成されるという:1473のassertだけ、後者の禁止確認直後へ移す。

禁止が全手へ波及するバグ、禁止の発生漏れ、禁止が2手以上残るバグはいずれも残る。重複したF9構築と着手を1回省ける。

C18では、獅子が獅子を取った後の取り返しを重複確認する。

削除対象は [rules.rs:1519](../../../src/core/rules.rs:1519) `article_15_6_a_lion_capturing_a_lion_does_not_trigger_senjishi`である。

:1336 `article_14_1_an_adjacent_lion_can_be_captured_unconditionally` が隣接・足ありの獅子捕獲を実行し、直後に足の金で取り返せることまで検査する。両者は先後と升配置を変えた同じ規則であり、C01の先後対称性も残る。先獅子を誤発動させる現実的バグは保持先で検出する。

C19では、空経由升という説明は独立した正準Moveを持たない。

削除対象は [rules.rs:1676](../../../src/core/rules.rs:1676) `article_16_1_passing_an_empty_mid_square_is_not_tsukegui`である。

空経由升は正準化によりmidなしの跳びになり、このテストは単に距離2の足あり捕獲禁止と足なし許可を確認している。保持先は:1153 `article_13_1_a_footed_lion_cannot_be_captured_by_a_lion_at_distance_two` と:1363 `article_14_3_an_unfooted_lion_at_distance_two_can_be_captured`。両者の向きを変えた局面を増やしても、別の「空の経由捕獲」分岐は存在せず、失う検出力はない。

C20では、非直線の付け喰いを別テストがより強い局面で検査する。

削除対象は [rules.rs:1721](../../../src/core/rules.rs:1721) `article_16_11_a_mid_square_off_the_straight_line_still_counts_as_between`である。

:1742 `articles_16_4_and_16_5_tsukegui_stands_even_if_a_sliding_foot_opens` は同じ獅子6h・6f、経由銀5gに角3iを追加し、非直線の付け喰いを生成・適用して取り返しまで検査する。削除側は経由銀が唯一の足なので、足を消してしまう実装でも合法となり得て、付け喰い判定の検査として弱い。角の足が残る保持先の方が非直線付け喰いの欠落を検出しやすい。唯一の足が銀である場合だけに特殊処理を導入する不具合は検出点が減るが、その処理に仕様上の根拠はない。

C21では、L2の最小例は対象限定のテストに含まれる。

削除対象は [rules.rs:1955](../../../src/core/rules.rs:1955) `article_29_l2_allows_immediate_capture_of_the_new_promoted_lion`である。

:2003 `article_29_l2_exempts_only_the_new_promoted_lion` が同じ麒麟成りによる新獅子への取り返しを許可し、別の既存獅子への保護は残ることも確認する。L0+L2の例外が消える不具合は保持先で失敗する。もう一方のL1+L2は:1972が独立した規則の組合せを検査するので残す。

C22では、等しい設定で同じ規則シナリオを再実行している。

[rules.rs:1896](../../../src/core/rules.rs:1896) `articles_29_l0_and_33_1_explicit_l0_is_identical_to_the_standard_rules` の `assert_eq!(l0, base())` より後のF9足あり・足なし対局。設定値の同一性が確認できれば、同じ入力の `MoveGenerator` を2度動かす必要はない。挙動はC17の保持先と:1482 `article_15_2_without_a_foot_the_lion_can_be_recaptured_immediately` に残る。

削除対象は [rules.rs:2269](../../../src/core/rules.rs:2269) `article_33_6_lishogi_move_rules_reproduce_the_capture_exception`である。L1+L2の同一F11(true)は:1972、プリセットの具体コードは:2243、プリセット展開は:2211で固定される。無関係なP3を含む設定でも同じL2例を繰り返すだけである。Lishogi固有の連携は外部棋譜再生を残す。

C23では、L4テスト内の標準規則の対照が同一F17で重なる。

[rules.rs:2084](../../../src/core/rules.rs:2084) `article_29_l4_limits_senjishi_to_non_lion_recaptures` の:2096〜2103 `adjacent_standard` 部分。

:1589 `articles_15_1_and_14_1_senjishi_blocks_even_an_adjacent_lion_recapture` と完全に同じF17・同じ飛車捕獲・同じ禁止手。L4での許可と非獅子禁止の2観測は残し、標準規則の禁止を失わない。

C24では、単純な王捕獲勝ちのテストを終局後入力テストへ統合済みである。

削除対象は [src/core/game.rs:661](../../../src/core/game.rs:661) `article_21_1_capturing_the_last_royal_wins`である。

:1470 `article_26_12_no_moves_are_accepted_after_the_game_ends` の後半が同一の黒王0,0・黒飛5,5・白王5,8から同じ捕獲を指し、同じFinished結果と `game.result()` を確認し、その後の入力拒否も検査する。削除によって王捕獲勝ちや結果保存の検出力は失われない。

C25では、R2/R3の拒否列を2回実行している。

[game.rs:1511](../../../src/core/game.rs:1511) `article_27_4_r2_r3_reject_before_acceptance_while_r1_adjudicates_after` と:1564 `article_27_1_rejected_moves_leave_the_game_state_unchanged`。

後者の状態保存ヘルパを前者のR2・R3の拒否直前/直後へ適用し、型付きのRepetitionエラー確認も残す。後者で同じking_cycleを再演しているR2とR3の2ブロックを削る。後者の通常移動エラーは反復と異なる検証経路なので残す。

これによりR2の3手、R3の11手の準備を1回ずつ減らせる。受理前拒否・出現回数境界・状態不変・別手での継続という独立した保証はすべて残る。名前を1つにすること自体ではなく、同じ履歴の二重作成を除く候補である。

C26では、増分整合とundoが同じ9シナリオを2回組み立てる。

[src/core/position.rs:1987](../../../src/core/position.rs:1987) `incrementally_maintained_state_matches_full_recomputation` と:2006 `unmake_restores_every_observable_component`。

undoテストの着手適用直後へ前者のキー再計算・権利キー再計算・`validate` のassertを移し、前者を削る。`try_make_move_with_undo` で実際に適用した同じ局面の前進整合と復元整合を1回の往復で検査できる。9シナリオの構築・前進・合法手生成の重複を省け、更新漏れとundo漏れは両方残る。

C27では、初期局面や同じ設定からの構築を細分しすぎている。

- [game.rs:607](../../../src/core/game.rs:607) `plan_referee_from_position_defers_adjudication_to_the_first_move` の末尾「parse_sfenで構築した局面」(:647〜655)。SFEN由来のPositionだけ異なるGame経路を持たず、王欠落・駒枯れという2つの本来の構築時非裁定例が既にある。SFEN解釈はnotation、規則保持は実際のE系・R系の挙動で検査する。`rules()==rules`だけの受け渡し確認も不要である。
- [game.rs:1413](../../../src/core/game.rs:1413) `article_25_2_repetition_rule_is_mandatory_and_exclusive` の「R1/R2/R3からGameを構築しOngoing」3ケースと最後の既定Game再構築。各R規則は:1511の実対局で受理・効果ともに固定され、`Game::from_position` は検査済みRulesを値として受ける。必須・競合エラーのassertは残す。
- [game.rs:2041](../../../src/core/game.rs:2041) `article_33_9_e2_and_e3_conflict_while_e1_composes` のE1+E2/E1+E3構築だけの2ケース。前者は:1790のE1継続、後者は代表規則対局やLishogi再生で実際に動く。E2/E3競合拒否は固有なので残す。

削除で失うのは、既存の実対局でも必ず通るコンストラクタの浅い再検査である。規則の排他・文字列境界そのものは型だけで保証されるわけではないので、入力検証テストは削らない。

以下は、assertと入力ケースを削減する候補である。

C28では、完全集合との比較後に要素数・不在要素を再確認している。

[src/core/movegen/tests/pieces.rs:37](../../../src/core/movegen/tests/pieces.rs:37) `assert_destinations` の:45 `destinations.len()==count` と:46〜48 `excluded` のループ。全28駒の以下のテストが対象で、:44の完全集合比較を残す。

`article_9_king_steps_one_square_in_eight_directions`, `article_9_gold_general_steps`, `article_9_silver_general_steps`, `article_9_copper_general_steps`, `article_9_ferocious_leopard_steps`, `article_9_blind_tiger_steps`, `article_9_drunk_elephant_steps`, `article_9_pawn_steps_forward_only`, `article_9_go_between_steps`, `article_9_lance_slides_forward`, `article_9_reverse_chariot_slides`, `article_9_side_mover_moves`, `article_9_vertical_mover_moves`, `article_9_bishop_slides_diagonally`, `article_9_rook_slides_orthogonally`, `article_9_dragon_horse_moves`, `article_9_dragon_king_moves`, `article_9_kirin_steps_and_jumps`, `article_9_phoenix_steps_and_jumps`, `article_9_free_king_slides_in_eight_directions`, `article_10_1_white_horse_slides`, `article_10_2_whale_slides`, `article_10_3_flying_stag_moves`, `article_10_4_free_boar_slides`, `article_10_5_flying_ox_slides`, `article_10_6_crown_prince_steps`, `article_10_7_horned_falcon_moves`, `article_10_8_soaring_eagle_moves`。

この集合比較は到達升の漏れも余分な升も検出する。件数・除外升を別入力で維持するのは正解表の二重記述であり、製品の不具合検出力は増やさない。28呼び出しから余分な引数を除ける。[movement.rs:154](../../../src/core/movegen/tests/movement.rs:154) の自駒升不在assert、:193の完全集合比較後の3つの不在assertも同じ理由で削れる。

C29では、Position全体の等価比較後にキーを再比較している。

[src/core/position.rs](../../../src/core/position.rs) の次のassert。Positionは:120で `PartialEq, Eq` を導出し、盤面・集合・手番・先獅子・zobrist・promotion_deferred・rights_zobristを全て含むため、全体等価の直後のキー等価は厳密に重複する。

| 対象テスト | 削る行 |
|---|---|
| `article_4_4_captured_pieces_are_removed_and_never_reused` | :1296。 |
| `article_24_1_a_key_distinguishes_placement_kind_owner_and_promotion` | :1370。なお末尾の構築順序比較自体も:1807の構築経路テストへ集約する候補である。 |
| `article_24_1_c_key_distinguishes_the_prohibition_target_square` | :1508。 |
| `article_30_p2_waiting_right_survives_the_reply_and_expires_on_the_next_own_move` | :1636。 |
| `article_24_3_key_depends_on_the_position_not_on_the_path` | :1735、:1749。 |
| `equal_configurations_share_one_key_regardless_of_construction_path` | :1848。 |
| `unmake_restores_every_observable_component` | :2018〜2019。 |
| `d4_imp_10_null_move_clears_lion_trigger_and_round_trips` | :2068〜2069。 |

キーを全体等価から外す将来設計へ変える場合には契約を再検討するが、現在この重複を残してRustの導出等価を再確認する必要はない。

C30では、列挙の網羅検査後の四隅・型で決まる配列長を再確認する。

- [src/core/square.rs:139](../../../src/core/square.rs:139) `article_4_1_board_enumerates_all_144_squares_exactly_once` の:162〜169の四隅ループ。有効全144組を:155〜160ですでに構築している。盤外・最大値拒否は残す。
- [src/core/piece.rs:391](../../../src/core/piece.rs:391) `article_4_3_piece_kinds_are_21_initial_plus_8_promoted_only` の:392 `PieceKind::ALL.len()==PIECE_KIND_COUNT`。配列の型で長さが固定される。種の構成・重複・全体集合の検査は残す。
- [src/core/direction.rs:111](../../../src/core/direction.rs:111) `eight_directions_form_consistent_unit_displacements` の:112 `Direction::ALL.len()==DIRECTION_COUNT` と:130付近の `seen.len()==DIRECTION_COUNT`。前者は配列型、後者は同じ配列の各要素を必ずpushするテスト側の制御で決まる。方向の単射性と逆方向の実際の変換は残す。

C31では、step_squareを関数本体と同じ式に対して照合する。

[direction.rs:111](../../../src/core/direction.rs:111) の:132〜140、全144升×8方向の `step_square(square,direction) == square.offset(direction.file_delta(), direction.rank_delta())`。

これは薄いラッパーの実装式を期待値に書いている。盤端とoffsetの正解は [square.rs:222](../../../src/core/square.rs:222) の座標数学との比較、走りの停止は `attacks/tables.rs:229` の歩行照合、駒の正しい方向は [pieces.rs](../../../src/core/movegen/tests/pieces.rs) が残す。ラッパーだけの誤配線もこれらが通る限り実際の手生成を壊せない。1,152回の同式照合を削れ、ラッパーの実装変更へテストが追従する必要が減る。

C32では、生成した全手を同じ駒生成器へ再投入している。

[src/core/movegen/tests/properties.rs:409](../../../src/core/movegen/tests/properties.rs:409) 付近の `moves.iter().all(|mv| generator.is_legal_move(position,mv))`。対象テストは:434 `capture_generation_matches_full_generation_in_seeded_playouts`。

全手生成と `is_legal_move` は同じ `generate_piece_moves::<false>` を呼ぶため、駒別の誤った合法手を二重に生成しても通る。`is_legal_move` の候補起点選択を間違える回帰だけは検出するが、Gameの全合法手から指すC05の対局、相手駒入力拒否、成り・特殊手の実受理でも観測できる。

毎局面で全駒の全候補を各起点から再生成する費用を削れる。一方、`generate_captures` と全手の捕獲部分列の同値は別のCAPTURES_ONLY経路を検査し、取りこぼしを防ぐため残す。

C33では、逆引き利きの標本検査と手書き境界の全域照合が重なる。

[src/core/movegen/tests/attackers.rs:115](../../../src/core/movegen/tests/attackers.rs:115) `attackers_to_respects_removed_pieces_and_opens_xrays` にある `assert_attackers_match_piece_controls` の3回（:137、:147、:163）。

ここは遮蔽物1枚除去・2枚除去・獅子自身除去を手書きの期待で照合するテストである。その境界の明示assertはすべて残す。各状態で全144目標×全駒を既存実装と照合する一般性は:86 `attackers_to_matches_piece_control_on_reference_positions` が標本群と仮想占有3種で持っている。削るとこの特定3盤面の無関係な目標だけが欠けるが、主要なX線と除去駒の再出現は確実に残る。

C34では、捕獲専用生成で全144起点×極端盤面を繰り返す。

[src/core/movegen/tests/properties.rs:140](../../../src/core/movegen/tests/properties.rs:140) `capture_generation_matches_full_generation_for_every_kind_and_square`。全39駒コード×144起点×空/全敵駒盤の最大11,232局面を、駒コードは維持し、起点を四隅、中央、入陣直前/直後、最奥直前など9代表升へ減らす。

起点依存の添字・番兵エラーはsquare・bitboardの全数テストとC01の全升Move対称性が別に残り、捕獲の実際の集合差は:181の特殊・成り捕獲と:434の5規則×128手の実局面系列で残る。特殊移動や成り境界を含む全駒コードは削らない。各駒×全144升の飽和敵盤を作ることは、到達可能局面の多様性を増やしていない。

これは同値削除ではなく、特定の未選択升にだけ生じるCAPTURES_ONLY固有バグの検出が減る。実測で軽微ならC01〜C04より優先度は下がるが、全数基盤テストとの保証の重複に対し局面構築負担が大きい場合の削減先として挙げる。

C35では、規則エラー・文字列の小さな重複を除く。

- [src/core/rules.rs:2211](../../../src/core/rules.rs:2211) `article_33_5_and_33_6_presets_expand_from_rule_constants` の各3表記は通常表記と大小混在の2表記へ減らせる。大小混在は大文字を含み、大小を無視する契約の検査には十分である。小文字・大文字・混在の3区分を別々に処理する仕様はない。
- 同テストの `lishogi,R1` を単にis_errで見るassertは:2228 `rule_set_parse_errors_preserve_the_failure_kind` が同じ入力を正確な `PresetMustBeAlone` で確認するので削る。`engine-default,lishogi` の2preset混在は別の入力分類なので残す。
- [rules.rs:2144](../../../src/core/rules.rs:2144) `article_33_4_from_codes_reports_duplicate_conflict_and_missing_in_contract_order` の `from_codes(&[R1]) -> Missing(Lion)` は、`[]` と `[L4,P0,R1,E0]` の同群欠落検査に比べ独立の保証がない。L4を基底群と誤認する境界、重複優先、衝突優先は残す。
- [rules.rs:2249](../../../src/core/rules.rs:2249) `rules_error_display_is_stable` は4群の全文固定を1代表例へ減らす候補。エラー種別・群は型付き比較で既に固定される。[rules-type.md](../../plans/rules-type.md) は群名の表示を要求するため全削除より、1代表文面と必要な群の含有確認への縮小が適切である。未定義の句読点などへ広げない。

C36では、外部棋譜再生の件数とプリセット自己照合を外す。

[tests/lishogi_replay.rs:16](../../../tests/lishogi_replay.rs:16) `lishogi_replays_match_legal_moves_and_adjudication` の:40 `replay_count==10` は、空fixture防止の `replay_count > 0` へ縮小する。10という数自体に規則上の意味はなく、価値ある棋譜の差し替え・追加を不要に失敗させる。

`replay_game` の:55〜58 `parse_rule_set("lishogi")` の結果を `Rules::LISHOGI` のコード列と毎局比較する部分も削る。[rules.rs:2211](../../../src/core/rules.rs:2211) のプリセット展開が同じ比較を行っている。再生は `Rules::LISHOGI` をそのまま使うか、文字列から1回だけ解析する。

10棋譜そのものと各手の受理・早期終局禁止・記録された終局理由の照合は残す。独立した外部記録なので、同じ結果を返す内部コピーテストとは保証の質が違う。重いことだけを理由に外部棋譜を削る根拠は今回見つからなかった。

C37では、借用で不変な裁定状態を比較する。

[src/core/adjudication.rs:641](../../../src/core/adjudication.rs:641) `article_21_3_mate_requires_every_escape_clause_to_fail` の `state_before = state.clone()` と:666の `assert_eq!(state,state_before)`。

`AdjudicationContext::new` は `&AdjudicationState` を受け、状態は内部可変性を持たない。呼び出し先がこれを書き換えられないのはRustの共有借用の保証である。`&mut Position` は実際に仮想着手で変更されるので、:665のPosition復元比較は残す。また王先取り後に反撃の利きが残る境界は変異検証で補強された有用な例であり削らない。

C38では、null moveの3局面のうち通常盤面の観測を状態付き局面へ集約する。

[position.rs:2027](../../../src/core/position.rs:2027) `d4_imp_10_null_move_flips_side_preserves_board_and_round_trips` の盤面不変・明示手番反転・キー変化のassertを:2054 `d4_imp_10_null_move_clears_lion_trigger_and_round_trips` と:2075 `d4_imp_10_null_move_preserves_promotion_rights` のどちらかへ移す。その上で:2027の通常4駒局面を削る。

先獅子ありと成り権ありの2局面は意味の異なる状態なので残す。各テストは既にnull→undoを実行しており、盤面・手番の共通保証をそこで1回観測できる。通常局面だけで異なる動作をするという仕様上の理由はなく、独立した3回目の往復より明示的な状態差を残す価値が高い。

C39では、P3・P4の成り適用の正解を再び確認する。

[promotion.rs:607](../../../src/core/movegen/tests/promotion.rs:607) `article_30_p3_lance_gains_last_rank_relief` の成り適用と白駒のassert（:617〜624）、:637 `article_30_p4_go_between_gains_last_rank_relief` の成り適用と醉象のassert（:648〜654）。

P3/P4は成れる着手集合を増やす規則であり、成り先は第9条の固定対応である。[promotion.rs:63](../../../src/core/movegen/tests/promotion.rs:63) が全18対について生成された成り手を適用し成り先を検査する。P3/P4側には成り・不成の正確な2選択と通常規則との差、最奥段以外での制限を残す。失うのはP3/P4だけを理由に成り先を変更する独自バグの専用検査であり、その処理の仕様はない。

以下の保証と仕様上の留意点は、候補の採用時にも保持する。

- [pieces.rs](../../../src/core/movegen/tests/pieces.rs) の各駒の独立期待到達集合は、コード内の移動表のコピーではなく第9・10条の仕様表から導く参照であり保持する。王・太子の動きが同じでも、駒種から別プロファイルへの誤配線を検出する公開観測には価値がある。
- `attacks/tables.rs` の `sliding_control_matches_a_stepwise_walk_with_blockers_and_limits` は、最初の遮蔽物を含んで停止する独立逐次歩行とテーブル方式を比較している。単なる実装コピーではない。0/1/2、盤幅付近、無制限の区別を不用意に削らない。
- 獅子の足・裏足・王足・非再帰・歩/仲人だけの特例・付け喰い優先・L1/L2/L3/L4の違い、P1/P2の待機単位、P5/P6の最奥段・捕獲・組合せは実際の誤合法/誤不合法を検出する。見かけが似ていても意味の違う境界として残す。
- [game.rs](../../../src/core/game.rs) の駒枯れの猶予、待機中の成り/不成・歩/仲人/香車、王と太子の喪失順、連続攻撃の片側/両側/据え置き、R2と詰み・合法手なしの相互作用は残す。独立する条件を単純な表駆動へ変えるだけでは削減候補と数えない。
- `Position` のキーが配置・種類・所有者・成否・手番・一時状態を区別する観測は、反復と探索の誤同一視を防ぐ。公開結果だけで再現しにくい故障であり、ハッシュの具体定数を固定しない現行方式は価値がある。C29の重複assert以外は原則保持する。
- `article_15_1_each_remaining_footed_lion_is_protected_independently` の複数獅子への保護、`articles_15_1_and_14_1_senjishi_blocks_even_an_adjacent_lion_recapture` の居喰い形の足判定はD2で解釈固定の注記がある。解釈の不確定さを「不要」の理由にせず、現行の意図した挙動として保持する。
- D1マトリクスのP2の古い不確定記述は第13版追補で解消している。P5の追補には「捕獲の有無を問わず最奥段で強制」と残る一方、現行RULES.mdとテストは標準下の非捕獲では成れずP2等による機会が必要という扱いである。古い記述だけを根拠にP5境界テストを削らない。
- `piece_codes_are_injective_over_owner_kind_and_promotion` は単なるRust enum保証ではなく自前の符号化の検査なので残す。なお成駒専用種を `PieceCode::new` の失敗でcontinueするため、名前ほどの全成駒コードの網羅ではない。これは削除根拠ではなく、既存台帳の「94コード」という記述を信頼して全数性を主張しないための注記である。

調査済みファイルとテスト数は次のとおりである。

| ファイル | #[test]数 |
|---|---:|
| src/core/adjudication.rs | 3 |
| src/core/attacks/fixed.rs | 2 |
| src/core/attacks/mod.rs | 0 |
| src/core/attacks/sliding.rs | 0 |
| src/core/attacks/tables.rs | 2 |
| src/core/bitboard.rs | 3 |
| src/core/direction.rs | 1 |
| src/core/game.rs | 54 |
| src/core/mod.rs | 0 |
| src/core/movegen/mod.rs | 0 |
| src/core/movegen/tests/attackers.rs | 2 |
| src/core/movegen/tests/lion_moves.rs | 19 |
| src/core/movegen/tests/mod.rs | 0 |
| src/core/movegen/tests/movement.rs | 12 |
| src/core/movegen/tests/pieces.rs | 30 |
| src/core/movegen/tests/promotion.rs | 37 |
| src/core/movegen/tests/properties.rs | 6 |
| src/core/mv.rs | 0 |
| src/core/piece.rs | 4 |
| src/core/position.rs | 22 |
| src/core/repetition.rs | 1 |
| src/core/rules.rs | 48 |
| src/core/square.rs | 3 |
| tests/lishogi_replay.rs | 1 |
| 合計 | 250 |
