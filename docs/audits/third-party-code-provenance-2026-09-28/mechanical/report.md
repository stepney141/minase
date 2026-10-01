# M1〜M4の機械的照合結果

基準コミット `a05478a` と全 ref の805コミットを調べ、minase の1,547個のコード系 blob を解析した。
manifest の53行を処理し、比較用入力を8,027件保存した。
履歴版と出現箇所を別件として数えるため、以下の件数は独立した複製の数ではない。

## 実行結果

| 手法 | 候補件数 | 除外または単独一致 | 状態 |
|---|---:|---|---|
| M1 | 820,797 | 除外 185,859 | 取得できた入力の注釈と文字列を照合した。 |
| M2 | 492,346 | 単独一致 4,879,217 | 数値集合と係数導入時の値を照合した。 |
| M3 | 199,405 | 反復表を注記した。 | 配列照合とHaChuの較正を実行した。 |
| M4 | 5,034,236 | 単純構造を注記した。 | tree-sitter補助検査を実行。指定ツールの検証は未完了。 |

M1 の除外内訳は {"license_boilerplate": 2589, "todo_boilerplate": 0, "protocol_specification": 171722, "RULES_quote": 11548} である。
M2 の数値正規化失敗は0件、係数マクロの記録は158件、初出係数は48件である。
M3 の内訳は {"multiset": 919, "scaled": 63493, "scaled_reordered_subset": 40024, "subsequence": 90034, "exact": 4935} である。
M2とM3では候補の意味による除外を行わず、自明値の除外または明らかな偽陽性の注記を行った。

## 較正と被覆

M1は英語5語、日本語12文字を最小単位とし、短い希少句、識別子、式を別枠に残した。
M2は自明値を除いた共通値が2個以上の関数または係数接頭辞の組を候補とした。
M3は8要素以上を比較し、正の倍率では各要素の絶対誤差1以内を許容した。
HaChu `hachu.c:194` の `chuPieces[]` と `src/eval/handcrafted.rs:19` の表は、倍率2.5で29要素中27要素が一致した。
表の並べ替えと王駒2要素の変更があるため、単純な全配列一致に加え、倍率付きの多重集合部分一致で再検出した。

M4は10トークンのハッシュを使い、較正で20トークンを採用した。
改名と独立した宣言の並べ替えを行った人工複製の検出率と、独立対照3組の一致率を次に示す。

| 最小トークン長 | 人工複製検出 | 人工例の被覆率 | 独立対照の一致 |
|---:|---:|---:|---:|
| 20 | 1/1 | 100.0% | 0/3 |
| 40 | 1/1 | 100.0% | 0/3 |
| 80 | 1/1 | 100.0% | 0/3 |

minase の Rust は基準版247/247ファイル、履歴1278/1278 blob で構文解析に成功した。
コーパスでは1,177件の入力に構文エラーがあり、回復した構文木からの抽出を partial_parse と明示した。
パッチは旧側と新側の各ハンクを解析し、断片ゆえの構文エラーを別に残した。
UTF-8として読めない入力は2件あり、元のバイト列を保持したまま `encoding-errors.tsv` と被覆表に抽出上の制限を記録した。
完全な構文木を得られない入力について、候補がないことから非複製とは判断できない。
独立対照には閾値未満の短い関数も含まれるため、この3組の結果から一般の偽陽性率を推定しない。

## 上位候補

候補は基準版を優先し、同一内容と位置の履歴重複を抑えて表示した。
各手法の上位60行は `m1_top.tsv` から `m4_top.tsv` に保存し、全候補は指定の `m1_hits.tsv` から `m4_hits.tsv` に残した。
ライセンス欄は manifest を基本とし、ファイル冒頭のSPDX表示で補正した。各参照版における法的評価は行っていない。

### M1の候補

| minase側位置 | コーパス側位置 | ライセンス | 一致内容 | 注記 |
|---|---|---|---|---|
| minase@a05478a,all-refs:src/core/position/validate.rs:70 [blob=b858904e2296] | kelseyde/hobbes-chess-engine@HEAD:src/board.rs:520 [blob=c2f7822b782a] | MIT | square occupied by the side; en5; 出現1回 | 未判定 |
| minase@a05478a,all-refs:src/protocol/engine.rs:93 [blob=86831a659a88] | fairy-stockfish/Fairy-Stockfish@HEAD:src/position.cpp:2695 [blob=6d59ef648c45] | GPL-3.0-or-later | there is no previous move; en5; 出現1回 | 未判定 |
| minase@a05478a,all-refs:src/protocol/usi.rs:1595 [blob=3c479ed4520a] | SH11235/rshogi@HEAD:crates/rshogi-core/src/nnue/effect_bucket_features.rs:176 [blob=ce5cb7aaa1db] | GPL-3.0-or-later | piece must have a color; en5; 出現1回 | 未判定 |
| minase@a05478a,all-refs:src/protocol/usi.rs:1551 [blob=3c479ed4520a] | SH11235/rshogi@HEAD:crates/rshogi-core/src/nnue/effect_bucket_features.rs:176 [blob=ce5cb7aaa1db] | GPL-3.0-or-later | piece must have a color; en5; 出現1回 | 未判定 |
| minase@a05478a,all-refs:src/core/position/placement.rs:34 [blob=2956056c3453] | SH11235/rshogi@HEAD:crates/rshogi-core/src/nnue/effect_bucket_features.rs:176 [blob=ce5cb7aaa1db] | GPL-3.0-or-later | piece must have a color; en5; 出現1回 | 未判定 |
| minase@a05478a,all-refs:src/notation/usi.rs:210 [blob=655959475393] | SH11235/rshogi@HEAD:crates/rshogi-core/src/nnue/effect_bucket_features.rs:176 [blob=ce5cb7aaa1db] | GPL-3.0-or-later | piece must have a color; en5; 出現1回 | 未判定 |
| minase@a05478a,all-refs:src/training/records.rs:734 [blob=c0b68bc96949] | SH11235/rshogi@HEAD:crates/rshogi-core/src/nnue/effect_bucket_features.rs:176 [blob=ce5cb7aaa1db] | GPL-3.0-or-later | piece must have a color; en5; 出現1回 | 未判定 |
| minase@a05478a,all-refs:src/bin/match_runner/storage.rs:364 [blob=0f4f44d1d5cc] | cosmobobak/viridithas@HEAD:src/errors.rs:123 [blob=885cfd898e18] | AGPL-3.0-only | number must be at least; en5; 出現1回 | 未判定 |
| minase@a05478a,all-refs:src/notation/sfen.rs:173 [blob=282e14d5ee1c] | gab8192/Obsidian@HEAD:src/position.h:88 [blob=e7966018d6b3] | GPL-3.0-only | after the side to move; en5; 出現1回 | 未判定 |
| minase@a05478a,all-refs:src/harness/limit.rs:142 [blob=4f2b9729da50] | SH11235/rshogi@HEAD:crates/tools/src/bin/compare_nodes.rs:1 [blob=48b5a549a02e] | GPL-3.0-or-later | depth n nodes m depth; en5; 出現1回 | 未判定 |

### M2の候補

| minase側位置 | コーパス側位置 | ライセンス | 一致内容 | 注記 |
|---|---|---|---|---|
| minase@a05478a,all-refs:src/training/rescore.rs:540 [blob=d2d55826fd98] | tugajin/cpp_animal_shogi@HEAD:ai/nlohmann/json.hpp:10279 [blob=4d1a37ad7cb8] | MIT | header_rejects_undefined_fields_and_reserved_bytes@538 ↔ parse_msgpack_internal()@10270; 40,48,80,88,120,124,140,156,160,161,162,163,164,236,237,238,239,240 | 未判定 |
| minase@a05478a,all-refs:src/eval/pst/features.rs:101 [blob=86b907892ca3] | peterosterlund2/texel@HEAD:test/texelutil/gameTreeTest.cpp:97 [blob=00ba303220f6] | GPL-3.0-or-later | every_piece_code_maps_to_the_specified_state@98 ↔ TEST(GameTreeTest, testReadInsert)@55; 11,17,21,24,26,27,30,31,33,34,37,38,42,43,45,46 | 未判定 |
| minase@a05478a,all-refs:src/eval/pst/features.rs:101 [blob=86b907892ca3] | codedeliveryservice/Reckless@HEAD:src/search.rs:306 [blob=578683d21d4a] | AGPL-3.0-only | every_piece_code_maps_to_the_specified_state@98 ↔ search@300; 11,17,21,22,24,25,26,27,31,36,37,39,42,43,45,46 | 未判定 |
| minase@a05478a,all-refs:src/eval/pst/features.rs:101 [blob=86b907892ca3] | Witek902/Caissa@HEAD:src/backend/gaviota/compression/zlib/inftrees.c:52 [blob=a8d23dbbc43f] | MIT | every_piece_code_maps_to_the_specified_state@98 ↔ inflate_table(codetype type, short unsigned int *lens, unsigned int codes, code **table, unsigned int *bits, short unsigned int *work)@32; 11,17,20,21,22,23,24,25,26,27,28,29,31,33,35,43 | 未判定 |
| minase@a05478a,all-refs:src/eval/pst/features.rs:101 [blob=86b907892ca3] | SH11235/rshogi@HEAD:crates/tools/src/packed_sfen.rs:2257 [blob=db7b35d6dea8] | GPL-3.0-or-later | every_piece_code_maps_to_the_specified_state@98 ↔ test_packed_sfen_value_to_bytes_roundtrip@2254; 11,17,20,21,22,23,24,25,26,27,28,29,30,31,42 | 未判定 |
| minase@a05478a,all-refs:src/training/rescore.rs:540 [blob=d2d55826fd98] | peterosterlund2/texel@HEAD:app/uciadapter/ctgbook.cpp:335 [blob=fca6064fab57] | GPL-3.0-or-later | header_rejects_undefined_fields_and_reserved_bytes@538 ↔ PositionData::staticInitialize()@333; 40,48,80,124,140,160,162,163,236,237,238,239,240 | 未判定 |
| minase@a05478a,all-refs:src/training/rescore.rs:144 [blob=d2d55826fd98] | tugajin/cpp_animal_shogi@HEAD:ai/nlohmann/json.hpp:10279 [blob=4d1a37ad7cb8] | MIT | decode@142 ↔ parse_msgpack_internal()@10270; 40,48,80,88,120,124,156,160,161,164,204,236,240 | 未判定 |
| minase@a05478a,all-refs:src/stats.rs:206 [blob=1a3d1bd2ca31] | tugajin/cpp_animal_shogi@HEAD:ai/nlohmann/json.hpp:10279 [blob=4d1a37ad7cb8] | MIT | public_gsprt_llr_matches_h1_10_fishtest_reference_values_within_1e_6@205 ↔ parse_msgpack_internal()@10270; 25,30,45,50,60,65,120,141,150,158,180,200,240 | 未判定 |
| minase@a05478a,all-refs:src/stats.rs:183 [blob=1a3d1bd2ca31] | tugajin/cpp_animal_shogi@HEAD:ai/nlohmann/json.hpp:10279 [blob=4d1a37ad7cb8] | MIT | fishtest_reference_values_match_within_1e_6@182 ↔ parse_msgpack_internal()@10270; 25,30,45,50,60,65,120,141,150,158,180,200,240 | 未判定 |
| minase@a05478a,all-refs:src/eval/pst/features.rs:101 [blob=86b907892ca3] | yaneurao/YaneuraOu@V9.00,v9.10-fukauraou:source/position.cpp:3312 [blob=96a849c3b41a] | GPL-3.0-only | every_piece_code_maps_to_the_specified_state@98 ↔ Position::UnitTest(Test::UnitTester& tester, IEngine& engine)@3271; 17,21,25,27,28,30,31,34,36,37,41,44 | 未判定 |

### M3の候補

| minase側位置 | コーパス側位置 | ライセンス | 一致内容 | 注記 |
|---|---|---|---|---|
| minase@a05478a,all-refs:src/eval/handcrafted.rs:195 [blob=d011dda2d7ef] | HaChu (salsa.debian.org/debian/hachu)@HEAD:hachu.c:195 [blob=37d21c4d392a] | LicenseRef-public-domain | scaled_reordered_subset; 27要素; 倍率5/2 | 未判定 |
| minase@a05478a,all-refs:src/eval/handcrafted.rs:20 [blob=d011dda2d7ef] | HaChu (salsa.debian.org/debian/hachu)@HEAD:hachu.c:195 [blob=37d21c4d392a] | LicenseRef-public-domain | scaled_reordered_subset; 27要素; 倍率5/2 | 未判定 |
| minase@a05478a,all-refs:src/eval/handcrafted.rs:195 [blob=d011dda2d7ef] | HaChu (salsa.debian.org/debian/hachu)@HEAD:hachu.c:352 [blob=37d21c4d392a] | LicenseRef-public-domain | scaled_reordered_subset; 24要素; 倍率5/2 | 未判定 |
| minase@a05478a,all-refs:src/eval/handcrafted.rs:20 [blob=d011dda2d7ef] | HaChu (salsa.debian.org/debian/hachu)@HEAD:hachu.c:352 [blob=37d21c4d392a] | LicenseRef-public-domain | scaled_reordered_subset; 24要素; 倍率5/2 | 未判定 |
| minase@all-refs:src/eval/handcrafted.rs:195 [blob=e6ea5abdf7ac] | HaChu (salsa.debian.org/debian/hachu)@HEAD:hachu.c:195 [blob=37d21c4d392a] | LicenseRef-public-domain | scaled_reordered_subset; 27要素; 倍率5/2 | 未判定 |
| minase@all-refs:src/eval/handcrafted.rs:20 [blob=e6ea5abdf7ac] | HaChu (salsa.debian.org/debian/hachu)@HEAD:hachu.c:195 [blob=37d21c4d392a] | LicenseRef-public-domain | scaled_reordered_subset; 27要素; 倍率5/2 | 未判定 |
| minase@all-refs:src/eval/handcrafted.rs:195 [blob=e6ea5abdf7ac] | HaChu (salsa.debian.org/debian/hachu)@HEAD:hachu.c:352 [blob=37d21c4d392a] | LicenseRef-public-domain | scaled_reordered_subset; 24要素; 倍率5/2 | 未判定 |
| minase@all-refs:src/eval/handcrafted.rs:20 [blob=e6ea5abdf7ac] | HaChu (salsa.debian.org/debian/hachu)@HEAD:hachu.c:352 [blob=37d21c4d392a] | LicenseRef-public-domain | scaled_reordered_subset; 24要素; 倍率5/2 | 未判定 |
| minase@a05478a,all-refs:tools/train/pst/train_pst.py:51 [blob=2d58f624bcff] | HaChu (salsa.debian.org/debian/hachu)@HEAD:hachu.c:195 [blob=37d21c4d392a] | LicenseRef-public-domain | scaled_reordered_subset; 27要素; 倍率5/2 | 未判定 |
| minase@a05478a,all-refs:docs/measurements/magic-bitboard-prototype/diagonal.patch:565 [blob=9fdb908c1e07] | lightvector/KataGo@HEAD:cpp/tests/testnn.cpp:292 [blob=93918500bed4] | MIT | subsequence; 26要素; 倍率1 | 未判定 |

### M4の候補

| minase側位置 | コーパス側位置 | ライセンス | 一致内容 | 注記 |
|---|---|---|---|---|
| minase@a05478a,all-refs:src/notation/sfen.rs:730 [blob=282e14d5ee1c] | HiraokaTakuya/apery_rust@HEAD:src/position.rs:3285 [blob=d5afbe3dbd5e] | GPL-3.0-only | 100トークン; minase被覆率1.53% | 未判定 |
| minase@a05478a,all-refs:src/notation/sfen.rs:675 [blob=282e14d5ee1c] | HiraokaTakuya/apery_rust@HEAD:src/position.rs:3285 [blob=d5afbe3dbd5e] | GPL-3.0-only | 100トークン; minase被覆率1.53% | 未判定 |
| minase@a05478a,all-refs:src/core/piece/kind.rs:155 [blob=10d5c8572948] | SH11235/rshogi@HEAD:crates/rshogi-core/src/types/piece_type.rs:59 [blob=ca1f584ca612] | GPL-3.0-or-later | 82トークン; minase被覆率9.24% | 未判定 |
| minase@a05478a,all-refs:src/core/piece/kind.rs:120 [blob=10d5c8572948] | SH11235/rshogi@HEAD:crates/rshogi-core/src/types/piece_type.rs:59 [blob=ca1f584ca612] | GPL-3.0-or-later | 82トークン; minase被覆率9.24% | 未判定 |
| minase@a05478a,all-refs:src/core/rules/code.rs:94 [blob=24df498d9be8] | niklasf/shakmaty@HEAD:shakmaty/src/position.rs:56 [blob=73406203f2c7] | GPL-3.0-or-later | 78トークン; minase被覆率15.26% | 未判定 |
| minase@a05478a,all-refs:src/training/records.rs:175 [blob=c0b68bc96949] | SH11235/rshogi@HEAD:crates/rshogi-core/src/nnue/net_bin_layout.rs:120 [blob=f8b860190f01] | GPL-3.0-or-later | 78トークン; minase被覆率1.55% | 未判定 |
| minase@a05478a,all-refs:src/harness/commit.rs:176 [blob=c618f7191453] | SH11235/rshogi@HEAD:crates/tools/src/bin/gensfen.rs:2956 [blob=0c74bd1c15d2] | GPL-3.0-or-later | 76トークン; minase被覆率4.98% | 未判定 |
| minase@a05478a,all-refs:src/core/board/direction.rs:83 [blob=35f361bf3a25] | SH11235/rshogi@HEAD:crates/rshogi-core/src/search/movepicker.rs:95 [blob=608243cbef54] | GPL-3.0-or-later | 75トークン; minase被覆率12.34% | 未判定 |
| minase@a05478a,all-refs:src/notation/sfen.rs:230 [blob=282e14d5ee1c] | SH11235/rshogi@HEAD:crates/rshogi-core/src/nnue/net_bin_layout.rs:120 [blob=f8b860190f01] | GPL-3.0-or-later | 69トークン; minase被覆率1.06% | 未判定 |
| minase@a05478a,all-refs:src/harness/engine/usi.rs:282 [blob=6b028255b6f4] | SH11235/rshogi@HEAD:crates/rshogi-core/src/nnue/network.rs:3065 [blob=6aa830c89974] | GPL-3.0-or-later | 69トークン; minase被覆率3.67% | 未判定 |

## 未完了部分と限界

YaneuraOu の `v8.00` と `v9.10` は存在せず、要求どおりのタグによる照合は実行できなかった。
存在する `v8.00-fukauraou` と `v9.10-fukauraou` は補足版として明示して解析した。
fishtest の指定コミットにはローカルにない blob が29個あり、ネットワーク取得もできなかった。
該当パスは `missing-inputs.tsv` と `coverage.tsv` に記録した。
これらの欠落により、指定入力の全件についてM1〜M4を完遂したとはいえない。

JPlag 6.2.0とDolosは取得できず、指定ツールによるM4は未完了である。
tree-sitterによる代替の解析と照合は実行したが、JPlagの構文解析失敗を確認したという条件は満たしていない。
C++との言語をまたぐ比較は、翻訳例で検出力を確認していないため実施していない。

帰属表示の確認対象は `attribution.tsv` に記録した。
候補の複製／偶然の判定、コーパス外の実装の検査、重みファイルの来歴調査はこの仕様のM1〜M4では行っていない。
M3では反復配列の長さを解決できなかった箇所が2,305件あり、式と位置を `m3_array_limitations.tsv` に記録した。
部分構文木、マクロ内の式、動的に生成される配列、単位の明記がない数値は検出力の制限になる。
M3の追加検査は比を小数第2位に丸めて候補化するため、任意の倍率による部分的な並べ替えすべてを保証しない。

## 再実行と検証

各手法のコマンド、版、正規化、閾値、出力の説明は `README.md` に記録した。
`logs/inventory.log`、`logs/extract.log`、`logs/m1.log`、`logs/m2.log`、`logs/m3.log`、`logs/m4.log` は実行記録である。
仕様に基づく単体検証は `logs/tests.log`、構文解析の記録は `parse-errors.tsv` にある。
成果物の行数は `output-counts.json`、ファイルの整合性確認は `verification.json` に保存する。
