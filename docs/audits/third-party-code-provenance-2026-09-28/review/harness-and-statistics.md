# 対局ハーネスと統計の対照レビュー記録

担当範囲は minase a05478a の src/stats.rs、src/bin/spsa_runner/、src/harness/、src/bin/match_runner/、src/bin/match_report/、src/protocol/cecp.rs、および scripts/ の統計・SPSA関連である。対照は fishtest（ライセンスなし）、OpenBench（GPL-3.0-or-later）、fastchess（MIT）、cutechess（GPL-3.0-or-later）、spsa_simul（MIT）、fishutils（GPL-3.0-or-later）、YaneuraOu-ScriptCollection（MIT）、HaChu（public domain）である。

## 1. src/stats.rs（ペンタノミアルGSPRT、LLR_logisticのMLE、Elo推定）

判定は3（判定保留）である。

### 対象箇所
- minase: a05478a:src/stats.rs:7-121（LLR）、134-169（Elo推定）。導入は b5c1e91（2026-08-11「探索部フェーズ1の自己対局ハーネスとペンタノミアルGSPRTを実装」、Co-Authored-By: Claude Fable 5、設計書 search.md は実装をcodexへ委任と記す）。b5c1e91:src/stats.rs:3-105 の時点で現在と同じ構造であり、以後の変更はH1の値の引数化（17cdd68）、`midpoint`化、docコメント追加だけである。
- fishtest: 導入時点で有効な版は fea7160（2026-02-24）の server/fishtest/stats/LLRcalc.py。93fe81e（committer 2026-08-13）と引用の b8eecff（2026-08-22）は stats() の epsilon と LLR_drift_variance だけを変えており、secular、MLE_expected、regularize、results_to_pdf、LLR_logistic は3版で同一である（b8eecff:LLRcalc.py:24-46, 54-71, 216-240）。HEAD 4136e07 とも差分なし（3版と HEAD で secular、MLE_expected、L_ から LLR_logistic までの各範囲の md5 が一致することを確認した）。
- 設計書の参照先: b5c1e91 の search.md は「fishtestの`LLR_logistic`（statistic="expectation"のMLE法）と同一のアルゴリズム」「fishtest本家LLRcalc.pyから事前計算した参照値5件」と明記する。stats.rs:3-5 のモジュールコメントも同じ。fishtestのコードを参照したことは推測ではなく、文書で確定している。

### 行ごとの対応
| minase | fishtest | 種類 |
|---|---|---|
| `REGULARIZATION = 1e-3`、度数0だけを置換（90-101） | `regularize`: `epsilon = 1e-3`、0だけを置換（216-224） | 必須。参照値[0,3,65,4,2]との1e-6一致に必要。fastchess（MIT、sprt.cpp:73-76）も同じ置換をする |
| `results_to_pdf(results) -> (count, pdf)`（90） | `results_to_pdf(results)`: `return N, [...]`（227-231） | 実装固有（関数名と戻り値の順序） |
| `type Pdf = [(f64, f64); 5]`（値, 確率）のタプル列、かつ別に定数`SCORES` | pdfを`(ai, pi)`のタプル列で表す（8-12行の文書化） | 実装固有。`SCORES`定数があるので確率配列だけで足りる。fastchessは`scores`と`probs`の別配列 |
| `secular(pdf, expected)`（54-78） | `secular(pdf)`（24-46） | 名前は実装固有。Van den Bergh の論文（support_MLE_multinomial.pdf、取得して全文検索）は「secular」を使わず θ と Proposition 1.1 と書く。fastchessは「equation 1.3」と呼ぶ |
| 区間 `-1/max + 1e-9`, `-1/min - 1e-9`（55-58） | `lower_bound + epsilon`, `upper_bound - epsilon`、`epsilon = 1e-9`（28-42） | 区間の両端は数式から必須。余白1e-9は実装固有（下記） |
| 二分法、停止幅1e-14、値の符号で片側更新（60-77） | `scipy.optimize.brentq`（41-43） | 異なる（反証） |
| `mle_expected(pdf, expected)`: `p/(1+x(a-s))`（81-87） | `MLE_expected(pdfhat, s)`: 同式（54-71） | 式は必須、名前は実装固有 |
| LLR = count × Σ p̂(ln p1 − ln p0) をインラインで計算（104-116） | `LLRjumps` → `stats` → `LLR` → `LLR_logistic`（130-149, 234-240） | 式は必須。分解は異なる（反証） |
| `logistic_score`（49-51） | `L_`（212-213） | 式は必須、名前は異なる |

`SECULAR_MARGIN = 1e-9` は、二分法では機能上不要である。二分法は区間の中点だけを評価し、端点の極では評価しない。正則化により両端の確率が正なので、根は区間の内部にある。fastchess の ITP 法は端点の値に ±∞ を渡し、余白なしで同じ区間を扱う。brentq は端点で関数値を評価するので fishtest には余白が要るが、minase の余白はその要件の名残と読める。担当範囲で最も強い単一の実装固有の一致である。

### 実装固有の一致（まとめ）
1. 補助関数の名前と分割 `secular`／`mle_expected`／`results_to_pdf`。数式からは「乗数を解く→最尤分布→度数の正規化」の3段が自然に出るが、名前は fishtest のもので、論文にも fastchess にもない。
2. `results_to_pdf` の戻り値 `(count, pdf)` の順序。
3. pdf を（値, 確率）の組の列で持つ表現（`SCORES`定数と冗長）。
4. 区間余白 1e-9（二分法では不要）。

### 標準的な方式として除外した一致
零置換の定数1e-3は、度数0を含む参照値[0,3,65,4,2]と1e-6で一致させる設計上の要求から出所にかかわらず決まる。fastchess（MIT、fishtest を名指しせず論文を引用する別実装。ただし補助関数名 `regularize` と定数1e-3は fishtest と同じなので、fishtest を参照した可能性は残る）も、1e-3 の零置換、`-1/(max-s)`から`-1/(min-s)`の区間、区間を狭める求根法、得点配列、`total × mean(ln p1 − ln p0)` のインライン計算を、すべて同じ形で持つ。区間、得点配列、インラインのLLRは数式から決まる。`secular` の符号規則（値が正なら下端を上げる）も、下側の極で関数が +∞ に発散することから決まる必須の一致である。したがってこれらは fishtest 固有の表現ではない。

### 反証（自由な選択で fishtest と異なる側を選んだ箇所）
- 求根法: 二分法（停止幅1e-14）対 brentq。
- `secular` に期待値を渡して内部でずらす（fishtest は事前に `pdf1 = [(ai - s, pi)]` を作る）。
- `regularize`、`LLRjumps`、`stats`、`L_`、`uniform` に当たる補助関数がない。
- `MLE_expected` の検証用 assert（`abs(s - s_) < 1e-6`）、`stats` の範囲検査、`converged` 検査がない。
- fishtest の docstring に対応するコメントがない（minase のコメントは日本語の独自記述）。
- 得点は定数 `SCORES`（fishtest は `i / (count - 1)`）。
- `estimate_elo`（146-169）は fishtest `stat_util.get_elo`（b8eecff:stat_util.py:52-68）と共有点がない。minase はペア単位の正規化得点の分散、正則化なし、±∞を返す、非対称の区間端を返す。fishtest は1局単位の分散、正則化あり、`elo()` で[1e-3, 1−1e-3]にクランプ、対称の ±elo95 と LOS を返す。OpenBench（stats.py:74-88）は t 分布を使う。minase の 1.96 は正規近似の定数である。
- GSPRT 境界 ±ln(19) と3値判定は Wald の SPRT の標準形であり、必須。
- テストの参照値10件は fishtest のコードを実行して得た出力値であり、コードの表現の複製ではない。モンテカルロ検証（297-377）は fishtest に対応物がない。

### 結論
コードの骨格（関数分割と識別子）と区間余白は fishtest から取り、求根法の表現、補助関数、検証、コメントは作り直している。対象は fishtest の約30行である。文書どおり「fishtest を参照した再実装」であり、逐語的な翻訳ではない。ただし識別子と分割の一致は独立実装では説明しにくく、1の判定には足りないが2にも下げられない。fishtest にはライセンスがないため、担当範囲で最優先の指摘とする。対処としては、識別子を論文の用語（θ、Proposition 1.1）へ改め、`SECULAR_MARGIN` を除き、帰属表示を付けることが考えられる。

## 2. src/bin/spsa_runner/（更新則、利得、パラメーター処理、apply、模擬比較）

判定は2である。

- minase: a05478a:src/bin/spsa_runner/model.rs:70-77（`rates`）、97-104（`round_pair`）、207-218（`update`）、params.rs:134-183（6欄の係数ファイル）。導入は 175b97d（2026-09-22、model.rs:30 の `rates`、tests.rs:156 `rates_match_independent_fishtest_reference`）。指数の既定は 9b85505（2026-09-25）で C5 に確定した。
- fishtest b8eecff:server/fishtest/spsa_workflow.py:430-432（`c = c_end·N^γ`、`a_end = r_end·c_end²`、`a = a_end·(A+N)^α`）、486-501（`c/iter^γ`、`R = a/(A+iter)^α/c²`）、504-525（`θ += R·c·result·flip` とクランプ）、worker/games.py:1379, 1388（`floor(value + uniform)`）。
- OpenBench spsa_utils.py:68-72, 149-168（同じ c_end/r_end の式、θ+とθ−で共有する確率的丸め、整数の摂動幅の下限0.5）。
- 設計書 spsa.md:104-111, 186-191, 298-306 は、fishtest の更新式、ワーカーの丸め、OpenBench の実装、6欄の入力形式を「更新則だけを借りる」「fishtestの入力形式と同じ」と明記する。
- 一致: c_end／r_end による SPSA の再パラメーター化、α = 0.602、γ = 0.101（Spall の標準値）、A = 0.1N、6欄の書式（Stockfish tune.cpp の出力形式でもある）、`floor(x+u)`。いずれも数式、公開の入力形式、または既定値の採用であり、fishtest と互換にする設計から必然的に同じになる。
- 反証: 各項をその場で計算し、fishtest のように `c`、`a` を事前に保存しない。更新は `(a/c)·d·flip` で、`R·c` の形を取らない。反復を8ペア単位で数える（fishtest はペアごと）。丸めの乱数を θ+ と θ− で共有する（fishtest は別々に引き、OpenBench と同じ）。OpenBench の下限0.5は採らない。flip は反復ごとにシードから決定的に作り、fishtest の packbits と CRC32 署名はない。
- apply.rs:141-291 は `parameters! {` ブロックを行単位で解析し、既定値のバイト範囲を置換する。fishutils.py と YaneuraOu-ScriptCollection の SPSA/tune.py:222-278 は正規表現と `@` や `%%` の目印で置換する。方式が異なる。
- tests/simulation.rs は2次の損失曲面を実測信号から作り、ペンタノミアル度数で結果を抽出する。spsa_simul の spsa_sim.c:6-35 は draw_ratio 付きの3分類の対局模擬と lf_eval による損失である。2次曲面という考え方は理論文書（spsa-gain-calibration.md:159 が引用）の公知のモデルであり、コードの構造は共有しない。
- ライセンス: fishtest はなし、OpenBench は GPL-3.0-or-later、spsa_simul は MIT、fishutils は GPL-3.0-or-later、ScriptCollection は MIT。

## 3. src/harness/（プロセス管理、USI・CECPのドライバー、時計、裁定、記録）

判定は2である。

- minase: a05478a:src/harness/engine/process.rs:40-120（`std::process` と読み取りスレッド）、engine/cecp.rs:25-151（`usermove`、`time`、`otim`、`go`、`ping`、`pong` まで読む）、58-75（`level 0 m[:ss] inc`、`st`）、engine/usi.rs:9-153（`info` と `option` の解析）、clock.rs:30-37（`elapsed > remaining + byoyomi` で時間切れ）。
- cutechess xboardengine.cpp:39-48（`msToXboardTime`）、164-177（`st`／`level`）、253-268（`time`／`otim`）。
- 一致: `level` の分または分:秒の書式、`time`／`otim` のセンチ秒、`usermove`、`ping`、`force`、`memory`、`easy`。いずれも CECP 仕様の書式であり、必須。
- 反証: minase は `post` の代わりに `nopost` を送り、`ping` を毎手の `go` の直後に送って `pong` までを1応答とする。cutechess は機能交渉に従い `ping` の有無で分岐する。時計には余白がない（cutechess と fastchess は timemargin を持つ）。引き分けと投了の裁定もなく、ResignValue 99999 で投了を無効化し、手数上限ではペアごと破棄する。これらは cutechess と fastchess の裁定機能と異なる。USI（`btime wtime binc winc byoyomi`）は仕様どおりであり、cutechess と fastchess は USI を実装しない。
- 設計書 match-harness.md:64-75, 144 は fishtest と fastchess の構造（ペア対局、異常時の負け帰属）を「倣う」対象として挙げるが、これは方式の採用であってコードの対応はない。
- 導入: b5c1e91 の src/bin/selfplay.rs、f7a228d と b19afb9 の src/bin/match_runner.rs（2026-08-11）、3f2c5c8（2026-09-22）で src/harness.rs へ移動し、後に分割した。旧版を `margin`、`adjudic`、`resign`、`draw`、`kill`、`timeout`、`fishtest`、`cutechess`、`fastchess` で検索した。b19afb9:src/bin/match_runner.rs:338, 741 の `Adjudicated` は審判層（minase の規則実装）が判定した終局を指し、評価値による裁定ではない。63d0f69^:src/harness.rs にも時計の余白や評価値裁定はなく、`timeout` は応答期限（既定120秒）だけである。旧版にも cutechess／fastchess の慣用は見当たらない。

## 4. src/bin/match_runner/、src/bin/match_report/

判定は2である。match_report は 3b35793（2026-08-28）で導入し、e4e9265（2026-09-27）で分割した。summary.rs:5-80 は stats.rs を呼ぶだけである。report.rs:378-396 の分散・時間積（`variance × CPU時間`）と compare.rs は minase 独自の校正指標であり、対照側に同じ量はない。保存、再開、ロックの形式は独自である。

## 5. src/protocol/cecp.rs（エンジン側のCECP）

判定は2である。導入は 4d4bf33（2026-08-10）。a05478a:src/protocol/cecp.rs:508-533 の `feature` 群（`setboard`、`usermove`、`ping`、`colors=0`、`sigint=0`、`sigterm=0`、`memory=1`）は HaChu hachu.c:3001-3010 と重なるが、XBoard／HaChu と相互運用するための CECP 機能交渉であり、必須である。書式は1機能1行で、HaChu の1行まとめとは異なる。HaChu は public domain なのでライセンス上の問題はない。

## 6. scripts/*.py

判定は2である。12本のうち SPRT、SPSA、Elo 推定を実装するものはない。bench_compare.py は `statistics.median` だけを使う。

## 判定の表

| モジュール | 判定 | 理由 |
|---|---|---|
| src/stats.rs（LLR） | 3 | fishtest の補助関数名と分割（secular、mle_expected、results_to_pdf）、(count, pdf)、組の列の pdf、不要な余白1e-9が一致する。求根法、補助関数、検証、コメントは別物である。fishtest はライセンスなし |
| src/stats.rs（Elo推定） | 2 | 正規近似の標準式。fishtest get_elo、OpenBench Elo とは分散単位、クランプ、区間形がすべて異なる |
| src/bin/spsa_runner/ | 2 | 更新式、既定値、6欄書式、確率的丸めは fishtest／OpenBench 互換の必須の一致。コード表現は独自 |
| src/harness/ | 2 | CECP、USI の仕様どおり。余白なしの時計、裁定なしなど、cutechess／fastchess と異なる選択 |
| src/bin/match_runner/、match_report/ | 2 | 統計は stats.rs を呼ぶだけ。校正指標は独自 |
| src/protocol/cecp.rs | 2 | feature 群は CECP の必須項目。HaChu は public domain |
| scripts/*.py | 2 | 統計、SPSA のコードを含まない |
