# ゲームの規則実装を形式検証した先行例とRustの検証器の調査

本書は、盤上ゲームの規則と合法手生成を形式検証した先行例、Rustのコードを検証する道具の現状、および仕様と実装を突き合わせる手法の先例を、一次資料で調べた結果をまとめる。
形式検証とは、プログラムが仕様を満たすことを、テストの標本ではなく、数学的な証明または全入力の機械的な検査で示すことをいう。
調査日は2026年11月25日であり、ウェブ資料とリポジトリはすべてこの日に確認した。
二次資料でしか確かめられなかった事実には、その旨を書く。
調べた事実を中将棋エンジンminaseへどう当てはめるかは、[規則実装の形式検証の実現可能性と検証を強める候補](../proposals/rules-formal-verification.md)に書く。

本書で使う用語を先に定める。
証明支援系は、人が書いた証明を小さな検査プログラム（核）が検査する道具であり、Lean 4、Isabelle/HOL、Rocq（旧名Coq）、およびHOL4がこれに当たる。
合法手生成器の健全性は生成された手がすべて規則上合法であること、完全性は規則上合法な手がすべて生成されることをいう。
信頼の基底は、証明や検査の結論が正しいと言うために、検証されないまま信頼する部分をいう。
SATソルバは論理式を真にする割り当てがあるかを判定するプログラムであり、SMTソルバはそれを整数やビット列の式へ広げたものである。
有界モデル検査は、ループの回数などに上限を置いてプログラムを論理式へ展開し、上限の範囲の全入力をソルバで検査する方法である。
演繹的検証は、関数の事前条件と事後条件、およびループの各反復で成り立つ不変条件を論理式で書き、上限を置かずに全入力で仕様が成り立つことを証明する方法である。
ビットボードは、盤の升の集合を整数のビットの並びで表すデータ構造であり、多くのゲームエンジンが利きの計算に使う。
走り駒は、飛車や角行のように、ほかの駒に遮られるまで任意の升数を進む駒をいう。

## 結論

合法手生成器の健全性と完全性を証明支援系で証明した例は、本将棋を対象にしたlean-shogi（Lean 4、2026年9月）の1件だけが見つかった。
この例の定理は1局面の規則に限られ、千日手のような履歴に依存する規則は対象外である。
中将棋の規則の形式化は、どの証明支援系にも見つからなかった。
ゲームの規則を記述する言語では、汎用のゲームシステムLudiiに中将棋の定義があるが、作者が未完成と表示しており、終局条件が空欄で獅子の捕獲制限の記述がない。
したがって、今回確かめた範囲には、中将棋の規則を実行できる形で書いた、そのまま使える完成した記述はなかった。

調べた先行例に、規則の形式化が規則の原文に合っていることを証明したものはなかった。
先行例は、定義を短く読みやすく保って人が読んで確かめること、別実装と突き合わせること、または独立に計算された結果と照合することで、形式化の妥当性を支えている。

Rustのコードを検証する道具は、2026年11月の時点で7つが保守されている。
既存のコードを書き換えずに使えるのは有界モデル検査器のKaniであり、演繹的に証明する道具（Verus、Creusot、Aeneasなど）は、コードへの注釈か、道具が扱える書き方への書き換えを求める。
演繹的な検証の先例の証明の行数は、Verusの事例で実装の約3.3倍から13.3倍、AeneasとLeanによる事例で約14倍である。
ビットボードやゲームの規則エンジンをこれらの道具で検証した例は見つからなかった。

証明支援系で書いた実行可能な仕様と本番の実装を乱数で突き合わせる方法には、認可言語Cedarの先例がある。
この方法は実装の誤りと仕様の誤りの両方を見つけた一方で、入力を生成できなかった箇所の誤りを見逃している。
同じ仕様から独立に作った二つの実装が同じ誤りを共有することも、実験と事例の両方で報告されている。

## 規則と合法手生成を形式検証した先行例

### チェスの終盤

チェスでは、終盤の表（残り駒数が少ない全局面の勝敗の表）と終盤の戦略が、証明支援系による検証の主な対象になってきた。
合法手生成器そのものを証明した例は、この系統にはない。

| 例 | 仕様としたもの | 証明または検査したもの | 信頼の基底に残したもの |
|---|---|---|---|
| Hurd「[Formal Verification of Chess Endgame Databases](https://www.gilith.com/papers/chess.pdf)」（HOL4、2005年） | ポーンとキャスリングを除くチェスの規則を対象とする。盤を自然数の対、局面を手番と升から駒への部分関数で表し、約60個の定義で書く。 | 4駒までの終盤の全局面について、何手で勝てるかの集合を二分決定図（論理関数をグラフで表すデータ構造）で構成し、定理にした。 | HOL4の核と二分決定図の計算を信頼する。規則の定義が妥当であることは、国際チェス連盟（FIDE）の条文と定義を並べて人が読んで確かめる。論文は定義を「designed to be as natural as possible, so that a human reader … can be easily convinced that it is a faithful translation」と説明する。 |
| Maliković、Čubrilo、Janičić（[Coq、2012年](https://poincare.matf.bg.ac.rs/~janicic/papers/2012-ceciis-chess.pdf)） | キングとルーク対キングの終盤だけを対象とする。局面を6つの座標と手番で表し、規則を線形算術で書く。 | 戦略を形式化し、詰みに関する補題を証明した。戦略の正しさの証明は今後の課題とされている。 | Coqの核を信頼する。論文は規則の定義について「it is essential to have the chess rules … defined in a simple, concise, intuitive, and convincing way」と書く。 |
| Janičić、Marić、Maliković（[Isabelle/HOL、Logical Methods in Computer Science 15(1):34、2019年](https://lmcs.episciences.org/5328/pdf)） | 一般のチェスの定義と、キングとルーク対キング専用の簡約した定義を同じ体系に置き、両者の対応を証明で結ぶ。 | 戦略の正しさを証明した。終局の局面から逆向きに手をたどる後退解析で全数を検査し、戦略の初期実装の誤りを見つけた。読みやすい定義と高速な定義が等価であることを、ソルバで検査した。 | Isabelleの核とソルバを信頼する。論文は「the proofs are checked only modulo these [basic definitions]」と書き、基本定義の妥当性が証明の外にあることを認めている。 |
| Marzion「[coqtbgen](https://github.com/emarzion/coqtbgen)」（Coq） | ゲームを抽象的なレコードとして定義し、循環による引き分けを、無限に続く遷移の列を扱える余帰納型で表す。 | 終盤の表を生成するプログラムの正しさを証明した。実例は小さな別のゲームだけである。 | Coqの核と、正しく振る舞うと仮定した写像の型を信頼する。 |

HurdとHaworthの続報「[Data Assurance in Opaque Computations](https://www.gilith.com/papers/egt.pdf)」（Advances in Computer Games 2009、Lecture Notes in Computer Science 6048）は、検証されていない終盤の表に過去に見つかった誤りを分類している。
この続報は、チェスで広く使われたNalimovの終盤の表について、自己整合性の検査が表を生成するコードと8割を共有しており、独立な検査ではなかったと指摘する。
Hurdの結果は、Nalimovの表と一致した。

### 証明支援系で書かれた盤上ゲームの規則

| 例 | 仕様としたもの | 証明または検査したもの | 信頼の基底に残したもの |
|---|---|---|---|
| [TadaoYamaoka/lean-shogi](https://github.com/TadaoYamaoka/lean-shogi)（Lean 4.34.0、本将棋、最新コミットは2026年9月22日） | 仕様は`Shogi.Spec.LegalMove`であり、駒の利き、経路の空き、成り、二歩、行き所のない駒、自玉の安全、および打ち歩詰めを命題として定義する。仕様は、生成器も真偽値を返す合法性の判定関数も呼ばない。 | 任意の局面`p`と手`m`について`m ∈ legalMoves p ↔ Spec.LegalMove p m`を証明した。健全性と完全性の両方を含む。`sorry`（証明の穴）、独自の公理、および`native_decide`（コンパイルした計算を信頼する証明法）は使っていない。 | Leanの核と標準の3公理を信頼する。状態遷移`applyUnchecked`、玉の位置の検索、および盤面の参照は仕様と実装が共有する。READMEは、次の3つを定理の対象外と明記する。状態遷移と局面の文字列表記（SFEN）の解析が独立な仕様と一致すること、局面が初期局面から到達できること、および千日手などの履歴に依存する規則である。 |
| [SamuelSchlesinger/chess](https://github.com/SamuelSchlesinger/chess)（Lean 4とRust） | Leanの定義`legalMoves := Move.all.filter (isLegal position)`を仕様とする。全部の手を列挙して合法性の判定で絞る。 | RustのチェスエンジンをLeanの定義に対して証明してはいない。共有の試験データと棋譜（3,803系列、36,840手）で両者の一致を検査する。 | 設計文書は「Lean is authoritative for modeled semantics … Rust is authoritative for the shipped executable behavior after it passes shared conformance tests」と分担を書く。 |
| [double-ai/double-check](https://github.com/double-ai/double-check)（Lean 4） | Leanで書いた手の生成器と、SATへの符号化を使う。 | 連続する王手の最大手数についての定理を証明した。Leanの生成器はpython-chessと照合した。 | SATの事例ごとに1つの公理を置く。 |
| KringsとKörner（[仕様記述言語Bとそのモデル検査器ProB、2019年](https://fmfun.github.io/Papers-2019/Krings-Koerner.pdf)） | チェスをBの状態遷移の機械として記述する。 | ProBでモデル検査を行った。 | ProBを信頼する。論文は千日手について「keeping track of all prior positions leads to a combinatorial explosion, effectively rendering model checking impossible」と書く。 |

lean-shogiの`Shogi.lean`は約730行で、全升を走査する素朴な生成器であり、ビットボードは使っていない。
作者は、Pythonの将棋ライブラリcshogiとも1,000局面で合法手を比べている。

中将棋とその他の変則将棋、囲碁、およびチェッカーの規則の形式化は、Lean、Isabelle、Coq、Agda、HOL4、およびACL2のいずれについても見つからなかった。
Isabelleの証明の書庫（Archive of Formal Proofs）のゲームの分類にも、盤上ゲームの規則はなかった。

### ビットボードの検証

ビットボードの演算を証明支援系で証明した例は見つからなかった。
Cの有界モデル検査器CBMC、Rustの有界モデル検査器Kani、または入力を記号として扱って分岐の経路を調べる記号実行器KLEEで、合法手生成器を素朴な参照実装と等価であると検査した例も見つからなかった。

近い例は、Fiekasの修士論文「[Finding Hash Functions for Bitboard Based Move Generation](https://backscattering.de/magics2.pdf)」（2018年）である。
この論文は、走り駒の利きを掛け算と表引きで求めるmagic bitboardの定数について、素朴な利きの計算で全部の占有の組合せに対する正しい利きを作り、定数の候補を全部の組合せで検査している。
特定の升で、ある大きさの表に収まる定数が存在しないことも全数探索で示したが、著者は「there does not seem to be an efficiently checkable certificate for this exhaustive search」と書き、探索プログラムそのものを信頼するしかないことを認めている。

### 規則を記述する言語

ゲームの規則を機械が読める形で書く言語を4つ調べた。
中将棋の完成した記述は、どの言語にも見つからなかった。

| 言語 | 意味の定義 | 将棋類の記述 | 中将棋 |
|---|---|---|---|
| Game Description Language（[仕様](http://ggp.stanford.edu/readings/gdl_spec.pdf)） | 論理プログラムとして形式的に定義される。 | 王を取れば勝ちとし、打ち歩詰めと入玉を除いた本将棋の変種がある。 | 記述の一覧にない。 |
| Ludii（[文法の論文](https://arxiv.org/abs/2205.00451)） | 文法は実装のJavaのソースから自動で導かれる。形式的な意味の文書は見つからなかった。 | 本将棋、泰将棋、大局将棋など。 | [Chu-Shogi](https://ludii.games/lud/games/Chu-Shogi.lud)がある。規則セットは「Historical Information (Incomplete)」と表示され、終局条件は`(end [#])`と空欄のままで、獅子の捕獲制限の記述がない。版は1.3.14である。 |
| Regular Boardgames（[論文](https://arxiv.org/pdf/1706.02462)） | 行動の列の正規言語として形式的に定義される。 | 公式の記述の一覧に将棋はない。 | ない。 |
| Zillions of Gamesの規則ファイル | 形式的な意味の定義は見つからなかった。処理系は商用で非公開である。 | 本将棋が同梱される。 | Steve Evans作の記述が存在するが、中身は確認していない。 |

## Rustのコードを検証する道具

### 版と保守の状況

Rustのコンパイラには、安定版（stable）と、毎日ビルドされる開発版（nightly）がある。
検証の道具の多くはコンパイラの内部に依存するので、特定の日付のnightlyを必要とする。

| 道具 | 方式 | 最新の版 | 必要なRustのコンパイラ |
|---|---|---|---|
| [Kani](https://github.com/model-checking/kani/releases) | 有界モデル検査である。RustをCBMCの入力へ変換する。 | 0.68.0（2026年9月16日）。AWSが保守する。 | 特定の日付のnightly。0.68.0は`nightly-2026-08-21`を使う。 |
| [Verus](https://github.com/verus-lang/verus/releases) | 演繹的検証である。仕様と証明をRustの構文で書き、SMTソルバで検査する。 | 0.2026.11.22（2026年11月23日）。週1回から数週に1回の頻度で出る。 | stableの1.100.0。 |
| [Creusot](https://github.com/creusot-rs/creusot/releases) | 演繹的検証である。Why3という検証基盤へ変換する。 | 0.15.0（2026年11月10日）。 | 特定の日付のnightly。 |
| [Prusti](https://github.com/prusti/prusti) | 演繹的検証である。Viperという検証基盤へ変換する。 | 毎日の開発版だけが出ている。 | 特定の日付のnightly。 |
| [Aeneas](https://github.com/AeneasVerif/aeneas) | Rustを純粋な関数型のプログラムへ翻訳し、Lean 4などで証明する。翻訳の前段はCharonという別の道具が担う。 | 毎日のプレリリースが出ている。 | Charonが特定の日付のnightlyを使う。 |
| [hax](https://github.com/pereq-dev/hax) | Rustを証明支援系のF*、Rocq、またはLeanへ抽出する。 | 0.5.0（2026年11月9日）。 | 特定の日付のnightly。 |
| [Flux](https://github.com/flux-rs/flux) | 型に述語を付ける篩型（liquid types）で検査する。 | リリースはない。数日おきにコミットがある。 | 特定の日付のnightly。 |

### コードに求める条件

Kaniは、検査したい性質をテストに似た関数（ハーネス）として書き、入力を`kani::any()`で記号的な値（その型が取り得る任意の値を表す値）にする。
[機能の対応表](https://model-checking.github.io/kani/rust-feature-support.html)は、配列、添字、ループ、`match`、定数、静的変数、トレイト、およびクロージャを対応済みとし、`count_ones`や`trailing_zeros`に当たる[組込み関数](https://model-checking.github.io/kani/rust-feature-support/intrinsics.html)も対応済みとする。
ハーネスは`#[cfg(kani)]`で囲めるので、検査対象の関数の本体を変えずに済む。
制約は次のとおりである。

- ループは既定で展開され、回数が入力に依存するループには`#[kani::unwind]`で上限を与える（[手引き](https://model-checking.github.io/kani/tutorial-loop-unwinding.html)）。ループの不変条件、関数の契約（事前条件と事後条件）、および関数の差し替え（stubbing）は不安定な機能である。
- ヒープ上の大きさが決まらないデータに弱い。[手引き](https://model-checking.github.io/kani/tutorial-nondeterministic-variables.html)は「Kani does not (yet!) scale well to nondeterministic-size data structures involving heap allocations」と書く。
- CBMCが結果を書き出す途中でメモリ不足になると、Kaniが検証の成功を報告する不具合がある（[issue #4905](https://github.com/model-checking/kani/issues/4905)）。0.68.0で再現し、2026年9月30日に修正されたが、0.68.0は同月16日のリリースなので、修正を含むリリースは調査日の時点で出ていない。

Verusは、検証するコードを`verus!`マクロの中に置くか、属性を付ける。
[機能の表](https://verus-lang.github.io/verus/guide/features.html)（2026年5月13日更新）によると、配列と`Vec`は対応済みだが、`const fn`、`const`の項目、`static`の項目、`for`、およびイテレータは部分的な対応であり、クロージャは変数を可変で捕捉できない。
ループには不変条件を手で書く必要がある。
ビット演算は既定では中身を持たない未解釈の関数として扱われ、性質は`by(bit_vector)`という専用の証明で個別に示す（[手引き](https://verus-lang.github.io/verus/guide/bitvec.html)）。

Creusotは、ループに不変条件を求め、`vec!`マクロに対応しない（[制約の一覧](https://raw.githubusercontent.com/creusot-rs/creusot/master/guide/src/limitations.md)）。
Prustiはクロージャに対応せず、ビット演算をビット列として扱う符号化は実験的な設定である。

Aeneasは、安全なRustの部分集合だけを翻訳する。
[README](https://github.com/AeneasVerif/aeneas)は、入れ子のループの中の`return`と、外側のループへの`break`と`continue`を非対応に挙げる。
標準ライブラリを含む外部の関数は、手書きのモデルを用意する必要がある。
haxも安全なRustだけを対象とし、開発元は既存のプロジェクトへの適用について「it may use various Rust features that are probably not supported yet」と書く（[開発元の記事](https://hax.cryspen.com/blog/2025/02/25/hax-for-everyone/)）。

### 規模と労力の先例

演繹的な検証では、証明や注釈の行数が実装の行数を上回る事例が多い。
次の表の数は、事例によって証明の行、生成先の言語の行、または注釈の行を数えており、同じ指標ではない。

| 事例 | 道具 | 実装と証明の規模 |
|---|---|---|
| Verusの事例群（[Lattuadaほか、SOSP 2024](https://www.chajed.io/papers/verus:sosp2024.pdf)） | Verus | 実装が約6,100行、証明が約31,000行である。ページテーブルは証明が実装の13.3倍、メモリ割り当て器mimallocは4.3倍である。 |
| マイクロカーネルAtmosphere（[Chenほか、SOSP 2025](https://mars-research.github.io/doc/2025-sosp-atmo.pdf)） | Verus | 実行コードが約6,000行で、証明は3.32倍である。2.5人年のうち検証が1.5人年を占めた。 |
| 暗号ライブラリSymCrypt（[Hoほか、2026年](https://arxiv.org/abs/2609.15648)） | AeneasとLean | Rustが約16,700行、Leanが約237,000行（約14倍）である。証明の大半はAIエージェントが生成し、難しい証明7件は1件あたり平均15.4ドルと29.4分で解けた。 |
| 暗号ライブラリlibcruxのML-KEM（[開発元の発表資料](https://cryspen.com/post/rwc-2025/RWC_2025_Slides.pdf)） | haxとF* | Rustが約16,000行、F*が約21,000行である。最初から検証を前提に書かれたコードである。 |
| OSカーネルTockの一部（[Rindisbacherほか、SOSP 2025](https://nilehmann.github.io/papers/sosp25-ticktock.pdf)） | Flux | Rustが約22,000行、注釈が約3,600行である。検証に合わせて、カーネルの内部の構成を設計し直した。 |

Kaniは、既存のコードへ後から足した事例が多い。
Kaniの論文（[Delmasほか、ASE 2026](https://arxiv.org/html/2607.01504v1)）によると、通信ライブラリs2n-quicは102個のハーネスを23分で、時刻ライブラリHifitimeは153個を42分で検査している。
Hifitimeでは、自動で生成した158個のハーネスのうち57個が60秒の制限時間を超えたので、関数の契約を手で書いて整理し直した。

### 参照実装との等価性を検査した事例と所要時間

Kaniで、最適化した関数と素朴な参照の関数の一致を検査した事例がある（[s2n-quicの記事](https://model-checking.github.io/kani-verifier-blog/2023/05/30/how-s2n-quic-uses-kani-to-inspire-confidence.html)）。
最適化した加重平均と最適化前の式の一致は、32ビット整数2つの全入力について44.8秒で証明された。
パケット番号の復号を通信規格の擬似コードと比べるハーネスは、最適化で抜けた処理を2.9秒で見つけ、修正後は2.7秒で通った。
乱数によるファジングが約1,600万回の実行で見つけられなかった反例を、Kaniが20秒で出した例もある。

所要時間は対象によって桁が変わる。
Kaniの論文は、64ビット整数の最大公約数の反復の計算を、上限94回の展開で検査しようとして1時間で終わらなかったと報告する。
[遅い証明の手引き](https://model-checking.github.io/kani/debugging-slow-proofs.html)は、大きな型の乗算と除算、および複雑で大きな記号的な値を、遅くなる原因に挙げる。
第三者の監査報告には、128ビット整数の記号的な入力でSATの変数が約500万個になり、メモリ不足になったという記述がある（[報告](https://github.com/Blockchains/PERColator/blob/master/audit.md)、二次資料）。
128ビット以上の幅のビットボード、複数の整数からなる盤面、または数百バイトの構造体を記号的な値にしたときの所要時間の報告は、見つからなかった。

## 実行可能な仕様と実装を突き合わせる方法

### Cedarの先例

AWSの認可言語Cedarは、Lean 4で書いた実行可能なモデルとRustの本番実装を、乱数で生成した入力で突き合わせている（[Disselkoenほか、FSE Companion 2024](https://arxiv.org/abs/2407.01688)、以下「報告」）。
入力に対する二つの実装の出力を比べる試験を、差分テストという。

- Leanのモデルは、式の評価器、許可と拒否を決める認可器、および方策が型に合うかを検査する検査器（validator）の合計で1,673行、証明は5,714行である。構文解析器はモデルにしていない。
- validatorの健全性の証明4,686行には、18人日を要した（[AWSの記事](https://aws.amazon.com/blogs/opensource/lean-into-verified-software-development/)）。
- LeanとRustの結合には、Leanが出力する静的ライブラリをRustから`unsafe extern "C"`で呼ぶ方法を使い、入力はProtocol Buffersで渡す（[cedar-spec](https://github.com/cedar-policy/cedar-spec)）。
- 証明の過程で4件、差分テストと、乱数の入力に対して性質を確かめるテスト（property-based testing）で21件の不具合を見つけた。
- 差分テストは、モデルの側の誤りも見つけた。報告の表は、モデルがIPv6アドレスの「::」を誤って拒否した例を挙げる。
- 報告は、見逃した不具合9件も挙げる。原因は、不具合を起こす入力を試験の入力の生成器が作れなかったこと、および試験が該当するAPIを呼んでいなかったことである。

報告は、Rustを直接証明する案を、Aeneas、Kani、Prusti、Creusot、およびVerusが標準ライブラリや慣用的な書き方に対応しきれないことを理由に退けている。

### 小さい範囲の全数検査の根拠と限界

小さい範囲の入力をすべて調べれば大半の誤りが見つかるという経験則は、小スコープ仮説（small scope hypothesis）と呼ばれる。

- Andoni、Daniliuc、Khurshid、Marinovの報告「Evaluating the “Small Scope Hypothesis”」（MIT、2002年）は、Javaのデータ構造10種で、大きさ3から7までの全入力が、人工的に入れた誤りの89%から99%を検出したと報告する。対象はデータ構造の実装に限られる。
- Amazon S3の保存ノードShardStoreの検証（Bornholtほか、SOSP 2021）は、試験の設定でキャッシュを大きくしていたために、キャッシュに載らない経路へ届かず、不具合を見逃したと報告する。
- Kaniの[手引き](https://model-checking.github.io/kani/tutorial-loop-unwinding.html)は、有界な証明の限界を「Who's to say, if we prove everything works up to size 10, that there isn't a novel bug lurking, reachable only with problems of size 11+?」と書く。

### 独立に作った実装が同じ誤りを共有すること

同じ仕様から別々に作った実装について、失敗が独立に起こると仮定することはできない。

- KnightとLevesonの実験（IEEE Transactions on Software Engineering 12(1)、1986年）は、同じ仕様から27の版を独立に作らせ、100万件の入力で試験した。複数の版が同じ入力で同時に失敗した件数から、失敗が独立であるという仮定を統計的に棄却した。
- Reid「Who Guards the Guards? Formal Validation of the Arm v8-M Architecture Specification」（OOPSLA 2017）は、仕様を実装と照合する方法が「common mode failure (i.e., the specification and the implementation do the same wrong thing)」を見逃すと指摘する。
- Fonsecaほか（EuroSys 2017）は、検証済みの分散システム3つから16件の不具合を見つけ、うち2件は仕様の誤りだった。
- 参照に使うモデルの側の誤りは、ShardStore、Cedar、およびs2n-quicのいずれでも報告されている。

### 自然言語の規範を形式化して原文の不備が見つかった先例

自然言語で書かれた規範を形式化する過程で、原文の誤りや曖昧さが見つかった先例は多い。

- 法令を形式化する言語Catalaは、フランスの家族手当の形式化で、既存の実装が条文の例外を漏らしていたことを見つけた（Merigouxほか、ICFP 2021）。住宅手当の形式化では、省令の数値の誤植が見つかった。
- WebAssemblyの仕様を証明支援系で形式化する作業は、公開前の仕様の不備を見つけた（Watt、CPP 2018）。
- Armのプロセッサの仕様では、英文と擬似コードの食い違いが見つかり、設計者の判断で英文の側が誤りとされた（Reid、OOPSLA 2017）。

ゲームの規則の形式化が、規則の原文の不備を見つけたという記録は見つからなかった。

## 確認できなかった事項

- Marić、Janičić、MalikovićのCADE 2015の論文は本文を読めず、同じ著者の2019年の論文の記述で内容を確かめた。
- Zillions of Gamesの中将棋の記述は、配布がFTPだったので中身を読んでいない。
- 小スコープ仮説の原典（Jackson、MITの技術報告、1997年）は取得できず、引用している資料で確かめた。
- SamuelSchlesinger/chessとdouble-ai/double-checkは、確認した版のコミットを特定できなかった。
- CreusotとFluxが128ビット幅のビット演算をどこまで扱えるかは、資料から分からなかった。
