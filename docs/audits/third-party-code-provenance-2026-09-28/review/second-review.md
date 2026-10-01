# M5 再判定（独立の第2レビュー）

他のレビュアーの5本の記録（search.md、infrastructure.md、core.md、harness-and-statistics.md、training-and-evaluation.md）は、本記録の判定を書き終えるまで読んでいない。

## 判定表

| 項目 | 判定 | 一言の理由 |
| --- | --- | --- |
| 1. src/stats.rs（LLRとMLE） 対 fishtest LLRcalc.py | 1 | 関数名3つ（`secular`、`results_to_pdf`、`mle_expected`）、(値, 確率)の組の分布表現、区間端の余白1e-9が一致し、同じ数式の独立実装であるfastchessはいずれも別の選択をしている |
| 2. params.rs／pruning.rs 対 akimbo・Hobbes | 3 | feature `tuning`、`AtomicI32`の`Relaxed`読み、非調整版の`#[inline]`リテラル関数、`stringify!`の照合による設定という骨格がakimbo/Hobbesと一致し、設計書も両者を手本と明記するが、マクロの記述は大きく異なる |
| 3. 6d30d76 time_management.rs 対 Stockfish search.cpp | 1 | 調整済み定数22個の組、式の構造、4要素の環状履歴、最善手交替回数の半減が一致し、コミットと設計書がStockfishの現行値の転用を明記する |
| 4. diagonal.patch の tools/generate_diagonal_magics.rs 対 Stockfish init_magics／PRNG | 3 | xorshift64*と3回ANDの疎な乱数は公知の定型だが、`epoch`の世代カウンタとその名前はStockfish系統にだけ見られる実装側の選択であり、研究文書もStockfish 17の`init_magics`を参照している。他の構造は方向別の射線に合わせた独自のもので、1には足りない |

## 項目1　src/stats.rs 対 fishtest LLRcalc.py

- minase側: a05478a:src/stats.rs:7-121（`REGULARIZATION`〜`gsprt_decision`）。導入はb5c1e91（2026-08-11「探索部フェーズ1の自己対局ハーネスとペンタノミアルGSPRTを実装」、codexへ委任）。導入時点で既に同じ構造であり、以後の変更はH1の値の変更（17cdd68）と`midpoint`化、文書コメントの追加だけである。
- 設計書: b5c1e91:docs/plans/search.md:19 は「fishtestの`LLR_logistic`（statistic="expectation"のMLE法）と同一のアルゴリズム」「fishtest本家LLRcalc.pyから事前計算した参照値5件と1e-6以内で一致」と記す。ファイル冒頭のdocコメントも同じ趣旨を述べる。
- 対照側: official-stockfish/fishtest の server/fishtest/stats/LLRcalc.py。minase導入日（2026-08-11）に、コミット日時順で直前にあたるmasterの先端はb571c90（2026-08-02）である。作成日2026-07-20の93fe81e（「Fix for issue 2571」）はb571c90の祖先ではなく、その後にmasterへ入った。b8eecff（2026-08-22）のLLRcalc.pyは93fe81eと同一である。b571c90と93fe81eの差は`stats`のepsilonと`LLR_drift_variance`系だけであり、`secular`、`MLE_expected`、`regularize`、`results_to_pdf`、`LLR_logistic`は3版とも同一である。行: 24-46（secular）、54-71（MLE_expected）、130-149（LLRjumps/LLR）、212-240（L_、regularize、results_to_pdf、LLR_logistic）。ライセンス: manifestでは none（LICENSEなし、ファイルヘッダなし）。
- 数式が強制する部分（必須の一致、証拠にしない）: Van den Bergh "The generalized likelihood ratio for the expectation value of a multinomial distribution"（cantate.be/Fishtest/support_MLE_multinomial.pdf、fishtestが参照）の命題1.1は、式(1.2) p_i = p̂_i / (1 + θ(a_i − s))、式(1.3) Σ p̂_i (a_i − s)/(1 + θ(a_i − s)) = 0、および根の区間 [−1/(a_N − s), 1/(s − a_1)] を与える。したがって、区間端の式、MLE分布の式、LLR = N·Σ p̂_i (ln p1_i − ln p0_i)、ロジスティックの`1/(1+10^(-elo/400))`、5分類の得点0, 0.25, 0.5, 0.75, 1は数式から決まる。論文は p̂_i ≠ 0 を仮定するので度数0の補正は必要だが、その値1e-3はfishtestの選択である。ただしminaseの契約はfishtestの参照値との一致なので、度数0を含む参照値を再現するには1e-3が必要であり、必須の一致に数える（fastchessも1e-3を使う）。GSPRT境界±2.944もα=β=0.05から決まる。
- 実装固有の一致:
  1. 関数名`secular`。論文は「secular equation」という語を使わず（本文は式(1.3)を「the equation」と呼ぶ）、fishtestのコードが付けた名前である。fastchessは同じ方程式を無名のラムダで解き、`mle`という名前だけを持つ。
  2. 関数名`results_to_pdf`はfishtestと同名であり、戻り値も(総数, 分布)の組という同じ形である。fastchessは総数と確率配列をその場で作る。
  3. `mle_expected`はfishtestの`MLE_expected`の小文字化である。
  4. 分布を(値, 確率)の組の列（`type Pdf = [(f64, f64); 5]`）で表す。fishtestの冒頭のdocstringが定めた表現である。fastchessは得点配列と確率配列を分けて渡す。
  5. 二分法の探索区間を両端から1e-9ずつ内側へ寄せる（`SECULAR_MARGIN`）。fishtestの`secular`の`epsilon = 1e-9`と同じ値・同じ位置である。二分法は区間端の特異点を評価しないので余白は数値的に不要であり、fastchessは余白を置かない（端の関数値に±∞を渡す）。
  6. 度数0の分類だけを1e-3に置き換える方式（fishtestの`regularize`）。OpenBenchは`max(1e-3, x)`で同値。これは上記のとおり必須側に寄るので、証拠としての重みは低い。
- 反証（異なる選択）: 根の求解がbrentqではなく二分法（停止幅1e-14）である。`secular`が期待値を引数に取り、シフトを内側で行う（fishtestは`pdf1`を先に作る）。`MLE_expected`の事後検証`assert`がない。`LLRjumps`、`LLR`、`stats`の分割がなく、LLRを1つの関数で計算する。H1とH0の仮説と境界を定数で持つ。`t_value`系（正規化Elo）を持たない。
- 他の実装との比較: fastchess（MIT）app/src/matchmaking/sprt/sprt.cpp:201-242は同じ論文から独立に実装し、ITP法、配列の分離、余白なしを選ぶ。OpenBench（GPL-3.0-or-later）OpenBench/stats.py:21-22 は「taken directly from Fishtest」と明記し、`secular`を含むfishtestの関数を移植している。つまり`secular`という名前はfishtestとその直接の移植にだけ現れる。
- 結論: 数式から決まる部分を除いても、fishtest固有の関数名3つ、分布の表現、余白の値と位置が一致し、LLR_logisticの処理経路（results_to_pdf → MLE_expected×2 → N×期待値）をRustへ移した翻案と判断する。明記された出典に沿う翻案であり、隠した複製ではない。対象は約60行の数値手続きで、表現の選択肢は狭い。著作物性とライセンス（fishtestはライセンス表示なし）の評価は本判定の範囲外である。

## 項目2　params.rs／pruning.rs 対 akimbo・Hobbes

- minase側: a05478a:src/search/alphabeta/params.rs:6-99（マクロと係数表）、a05478a:src/search/alphabeta/pruning.rs:21-38（LMR表と`lmr_base`）。導入は5464779（2026-09-22「Add the tunable search parameter table behind the tuning feature」、当時src/search/params.rs）。LMR表自体は73578d4（2026-09-06）で`const LMR_DIVISOR: f64 = 2.0`として導入され、5464779で百分率の整数係数に変わった。
- 設計書: 5464779:docs/plans/spsa.md:94 は「Rust製チェスエンジンのHobbesとakimboがこの形を採る」「Recklessは`static mut`を使うので採らない」と記し、311-312行でHobbesの`tunable_params!`（コミット1f3e466）、akimboの`util.rs`（f7dd767）、Reckless、Viridithasを参照先に挙げる。
- 対照側: jw1912/akimbo@f7dd767 src/util.rs:37-103、src/search.rs:27-28, 509-510（MIT）。kelseyde/hobbes-chess-engine@1f3e466 src/tools/utils.rs:3-73（MIT）。codedeliveryservice/Reckless@31d9cd6 src/parameters.rs:1-33（AGPL-3.0-only）。cosmobobak/viridithas@13a3fe1 src/search/parameters.rs（AGPL-3.0-only、構造体とパーサ生成マクロで別方式）。
- 実装固有の一致: (a) cargo feature名`tuning`、(b) 調整版を`static AtomicI32`の`load(Ordering::Relaxed)`で読む、(c) 非調整版を`#[cfg(not(feature = "tuning"))] #[inline]`の既定値リテラル関数とする、(d) 入れ子のモジュールに静的変数を置く、(e) `match name { stringify!($name) => ... }`で設定する、(f) LMRの除数を百分率の整数係数`lmr_divisor`とし、`f64::from(...) / 100.0`で割る（akimbo search.rs:510）。
- 必須または慣用に寄る点: `unsafe_code = "forbid"`の下で可変な大域係数を持つなら`AtomicI32`はほぼ唯一の選択である。`stringify!`による名前照合と非調整版の定数関数はRecklessも同じである。整数SPSAでは係数を百分率にするのが通例であり、`f64::from`はclippyの推奨形である。ln(depth)·ln(move)/Cの式は公知である。
- 反証: minaseは名前（`LmrDivisor`）とアクセサ（`lmr_divisor`）を分け、係数ごとに別モジュール`mod $accessor { static VALUE }`を作る（akimbo/Hobbesは1つの`mod vals`）。非調整版は`const fn`である。範囲検査付きの`Result`と`Error`列挙型を返し、`println!`で出力しない。`PARAMETERS`定数表を持ち、`list_params`、`print_params_ob`（OpenBench形式の出力）、step値を持たない。docコメントを係数ごとに通す。LMRは基底項（akimboの`lmr_base`）を持たず、表を`OnceLock`で1回生成する。
- 結論: 設計の骨格は明記された手本（MIT）から借りているが、マクロの本体とAPIは書き直されている。骨格の一致は慣用の範囲を少し超えるので判定3とする。手本のライセンスはMITであり、表示の要否は別途の論点である。

## 項目3　6d30d76:src/search/time_management.rs 対 Stockfish

- minase側: 6d30d76:src/search/time_management.rs:1-183（とくに89-91の`interpolate`、115-179の`complete_iteration`）。コミット6d30d76（2026-09-17「Scale the time budget by Stockfish's position-adaptive coefficients」）。同日のマージ9b1c3ac（第1親2945d7e）でファイルは消えたが、6d30d76はa05478aとorigin/masterの祖先であり、公開済みの履歴に残る。
- 設計書: 6d30d76:docs/plans/time-management-efficiency.md:72 は「Stockfishの`totalTime`に倣う」「定数はStockfishの現行値をそのまま使う」と記し、式と定数を全て書き出す。189行でStockfish `src/search.cpp`（2026年9月16日参照）を参照先に挙げる。コミット本文も`fallingEval * reduction * bestMoveInstability * highBestMoveEffort`を名指しする。
- 対照側: official-stockfish/Stockfish@5062aee（2026-08-10）src/search.cpp:584-636、src/misc.h:486-489（`interpolate`）。e7b67b9（2026-09-18）とHEAD 17a6c8f（2026-09-19）の同じ箇所（search.cpp:584-602）は定数と式が同一である。sf_17（search.cpp:431-443）は定数が異なる（1067、223、0.580、1.667、recapture項）ので、minaseの定数は2026年時点の版に一致する。ライセンス: GPL-3.0-or-later。
- 実装固有の一致:
  1. 調整済み定数の組: fallingEvalの11.48、2.30、1.1、/100、clamp 0.576〜1.728。timeReductionの補間4.96、18.79、0.639、1.712とclamp 0.629〜1.544。reductionの1.468と2.284。bestMoveInstabilityの1.077と2.229。highBestMoveEffortの補間75,800、104,510、0.969、0.714とclamp 0.693〜0.838。nodesEffortの100,000倍。22個の定数がすべて一致する。いずれもSPSAなどで調整された値であり、独立に同じ値に至ることはない。
  2. 式の構造: 4係数を`optimum`（minaseはsoft）に掛け、`elapsed > min(total, maximum/hard)`で完了反復の後に止める。
  3. 4反復前の評価値を4要素の環状履歴（Stockfishの`iterValue[iterIdx]`と`(iterIdx+1)&3`、minaseの`scores[score_index]`と`% 4`）から取る。
  4. 最善手の交替回数を反復ごとに半減して加算する（Stockfishの`totBestMoveChanges /= 2`）。
  5. 前の手の平均評価値（`bestPreviousAverageScore`）と前の手の`timeReduction`（`previousTimeReduction`）を手をまたいで持ち越し、初期値を1とする。
  6. 名前の対応: `time_reduction`、`previous.time_reduction`、`last_change_depth`（`lastBestMoveDepth`）、`nodes_effort`、`total`、`falling`、`instability`、`effort`。
- 反証（minaseの変更）: 評価値の単位を`208 / pawn_value`で換算する。積に正規化定数0.48を掛ける。交替回数はワーカー数で割らず主ワーカーだけを使う。`interpolate`の内側でclampする（Stockfishは外側）。合法手1つの500 ms上限と詰みの停止条件を持たない。固定時間などでは係数を1に戻す。これらはすべて設計書に独自の変更として記録されている。
- 結論: 調整済み定数の組と式の構造を明示的に移した翻案である。出典の明記はあるが、Stockfishは GPL-3.0-or-later であり、minaseの公開履歴に残る点を記録する。a05478aのツリー全体（src と docs）に`11.48`、`4.96`、`2.284`は残っていない（`git grep`で確認）。設計書 time-management-efficiency.md の第3段階の定数の記述も、その後の書き直しで消えている。

## 項目4　diagonal.patch の tools/generate_diagonal_magics.rs 対 Stockfish

- minase側: a05478a:docs/measurements/magic-bitboard-prototype/diagonal.patch:835-940（新規ファイル tools/generate_diagonal_magics.rs、103行）。導入は5a8a802（2026-09-21「Record magic bitboard feasibility experiments」）。パッチは採用されず測定記録として残る。
- 対照側: Stockfish sf_17 src/misc.h:158-185（`PRNG`、`sparse_rand`）、src/bitboard.cpp:145-219（`init_magics`）。HEADでは src/attacks.cpp:102-150 へ移り、同じ`epoch`と`sparse_rand`を使う。GPL-3.0-or-later。
- 必須または公知の一致: xorshift64*（シフト12、25、27と乗数0x2545F4914F6CDD1D＝2685821657736338717）はVignaの公知の乱数生成器で、EtherealやapeironのZobrist生成にも現れる。乱数3つのANDで疎な候補を作る方法はchessprogramming wikiのRomstadのコード（`random_uint64_fewbits`）以来の定型である。候補magicを全占有パターンで検証する手順はmagic bitboardの定義から決まる。
- 設計書・研究文書: a05478a:docs/research/magic-bitboard-primary-sources.md:18 が「Stockfish 17の表生成」として sf_17 相当のコミットe0bfc4b の src/bitboard.cpp#L145-L217（`init_magics`）を参照する。生成器の作者側がStockfishの生成手順を読んでいたことを示す。測定記録 docs/measurements/magic-bitboard-prototype.md（5a8a802）は生成器の参照先を挙げない。
- 実装寄りの一致: 試行回数を世代として`epoch`配列に記録し、表を試行ごとに消去しない工夫と、その`epoch`という名前。代案（試行ごとに`used`配列を消去する）が自然にあるので実装側の選択だが、コーパス中でこの工夫を持つのはStockfish系統（Fairy-Stockfish variant-nnue-tools、apeironのgenerate_magics.rs、GPL-3.0-only）だけであり、Stockfish系統でない実装（shakmaty、texel、Leorik、Gikou、apery_rust、Ethereal、Berserk）には見つからなかった。したがって独立の起源を示す例ではなく、Stockfishからの伝播を示す。
- 反証: minaseは方向別の射線（4方向×144升）ごとに、到達距離（`reach`）を値とする表を作る。Stockfishは駒種ごとに攻撃ビットボードを値とする。12×12盤の升を`(16·rank + file) % 64`で64ビットへ写す独自の写像を使う。Carry-Rippler列挙ではなく添字の部分集合列挙を使う。Stockfishの`popcount((magic*mask)>>56) < 6`の事前選別と、段ごとの最適シード表を持たず、単一のシードを使う。照合の分岐順も異なる（minaseは`epoch == attempt && 不一致`で失敗、Stockfishは`epoch < cnt`で登録し、そうでなければ比較）。
- 結論: 公知の定型に加えて、Stockfish系統固有の`epoch`の工夫と名前が一致し、研究文書がStockfish 17の`init_magics`を参照している。一方、値の型、盤の写像、列挙の方法、事前選別の有無、シードの扱いという主要な構造は独自であり、103行の試作生成器で採用もされていない。実装固有の一致は1つにとどまるので判定3とする。
