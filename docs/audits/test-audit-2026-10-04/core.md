対象は、合法手生成（`src/core/movegen`、`src/core/attacks`、`src/core/board`、`src/core/piece`）の148テスト、対局進行・局面・規則セット・SFEN（`src/core/game`、`src/core/position`、`src/core/rules`、[src/notation/sfen.rs](../../../src/notation/sfen.rs)、[tests/lishogi_import.rs](../../../tests/lishogi_import.rs)、[tests/lishogi_replay.rs](../../../tests/lishogi_replay.rs)）の122テスト、直前局面の生成（`src/core/predecessor`、[tests/predecessor_search.rs](../../../tests/predecessor_search.rs)）の58テストで、計328テストである。全テスト本文と補助関数、対応する実装を確認した。本番コードとテストは変更していない。実行結果は[全体の報告](../test-audit-2026-10-04.md)に記載した。行番号は監査時点（master 9027662）のものである。

根拠は RULES.md 第15版、[d1-movement-promotion.md](../../plans/spec-first-tests/matrices/d1-movement-promotion.md)、[d2-lion-local-rules.md](../../plans/spec-first-tests/matrices/d2-lion-local-rules.md)、[d3-game-adjudication.md](../../plans/spec-first-tests/matrices/d3-game-adjudication.md)、[d4-position-foundations.md](../../plans/spec-first-tests/matrices/d4-position-foundations.md) と各領域台帳、[predecessor-generator.md](../../plans/predecessor-generator.md)、[movegen-speedup-2.md](../../plans/movegen-speedup-2.md)、[debugging-tools.md](../../plans/debugging-tools.md)、および前回の[テスト監査](../test-audit-2026-09-12.md)である。前回の監査が保持と決めたものは、その後に状況が変わった場合を除いて再提案しない。規則に関するテストは、各ケースを RULES.md の条文と規則コードへ対応付け、同じ条文を同じ規則コードで検査する2ケースだけを過剰と判断した。

この領域の実行時間は合計で約6秒であり、最長は直前局面の生成の1.68秒である。したがって候補の根拠は実行時間ではなく、重複、自己参照、および期待値の保守負担である。

判断の前提が2つある。1つ目は `invariants` フィーチャである。[Cargo.toml](../../../Cargo.toml) の自己dev-dependencyにより、`cargo test` ではこのフィーチャが常に有効になる。有効な間は、着手（[make_move.rs:219](../../../src/core/position/make_move.rs:219)）、取消し、ヌルムーブとその取消しの直後に `validate()` が走り、2種のZobrist値が再計算と一致することまで確かめる。したがって、これらの操作の直後に置いた明示的な `zobrist == recompute` や `validate() == Ok` は何も検出しない。ただし `set_lion_capture` には検査フックがないので、その直後の `validate()` は対象外である。2つ目は、直前局面の生成の補助関数 `checked()`（[predecessor/tests/mod.rs:33](../../../src/core/predecessor/tests/mod.rs:33)）である。これは返された局面ごとに、対象局面への合法な辺があること（:49）と、設計書「直前局面の定義」の集合Aを製品の入力検査とは独立に満たすこと（`assert_admissible`、:165-245）を検査する。このため `checked()` の結果に対する否定形の `!contains` や `all(..)` は、それ自体では何も追加しない。

以下は、重複や自己参照が大きく、優先して整理できる候補である。

C01では、付け喰いと獅子捕獲の同じ条文を、駒や局面を変えて重ねて検査している。

削除対象は [src/core/movegen/tests/lion_capture.rs:282](../../../src/core/movegen/tests/lion_capture.rs:282) `article_3_11_lance_in_the_mid_square_is_a_valuable_piece`、[:296](../../../src/core/movegen/tests/lion_capture.rs:296) `article_3_14_igui_returns_the_lion_and_leaves_no_recapture_target`（使用箇所がこのテストだけの fixture `f10()` も削除）、[:528](../../../src/core/movegen/tests/lion_capture.rs:528) `article_14_3_an_unfooted_lion_at_distance_two_can_be_captured`、[:544](../../../src/core/movegen/tests/lion_capture.rs:544) `article_14_5_a_non_lion_captures_a_lion_regardless_of_feet` である。一部削除の対象は、[:726](../../../src/core/movegen/tests/lion_capture.rs:726) の746-753行（先獅子の第3手以降の失効）、[:813](../../../src/core/movegen/tests/lion_capture.rs:813) の832-837行（跳びのassert）、[:1018](../../../src/core/movegen/tests/lion_capture.rs:1018) の1029-1041行（標準規則の対照）、[:1045](../../../src/core/movegen/tests/lion_capture.rs:1045) の1054-1061行（`l1_footless`）、[:1098](../../../src/core/movegen/tests/lion_capture.rs:1098) の1117-1133行（歩兵の対と銀経由）である。

価値ある駒の判定は `!matches!(Pawn | GoBetween)` という否定形で、香車だけを扱う分岐がない。同じ局面F12で経由駒を銀にした [:792](../../../src/core/movegen/tests/lion_capture.rs:792) `articles_16_1_and_16_4_tsukegui_captures_a_footed_lion` が同じ経路を通る（第16条1・4項、L0）。居喰いの停止形と居喰い形の生成は [:501](../../../src/core/movegen/tests/lion_capture.rs:501) が、居喰いの適用は [lion_moves.rs:368](../../../src/core/movegen/tests/lion_moves.rs:368) `article_12_8_lion_igui` が確認している。足のない獅子を距離2から取れること（第14条3項）は、:375、:413、:457、:827、:833、:1037 の境界ケースで観測される。非獅子が足のある獅子を取れること（第14条5項）は、足が残った局面で非獅子が距離2から獅子を取る [:684](../../../src/core/movegen/tests/lion_capture.rs:684) の不成の麒麟が同じ分岐で検出する。先獅子の失効は `lion_taken_by_non_lion()` の1値で持ち、取り返す駒種に依存しないので、[:660](../../../src/core/movegen/tests/lion_capture.rs:660) が担う。L1の分岐は `moving_kind != Lion` だけで足の判定を呼ばず（lion_capture.rs:47）、L1の禁止は [:1003](../../../src/core/movegen/tests/lion_capture.rs:1003) が足のないF9aで確認している。L3は `if rules.l3 { false }` という1つの分岐だけで駒種の分岐がないため、仲人の対と `opened_slider` を残せば足りる。標準規則で歩兵と仲人を分ける根拠（典拠J2は仲人だけを挙げる）はL3には当てはまらない。

失う現実的な検出力はない。前回の監査が解釈を固定した :726 の前半（禁止と居喰いの2観測）は残す。

推奨: 適用する。

C02では、隠れた走り駒の足（第13条2項）を、より強い第13条4項のテストが包含している。

削除対象は [src/core/movegen/tests/lion_capture.rs:334](../../../src/core/movegen/tests/lion_capture.rs:334) `article_13_2_a_slider_blocked_by_the_lion_itself_is_a_hidden_foot` である。

自獅子の背後にある走り駒は、着手前の盤面でも獅子の升を利きに含む。このため、着手前の占有で足を判定する誤実装でもこのテストは通る。保持先の [:390](../../../src/core/movegen/tests/lion_capture.rs:390)（第13条4項）はその誤実装で失敗し、線外の対照（:404-417）も同じ形で持つ。条文は別なので過剰なケース分けではなく、検出できる不具合が包含される重複である。

失う現実的な検出力はない。削除に伴い、[ledgers/d2.md](../../plans/spec-first-tests/ledgers/d2.md) の19行目（D2-013-02・D2-014-04）の担い手を :390 へ書き換える。

推奨: 適用する。

C03では、L0の明示指定が標準規則と同じ設定値になることを、規則セット側のテストが既に検査している。

削除対象は [src/core/movegen/tests/lion_capture.rs:995](../../../src/core/movegen/tests/lion_capture.rs:995) `articles_29_l0_and_33_1_explicit_l0_is_identical_to_the_standard_rules` である。

このテストは `from_codes(L0,P0,R1,E0).moves == ENGINE_DEFAULT.moves` だけを確かめる。挙動の比較部分は前回の監査（C22）で削除済みであり、残っているのは設定値の等価だけである。保持先の [src/core/rules/tests.rs:100](../../../src/core/rules/tests.rs:100) `article_33_5_and_33_6_presets_expand_from_rule_constants` が、同じコード列が `Rules` 全体として `ENGINE_DEFAULT` と一致することを確認する。

失う現実的な検出力はない。

推奨: 適用する。

C04では、成れない駒に成りの選択がないことを、2つのガードを固定するテストとは別に駒種ごとに重ねている。

削除対象は [src/core/movegen/tests/promotion.rs:263](../../../src/core/movegen/tests/promotion.rs:263) `article_18_6_7_two_stage_moves_never_carry_promotion`、[src/core/movegen/tests/lion_moves.rs:276](../../../src/core/movegen/tests/lion_moves.rs:276) `article_11_10_falcon_and_eagle_moves_never_promote`、[src/core/movegen/tests/pieces.rs:293](../../../src/core/movegen/tests/pieces.rs:293) `article_10_promoted_only_pieces_cannot_promote_again` である。

2段階移動も `push_with_promotion` から `promotion_choice_for` を通る（expand.rs:28）。成駒は `is_promoted`、獅子と成駒専用種は `!can_promote()` のガードで、成りの選択が `NoPromotion` に限られる（promotion.rs:79）。前者は [promotion.rs:117](../../../src/core/movegen/tests/promotion.rs:117) `article_17_4_promoted_pieces_never_promote_again` が、後者は [src/core/piece/tests.rs:116](../../../src/core/piece/tests.rs:116)（`can_promote` が表に載る駒種と一致する）が固定している。獅子の入陣で成りがないことは [promotion.rs:83](../../../src/core/movegen/tests/promotion.rs:83) が、経由升のある手は成らないという不変条件は [properties.rs:287](../../../src/core/movegen/tests/properties.rs:287) が確認する。

失う現実的な検出力はない。前回の監査（C16）は pieces.rs:293 の後半を「残してよい」としたが、当時は2つのガードによる分析がなかった。

推奨: 適用する。

C05では、成りの不可逆性と移動不能な駒の扱いを、プレイアウトの不変条件と通常の捕獲経路が既に守っている。

削除対象は [src/core/movegen/tests/promotion.rs:104](../../../src/core/movegen/tests/promotion.rs:104) `article_17_3_promotion_is_irreversible` と [:353](../../../src/core/movegen/tests/promotion.rs:353) `article_19_5_immobile_pieces_remain_capturable_and_blocking` である。

[properties.rs:372](../../../src/core/movegen/tests/properties.rs:372) の `assert_application_effects` が、プレイアウトの毎手で `promote=false` の駒コードが変わらないことを検査する。不成で適用した駒が歩のまま残ることは [promotion.rs:301](../../../src/core/movegen/tests/promotion.rs:301)（第19条2項）が、成駒のコードの保存は [lion_moves.rs:60](../../../src/core/movegen/tests/lion_moves.rs:60) と :177 が確認する。移動不能は手生成の結果にすぎず、盤上の表現に特別な状態はない。走りの遮蔽は [movement.rs:34](../../../src/core/movegen/tests/movement.rs:34) と [src/core/attacks/tables.rs:229](../../../src/core/attacks/tables.rs:229) が、捕獲は通常の捕獲と同じ経路で検査される。

失う現実的な検出力はない。

推奨: 適用する。

C06では、成りのテストが同じガードや同じ盤端の切り詰めを複数のケースで繰り返している。

一部削除の対象は、[src/core/movegen/tests/promotion.rs:83](../../../src/core/movegen/tests/promotion.rs:83) の95-99行（後手の玉将）、[:117](../../../src/core/movegen/tests/promotion.rs:117) の145-167行（成歩と生金の対）、[:176](../../../src/core/movegen/tests/promotion.rs:176) の184-185行（斜め入陣2件）と189-195行（成りの適用）、[:319](../../../src/core/movegen/tests/promotion.rs:319) の324行（6,3→6,2の不成）と326-334行（最奥段の香車の着手0）、[:562](../../../src/core/movegen/tests/promotion.rs:562) の571-579行、[:625](../../../src/core/movegen/tests/promotion.rs:625) の645-647行、[:701](../../../src/core/movegen/tests/promotion.rs:701) の713-717行（いずれも最奥段で以後は移動不能であることの確認）である。

先後の対称は [properties.rs:47](../../../src/core/movegen/tests/properties.rs:47) `article_9_black_and_white_move_sets_are_180_degree_symmetric` が全駒種・全144升の `Move` 集合で照合する。成歩と生金の対は醉象の対と同じ `is_promoted` のガードを通るので、王駒が増える醉象の対だけを残す。入陣の判定 `enters_zone` は方向を見ず、成り先は [:63](../../../src/core/movegen/tests/promotion.rs:63) `article_9_17` が18組すべてを適用して確認する。最奥段の歩と香は前方が盤外なので移動表だけで着手が0になり、凍結専用の分岐はない。この観測は [pieces.rs:197](../../../src/core/movegen/tests/pieces.rs:197) と [:301](../../../src/core/movegen/tests/promotion.rs:301) が代表として担う。P3は救済フラグだけに作用し、移動生成に関与しない。P5の各テストの成りの選択肢のassertは残す。

失う現実的な検出力はない。

推奨: 適用する。

C07では、P群の排他と併用の検査が、規則セットの組立てのテストと重なっている。

統合対象は [src/core/movegen/tests/promotion.rs:799](../../../src/core/movegen/tests/promotion.rs:799) `article_30_p1_and_p2_are_exclusive_while_p3_p4_compose` であり、統合先は [src/core/rules/tests.rs:37](../../../src/core/rules/tests.rs:37) `article_33_4_from_codes_reports_duplicate_conflict_and_missing_in_contract_order` である。

併用5通りの受理は [rules/tests.rs:17](../../../src/core/rules/tests.rs:17)（P1、P3、P4、P5、P6の受理）と重なり、衝突は assembly.rs の同じ `assign_group` を通る。ただし、P2が別の群に割り当てられる誤りはP1とP2の衝突でしか検出できないので、このassertは統合先へ移して残す。統合すると、promotion.rs から `Rules` と `RuleCode` のimportが不要になる。

統合すれば失う検出力はない。

推奨: 適用する。

C08では、角鷹と飛鷲の2段階移動のテストが、同じループと同じ適用経路を方向別に重ねている。

一部削除の対象は、[src/core/movegen/tests/lion_moves.rs:83](../../../src/core/movegen/tests/lion_moves.rs:83) の105-122行（飛鷲の右前斜め）と、[:123](../../../src/core/movegen/tests/lion_moves.rs:123) の140-147行（跳びを適用した後の中間駒の保存）である。

2段階移動は `profile.directions` を同じループで生成する（lion_like.rs:24）。片側の方向データが欠ければ、[pieces.rs:278](../../../src/core/movegen/tests/pieces.rs:278) `article_10_8` の直接到達集合が検出する。経由升のない手の適用は駒種によらず同じ `make_move` の経路であり、[movement.rs:54](../../../src/core/movegen/tests/movement.rs:54)（麒麟）と [lion_moves.rs:339](../../../src/core/movegen/tests/lion_moves.rs:339)（獅子）が同じことを確認する。所有者別の生成と、自駒への跳び先の拒否は残す。

失う現実的な検出力はない。

推奨: 適用する。

C09では、自駒の升を到達先にしないこと、および非捕獲の生成順を、プレイアウトの性質テストが網羅している。

削除対象は [src/core/movegen/tests/movement.rs:18](../../../src/core/movegen/tests/movement.rs:18) `article_7_2_own_occupied_square_is_not_a_destination` である。一部削除の対象は [properties.rs:510](../../../src/core/movegen/tests/properties.rs:510) `quiet_generation_preserves_order_for_special_moves_and_all_rules` の580-588行（`sampled_random_positions` のループ）である。

[properties.rs:274](../../../src/core/movegen/tests/properties.rs:274) は、全駒が自駒に塞がれた初期局面から、生成した全手が自駒の升に到達しないことを検査する。残りの方向が存在することは pieces.rs の空盤の集合が確認する。非捕獲の列が全手から捕獲を除いた列と一致することは [properties.rs:418](../../../src/core/movegen/tests/properties.rs:418) が5規則×128手で検査している。規則集合の5番目は異なるが、L3とL4は獅子の捕獲だけに作用するので非捕獲の列に影響しない。[movegen-speedup-2.md:268](../../plans/movegen-speedup-2.md) が求める特殊手を含む局面の契約（:515-579）は残す。

失う現実的な検出力はない。

推奨: 適用する。

C10では、private関数 `ordinary_capturer` の利き集合を、公開の捕獲生成の照合とは別に検査している。

削除対象は [src/core/movegen/tests/ordinary_capturer.rs:63](../../../src/core/movegen/tests/ordinary_capturer.rs:63) `ordinary_capturer_matches_control_for_capture_test_positions` と [:74](../../../src/core/movegen/tests/ordinary_capturer.rs:74) `ordinary_capturer_matches_control_in_seeded_playouts` であり、[search_captures.rs:15-17](../../../src/core/movegen/search_captures.rs:15) のテストモジュール宣言も削除する。

[search_captures.rs:47](../../../src/core/movegen/tests/search_captures.rs:47) `ordinary_targets_select_exactly_the_public_subsequence` が、同じ局面・規則・マスク族で、対象を絞った再収集と全体からの抽出の一致を確認する。[search_captures.rs:8](../../../src/core/movegen/tests/search_captures.rs:8) `split_captures_preserve_public_order_and_captured_squares` は、全体が公開生成と一致することを確認する。この2つの連鎖で、`ordinary_capturer` の結果は手の列として完全に照合される。残る差は王駒マスク（単升マスクに包含される）と、外部から観測できない `None` と `Some(empty)` の区別だけである。:74 は規則を参照しない純粋な利きの計算（search_captures.rs:167-195）を規則ごとに回しており、乱択される局面が違うだけである。

失う現実的な検出力はない。

推奨: 適用する。

C11では、`reach` の期待値を構築と同じ式で計算している。

削除対象は [src/core/attacks/tables.rs:206](../../../src/core/attacks/tables.rs:206) `reach_matches_fixed_and_unblocked_slides` である。

期待値の `fixed ∪ sliding_control(EMPTY)` は、tables.rs:93-102 の構築と同じ式であり、実装のコピーである。`reach` の利用箇所は search_captures.rs:178 の事前判定だけである。過小なら `ordinary_capturer` が捕獲を落とし、C10の保持先と [search_captures.rs:96](../../../src/core/movegen/tests/search_captures.rs:96) が検出する。過大な場合は性能にしか影響しない。

失う現実的な検出力はない。

推奨: 適用する。

C12では、未使用の定数を検査している。

削除対象は [src/core/board/bitboard.rs:282](../../../src/core/board/bitboard.rs:282) `file_masks_partition_the_board_into_twelve_files` であり、定数 `FILE_MASKS`（[bitboard.rs:17](../../../src/core/board/bitboard.rs:17)）と [board/mod.rs:7](../../../src/core/board/mod.rs:7) の再エクスポートも削除する。

`FILE_MASKS` の唯一の利用者だった `eval/king_features.rs` はコミット e80cbf4 で削除された。現在の参照は定義と再エクスポートとこのテストだけである。

失う現実的な検出力はない。この定数とテストは、[全体の報告](../test-audit-2026-10-04.md)の「先に直すべき問題」にも挙げた。

推奨: 適用する。

C13では、方向と駒種の基本表を、それを使う上位のテストが独立に検査している。

削除対象は [src/core/board/direction.rs:110](../../../src/core/board/direction.rs:110) `eight_directions_form_consistent_unit_displacements` である。一部削除の対象は [src/core/piece/tests.rs:45](../../../src/core/piece/tests.rs:45) `article_4_3_piece_kinds_are_21_initial_plus_8_promoted_only` の65-75行（成りで新たに現れる駒種）である。

変位の誤りは pieces.rs の独立に組んだ期待集合が検出する。`opposite` の誤りは、それを使う逆引き（control.rs:79、:112）を全升で照合する [attackers.rs:46](../../../src/core/movegen/tests/attackers.rs:46) が検出する。前回の監査が削除したのは direction.rs:110 の `step_square` の部分だけで、本体を明示的に残すとは決めていない。`promoted()` の表は [piece/tests.rs:81](../../../src/core/piece/tests.rs:81) が18組すべてで固定しており、初期駒以外の像は定数から決まる。`ALL` の重複と分類の部分は、成駒専用種の重複を他のテストが検出しないので残す。

失う現実的な検出力はない。

推奨: 適用する。

以下は、対局進行・局面・規則セットの候補である。

C14では、R2の反復手の除外を、`Game::legal_moves` と同じ関数を呼ぶ低層の手順と比べている。

削除対象は [src/core/game/adjudication/tests.rs:185](../../../src/core/game/adjudication/tests.rs:185) `plan_adjudication_r2_filter_matches_game_legal_moves_and_restores_the_position` である。

`Game::legal_moves`（referee.rs:136-156）は同じ `retain_repetition_allowed_moves` を局面の複製へ直接呼ぶので、集合比較は自分自身との比較になる。前回の監査（C07）で削除した `plan_adjudication_shared_functions_match_game_play` と同じ形である。反復手の除外（第31条R2、第27条4項）は、同じフィクスチャで [illegal_move.rs:133-138](../../../src/core/game/tests/illegal_move.rs:133) が拒否として、[no_legal_move.rs:55](../../../src/core/game/tests/no_legal_move.rs:55) が絞り込み後の合法手なしとして観測する。復元漏れは、毎手 `is_mate` 経由で同じ関数を通るR2の自己対局 [self_play.rs:198](../../../src/core/game/tests/self_play.rs:198) が局面の破壊として検出する。

失う現実的な検出力はない。

推奨: 適用する。

C15では、詰みの判定を `is_mate` の直接呼出しで、対局テストと同じ配置について重ねて確かめている。

一部削除の対象は [src/core/game/adjudication/tests.rs:90](../../../src/core/game/adjudication/tests.rs:90) `article_21_3_mate_requires_every_escape_clause_to_fail` の、mated、escapable、counter_capture の3局面と末尾の stuck 局面である。`mated_before` による局面復元のassertは、反撃の利きが残る局面へ移して残す。

mated 局面は [win.rs:6](../../../src/core/game/tests/win.rs:6) `article_21_2` の着手後局面と、escapable 局面は [win.rs:16](../../../src/core/game/tests/win.rs:16) `article_21_3_a` と同じ配置である。counter_capture は [win.rs:36](../../../src/core/game/tests/win.rs:36) `article_21_3_b_and_21_4` が対局を通して観測する。stuck の `!is_mate` は対局から観測できない内部の順序である。`is_mate` の呼出しは mod.rs:260 の1か所だけで、合法手なしの判定より後に限られるからである。

失う現実的な検出力はない。前回の監査が保持を決めた局面復元の比較と、反撃の利きの境界はどちらも残す。

推奨: 適用する。

C16では、R1の連続攻撃カウンタを private関数への配列リテラルで検査している。

削除対象は [src/core/game/repetition/r1.rs:345](../../../src/core/game/repetition/r1.rs:345) `article_31_r1_attacking_moves_extend_the_run_and_others_reset_it` である。

[ledgers/d3.md](../../plans/spec-first-tests/ledgers/d3.md) の82行目は、このテストをD3-031-05の担い手として新設したと記録している。しかしD3-031-05は、候補手の評価で0へ戻ることを観測する [win.rs:60](../../../src/core/game/tests/win.rs:60) `article_21_3_c_and_31_r1_repetition_win_rescues_mate` も担っている。加算漏れや相手側の誤更新は [repetition.rs:202](../../../src/core/game/tests/repetition.rs:202) `sole_continuous_checker_loses` が、双方の攻撃継続は [repetition.rs:298](../../../src/core/game/tests/repetition.rs:298) `mutual_perpetual_attacks_draw` が、据え置きの脅威を数えないことは [repetition.rs:235](../../../src/core/game/tests/repetition.rs:235) が、それぞれ対局結果として検出する。

失う現実的な検出力はない。削除に伴い、ledgers/d3.md の82行目にある担い手の記述を win.rs:60 と repetition.rs:202 へ書き換える。

推奨: 適用する。

C17では、R1の可逆手12手の前提条件と12手目の裁定を、同じ局面と同じ周期で重ねて検査している。

削除対象は [src/core/game/tests/repetition.rs:143](../../../src/core/game/tests/repetition.rs:143) `article_31_r1_requires_twelve_reversible_plies_from_start` である。一部削除の対象は [src/core/game/tests/illegal_move.rs:114](../../../src/core/game/tests/illegal_move.rs:114) `article_27_4_r2_r3_reject_before_acceptance_while_r1_adjudicates_after` の123-131行（R1ブロック）である。

[repetition.rs:172](../../../src/core/game/tests/repetition.rs:172) `…after_an_irreversible_move` の前半11手が、同じ開始局面と同じじっと周期で「開始から12手未満は裁定しない」を検査する。開始時の計数の初期値が誤っていれば、早すぎる裁定はこの前半が、遅すぎる裁定は [exhaustion.rs:87](../../../src/core/game/tests/exhaustion.rs:87) などの「開始から12手目で裁定」が検出する。さらに lishogi_replay の EHUTJJu4 と A4EO2swa が、開始局面からのじっと往復で可逆手12手目に裁定されることを実棋譜で照合している。illegal_move.rs:114 のR1ブロックは、[repetition.rs:6](../../../src/core/game/tests/repetition.rs:6) `articles_24_1_and_31_r1_f1_cycle_draws_at_ply_12` と同じ第24条1項・第31条R1をR1の下で検査する。f1は ledgers/d3.md の49行目でマトリクスF1どおり実際の初期局面へ書き直した経緯があるので、f1の側を残す。R2とR3の拒否と、その前後で状態が変わらないことの検査は残す。

失う現実的な検出力はない。

推奨: 適用する。

C18では、相手の駒を動かす入力の拒否が、拒否後の状態保持のテストと重なっている。

統合対象は [src/core/game/tests/illegal_move.rs:28](../../../src/core/game/tests/illegal_move.rs:28) `article_26_1_moving_the_opponents_piece_is_rejected` であり、統合先は [illegal_move.rs:153](../../../src/core/game/tests/illegal_move.rs:153) `article_27_1_rejected_moves_leave_the_game_state_unchanged` である。

局面と手数が変わらないことの確認は、統合先の `assert_rejection_is_pure`（:89-111）が見る範囲に含まれる。拒否後の合法手の受理も統合先の :164-167 と同じである。相手駒の入力を `assert_rejection_is_pure` の1回の呼出しとして統合先へ加え、第26条1号の分類を残す。

統合すれば失う検出力はない。

推奨: 適用する。

C19では、E2による駒枯れ裁定の無効化を、同じ早期returnを通る3局面で検査している。

一部削除の対象は [src/core/game/tests/local_rules.rs:63](../../../src/core/game/tests/local_rules.rs:63) `article_32_e2_disables_all_piece_exhaustion_adjudication` の、67-78行（双方王駒のみの局面）と80-90行（猶予の局面）である。92-104行の即時勝ちの局面は残す。

3局面とも exhaustion.rs:106-118 の `disabled` による1つの早期returnを通り、同じ第32条E2をE2の下で検査している。E2を判定するもう1か所の mod.rs:168（第21条3項cの回避手評価）は、どのケースも通らない。3局面は第22条の別々の項（8項、5項、1項）に対応するが、実装はその区別をしない。

失うのは、第22条の結果の種類ごとにE2の無効化を書き分ける実装へ変えた場合に、引き分けや猶予の側だけ無効化し損ねる不具合の検出である。現在の実装にその分岐はない。

推奨: 適用する。

C20では、対局開始時の状態、合意による引き分け、投了を、他のテストや実棋譜の再生が観測している。

削除対象は [src/core/game/tests/royals.rs:6](../../../src/core/game/tests/royals.rs:6) `article_20_1_game_starts_ongoing_with_one_royal_each` と [src/core/game/tests/win.rs:165](../../../src/core/game/tests/win.rs:165) `article_21_7_draw_agreement_ends_the_game` である。[win.rs:153](../../../src/core/game/tests/win.rs:153) `article_21_6_resignation_ends_the_game` も重複先を持つが、保持する。

王駒の枚数と升は [src/core/position/tests/setup.rs:170](../../../src/core/position/tests/setup.rs:170) `articles_5_and_20_1_each_side_starts_with_exactly_one_royal_king` が升まで特定して検査する。開始直後が継続中でなければ、[self_play.rs:183](../../../src/core/game/tests/self_play.rs:183) の `legal_moves` が空でないことのassertと、[illegal_move.rs:44](../../../src/core/game/tests/illegal_move.rs:44) の合法手の受理が失敗する。合意による引き分けは [illegal_move.rs:49](../../../src/core/game/tests/illegal_move.rs:49) `article_26_12` の前半が `result()` と手数0で検査し、`agree_draw` の戻り値は [tests/lishogi_replay.rs:116-131](../../../tests/lishogi_replay.rs:116) の `draw` 分岐（UUnYczs0、mU1oGkUg）が照合する。投了の戻り値は [tests/lishogi_replay.rs:133-147](../../../tests/lishogi_replay.rs:133) の `resign` 分岐（gZ0HcfLK、2u7dwJf9）が照合する。

royals.rs:6 と win.rs:165 の削除で失う現実的な検出力はない。win.rs:153 は、投了の重複先が外部棋譜の2局だけに依存し、フィクスチャを差し替えると投了者と勝者の取り違えを検出できなくなるため残す。

推奨: win.rs:153 を保持し、royals.rs:6 と win.rs:165 の削除は適用する。

C21では、`invariants` フィーチャが毎回行う検査を、明示的なassertで重ねている。

削除対象は [src/core/position/tests/validate.rs:24](../../../src/core/position/tests/validate.rs:24) `bench_and_sampled_random_positions_validate` である。一部削除の対象は次のとおりである。

- [builder.rs:8](../../../src/core/position/tests/builder.rs:8) `builder_finish_preserves_consistent_incremental_zobrist_values` の12-17行（再計算一致のassert 2件）、検査対象への `Position::initial()` の追加、21-26行（空ビルダーのループ）。
- [make_move.rs:269](../../../src/core/position/tests/make_move.rs:269) `unmake_restores_every_observable_component` の276-282行（再計算一致2件と `validate()`）。
- [make_move.rs:346](../../../src/core/position/tests/make_move.rs:346) `seeded_random_playouts_uphold_conservation_and_undo_invariants` の388-398行（再計算一致2件と `validate()`）。
- [make_move.rs:295](../../../src/core/position/tests/make_move.rs:295) の310行と [:319](../../../src/core/position/tests/make_move.rs:319) の336行（ヌルムーブ後の `zobrist == recompute_zobrist()`）。
- [promotion_rights.rs:36](../../../src/core/position/tests/promotion_rights.rs:36) の58行と86行（`rights_zobrist` が0でない・0であるという直値のassert）、および59-62行と87-90行（再計算一致）。

bench局面は `parse_sfen` から `builder.finish()`（sfen.rs:623、builder.rs:62-66）を通るので、その時点で `validate()` 済みである。乱択局面は `make_move_unchecked` で作られ、`invariants` が毎手 `validate()` を実行する。`try_make_move_with_undo` は `make_move_unchecked` を呼ぶ（movegen/checked.rs:41）。`make_null_move` の末尾でも `invariants` が検査を行う（make_move.rs:65-66）。権利キーが0になるのは再計算式の内部表現であり、保留集合が空かどうかは同じテストの `promotion_deferred()` のassertで観測できている。

失う現実的な検出力はない。`set_lion_capture` の後の `validate()`、取消し後の完全一致の比較、駒数の保存、手番、全取消し、捕獲・成り・2段階移動の出現確認、ヌルムーブ後の `expected.recompute_zobrist()` との比較、P2の待機の保持と満了は残す。この判断は自己dev-dependencyによる `invariants` の常時有効化が続く間だけ成り立つ。

推奨: 適用する。

C22では、`finish()` が `validate()` を呼ぶ配線を、壊したZobrist値で観測している。

対象は [src/core/position/tests/builder.rs:30](../../../src/core/position/tests/builder.rs:30) `builder_finish_rejects_corrupted_zobrist_values` である。

2種の不一致エラーそのものは、同じ壊し方の [validate.rs:8](../../../src/core/position/tests/validate.rs:8) `validate_rejects_corrupted_zobrist_values` が検査している。一方、`finish()` の中身は `validate().map_err(InvalidPosition)` の1行（builder.rs:62-66）であり、この配線を直接観測するテストはほかにない。SFENの `PositionBuild(_)` エラーは `mark_promotion_deferred` から来るので代わりにならない。[predecessor-generator.md](../../plans/predecessor-generator.md) の188行目と265行目もこの配線を記述している。

削除すると、`finish()` が `validate()` を呼ばなくなる回帰を見逃す。公開APIだけでは不整合な局面を作れないため、実害が出るのは `put` 側の不具合と重なった場合に限られるが、唯一の観測点を残す価値がある。

推奨: 保持する。

C23では、`invariants` の診断メッセージの書式を、実装と同じ書式文字列で再現している。

一部削除の対象は、[src/core/position/tests/validate.rs:82](../../../src/core/position/tests/validate.rs:82) `move_invariants_report_corrupted_keys` ほか :106、:126、:141 の4テストが共有する補助関数 `assert_diagnostic` のうち、53-62行（Zobrist行の書式の再現）と63-75行（盤面行の書式の再現）である。

:53-62 は make_move.rs:276 と同じ書式文字列で期待値を作る。:69-73 は make_move.rs:286 と同じ `format!("{:?}", &board[rank*16..rank*16+12])` で期待値を作る。いずれも実装のコピーである。[debugging-tools.md](../../plans/debugging-tools.md) の244行目が求める診断項目の存在は、`contains("zobrist: incremental=")` のような項目名の確認と、既存の理由・操作・手・手番の確認で足りる。4テストはそれぞれ4つの呼出し箇所（同書240行目）の存在を担うので、テスト自体は残す。

失う現実的な検出力はない。

推奨: 適用する。

C24では、成り権保留の参照と引き継ぎを、SFENと合法手生成のテストが観測している。

削除対象は [src/core/position/tests/promotion_rights.rs:11](../../../src/core/position/tests/promotion_rights.rs:11) `promotion_deferred_returns_the_marked_square_set` と [:97](../../../src/core/position/tests/promotion_rights.rs:97) `article_30_p5_deferred_pawn_state_moves_with_an_unpromoted_pawn` である。

`promotion_deferred()` はフィールドを返すだけのアクセサである。保留の指定から参照までの経路は、[sfen.rs:1101](../../../src/notation/sfen.rs:1101) `deferred_field_requires_p1_p2_or_p5_and_round_trips_for_each` と [sfen.rs:1059](../../../src/notation/sfen.rs:1059)（先手と後手の2升の保留集合の往復）が観測している。P5の保留の引き継ぎは、[src/core/movegen/tests/promotion.rs:651](../../../src/core/movegen/tests/promotion.rs:651) `article_30_p2_p5_deferred_pawn_must_promote_on_a_quiet_last_rank_move` が、保留した歩兵を不成で3升進めた後に最奥段で成りが強制されることで検出する。権利キーの整合は `invariants` が毎手検査する。

失う現実的な検出力はない。

推奨: 適用する。

C25では、先獅子の記録升の参照を、直前局面の生成とZobristのテストが観測している。

削除対象は [src/core/position/tests/lion_trigger.rs:22](../../../src/core/position/tests/lion_trigger.rs:22) `lion_capture_square_distinguishes_non_lion_and_lion_captures` と [:11](../../../src/core/position/tests/lion_trigger.rs:11) `lion_capture_square_tracks_explicit_state` である。

:22 の前半は [predecessor/tests/lion_state.rs:24](../../../src/core/predecessor/tests/lion_state.rs:24)（飛車が獅子を取った直後の `lion_capture_square() == Some(to)`）が、後半は [zobrist.rs:176-197](../../../src/core/position/tests/zobrist.rs:176) が、獅子が獅子を取った局面と先獅子なしで直接構築した局面の `Eq` 一致として検査する（`Eq` は先獅子のフィールドを含む）。:11 の設定と解除の効果は [zobrist.rs:242](../../../src/core/position/tests/zobrist.rs:242) `article_24_1_c_key_distinguishes_the_prohibition_target_square` がキーと `Eq` で検査し、明示的に設定した後の参照値は、[predecessor/tests/mod.rs:144-152](../../../src/core/predecessor/tests/mod.rs:144) の `with_state` を経由して [lion_state.rs:55](../../../src/core/predecessor/tests/lion_state.rs:55) の72-84行のループが使う。

失う現実的な検出力はない。:11 の削除は lion_state.rs:55 の72-84行のループを残すことを前提にするので、C38ではこのループを削除対象から外した。

推奨: lion_state.rs:55 の72-84行のループを残すことを条件に適用する。

C26では、ハッシュ集合の区別と保留指定の拒否を、より厳しい条件のテストやSFENの検証が既に検査している。

削除対象は [src/core/position/tests/zobrist.rs:39](../../../src/core/position/tests/zobrist.rs:39) `hash_set_distinguishes_side_and_temporary_states` である。一部削除の対象は [zobrist.rs:285](../../../src/core/position/tests/zobrist.rs:285) `article_24_1_d_promotion_rights_key_is_separate_from_the_board_key` の309-326行のうち、空升・王将・成駒への保留指定の拒否の3ケースである。

同値な局面のハッシュが一致することは [zobrist.rs:18](../../../src/core/position/tests/zobrist.rs:18) `equal_positions_have_equal_hashes` が明示的に比較している。8局面の区別と再挿入の拒否は、[zobrist.rs:53](../../../src/core/position/tests/zobrist.rs:53) `hash_set_preserves_distinct_positions_under_collisions` が、すべてのハッシュが衝突するより厳しい条件で同じ8局面に対して検査する。[predecessor-generator.md](../../plans/predecessor-generator.md) の475行目が求める `Hash` の契約2点は、残る2テストが1点ずつ担う。空升・王将・成駒への保留の拒否は、[sfen.rs:1138](../../../src/notation/sfen.rs:1138) `deferred_squares_must_hold_an_unpromoted_promotable_piece` が同じ `promotion_deferred_is_valid` を通して検査している。敵陣外の銀（:327-330）のケースはSFEN側にないので残す。前回の監査が保持を決めたのはキーを区別する部分であり、ここで対象にするのは拒否の検査だけである。

失う現実的な検出力はない。

推奨: 適用する。

C27では、`captured_squares` を旧実装の参照と比べる段階5の検証が、完了後も残っている。

削除対象は [src/core/position/tests/make_move.rs:443](../../../src/core/position/tests/make_move.rs:443) `captured_squares_match_reference_for_all_rules_and_seeded_positions` である。

参照実装（:426-438）は、現行（position/mod.rs:88-100）と同じ「経由升と到達升のうち相手駒がある升を、移動元と同じ到達升を除いて返す」を別の書き方にしただけである。通常の適用は make_move.rs:107-109 で `make_move_with_captures_unchecked(mv, rules, self.captured_squares(mv))` へ委譲しているので、前半のassertが通れば後半の同値は自明に成り立つ。捕獲升の誤りは、[make_move.rs:73](../../../src/core/position/tests/make_move.rs:73)（単独捕獲と2枚取りの駒数）と、:269 の居喰い・2枚取りのシナリオが観測する。

失う現実的な検出力はない。削除に伴い、[movegen-speedup-2.md](../../plans/movegen-speedup-2.md) の254行目と366行目にある段階5の検証の記述を、検証済みである旨へ書き換える。

推奨: 適用する。

C28では、反復規則の群の排他を、同じ割当て経路を通る3組で検査している。

一部削除の対象は [src/core/rules/tests.rs:153](../../../src/core/rules/tests.rs:153) `article_25_2_repetition_rule_is_mandatory_and_exclusive` の161-171行のうち、(R1,R3) と (R2,R3) の2組である。

3組ともassembly.rs:86-88の同じ `assign_group(&mut repetition, …)` を通り、第31条末文・第33条2項の同じR群の排他を検査している。R3を誤ったスロットへ割り当てる不具合は、R3を使う対局テスト [illegal_move.rs:140](../../../src/core/game/tests/illegal_move.rs:140) が `Missing` で失敗して検出する。

失う現実的な検出力はない。

推奨: 適用する。

C29では、全欄を既定値以外にした拡張SFENの往復を、欄ごとの往復テストと比べた。

対象は [src/notation/sfen.rs:1190](../../../src/notation/sfen.rs:1190) `extended_output_always_writes_five_fields_and_round_trips_all_state` である。

獅子捕獲升の往復は [sfen.rs:959](../../../src/notation/sfen.rs:959) が、手数の往復は [sfen.rs:1017](../../../src/notation/sfen.rs:1017) が、保留升の文字列一致は [sfen.rs:1101](../../../src/notation/sfen.rs:1101) が、5欄を常に書き出すことは [sfen.rs:877](../../../src/notation/sfen.rs:877) が担っている。しかし、全欄を同時に既定値以外にする検査はこのテストだけである。C35の削除もこのテストを重複先として前提にする。

削除すると、全欄が既定値でない場合に限って欄の順序や区切りを誤って書き出す不具合を見逃す。

推奨: 保持する。

C30では、開始局面ファイルの来歴に書く混合比の既定値を、単体テストと統合テストが二重に固定している。

一部削除の対象は [tests/lishogi_import.rs:236](../../../tests/lishogi_import.rs:236) `opening_generation_cli_requires_zero_random_moves_and_one_game_per_line` の270行（`provenance.lambda == 0.75`）である。

同じ既定値を [src/bin/selfplay_gen/provenance.rs:53](../../../src/bin/selfplay_gen/provenance.rs:53) の単体テストが検査している。`git log -S"0.75"` での書き換え履歴は0回なので、主な根拠は重複である。

失う現実的な検出力はない。

推奨: 適用する。

以下は、直前局面の生成の候補である。`PredecessorGenerator` はリポジトリ内に呼出し元がなく、公開APIとして外部のRustプログラムだけが使う想定なので、通常実行のテストが回帰を検出する唯一の手段である。逆生成（reverse.rs、transient.rs）は L1〜L4 と P6 の各フラグを一度も参照しない。これらの規則は順方向の検証（`is_legal_move`）だけが扱う。

C31では、入力検査のテストが、1行の委譲、表示文字列の写し、仕様にない順序、静的表明済みの性質を確かめている。

削除対象は、[src/core/predecessor/tests/input.rs:5](../../../src/core/predecessor/tests/input.rs:5) `standard_matches_explicit_rules`、[:21](../../../src/core/predecessor/tests/input.rs:21) `errors_keep_distinct_causes_and_display`、[:133](../../../src/core/predecessor/tests/input.rs:133) `overflow_order_is_color_then_kind_after_counting`、[:258](../../../src/core/predecessor/tests/input.rs:258) `no_previous_mover_returns_valid_empty_set`、[:265](../../../src/core/predecessor/tests/input.rs:265) `generator_is_shared_across_threads` である。

`standard()` は mod.rs:28-30 の `Self::new(MoveRules::standard())` の1行であり、大きな破綻は [tests/predecessor_search.rs:35](../../../tests/predecessor_search.rs:35) が初期局面を発見できなくなることで検出される。:21 の `format!("invalid position: {cause}")` は mod.rs:93 の写しで、`assert_ne!(error, other)` は `derive(PartialEq)` が保証する。`InvalidPosition` は公開経路から生成できない。:133 は mod.rs:131-144 のループ順を固定しているだけで、設計書「公開API」は報告順を定めていない。歩兵の超過検出は :48 と :84 が検査済みである。:258 と同じ状況（手番側の駒しかない）で `checked(..).is_empty()` を検査する箇所が :99、:115、:176-185 にある。Send+Sync は設計書が静的表明で固定すると定めており、その表明は mod.rs:207-208 にある。生成器は可変状態を持たず、2スレッドで小局面を1回呼ぶだけでは競合をほぼ検出できない。

失う現実的な検出力はない。

推奨: 適用する。

C32では、同じ局面と着手を別名で繰り返すテストと、`checked()` から導かれる否定形のテストがある。

削除対象は、[src/core/predecessor/tests/lion_state.rs:40](../../../src/core/predecessor/tests/lion_state.rs:40) `full_lion_origins_exclude_all_prior_records`、[lion_movement.rs:105](../../../src/core/predecessor/tests/lion_movement.rs:105) `blocked_lion_cannot_jitto`、[material.rs:79](../../../src/core/predecessor/tests/material.rs:79) `full_initial_stock_allows_no_capture_restoration` である。

lion_state.rs:40 は、局面（獅子(5,5)、麒麟(11,6)、在庫補充）も着手（(5,5)→(6,5)、非捕獲）も [lion_movement.rs:24](../../../src/core/predecessor/tests/lion_movement.rs:24) の初回反復とまったく同じであり、`all(record.is_none())` は tests/mod.rs:225-232 の `lion_count < 2` から導かれる。逆生成は獅子に対して常にじっとの候補を作り、合否は順方向の検証が決めるので、塞がれた獅子のじっとが返却されれば tests/mod.rs:49 で失敗する。順方向の規則（第12条第10項）は [src/core/movegen/tests/lion_moves.rs:405](../../../src/core/movegen/tests/lion_moves.rs:405) が検査する。不足在庫0の由来を復元しない分岐は [material.rs:33](../../../src/core/predecessor/tests/material.rs:33) が歩兵由来で検査し、駒数92の確認は tests/mod.rs:165 以降の在庫上限のassertから導かれる。

失う現実的な検出力はない。

推奨: 適用する。

C33では、有限状態オラクルが、列挙集合では発動しない規則を検査している。

削除対象は [src/core/predecessor/tests/oracle.rs:171](../../../src/core/predecessor/tests/oracle.rs:171) `predecessor_oracle_l1`（ignored）である。一部削除の対象は [oracle.rs:157](../../../src/core/predecessor/tests/oracle.rs:157) `predecessor_oracle_p2_p6` の規則コードからP6を外し、P2のオラクルとして残すことである。

列挙集合S（oracle.rs:28-）の駒は両王、先手の歩または成金、後手の獅子だけで、先手に獅子がない。このためL0とL1の取り返し制限はどちらの向きにも対象がなく、遷移は [oracle.rs:143](../../../src/core/predecessor/tests/oracle.rs:143) の標準規則と同一になる。Sは筋0-1・段7-8の4升だけで最奥段（段11）を含まないため、P6は一度も発動しない。同じ理由で :164 のP5も、最奥段の強制成りではなく入陣時の保留だけを検査している。

失う現実的な検出力はない。ignored のオラクル全体は約550秒（5.9 GB）を要するので、手動実行の費用も下がる。

推奨: 適用する。

C34では、設計書の固定テスト表が求める事例のうち、肯定側は別のテスト、否定側は `checked()`、規則の意味は合法手生成のテストが守っているものがある。

削除対象は次の8件である。

- [src/core/predecessor/tests/promotion.rs:4](../../../src/core/predecessor/tests/promotion.rs:4) `pawn_outside_zone_has_only_unpromoted_predecessor`：成金は合法な辺を持たず、否定形のassertは tests/mod.rs:49 と重複し、往復検査は [movement.rs:4](../../../src/core/predecessor/tests/movement.rs:4) と同じ分岐である。
- [promotion.rs:64](../../../src/core/predecessor/tests/promotion.rs:64) `promoted_pieces_cannot_promote_twice`：逆生成は成駒から1段階しか戻らない構造（reverse.rs の `before_choices`）で、二重成りの候補はそもそも作られない。成る辺の往復は [promotion.rs:40](../../../src/core/predecessor/tests/promotion.rs:40) と同じ分岐である。
- [movement.rs:55](../../../src/core/predecessor/tests/movement.rs:55) `rook_cannot_reverse_through_friendly_or_enemy_blocker`：逆生成の走りは空の盤面で候補を作り（`sliding_control(.., Bitboard::EMPTY)`）、遮蔽の判定は順方向の検証に任せる。肯定側は [movement.rs:33](../../../src/core/predecessor/tests/movement.rs:33) と重複する。
- [lion_movement.rs:77](../../../src/core/predecessor/tests/lion_movement.rs:77) `jitto_from_different_pieces_returns_one_position`：[lion_movement.rs:131](../../../src/core/predecessor/tests/lion_movement.rs:131) の角鷹・飛鷲テストは fixture()（同ファイル5-19行）が獅子を(0,6)に置くため、じっとの事例（150行）で同じ重複が生じ、重複は tests/mod.rs:45 の `seen.insert` が検査する。
- [lion_state.rs:103](../../../src/core/predecessor/tests/lion_state.rs:103) `l1_prohibits_undefended_lion_recapture`：許可側は [lion_state.rs:8](../../../src/core/predecessor/tests/lion_state.rs:8) の記録升ループ（26-34行）と同一で、L1の意味は [src/core/movegen/tests/lion_capture.rs:1003](../../../src/core/movegen/tests/lion_capture.rs:1003) と :1045 が検査する。
- [lion_state.rs:156](../../../src/core/predecessor/tests/lion_state.rs:156) `l3_removes_pawn_support_between_stages`：L3の意味は [lion_capture.rs:1098](../../../src/core/movegen/tests/lion_capture.rs:1098) が検査する。
- [lion_state.rs:187](../../../src/core/predecessor/tests/lion_state.rs:187) `l4_allows_lion_recapture_under_prior_record`：L4の意味は [lion_capture.rs:1157](../../../src/core/movegen/tests/lion_capture.rs:1157) が検査し、以前の記録升の復元は lion_state.rs:8 と重複する。
- [deferred.rs:139](../../../src/core/predecessor/tests/deferred.rs:139) `p2_p6_forces_lance_promotion_and_expires_other_waiting`（0.68秒）：非移動駒の待機満了の復元は [deferred.rs:29](../../../src/core/predecessor/tests/deferred.rs:29) が、P6の規則の受け渡しは [promotion.rs:91](../../../src/core/predecessor/tests/promotion.rs:91) が、P2+P6の順方向の意味は [src/core/movegen/tests/promotion.rs:768](../../../src/core/movegen/tests/promotion.rs:768) が検査する。

規則が順方向の生成器へ渡ることは、獅子の規則について [lion_state.rs:127](../../../src/core/predecessor/tests/lion_state.rs:127) `l2_allows_recapture_of_just_promoted_kirin`、P6について promotion.rs:91 が1件ずつ確かめるので、これらを残せばL1、L3、L4の規則別テストは追加の検出力を持たない。

失う現実的な検出力はない。将来、規則に依存する枝刈り（設計書「性能測定」で不採用とした第2案）を逆生成へ入れる場合は、そのときに規則別の事例を追加する。削除に伴い、[predecessor-generator.md](../../plans/predecessor-generator.md) の「固定テスト」節（500行目以降）の分類表のうち、成り、走り、じっと、先獅子（L1、L3、L4）、成り権保留（P2とP6の併用）の各行を、`checked()` による健全性検査と合法手生成側のテストで担保する旨へ書き換える。

推奨: 適用する。

C35では、直前局面の生成を呼ばない結合テストが、拡張SFENの往復だけを確かめている。

削除対象は [tests/predecessor_search.rs:66](../../../tests/predecessor_search.rs:66) `predecessor_search_extended_sfen_preserves_transient_states` である。

このテストは `PredecessorGenerator` を呼ばない。往復検査は、C29で保持する [src/notation/sfen.rs:1190](../../../src/notation/sfen.rs:1190)（記録升とP1保留を同時に往復）と、[sfen.rs:959](../../../src/notation/sfen.rs:959)、[sfen.rs:1101](../../../src/notation/sfen.rs:1101) が担う。

失う現実的な検出力はない。設計書の固定テスト節が「先獅子状態や成り権保留が存在する局面の往復」を求めているので、C34と同じ箇所でその担い手を sfen.rs のテストへ書き換える。

推奨: sfen.rs:1190 を保持することを条件に適用する。

C36では、計測用のコーパスの検証が通常実行に含まれ、計測テストは自分自身と比較するassertを持っている。

統合対象は [src/core/predecessor/tests/profile.rs:92](../../../src/core/predecessor/tests/profile.rs:92) `predecessor_profile_corpus_has_documented_transitions` であり、統合先は ignored の [profile.rs:209](../../../src/core/predecessor/tests/profile.rs:209) `predecessor_profile` である。一部削除の対象は、profile.rs:209 の234行（`assert_eq!(結果集合, expected)`）と221行（`membership(..).is_ok()`）である。

:92 は生成器を一度も呼ばず、計測コーパスの拡張SFENが記録した着手列と分類から再現できることだけを検証している。それでも通常実行に含まれ、bench[9]（src/test_util.rs）にも依存する。計測時に限って意味があるので、計測テストへ移すのが適当である。:234 の expected は `verify_candidates(enumerate_candidates(..))` の結果であり、これは reverse.rs:16-20 の `reverse::generate` そのものなので f(x)==f(x) の比較になっている。:221 は非公開関数の呼出し順を固定している。profile.rs:209 自体は、[docs/measurements/predecessor-generator-profile.md](../../measurements/predecessor-generator-profile.md) の9行目と12行目、および設計書「検証コマンド」が名指しする再現手順なので保持する。

失う現実的な検出力はない。計測記録の9行目にあるテスト名の記述を統合先へ書き換える。

推奨: 適用する。

C37では、入力検査の受理を確かめるテストが、全直前局面の健全性まで検証している。

一部削除の対象は [src/core/predecessor/tests/input.rs:236](../../../src/core/predecessor/tests/input.rs:236) `lion_record_accepts_empty_square_with_falcon_and_promoted_lion` の253行で、`checked(..)` を `generate_predecessors(..).is_ok()` に置き換える。

このテストは1.676秒で、通常実行で最も遅い。目的は、空升の記録と角鷹・飛鷲、および麒麟由来の成獅子の記録を入力検査が受理することであり、相手側の在庫が全欠損した局面の全直前局面を検証する必要はない。空升記録の候補の健全性は [lion_state.rs:55](../../../src/core/predecessor/tests/lion_state.rs:55) が `checked` で検査済みである。

失う現実的な検出力はない。

推奨: 適用する。

C38では、同じ分岐の重複、発動しない入力、`checked()` から導かれる否定形のassertが残っている。

一部削除の対象は次のとおりである。

- [lion_state.rs:55](../../../src/core/predecessor/tests/lion_state.rs:55) `empty_record_requires_enemy_falcon_or_eagle` の飛車の事例の否定形assert。72-84行のループは C25 の前提なので残す。
- [promotion.rs:27](../../../src/core/predecessor/tests/promotion.rs:27) `entering_zone_preserves_both_promotion_choices` の `promote=true` の反復と `wrong` のassert。`promote=true` の往復は [promotion.rs:52](../../../src/core/predecessor/tests/promotion.rs:52) と同じ局面・同じ着手である。
- [input.rs:48](../../../src/core/predecessor/tests/input.rs:48) `material_overflow_reports_origin_and_owner` の獅子2枚の事例。上限は駒種によらず `Position::initial()` から同じ方法で導出している。
- [movement.rs:77](../../../src/core/predecessor/tests/movement.rs:77) `fixed_jumps_ignore_intermediate_occupancy_and_capture_at_destination` の、中間の駒を先手か後手の一方に絞ることと95-96行の否定形assert。逆生成の固定跳びは中間升を一切参照しない（reverse.rs の `fixed_deltas` ループ）。設計書の固定テスト表は「中間升が空升、味方駒、相手駒の各ケース」を求めるので、C34と同じ箇所で書き換える。
- 否定形の `!contains` と `all(..)`：[movement.rs](../../../src/core/predecessor/tests/movement.rs) の25-26、48-49、69-72、95-96行、[material.rs](../../../src/core/predecessor/tests/material.rs) の37-42行、[promotion.rs](../../../src/core/predecessor/tests/promotion.rs) の207-208行、[deferred.rs](../../../src/core/predecessor/tests/deferred.rs) の20-24、125-133、195-200行、[lion_state.rs](../../../src/core/predecessor/tests/lion_state.rs) の98行（rule_difference の拒否側）。deferred.rs:125-126 と lion_state.rs:92-96 の `try_make_move(..).is_err()` は順方向だけの検査で、合法手生成のテストと重複する。

失う現実的な検出力はない。C34、C37と合わせて適用すると、通常実行の時間は約2.9秒短くなる。

推奨: 適用する。

## 保持と判断した主なテスト

前回の監査が明示的に保持としたものは再提案していない。対象は [attackers.rs](../../../src/core/movegen/tests/attackers.rs) の2件、[tables.rs:229](../../../src/core/attacks/tables.rs:229) `sliding_control_matches_a_stepwise_walk_with_blockers`、[pieces.rs](../../../src/core/movegen/tests/pieces.rs) の28駒の独立期待集合、`article_15_1_each_remaining_footed_lion_is_protected_independently`、`articles_15_1_and_14_1` の前半（解釈の固定）、`piece_codes_are_injective_over_owner_kind_and_promotion`、[make_move.rs:18](../../../src/core/position/tests/make_move.rs:18) `article_6_1_black_moves_first…`、[self_play.rs](../../../src/core/game/tests/self_play.rs) の代表規則セットの自己対局、[zobrist.rs](../../../src/core/position/tests/zobrist.rs) のキーの区別、および駒枯れ・王駒・連続攻撃・R2の相互作用の対局テストである。

[lion_capture.rs](../../../src/core/movegen/tests/lion_capture.rs) の article_13_5（足の駒が取られ得る場合）と article_13_6（王将が唯一の足）は保持する。前者は足の判定に安全性を加える誤実装を、後者は王を攻撃駒から除外する誤実装（本将棋エンジンにありがちな誤り）を捕まえる唯一のテストである。articles_16_8_to_16_10 の歩兵版（:917）も保持する。典拠J2は仲人しか挙げないため、J2に従って歩兵を落とす実装を検出できるのはこのテストだけであり、駒種分岐のないL3の歩兵の対（C01）とは扱いが異なる。

[promotion.rs](../../../src/core/movegen/tests/promotion.rs) の article_19_18（成りの機会の重なりで2手になること）は保持する。現在は `PromotionChoice` の列挙型により重複が構造的に起こらないが、成りを理由ごとに展開する書き換えへの回帰検出として価値がある。プレイアウトの重複検査（[properties.rs:263](../../../src/core/movegen/tests/properties.rs:263)）が最奥段での歩の捕獲に到達する保証はない。P1、P2、P5、P6 の各テストは、`promotion_choice_for` の別々の項（待機の発生、捕獲による入陣、他駒の着手による失効、P2+P5の恒久保留、P6の駒種限定）を1つずつ検出するので保持する。[lion_moves.rs](../../../src/core/movegen/tests/lion_moves.rs) の 11_9 と 12_12 は似た観測だが、生成器が別（lion_like.rs と lion.rs）なので両方残す。[search_captures.rs](../../../src/core/movegen/tests/search_captures.rs) の4件と、[properties.rs](../../../src/core/movegen/tests/properties.rs) の対称、捕獲、置換表検証の各テストは、互いに異なる関係を確認しており、相互に代替できない。

[repetition.rs:6](../../../src/core/game/tests/repetition.rs:6) のf1周期と [repetition.rs:172](../../../src/core/game/tests/repetition.rs:172) は保持する。後者は不可逆手で可逆手の数がリセットされることを検査する対局レベルで唯一のテストであり、C17で削除する開始局面版の検出力も引き受ける。[adjudication/tests.rs:31](../../../src/core/game/adjudication/tests.rs:31) `article_31_r1_candidate_at_eleven_reversible_plies_does_not_rescue_mate` は内部APIに依存するが、仮想着手側の閾値（r1.rs:128 の `R1_MIN_REVERSIBLE_PLIES - 1`）で可逆手11手では詰みのままになる側の境界を検出できる唯一のテストである。[r1.rs:260](../../../src/core/game/repetition/r1.rs:260) の不可逆手の分類表は、第31条R1の不可逆手の定義を、成った歩・香、獅子のじっと、王の手との境界まで直接固定しており、対局テストは歩の前進しか観測しない。local_rules.rs のE3の(a)(b)(c)と死に駒の除外は bare_king.rs の別々の条件を通るので、過剰なケース分けではない。[rules/tests.rs](../../../src/core/rules/tests.rs) の `RuleCode::ALL.len() == 19` は、`from_codes` が `RuleCode::ALL` を走査して組み立てる（assembly.rs:65）ため、`ALL` から漏れたコードが黙って無視される不具合を捕まえる唯一のassertである。

[tests/lishogi_replay.rs](../../../tests/lishogi_replay.rs)（13局、0.013秒）は、[lishogi-bot.md](../../plans/lishogi-bot.md) の124-130行目がリプレイ照合を検証手段に指定している要求先であり、前回の監査でも外部棋譜の再生として保持した。22,798局の全棋譜照合はこのテストとは別に行われたものである。[tests/lishogi_import.rs](../../../tests/lishogi_import.rs) の7テストは、取込み器（src/bin/lishogi_import/）に単体テストがないので唯一の検査であり、勝者不一致の2つの分岐や終局後の着手の無視はそれぞれ別のテストしか通らない。

直前局面の生成では、ignored の completeness 4件と、L1を除く oracle 4件を保持する。これらは設計書「完了条件」の第2層と第3層そのものであり、oracle はSの範囲で返却集合が期待と完全に一致すること、すなわち直前局面の漏れがないことを検査する唯一のテストである。逆に、ignored のテストは毎回は実行されないため、通常実行の固定テストを削る理由には使っていない。[promotion.rs:91](../../../src/core/predecessor/tests/promotion.rs:91) `p6_rejects_unpromoted_pawn_on_last_rank` は、oracle のP6部分が発動せず、completeness のランダム局面も最奥段にほとんど届かないため、P6の規則が順方向の生成器へ渡ることを確かめる唯一の直接の検査として保持する。[lion_state.rs:127](../../../src/core/predecessor/tests/lion_state.rs:127) は獅子の規則の受け渡しの検査として1件だけ残す。[promotion.rs:183](../../../src/core/predecessor/tests/promotion.rs:183) `p2_rejects_candidates_with_two_enemy_waiting_bits` は、否定形のassertは冗長だが、P2で相手側の保留ビットが引き継がれること（transient.rs の `expiring` 条件）を検査する唯一のテストである。[input.rs:153](../../../src/core/predecessor/tests/input.rs:153) と :213 は入力エラー `RuleStateMismatch` と `InvalidLionState` を検査する唯一のテストであり、[material.rs:33](../../../src/core/predecessor/tests/material.rs:33) と :54 は歩兵在庫12枚と11枚の境界の組、:110 は2枚捕獲で同じ由来の残数を引き継ぐ唯一の検査である。補助関数 `checked()` と `assert_admissible` は、設計書の集合Aを製品の入力検査から独立に再実装した検査器であり、この領域の否定形のassertを冗長と判断できるのはこの検査器があるからなので、検査器自体は削らない。
