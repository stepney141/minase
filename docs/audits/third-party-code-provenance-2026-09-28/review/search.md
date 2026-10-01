# 探索部（src/search/alphabeta/）の対照レビュー記録

対象は minase の基準コミット a05478a の `src/search/alphabeta/`（試験を除く本体 約3,700行）と、削除または取り消し済みの探索機能の各導入版である。
対照は Stockfish（手元複製の 17a6c8f と 0a215d6、タグ sf_10、sf_18、sf_19）、YaneuraOu（c1b80eaa と v9.40）、Ethereal 0e47e9b、Obsidian 720614a、Alexandria e8db34c、Reckless 31d9cd6、viridithas 13a3fe1、akimbo f7dd767、Hobbes 1f3e466、rshogi 871a5b8、apery_rust 8e64bc4 である。
ライセンスは manifest.tsv の値を用いた。Stockfish と Ethereal は GPL-3.0-or-later、YaneuraOu、Obsidian、Alexandria は GPL-3.0-only、Reckless と viridithas は AGPL-3.0-only、akimbo と Hobbes は MIT、rshogi は GPL-3.0-or-later である。

## 全体の所見

minase の探索は、各機能を中将棋向けの独自の設計書の決定表に沿って組み立てており、係数はすべて丸い初期値（2.0、128、半歩兵単位の [1, 3, 3]、`2 + depth / 6` など）から始まり、自前の診断benchまたは自前のSPSAで現在値（166、111、101/196/207 など）へ移っている。
いずれの機能にも、対照エンジンの調整済み定数の組、変数名の組、補助関数の分割、またはコメントの対応は見つからなかった。
唯一の実装固有の一致は、係数の公開マクロとLMR除数の百分率表現が akimbo（MIT）の形に近い点であり、これは設計書が明示的に採用元として挙げている。

## 判定ごとの記録

### 1. 主探索の骨格（negamax、search_move、PVS再探索）

- minase: `src/search/alphabeta/negamax.rs:21-266`（negamax）、`:275-330`（search_move）。骨格の導入は 93b4bd4（2026-08-11）、LMR再探索の3段は 38bef0a（2026-08-22）。
- 対照: Stockfish sf_10 `src/search.cpp`、sf_19 同。
- 必須の一致: 零窓→減深解除→全窓の3段再探索、置換表の即時打ち切り、フェイルソフト。
- 反証: minase は置換表の命中で窓を狭めない（コメントで理由を明記）、詰み値を `-MATE + ply` と王駒捕獲の `MATE - (ply + 1)` で扱う中将棋固有の分岐、反復検出は探索経路と対局履歴のキー集合で行う。Stockfish/YaneuraOu のテンプレート化された `search<NodeType>` や `Stack` 構造とは分割が異なる。
- 判定: 2（標準的な方式で説明可能）。

### 2. null move pruning

- minase: `negamax.rs:78-115`、`pruning.rs:62-65`。導入 06f7fd6（2026-08-22）で `reduction = 2 + depth / 6`、5464779（2026-09-22）で `(切片 + depth × 傾き) / 1200` に書き換え（切片2,400、傾き200で旧式と厳密一致、`docs/plans/spsa.md` 98行と174行）、SPSAで 3529/238。
- 対照: Stockfish sf_10 `search.cpp:788`（`(823 + 67 * depth) / 256 + min((eval - beta) / 200, 3)`）、17a6c8f `search.cpp:1035`（`7 + depth / 3 + max((staticEval - beta) / 256, 0)`）、YaneuraOu c1b80eaa `yaneuraou-search.cpp:3252`（`7 + depth / 3`）、Ethereal `search.c:518`（`4 + depth / 5 + MIN(3, (eval - beta) / 191)`）。
- 実装固有の一致: なし。分母1,200は minase が `depth / 6` を整数で厳密に表すために選んだ値であり、Stockfish sf_10 の256とは由来が異なる。
- 反証: 詰み値の結果を β へ置き換える（YaneuraOu と Stockfish の現行版は `!is_win(nullValue)` で返却自体を避け、検証探索を持つ）。連続null禁止は `null_move_ply` の記録で実装し、王駒以外の駒の有無を条件にする中将棋固有の判定を持つ。
- 取り消した試行: 3c7ee60（2026-09-05、取り消し ca935e5）の `2 + depth / 6 + clamp((eval - beta) / (2 × 歩兵価値), 0, 3)` は Stockfish sf_10 の加算項（`min((eval - beta) / 200, 3)`）と同形である。この項は多くのエンジンに共通する公知の形であり、歩兵価値の2倍という表現も minase 固有の単位系による。34385c6（2026-09-28、取り消し 9904dfd）の「非PVかつ補正後評価が β 以上」は stage12 設計書項目7が Stockfish の Step 9 を挙げて採った条件である。
- 判定: 2。

### 3. futility pruning、reverse futility、razoring、late move pruning、improving

- minase: `negamax.rs:117-165`、`pruning.rs:40-49`。導入 a641083（2026-09-05）で余裕値は半歩兵単位の表 `[1, 3, 3]`、SPSA後は歩兵価値の百分率 101/196/207。王駒への利きがある局面を除外する条件（`royal_under_attack`）は中将棋固有である。
- 取り消した試行: 495c7f4 reverse futility（半歩兵1）、2643294 LMP（手番号上限 `[12, 23, 13]`）、3820f5f razoring（半歩兵 `[8, 8]`）、189f874 improving（1/8歩兵単位の `[[1, 6, 6], [4, 12, 12]]`）。
- 対照: Stockfish 17a6c8f `search.cpp` 1016-1021行、1193行（LMP閾値 `(3 + depth²) / (2 − improving)`）、YaneuraOu c1b80eaa 3225行。
- 実装固有の一致: なし。深さ別の表で余裕値を持つ形は Stockfish の連続式とも YaneuraOu とも異なり、値の組も一致しない。
- 判定: 2。

### 4. LMR（減深量表と history による増減）

- minase: `pruning.rs:14-38`（表）、`:67-82`（`lmr_reduction`）、`negamax.rs:185-194`。導入 38bef0a（2026-08-22、固定1）、73578d4（2026-09-06、`floor(ln d · ln m / 2.0)`、上限 `min(3, d − 2)`、history閾値128で ±1）。設計書 `docs/plans/strength-stage5.md` は Stockfish の `r ≈ 0.47 · ln d · ln m` を係数の探索範囲の中心としてだけ挙げ、係数は自前の診断benchで決めたと記す。
- 対照: Stockfish 17a6c8f `search.cpp:725`（`reductions[i] = int(2872 / 128.0 * log(i))`、1,024分率の連続補正）、sf_10 `Reductions[2][2][64][64]`、Ethereal `search.c:155`（`0.7844 + log(depth) * log(played) / 2.4696`）、akimbo `search.rs:509-510, 607`（`lmr_base + ln(depth) / (lmr_divisor / 100) · ln(legal)`）。
- 実装固有の一致: 除数を百分率の整数 `lmr_divisor` で持ち `f64::from(divisor) / 100.0` で割る表現が akimbo と同じである（params の項で扱う）。
- 反証: minase の表は定数項を持たず `floor` で丸め、`[MAX_PLY + 1][256]` の `OnceLock` 表に保持する。history 補正は閾値による ±1 の3値で、Stockfish や Ethereal の連続的な `history / K` を設計書が明示的に不採用としている。
- 判定: 2。

### 5. internal iterative reduction

- minase: `negamax.rs:70-76`、導入 1797322（2026-09-10）。深さ3以上で置換表の手がなければ1だけ減深。
- 対照: Stockfish 17a6c8f の `depth -= 3`（PV）など、YaneuraOu c1b80eaa 3309-3342行、Ethereal。
- 反証: 条件（深さ3以上、記録手なし、減深1）はどの対照とも一致しない。
- 判定: 2。

### 6. SEE と SEE による枝刈り

- minase: `see.rs:24-140`（`see_prunes`）、`:148-240`（試験用参照実装）、`:270`（`least_valuable_attacker`）、`negamax.rs:166-184`、`pruning.rs:51-60, 84-92`。導入 1894d14（2026-09-05、静止探索）と c1c7227（2026-09-19、主探索）。
- 対照: Stockfish 17a6c8f `src/position.cpp:1389` の `see_ge`、YaneuraOu c1b80eaa `source/position.cpp:2553` の `see_ge`、Reckless `src/board/see.rs:12-`、Hobbes `src/search/see.rs:33-`、rshogi `movepicker_support.rs:489-`。
- 必須の一致: 利得配列による交換列の逆算 `gains[d] = v − gains[d − 1]` と `gains[d − 1] = −max(−gains[d − 1], gains[d])`（`see.rs:86, 134`）は Chess Programming Wiki「SEE – The Swap Algorithm」の公開擬似コードそのままの形である。関数名 `least_valuable_attacker` は一般的な名称で、Reckless、Hobbes、rshogi にも現れる。
- 反証: 対照エンジンはすべて閾値型（`balance` と `threshold`）で、利得配列を持たない。minase は獅子規則への依存を判定不能とし、成りの差額を交換列へ加え、`depth == 1` の早期終了で逆引きを省くなど、中将棋固有の分岐が大半を占める。主探索の余裕値は深さ別の百分率 2/210/7 で、Stockfish の `−margin × depth` とは形が異なる。
- 判定: 2。

### 7. 着手順序と MovePicker、killer、butterfly history

- minase: `ordering.rs:14-298`、`searcher.rs:28-35`。killer と history の導入 5a5ae33（2026-08-22）、`HISTORY_LIMIT = 1 << 14`、加点 `depth²`、上限超過で全体を半減。段階式の MovePicker は 7877531 と movegen-speedup-2 段階6。
- 対照: Stockfish sf_19 `movepick.cpp`（`MAIN_TT`、`CAPTURE_INIT`、`GOOD_CAPTURE`、`QUIET_INIT`、`BAD_CAPTURE`）、viridithas `movepicker.rs:27-35`、Reckless `movepick.rs:10-16`。
- 必須の一致: TT手、捕獲、killer、静かな手の順、MVV-LVA、killer 2枠のずらし、depth² 加点と半減による古典的な history（Chess Programming Wiki の history heuristic）。
- 反証: 段階名（`Tt`、`Captures`、`Killer0`、`Killer1`、`Quiets`、`Done`）は対照のいずれとも一致しない。捕獲は部分選択ではなく全体整列で、`Reverse` キーの `sort_unstable` で静かな手を整列する。現行の Stockfish 系の gravity 更新や continuation history は持たない。
- 取り消した試行: c002357 malus と gravity（`v += bonus − v·|bonus| / LIMIT`、設計書自身が「Stockfishなどが用いる標準の更新式」と明記）、6084078 piece-to 表、1ca19fc capture history（`[駒][到達升][被捕獲駒種]`、Stockfish 4bc11984 を設計書が参照元として明記）、b4bcd95 負のSEEの捕獲を後回し、1b96a3d 対局内の history 持ち越し（Stockfish ced9f698 を明記）。いずれも規則は参照元と同じだが表現は書き直されており、定数は minase の単位系による。
- 判定: 2。

### 8. 静的評価の補正（correction history）

- minase: `correction.rs:11-82`、`negamax.rs:241-262`。導入 a000948（2026-09-20）で、表は4,096要素、固定小数点1,024倍、更新 `fixed += (diff·1024 − fixed) · min(depth, 8) / 32`、上限は歩兵2枚。鍵は駒コードごとの splitmix64 定数の加算による枚数ハッシュである。直前の調査記録 `docs/research/forward-pruning-prior-art.md`（72fc171、2026-09-19）が Stockfish 17a6c8f と YaneuraOu 1308ab3 の更新条件を引用している。
- 対照: Stockfish 17a6c8f `search.cpp:1654-1660`（`bonus = clamp(diff · depth · k / 128, ±LIMIT/4)` と gravity、鍵は pawn や non-pawn の Zobrist）、YaneuraOu c1b80eaa 748行と4417行、akimbo `tables.rs:367-378`（`GRAIN = SCALE = 256`、`min(depth + 1, 16)` の加重平均）、Alexandria `history.cpp:186-214`、Obsidian `history.h:9`。
- 必須の一致: 王手中と最善手が捕獲のノードを除き、上限か下限の向きが補正の向きを確定するときだけ更新する条件（調査記録で Stockfish と YaneuraOu から学んだと明記）。指数移動平均による更新も公知の形である。
- 反証: 全コーパスを `.min(8)`、`min(depth, 8)` で検索しても補正に使う例はなく、4,096要素、1,024倍、`/ 32` の組も一致しない。材料鍵は Stockfish の旧 `materialKey`（駒種と枚数の Zobrist 排他的論理和）ではなく加算ハッシュであり、Stockfish は材料鍵を外している（設計書が 7f386d10 を挙げて記録）。
- 判定: 2。

### 9. 静止探索（stand pat、delta pruning、置換表、価値グループ生成）

- minase: `quiesce.rs:28-166`（本体）、`:169-476`（価値グループ単位の段階生成）。delta pruning の導入 facd37f（2026-08-22、余裕値200）。
- 対照: Stockfish sf_19 `qsearch`、YaneuraOu、Ethereal。
- 反証: 捕獲の対象升を価値順位のビット集合で持ち、通常駒と特殊駒（獅子など）の2列を併合する生成は minase 独自である。取り消した試行 a3845f2（捕獲数の上限と取り返しの例外）も独自の条件である。
- 判定: 2。

### 10. aspiration windows、反復深化、Lazy SMP の深さ飛ばし

- minase: `root.rs:15-73`、`deepening.rs:19-24`。aspiration の導入 75bb69d（2026-09-10、深さ5以上、初期幅は歩兵の50%、失敗ごとに2倍）、深さ飛ばしは 1cb255f（2026-08-22、周期 `2 + (k − 1) % 4`）。
- 対照: Stockfish 17a6c8f `search.cpp:392-432`（`failedHighCnt`、`delta += delta / 3`）、sf_10 の `SkipSize` と `SkipPhase` 表、YaneuraOu c1b80eaa 1670行と1795行。
- 反証: 窓幅と拡大率、周期の式はいずれも対照と異なる。取り消した試行 19b58f6（fail-high ごとに1減深、fail-low で0へ戻す、主ワーカーだけ）は Stockfish 081af908 の規則で、設計書が参照元と明記している。表現は minase の関数分割で書き直されている。
- 判定: 2。

### 11. 置換表（参考）

- minase: `tt.rs`。1スロット1エントリで、`critical` と `advisory` の2つの `AtomicU64` にビット詰めする。
- 対照: Stockfish の3エントリクラスタ、YaneuraOu の `tt.h`。
- 反証: 構造、フィールド配置、置換規則とも異なる（4エントリクラスタ案は 2767394 で試し 9d37246 で取り消し）。
- 判定: 2。

### 12. 調整係数の表（params.rs）

- minase: `params.rs:7-51`（マクロ）、`:53-99`（係数表）、`pruning.rs:35-38`（`lmr_base`）。導入 5464779（2026-09-22）。
- 対照: akimbo f7dd767 `src/util.rs:37-105`（`tunable_params!`）、`src/search.rs:18-28, 509-510`、Hobbes 1f3e466 `src/tools/utils.rs:3-73`、Reckless 31d9cd6 `src/parameters.rs:1-35`、viridithas 13a3fe1 `src/search/parameters.rs`。
- 設計書: `docs/plans/spsa.md` 94行が「Rust製チェスエンジンのHobbesとakimboがこの形を採る」と記して採用し、Reckless の `static mut` を不採用とする。302行と303行が akimbo `util.rs`、Hobbes、Reckless、viridithas をコミット付きで挙げる。
- 実装固有の一致:
  - cargo feature 名が `tuning` である（akimbo と Hobbes も `tuning`。Reckless は `spsa`）。
  - 1つのマクロが係数ごとに `cfg(not(feature = "tuning"))` の既定値関数と `cfg(feature = "tuning")` の `AtomicI32` 読み出し関数の2通りを生成し、両方に `#[inline]` を付け、`Ordering::Relaxed` で読み書きする。
  - 宣言が「名前、既定値、最小値、最大値」の組である（akimbo は刻み幅も持つ）。
  - LMR の除数を100倍の整数 `lmr_divisor` で持ち、`f64::from(divisor) / 100.0` で割る。73578d4 の `LMR_DIVISOR: f64 = 2.0` を 5464779 で調整可能にしたときに生じた表現で、akimbo `search.rs:510` の `(f64::from(lmr_divisor()) / 100.0)` と同じである。
  - 別の書き方としては、`static mut`（Reckless）、構造体への集約（viridithas の `Config`）、`OnceLock` や設定オブジェクトの受け渡し、千分率や浮動小数の直接保持があり得た。
- 反証:
  - 通常版は `const fn` であり、akimbo と Hobbes は通常の `fn` である。
  - 値は係数ごとの入れ子モジュール `mod $accessor { static VALUE }` に置き、akimbo と Hobbes の共有モジュール `mod vals { static $name }` と異なる。
  - `set` は範囲を検査して `Result<(), Error>` を返し、誤りを列挙型で表す。akimbo と Hobbes は範囲を検査せず `println!` で未知名を報告する。
  - 宣言は `PARAMETERS` 定数スライスで公開し、USIの `option` 行を直接出力する `list_params` や OpenBench 形式の `print_params_ob` は持たない。
  - USI名（CamelCase）とアクセサ名（snake_case）を別々に宣言する。
  - 係数の値、範囲、係数の集合は minase 固有である。
- ライセンス: akimbo と Hobbes はともに MIT である。
- 判定: 3（判定保留）。設計書が採用元を明記したうえで約30行の慣用形を採っており、実装固有の一致は複数あるが、式の表現は書き直されていて複製とまでは言えない。MIT の帰属表示が問題になるのは表現が実質的な部分の複製と判断される場合だけであり、設計書には既に出典の記載がある。

### 13. 削除済みの試験（src/search/tests.rs、最終版 b689184 の親）

- 4,653行を対照エンジン名とチェスのFEN表記で検索し、局面の定義箇所を抜き出して確認した。局面は12×12のSFENと中将棋の規則に依存するものであり、対照エンジンの試験局面や定数を流用した箇所は見つからなかった。
- 判定: 2。

### 14. YaneuraOu の日本語コメントとの対応

- YaneuraOu c1b80eaa の null move 節（3238-3300行）、IIR 節（3309-3342行）、補正の更新（4417行）のコメントを、minase の `negamax.rs` と `quiesce.rs` のコメントと照合した。
- minase のコメントは設計書の節名を参照する形式（「docs/plans/strength-stage6.md『internal iterative reduction』節」など）であり、YaneuraOu の「証明されていないmate scoreやTB scoreはreturnで返さない」などの訳注と対応する文はない。
- 判定: 2。

## モジュールごとの判定

| モジュール | 判定 | 理由 |
|---|---|---|
| negamax.rs（PVS、null move、IIR、futility、SEE枝刈り、LMR適用、補正更新） | 2 | 公知の手法の組合せで、条件と定数は中将棋固有の設計書と自前の調整に由来する |
| pruning.rs（LMR表、余裕値、null move 減深） | 2 | 対数積の表と線形の減深は公知の形。除数の百分率表現だけが akimbo と同じで、params の項で扱う |
| ordering.rs（MovePicker、killer、history） | 2 | 古典的な depth² と半減の history。段階名と整列方式は対照と一致しない |
| correction.rs | 2 | 更新条件は調査で学んだ公知の規則。鍵、定数、更新式の表現はいずれの対照とも一致しない |
| see.rs | 2 | Chess Programming Wiki の swap の擬似コード。対照エンジンはすべて閾値型 |
| quiesce.rs | 2 | 価値グループ単位の生成は独自。stand pat と delta pruning は公知 |
| root.rs、deepening.rs | 2 | 窓幅、拡大率、深さ飛ばしの式が対照と異なる |
| tt.rs | 2 | 2語のビット詰め1エントリ構造で、Stockfish のクラスタと異なる |
| params.rs | 3 | 設計書が akimbo と Hobbes（MIT）の形を採用元と明記し、feature 名、二重アクセサ、除数の百分率表現が一致する。表現は書き直されている |
| 取り消し済みの試行13件（RFP、LMP、razoring、null move の評価項、improving、malus と gravity、piece-to、capture history、負のSEE後回し、静止探索の上限、aspiration の fail-high 減深、history 持ち越し、非PVの null move） | 2 | 規則は設計書が挙げた Stockfish 等の導入版に倣うが、表現と定数は minase の単位系で書き直されている |
| 削除済み src/search/tests.rs | 2 | 中将棋固有の局面だけ |
