# 置換表・並列探索・時間管理・USIの対照レビュー記録

対象は minase a05478a（<minase-repo>）の置換表、Lazy SMP、時間管理（履歴上の全版）、先読み（ponder）、src/search/{handle,limits,events,snapshot,stats}.rs、src/protocol/{usi,engine}.rs、src/notation/、src/bin/minase である。対照は manifest.tsv の Stockfish、YaneuraOu、Reckless、viridithas、Ethereal、Obsidian、Alexandria であり、設計書が固定コミットで引く版（Stockfish 5062aee、YaneuraOu 33ccf1f）と、ponder.md が引く版（Stockfish e7b67b9）も読んだ。すべて読み取りだけで行った。

## 判定1　置換表（src/search/alphabeta/tt.rs）

- minase: a05478a:src/search/alphabeta/tt.rs:41-83（エントリ配置）、:242-275（store）、:395-416（詰み値の補正）。導入は 090839d（2026-08-12、単一エントリの初版）であり、原子型の2語配置は 49f6b8f（2026-08-22、Lazy SMPのフェーズ1）、置換規則と「手なし」符号は c817f79（2026-08-22）で入った。設計書 docs/plans/lazy-smp.md:330-336 は、Stockfish 5062aee の tt.cpp と YaneuraOu 33ccf1f の tt.h を「構造の参照先」「差を確認する参照先」として挙げる。
- 対照:
  - Stockfish@5062aee:src/tt.cpp:55-108。10バイトのエントリ（key16、depth8、genBound8、move16、value16、eval16）を3個束ねたクラスタ、世代5ビット、置換値は `depth8 - 8*relative_age`、書込み条件は `d - DEPTH_NONE + 2*pv > depth8 - 4`。GPL-3.0-or-later。
  - YaneuraOu@33ccf1f:source/tt.h:80-133、source/tt.cpp:156-177、:435-441。Stockfish と同じ置換式に、ClusterSize の切替と key の型の切替を加えたもの。GPL-3.0-only。
  - Reckless@31d9cd6:src/transposition.rs（8バイトのエントリ3個と21ビットの鍵3本を詰めた32バイトのクラスタ、世代5ビット）。AGPL-3.0-only。
  - viridithas@13a3fe1:src/transpositiontable.rs:43-357（3エントリのクラスタ、世代5ビット、世代差の2乗を使う置換値）。AGPL-3.0-only。
  - Ethereal@0e47e9b:src/transposition.c:133-158（`depth - 4*age`）、Obsidian@720614a:src/tt.cpp:108-129（`_depth + 4 + 2*isPV > depth`）、Alexandria@e8db34c:src/ttable.cpp:89-124（`depth - 4*age`、`depth + 5 + 2*pv`）。いずれも GPL-3.0。
- 必須の一致: 詰み値を現在ノードからの手数基準へ直して格納し、読み出し時に戻す処理（score_to_tt / score_from_tt）は、Stockfish の value_to_tt と同じ公知の定石である。世代を置換の優先度に使うことと、局面キーの一部を照合キーに使うことも一般的な方式である。
- 実装固有の一致: なし。
- 反証: minase はクラスタを持たない直接写像（1スロット1エントリ）で、エントリは2個の AtomicU64（`critical` に照合キー32ビット、評価値16、深さ8、バウンド2、予約6、`advisory` に指し手25ビット、世代8、予約31）で16バイトを占める。書込みは advisory を Relaxed、critical を Release で公開し、読出しは Acquire で行う。世代は8ビットで wrapping_sub、置換は「同一キーは既存以上の深さ、異なるキーは age>0 または深い結果」である。どの対照とも配置、ビット幅、置換式が異なる。2026-09-10 のクラスタ化の試み（2767394、9d37246 で revert）も、(世代差, 深さ, 添字) の辞書式順で選ぶ方式で、Stockfish 系の線形の置換値とは違う。
- 判定: **2（標準的な方式で説明可能）**。

## 判定2　Lazy SMP と結果の採用（team.rs、deepening.rs）

- minase: a05478a:src/search/alphabeta/team.rs:151-261（探索チーム）、:303-313（select_worker_outcome）、deepening.rs:19-24（auxiliary_depths）。導入は 62a5a58、1cb255f（2026-08-22）。参照先は lazy-smp.md:332-335（Stockfish 5062aee の search.cpp と thread.cpp、YaneuraOu 33ccf1f の yaneuraou-search.cpp）。
- 対照: Stockfish@5062aee:src/thread.cpp:349-400（評価値と最小値の差に14を足す投票）、YaneuraOu@33ccf1f:source/engine/yaneuraou-engine/yaneuraou-search.cpp:603-670（`(score - minScore + 14) * completedDepth` の投票）。Stockfish の旧版（sf_9、sf_10）の SkipSize/SkipPhase 表による深さ飛ばしも比べた。
- 必須の一致: 共有置換表を使う独立な反復深化、主スレッドが補助スレッドを起動して join 後に集約する骨格は、Lazy SMP という方式の定義そのものである。
- 実装固有の一致: なし。
- 反証: minase は投票を使わず、完了深さが最大のワーカーを選び、同じ深さなら番号が最小のものを選ぶ。深さ飛ばしは周期 `2 + ((k-1) % 4)` と `(d-1) % p == 0` による独自の規則であり、Stockfish の20要素の SkipSize/SkipPhase 表とは形も値も異なる。設計書は、この方式を自前の診断（lazy-smp-v1-diversification-bench）から導いた経緯を記す。
- 判定: **2**。

## 判定3　時間管理（a05478a の採用版）

- minase: a05478a:src/search/alphabeta/time.rs:31-107、params.rs:86-98。版の変遷は 7a9c43f（2026-08-12、`soft = remaining/50 + 0.7 inc + 0.8 byoyomi`、`hard = min(4 soft, remaining/4 + 0.8 byoyomi)`、安全余裕30 ms）、96aa2ef（2026-09-02、`moves_to_go = max(100, (450-ply)/2)`）、8f4e41c ほか（2026-09-10〜11、直近4反復の最善手の安定と予測比2.5）、2945d7e（2026-09-17、秒読み項への序盤の係数 `w = min(1, (ply+4)/40)`）、90d1446（2026-09-25、SPSA により 432、88、76、451、27、263 へ調整）である。
- 対照: Stockfish@5062aee・HEAD:src/timeman.cpp（optScale と maxScale の対数・累乗式、mtg=50）、YaneuraOu@HEAD:source/timeman.cpp:213-307（`remain_estimate = time + inc*MTG + byoyomi*MTG`、max_ratio 5、30%上限、1.2倍の最終押し込み、秒単位の切り上げ）、Reckless@31d9cd6:src/time.rs:32-67、viridithas@13a3fe1:src/timemgmt.rs:101-129、Ethereal@0e47e9b:src/timeman.c:59-67、Obsidian@720614a:src/timeman.cpp:20-33、Alexandria@e8db34c:src/time_manager.cpp:21-35。
- 必須の一致: soft/hard の二段制限、残り手数による按分、反復開始前の完了時刻の予測、最善手の安定による早期停止は、いずれも公知の方式である。
- 実装固有の一致: なし。hard を「soft の定数倍」と「残り時間の割合」の小さい方で抑える形は YaneuraOu（5倍と30%）と同じ構造だが、初版の定数（4倍と25%）は YaneuraOu と異なり、SPSA 後の値（4.51倍と27%）も異なる。Alexandria の `maxtime = 0.76*time` と minase の加算時間の使用率76%は、別の項に掛かる数値の偶然の一致である。
- 序盤の係数は、Apery（GPL-3.0-or-later）の10手目未満0.1と立ち上げ40 plyを借りた値であることを設計書（time-management-efficiency.md:71）が明記する。ただし Apery は段階的な係数を重要度に掛けるのに対し、minase は線形の係数を秒読みの項だけに掛けるので、式の形が異なる。
- 反証: moves_to_go の式、秒読みの8割、30 ms の余裕、予測比2.63は、対照のいずれにもない。
- 判定: **2**。

## 判定4　時間管理（履歴上の不採用版 6d30d76）

- minase: 6d30d76:src/search/time_management.rs:1-183（2026-09-17、件名「Scale the time budget by Stockfish's position-adaptive coefficients」）。設計書 time-management-efficiency.md:132-135 は「Stockfishの`totalTime`に倣い」「4係数はStockfishの現行の定数をそのまま使い」と記す。STC で H0 となり不採用になった。マージ 9b1c3ac（2945d7e と 3e425dd の統合）は src をこの枝から取り込まず、コミットメッセージに「The rejected search stages stay in history only」とある。`git grep -E '2\.229|11\.48|75_?800|104_?510' a05478a` は0件であり、a05478a の木には残っていない。master の履歴からは到達できる。
- 対照: Stockfish@e7b67b9:src/search.cpp:287、:346、:575-634（5062aee では :296-636、HEAD でも同一）、src/misc.h:486-489（interpolate）。GPL-3.0-or-later。
- 実装固有の一致:
  1. fallingEval の式 `(11.48 + 2.30*(前回の平均評価値 − 評価値) + 1.1*(4反復前の評価値 − 評価値)) / 100` と、その範囲 [0.576, 1.728]。
  2. timeReduction を `interpolate(深さ − 最善手が最後に変わった深さ, 4.96, 18.79, 0.639, 1.712)` で求め、[0.629, 1.544] に制限する式。
  3. reduction の式 `(1.468 + 前回の timeReduction) / (2.284 * timeReduction)` と、前回の timeReduction を go の間で持ち越す構造（Stockfish の previousTimeReduction、minase の TimeHistory.time_reduction）。
  4. bestMoveInstability の式 `1.077 + 2.229 * totBestMoveChanges` と、反復ごとに累積値を半分にする処理（Stockfish の `totBestMoveChanges /= 2`、minase の `total_changes / 2.0 + changes`）。
  5. highBestMoveEffort の式 `clamp(interpolate(nodesEffort, 75800, 104510, 0.969, 0.714), 0.693, 0.838)` と、`nodesEffort = 最善手のノード数 * 100000 / 全ノード数` の正規化。
  6. 4要素の評価値の環状バッファ（Stockfish の iterValue[4] と iterIdx、minase の `scores: [i32; 4]` と score_index）で4反復前の値を参照する構造。
  7. 評価値の単位換算に Stockfish の PawnValue 208 を使う `units = 208 / pawn_value`。
  8. 同じ引数順の補助関数 interpolate(x, x1, x2, y1, y2) と、4係数の積を最適時間に掛けて `elapsed > min(total, hard)` で止める停止条件。
  これらは約20個の調整済み定数の組と補助構造の組であり、独立実装では一致しない。
- 反証: minase は積の正規化定数 `TOTAL_SCALE = 0.48` を自前で加え、interpolate の内部で比を [0, 1] に制限する（Stockfish は外側で clamp する）。平均評価値は Option<f64> で持つ。Stockfish の increaseDepth と searchAgainCounter（設計書の段階2）は実装されておらず、`git log --all -G 'search_again|increase_depth'` は0件である。
- 判定: **1（翻案を支持する証拠あり）**。ただし対象は履歴上のコミットだけであり、監査対象の木 a05478a には含まれない。出典はコミットの件名と設計書で明示されている。

## 判定5　時間管理（履歴上の不採用版 551ea76 の最終押し込み）

- minase: 551ea76（2026-09-17）の clock_budget。`0 < remaining < 1.2 * byoyomi` のとき安全上限まで使う規則を「YaneuraOu's final push」としてコミットメッセージに記す。9b1c3ac で src は取り込まれず、a05478a には残っていない。
- 対照: YaneuraOu@HEAD:source/timeman.cpp:292-298（GPL-3.0-only）。
- 実装固有の一致: 1.2倍という定数1個の規則だけであり、コードの表現は異なる。
- 判定: **2**（出典を明示した単一の規則。履歴上のみ）。

## 判定6　先読み（ponder）と src/search/handle.rs

- minase: a05478a:src/search/handle.rs:40-52（ponderhit）、:91-138（start_search）、deepening.rs:47-78（的中後の反復開始の当て直し）、src/protocol/usi.rs:552-567、:1348-1362。導入は 817963a（2026-09-20）。参照先は ponder.md:296-300（Stockfish e7b67b9、YaneuraOu c1b80ea、HaChu、cutechess）。
- 必須の一致: `go ponder`、`ponderhit`、`bestmove X ponder Y` は USI の規定である。
- 実装固有の一致: なし。
- 反証: minase は経過時間を的中の時点から測り（hit_ns の原子値に u64::MAX を番兵として置く）、Stockfish と YaneuraOu の「探索の起点から測り、USI_Ponder で目標を25%増やす」方式を採らない。stopOnPonderhit も持たず、HaChu と同様に的中後の最初の検査点で進行中の反復を当て直す。予想手は PV の2手目だけで、置換表から補わない。
- 判定: **2**。

## 判定7　limits.rs、events.rs、snapshot.rs、stats.rs

- 検証済みの制限型、探索イベント、根局面の写し、マクロで生成する探索統計である。統計の項目（best_move_rank_1 から 4_plus など）は自前の診断設計（debugging-tools.md）に従う。
- 実装固有の一致: なし。判定: **2**。

## 判定8　src/protocol/usi.rs と src/protocol/engine.rs

- 必須の一致: USI のコマンド名（usi、isready、setoption、usinewgame、position、go、stop、ponderhit、gameover、quit）、`info depth ... score ... nodes ... nps ... time ... pv` の形式。`USI_Hash` と `USI_Variant` は USI と lishogi の相互運用の名前である。
- 相互運用のために借りた名前（コードではない）: `Threads` は Stockfish と YaneuraOu の慣例名。`ResignValue` の名前、意味、範囲はやねうら王に合わせたと usi-resignation.md:71 が明記する（既定値は 20000 で、YaneuraOu の 99999 と異なる）。
- `d` コマンドの盤面表示（288683e、2026-09-27）は、Stockfish の罫線の表、YaneuraOu の PRETTY_JP 表示（後手の駒に `^` を付ける、YaneuraOu@HEAD:source/position.cpp:177-180）に合わせたことを debugging-tools.md:149 とコミット件名が明記する。表示の慣例だけの一致であり、空升の記号（minase は「・」、YaneuraOu は「口」）、盤の向き、付帯行（zobrist、rights-zobrist など）は異なる。コードは中将棋の駒種に対する新規の Rust 実装である。
- engine.rs は中将棋の規則選択、対局状態、基底手数の管理であり、対照に相当する構造はない。
- 判定: **2**。

## 判定9　src/notation/（usi.rs、sfen.rs、cecp.rs）

- lishogi 系拡張 USI の指し手表記（2升または3升の連結、`+` 接尾辞）と、shogiops 互換 SFEN の駒文字表（shogiops@e295794:src/sfen.ts:270-355 の chushogiRoleToForsyth と同じ割当て）は、lishogi が定めた形式との互換のための必須の一致である。先獅子、手数、成り権保留の拡張欄は minase 独自である。cecp.rs は HaChu と XBoard の表記である。
- 実装固有の一致: なし。判定: **2**。

## 判定10　src/bin/minase（main.rs、io_log.rs）

- clap による `--protocol`、`--rules`、`--io-log` の起動引数と、時刻付きの入出力ログである。Stockfish と YaneuraOu はログを USI オプションで有効にするが、minase は握手前も記録できるように起動引数にしたと設計書が記す。
- 実装固有の一致: なし。判定: **2**。

## まとめ

| モジュール | 判定 | 理由 |
|---|---|---|
| 置換表（tt.rs） | 2 | 2語の原子型、直接写像、8ビット世代の独自配置。詰み値の補正だけが公知の定石として一致する |
| Lazy SMP・結果の採用（team.rs、deepening.rs） | 2 | 最深ワーカーの採用と周期による深さ飛ばしは、Stockfish と YaneuraOu の投票や SkipSize 表と異なる |
| 時間管理（a05478a の採用版、time.rs、params.rs） | 2 | 式と定数は自前の設計と SPSA による。序盤の係数は Apery の値を借りたと明記され、式の形が異なる |
| 時間管理（履歴上の 6d30d76） | 1（履歴のみ） | Stockfish の4係数の式、約20個の定数、環状バッファ、累積の半減、interpolate を翻案している。出典は明示。a05478a の木には無い |
| 時間管理（履歴上の 551ea76 の最終押し込み） | 2 | YaneuraOu の1.2倍規則1個だけ。出典は明示。a05478a の木には無い |
| 先読み（handle.rs ほか） | 2 | 的中時点からの計時と反復の当て直しは、Stockfish と YaneuraOu の方式と異なる |
| limits、events、snapshot、stats | 2 | 独自の型設計と診断項目 |
| protocol/usi.rs、engine.rs | 2 | USI の規定名、相互運用の名前（Threads、ResignValue）、表示慣例（d の `^`）だけが一致し、いずれも出典が明示されている |
| notation/ | 2 | lishogi と shogiops の形式との互換だけ |
| bin/minase | 2 | 独自の起動引数とログ |
