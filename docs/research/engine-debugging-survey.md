# エンジン開発のデバッグ手法の調査

本書のminaseに関する記述は2026年9月27日時点のものである。
結論でminaseが持たないとした局面と評価値の表示、置換表の照会、入出力のログ、および探索統計は、同日の[デバッグ機能の整備の設計書](../plans/archived/debugging-tools.md)により、USIの独自コマンド`d`、`eval`、`tt`、起動引数`--io-log`、およびCargoフィーチャ`search-stats`として加わった。
評価関数は2026年10月12日にNNUEへ替わり、[P型のNNUEは左右鏡映の対で第1層の重みを半幅だけ回して共有する](../adr/share-p-type-mirror-weights-by-half-width-rotation.md)ので、「minaseへの適用の可否」の第3点と異なり、現行の評価は左右鏡映でも変わらない。
「PV-MCTS型エンジンのデバッグ」の節と、「minaseへの適用の可否」の第4点および第5点は2026年11月16日に加えたもので、そこでのminaseに関する記述は同日時点のものである。
2026年11月17日に、[PV-MCTSのデバッグ道具の設計書](../plans/archived/mcts-debugging-tools.md)の起案のためにふかうら王のソースを読み直し、`DebugMessage`の出力についての記述を訂正した。
第5点でminaseにないとした3つの道具は、同日に同書により、USIのオプション`VerboseMoveStats`、フィーチャ`invariants`の下の探索木の検査`Tree::validate`、および`--search mcts`で起動したときの`bench`として加わった。

本書は、Chess Programming Wiki（以下CPW）の「Debugging」頁とその関連頁および引用投稿、ならびにStockfish、YaneuraOu、Fairy-Stockfish、HaChuのソースを調べ、エンジン開発で使われるデバッグ手法を整理した調査メモである。
2026年11月16日には、ニューラルネット（以下ネット）で着手の方策と局面の価値を求めるモンテカルロ木探索のエンジンを調べて加えた。
本書では、この型の探索をPV-MCTSと呼び、ネットを使わないものも含めた総称をMCTSと書く。
調べたのは、チェスのlc0（Leela Chess Zero）、Monty、およびCeres、囲碁のKataGoとLeela Zero、ならびに将棋のdlshogiとふかうら王のソースと、OpenSpielとAlphaZero.jlの試験、および関連する投稿である。
[デバッグ機能の整備](../plans/archived/debugging-tools.md)の設計書が方式の根拠として参照し、[デバッグの手引き](../guides/debugging.md)が手法の出典として参照する。
αβ探索の部分の調査日は2026年9月27日であり、PV-MCTSの部分の調査日は2026年11月16日である。

## 結論

調べた手法は、同じ計算を2通りに行って一致を確かめる照合、内部状態を人が読める形で表示する道具、探索の挙動を数える統計、および入出力と再現の記録の4種類に分かれる。
3つの主流エンジン（Stockfish、YaneuraOu、Fairy-Stockfish）に共通する最小限の組は、局面の表示（`d`）、評価値の表示（`eval`）、合法手の列挙、入出力のログファイル、決定的なbenchのノード数による回帰検出、および実行と取消しの後の局面の不変条件検査である。
minaseは、合法手の列挙（USIの`moves`）、不変条件検査（`Position::validate`）、benchのノード数照合（`scripts/bench_compare.py`）を持つ一方、局面と評価値の表示、入出力のログ、置換表の照会、および探索統計を持たない。

置換表を照会して探索後の木を辿る方法と、指し手順序の品質統計は、CPWの「Debugging」頁がTord Romstadの提案として挙げるが、3つの主流エンジンはいずれも常設していない。
3エンジンの統計関数（Stockfishの`dbg_hit_on`など）は呼び出し箇所を1つも持たず、開発者が一時的に差し込む道具として扱われている。
局面の完全な再計算照合は、速度と衝突して既定では無効にされる傾向がある。
Stockfishは2023年に局面状態全体の再計算照合を外し、Fairy-Stockfishは`Fast = true`で既定の検査を軽量版に限り、YaneuraOuはハッシュキーの照合を実行時の検査ではなく単体テストのランダム対局へ移した。

PV-MCTS型エンジンの手法は、ネットの入出力の照合、ネットを試験用の評価器に替えた探索の試験、根の子ごとの統計の表示、および探索木の不変条件の検査に重心がある。
探索の後に根の子（現局面の各候補手）ごとの統計を表示する専用の道具は、調べた7つのエンジンのうち、ふかうら王を除く6つが持つ。
ふかうら王も、2番目以降の候補手の訪問回数をMultiPVの出力で示す。
αβ探索と同じbenchのノード数による回帰検出を使うのはCPUで推論するMontyだけであり、GPUで推論するKataGoは、小さなネットで固定の訪問回数の探索を行い、探索の結果の全文をコミット済みの期待出力と見比べる。
ネットの照合では、KataGoが設定どおりの推論と32ビット浮動小数点の推論の誤差を99百分位と最大値で判定し、Leela ZeroがOpenCLの推論の約2,000回に1回をCPUで計算し直して比べる。
調べた範囲で原因まで記録された不具合は、終局と詰みの扱い、並列探索での節点の展開の競合、およびネットの入力の符号化に集中する。

## 調査した資料

CPWの頁は `https://www.chessprogramming.org/<頁名>` で取得した。
「Famous_Bugs」「Assert」「Sanitizer」の各頁は存在せず、Debugging頁の「Famous Bugs」はEngine_Testing頁の「Notable Bugs」節を指す。

| 資料 | 取得経路 |
|---|---|
| [CPW Debugging](https://www.chessprogramming.org/Debugging)、[Perft](https://www.chessprogramming.org/Perft)、[Engine Testing](https://www.chessprogramming.org/Engine_Testing)、[Logging](https://www.chessprogramming.org/Logging)、[InBetween](https://www.chessprogramming.org/InBetween)、[Color Flipping](https://www.chessprogramming.org/Color_Flipping)、[Search Statistics](https://www.chessprogramming.org/Search_Statistics)、[Transposition Table](https://www.chessprogramming.org/Transposition_Table)、[Shared Hash Table](https://www.chessprogramming.org/Shared_Hash_Table) | 直接 |
| Tord Romstad「Re: Testing and debugging chess engines」（Winboard Forum、2006年、`http://www.open-aurec.com/wbforum/viewtopic.php?f=4&t=5955`） | Wayback Machine |
| Eric Oldre「General Tips and Tricks for debugging a search」（CCC、2005年、`https://www.stmintz.com/ccc/index.php?id=431392`）と返信 | Wayback Machine（一部の返信は保存なし） |
| Onno Garms「Debugging regression tests」（CCC、2011年、[t=39390](http://www.talkchess.com/forum/viewtopic.php?t=39390)） | 直接 |
| Lucas Braesch「How to find SMP bugs ?」（CCC、2017年、[t=63454](http://www.talkchess.com/forum/viewtopic.php?t=63454)） | 直接 |
| konsolas「Debugging a transposition table」（CCC、2018年、[t=67599](http://www.talkchess.com/forum3/viewtopic.php?f=7&t=67599)）[^tang] | 直接 |
| Andrew Grant「Resolving once in a trillion crashes」（CCC、2022年、[t=79160](https://www.talkchess.com/forum3/viewtopic.php?f=7&t=79160)） | 直接 |
| Steven Edwards「A method guaranteed to localize the toughest perft() bugs」（CCC、2014年、t=53743）、Meni Rosenfeld「Best way to debug perft?」（CCC、2016年、t=59046）、Erik Madsen「Engine Crash Detective Story」（CCC、2020年、t=74931） | 直接 |
| Ísleifsdóttir、Björnsson「GTQ: A Language and Tool for Game-Tree Analysis」（CG 2008、`http://www.ru.is/faculty/yngvi/pdf/IsleifsdottirB08.pdf`） | Wayback Machine |
| Stockfish（手元の複製、コミット17a6c8f1、2026年9月19日） | 作業ツリーに改変なし |
| YaneuraOu（手元の複製、コミットc1b80eaa、2026年9月14日取得時点のorigin/master） | 作業ツリーに改変なし |
| Fairy-Stockfish（手元の複製、コミット2e591089、2026年9月20日） | 上流にない未追跡ファイル1件（`tests/minishogi_perft.sh`）のほかは改変なし |
| lc0（コミットbf15d55、2026年11月8日）、KataGo（d91ea85、2026年9月27日）、Leela Zero（既定のブランチnext、3ee6d20、2023年2月6日）、Monty（0950aff、2026年5月12日）、Ceres（0cb57d8、2026年9月12日） | 2026年11月16日に`git clone --depth 1`で取得、改変なし |
| DeepLearningShogi（dlshogi、手元の複製、コミットa9e7e5f、2026年11月10日）、YaneuraOu（ふかうら王の部分、手元の複製、コミット3b8d53fa、2026年11月11日） | 作業ツリーに改変なし |
| OpenSpielの`open_spiel/python/algorithms/mcts_test.py`（master、4956843d）、AlphaZero.jlの`src/scripts/test_game.jl`（master、177a6eb0） | 直接 |
| [OpenBench wiki「Requirements For Public Engines」](https://github.com/AndyGrant/OpenBench/wiki/Requirements-For-Public-Engines) | 直接 |
| MOBMAT「MCTS implementation question」（CCC、2019年、[t=69993](https://talkchess.com/viewtopic.php?p=790994)）、crem「A0 subtlety」スレッドへの投稿（CCC、2019年、[t=69881](https://talkchess.com/viewtopic.php?p=790239)）、RedBedHed「PUCT and Mate Evaluation for Monte-Carlo Alpha-Beta」（CCC、2022年、[t=80947](https://talkchess.com/viewtopic.php?p=937255)） | 直接 |
| HaChu（Debianパッケージの複製 hachu-debian、コミット822d512） | 作業ツリーに改変なし |

[^tang]: CPWはこの投稿をVincent Tangのものとしているが、フォーラム上の投稿者名はkonsolas（ToppleChessの作者）である。

以下の節で引用するソースの行番号は、上表のコミットにおける値である。

## 合法手生成、実行と取消し、ハッシュキーの照合

この群の手法は、同じ入力に対する2つの計算が一致するかを大量の局面で確かめる点で共通する。

**perftとdivide**：perftは指定深さまでの合法手の葉の数を数え、divideは根の指し手ごとに部分木の数を出す。
参照実装と数が食い違う手を1手進めて深さを1減らす操作を繰り返すと、誤りのある局面まで絞り込める（CPW Perft、Rosenfeld t=59046）。
Edwards（t=53743）は、深さnでだけ誤る型の不具合はdivideの二分探索で追えないとして、全変化を棋譜表記で1行ずつ書き出して参照実装の出力と整列して差分を取る方法を補完に挙げる。
Stockfishの`go perft N`はdivide形式で出力する（stockfish/src/uci.cpp:224-225、perft.h:34-55）。

**遅い実装と速い実装の照合**：Romstadは、自明に正しい遅い版と最適化版を両方書き、大量の局面で一致を確かめ、確信が持てるまで遅い版を残すよう勧める。
Oldreスレッドの返信は、ハッシュキー、駒位置評価、駒割りの差分更新値を盤面からの全再計算と比べる方法と、実行前の局面状態を保存して実行と取消しの後の状態と比べる方法を挙げる（id=431413、431434）。

**局面の不変条件検査**：Stockfishの`Position::pos_is_ok()`（position.cpp:1612-1671）は、ビットボードの互いの素、駒数の3者一致、キャスリング権、駒構成キーの再計算を検査し、`set`、`do_move`、`undo_move`の末尾で`assert`から呼ばれる（position.cpp:441、1072、1142）。
現行版は局面のハッシュキー本体を再計算しない。
YaneuraOuは6段階の`ASSERT_LV`を持ち（config.h:263-271）、最重のレベル5で`pos_is_ok()`、生成手の擬似合法性、および差分更新した駒割りと全計算の照合を実行時に行う（position.cpp:2214、2402、movegen.cpp:554、eval/nnue/evaluate_nnue.cpp:980）。
YaneuraOuの`unittest`は、固定シードの1,000局のランダム対局で毎手、局面を文字列から作り直した部分ハッシュキーと差分更新の値を照合する（position.cpp:3800-3846）。

**擬似合法性判定の総当たり**：Grant（t=79160）は、perftの各節点で16ビットの指し手符号の全値について擬似合法性判定を呼び、真となる手の集合が生成手の集合と一致するかを確かめる。
置換表の手とキラー手の検証関数の誤りは、通常のperftでは現れない。

## 探索のデバッグ

探索の手法は、探索後に木を調べる道具、探索中の判断を記録する道具、および統計で異常を見つける方法に分かれる。

**置換表の照会による木の閲覧**：Romstadは、現局面の置換表の項目（評価値、境界の種類、最善手）を表示する独自コマンドを置き、探索後に手を進めては照会して、ある手が捨てられた理由と反駁手を調べる方法を勧める（CPW Debugging）。
デバッグビルドでは、置換表の項目に全指し手と手ごとの延長、削減、枝刈りの判断も記録する。
3つの主流エンジンに置換表を照会するコマンドはない。

**指し手列による追跡**：HaChuは探索中の手順を`path[]`に記録し、約25箇所に`if(PATH) printf(...)`を置く（hachu-debian/hachu.c:2042-2424）。
`PATH`を指定手順の接頭辞に一致する述語へ定義し直すと、その部分木だけの窓、置換表の照会結果、各手の値、戻り値を出力できる（hachu.c:15-16）。
Oldreスレッドでは、Schröder（id=431452）とBöhm（id=431538）が同様に指し手列を条件にして探索を止める道具を述べる。

**探索木の問い合わせ**：GTQ論文は、探索木を節点の属性つきでログへ書き出し、節点、子、部分木の3部からなる問い合わせを1回の走査で評価する言語GTQLを示す。
Fruit 2.1に適用し、840節点に膨らんだ静止探索木を見つけた。
並列探索の木は扱えない。

**指し手順序の品質統計**：Romstadは、最初の手でβカットが起きる割合と、PV節点で何番目の手が最善になったかの頻度を測るよう勧める（CPW Debugging）。
CPW Search Statisticsは、これに加えて置換表の照会数、一致数、打ち切り数、静止探索の節点の比率、再探索の回数を挙げる。
konsolas（t=67599）のスレッドでは、置換表による打ち切り数が一致数の約1.5%しかないことから、PV節点で置換表の値を使わない条件の誤設定が見つかった。

**統計関数**：Stockfishは条件の成立率、平均、標準偏差、最小最大、相関係数を32スロットに集計する`dbg_*`関数を持ち、`bench`の終了時に`dbg_print()`で出力する（misc.cpp:311-442、uci.cpp:306）。
misc.cppとmisc.hを除くと、呼び出し箇所はソース中に1つもない。

**戦術問題集の定時実行**：Romstadは、探索を大きく変えるたびに問題集を固定時間で解かせ、正解数が急落しないことだけを確かめるよう勧める。
正解数を最適化の目標にはしない。

## 評価関数のデバッグ

**色反転による対称性**：盤を上下に反転し、駒の色と手番を入れ替えた局面で、手番側から見た静的評価が一致するかを確かめる（CPW Color Flipping）。
Stockfishの`flip`コマンドは、この用途を明記して局面を反転する（position.cpp:1573-1603）。
中将棋の初期配置は、180度回転で先後が入れ替わる点対称であり、醉象と玉将、麒麟と鳳凰の配置が左右非対称である（RULES.md第5条）。
一方、minaseの学習PSTは駒の升を所有者から見た段と絶対的な筋で引くため、評価はチェスの色反転と同じ「段反転と陣営交換」で不変になり、左右反転では不変にならない。

**評価の内訳**：Stockfishの`eval`はNNUEの層バケットごとの項を表示する（evaluate.cpp:75-95、nnue/nnue_misc.cpp:58-90）。
YaneuraOuの`eval`は現行版では出力部分がコメントアウトされ、`e`が評価値だけを出す（usi.cpp:398-399、yaneuraou-search.cpp:577-589）。
HaChuは最初の反復の思考出力に、差分更新した評価値と全計算した評価値を並べて表示する（hachu.c:2393）。

## 置換表

**署名と手の検証**：置換表の項目に鍵の上位ビットを保存して照会時に照合し、置換表から得た手は擬似合法性を確かめてから使う（CPW Transposition Table）。
検証関数そのものは、無意味な状態を含む乱数の手を大量に与えて確かめる（Braesch t=63454）。

**典型的な誤り**：konsolasのスレッドで指摘された誤りは、延長後の深さを保存と照会で一貫して使わないこと、手の走査途中の暫定最善手を保存すること、フェイルローの値を正確値として保存すること、詰みの評価値を手数で補正しないこと、および探索中断後に保存して表を汚すことである。

**キャッシュの縮小**：marは、置換表と評価キャッシュを1項目に縮めてキャッシュへの依存を断ち、メモリ検査と併用すれば稀なクラッシュを早く捕まえられると述べる（t=79160）。

## 並列探索

**再現性の確保**：Braesch（t=63454）のスレッドでは、同期処理を外しても続いたクラッシュが置換表を切ると止まり、置換表の値による打ち切りへの不正な項目の混入に絞り込まれた。
Diepeveen（Oldreスレッド id=431800）は、共有メモリの配列へ事象を追記する軽量な記録と、葉の近くでも分割する過酷な分割規則を使って、競合をすぐに再現させる。

**ロックレスハッシュ**：鍵とデータの排他的論理和を鍵の欄に保存し、照会時に復元した鍵が一致しなければ捨てる（CPW Shared Hash Table）。

## クラッシュと稀な不具合

**節点数による再現**：Grant（t=79160）は、時間制御の対局でだけ起きたクラッシュを、各手の節点数を記録と一致させる`go nodes`の列で再現した。
H.G.Mullerは、各手の探索節点数をファイルへ追記しておく方式を提案している。

**コアダンプと書き込み監視点**：petero2とjstanbackは、コアダンプから呼び出し履歴と変数値を得て、再現しないクラッシュを調べる（t=79160）。

**検査による速度低下**：Madsen（t=74931）は、差分更新の値を毎回再計算する検査が探索を大幅に遅くし、不具合の発生条件を満たさなくする例を報告する。

## 入出力のログ

**エンジン内のログファイル**：StockfishのUCIオプション「Debug Log File」は、標準入出力を複写して、入力行に「>> 」、出力行に「<< 」を付けてファイルへ書く（engine.cpp:69-73、misc.cpp:60-122）。
YaneuraOuは同じ方式を「DebugLogFile」オプションと`log`コマンドで提供する（engine.cpp:120-125、usi.cpp:441-442）。

**中継プログラムと対局管理ソフトの記録**：InBetweenはGUIとエンジンの間に入って全通信を記録し、cutechess-cliの`-debug`とArenaのデバッグログも全通信を記録する（CPW InBetween、t=66124、t=66366）。

## 回帰の検出

**benchのノード数署名**：Stockfishは、コミット本文の「Bench: 数字」を基準値として、CIで`bench`の総ノード数を照合する（.github/workflows/tests.yml:177-182、tests/signature.sh）。
この照合は`debug=yes`のビルドでも行い、検査を有効にした状態でbenchを完走することも確かめる。
Fairy-Stockfishの`tests/regression.sh`は、新旧2つのバイナリのbenchノード数を変種ごとに比べる。

**サニタイザ**：StockfishのCIは、ThreadSanitizer、UndefinedBehaviorSanitizer、valgrindで`tests/instrumented.py`を実行する（.github/workflows/sanitizers.yml:21-45）。
これらはC++のメモリ誤用とデータ競合を対象にする。

**事象番号によるログの二分探索**：Garms（t=39390）は、事象ごとに進むカウンタでN回に1回だけ記録する仕組みを置き、2つの版の記録が分かれる区間を粗い間隔から細かい間隔へ絞り込み、最後に一致した事象番号でブレークする。

## PV-MCTS型エンジンのデバッグ

PV-MCTS型エンジンは、探索の節点ごとにネットで局面を推論し、方策と価値を得る。
方策は着手の事前確率であり、価値は手番側から見た勝敗の予測である。
エンジンはこれらを、方策の事前確率で探索項を重みづけるPUCTなどの選択規則に使って探索木を育てる。
αβ探索との違いは、探索の結果がネットの出力の数値に直接依存すること、GPUの推論の数値が機械と精度によって僅かに変わること、およびバッチで推論するために評価待ちの葉を仮想損失で散らすことにある。
仮想損失は、評価待ちの経路を仮に負けとして数え、次の選択が同じ葉へ集まるのを避ける仕組みである。
調べたエンジンは、この違いに応じて、ネットの照合と探索の照合を分けて行う。

### 探索の状態の表示

探索の後に根の子ごとの統計を表示する専用の道具は、ふかうら王を除く6つのエンジンが持ち、部分木、時間管理、およびネットの生の出力を表示する道具を加えるエンジンもある。

**根の子ごとの統計**：lc0のオプション`VerboseMoveStats`は、探索の後に根の子ごとに、訪問回数、評価待ちの訪問数、事前確率、勝敗の差と引き分けの確率、部分木の価値の平均Q、探索項U、選択の得点S、ネットの生の価値V、および確定した勝敗を1行ずつ出す（lc0/src/search/classic/params.cc:290-293、search.cc:460-545）。
Vは探索木ではなく推論のキャッシュから引き直した値なので（search.cc:520-525）、ネットの評価Vと、探索で平均した価値Qとの食い違いを1行で見比べられる。
オプション`LogLiveStats`は同じ表を思考の出力のたびに出す（params.cc:294-297）。
`VerboseMoveStats`を切った状態でも、lc0はログファイルにこの表と、最善手を指した後の相手の手の表を書く（search.cc:576-603）。
dlshogiのオプション`DebugMessage`は、探索の終わりに根の子ごとの訪問回数、事前確率、勝率、および確定した勝敗を出し、探索の前には予定の時間とプレイアウト数を、木の再利用では持ち越した訪問回数を出す（DeepLearningShogi/usi/UctSearch.cpp:1116-1125、1156-1165、usi/Message.cpp:66-73）。
ふかうら王は同じ名前のオプションを引き継ぐが（YaneuraOu/source/engine/dlshogi-engine/SearchOptions.cpp:74）、根の子ごとの表は出さない。
探索の終わりに出すのは、総訪問数、前の探索から持ち越した訪問数、思考時間、勝率、および速度の要約と（PrintInfo.cpp:286-315）、時間管理が探索を打ち切った理由である（dlshogi_searcher.cpp:644、931-968）。
根の子ごとの訪問回数は、MultiPVの2行目以降の`nodes`として出す（PrintInfo.cpp:249-250）。
Leela Zeroは根の子ごとに訪問回数、勝率、勝率の下側信頼限界、事前確率、および主変化（以下PV）を1行ずつ出す（leela-zero/src/UCTSearch.cpp:320-324）。
Montyのオプション`report_moves`は、根の子ごとのQ、訪問回数、および確定した結果を出す（Monty/src/mcts.rs:762-777）。

**部分木と時間管理の表示**：KataGoの`evalsgf`は棋譜の1局面を探索し、`-print-branch`で指定した手順の先の部分木を、`-print-max-depth`の深さまで子ごとの統計つきで表示する（KataGo/cpp/command/evalsgf.cpp:60-82）。
囲碁の対局規約GTPの拡張コマンド`debug_moves`は現局面から指定した変化の各手の統計を出し、`kata-search_debug`は着手を決める探索の詳細を標準エラー出力へ書き、`kata-debug-print-tc`は両者の持ち時間の状態を出す（KataGo/docs/GTP_Extensions.md:271-285）。
Ceresは同じ局面の節点を共有する探索グラフを使うエンジンであり、根の候補手を出す`dump-move-stats`、PVを出す`dump-pv`、木の全節点を出す`dump-store`、時間管理の直前の判断を出す`dump-time`、および探索グラフを図示するコマンドを持つ（Ceres/src/Ceres.MCGS/UCI/UCIManagerMCGS.cs:417-449）。

**ネットの生の出力**：KataGoの`kata-raw-nn`は、現局面を、盤の回転と鏡映による8つの対称変換の1つまたは全部で推論し、勝率、目数の差、方策、および各点の地の所有の予測を出す（GTP_Extensions.md:193-214）。
Leela Zeroの`heatmap`は方策を盤の図で出し、対称変換を番号で指定するか、`all`で8通りを並べる（leela-zero/src/GTP.cpp:853-880）。
Montyの`eval`は価値として勝ち、引き分け、負けの確率を、`policy`は全合法手の方策を確率の降順に出す（Monty/src/uci.rs:103-152）。
KataGoの`evalsgf`のオプション`-dump-npz-input-to`は、ネットへ渡す入力のテンソルをファイルへ書き出す（evalsgf.cpp:84）。

### ネットの入出力の照合

ネットの照合は、参照となる計算との数値の比較、バッチ化と対称変換での一致、既知の局面での健全性、および入力の符号化の試験に分かれる。

**参照実装との数値の照合**：KataGoの`testgpuerror`は、16ビット浮動小数点（以下FP16）などの設定どおりの推論を、32ビット浮動小数点の推論と多数の局面で比べる（KataGo/cpp/command/gputest.cpp:148）。
比べる量は、勝率、目数の予測、最大の方策の確率、および地の所有の予測の各誤差と、方策のKLダイバージェンスであり、各量の99百分位と最大値を許容値と比べる（tests/testnnevalcanary.cpp:255-380）。
同じファイルのコメントは、照合を1局面とバッチの大きさ1に絞り、層ごとの活性値を書き出して、FP16の正規化層の桁あふれを突き止めた経緯を残している（testnnevalcanary.cpp:638-641）。
Leela ZeroのOpenCL版は、推論の約2,000回に1回、同じ局面をCPUの実装でも計算し、方策と勝率の差のL2ノルムが0.2を超えるかNaNになれば、ドライバの更新を促す文を出して例外で止まる（leela-zero/src/config.h:133-136、Network.cpp:666-690、781-790）。
lc0の推論器`check`は、2つの推論器の出力を、既定で絶対誤差1×10⁻⁵と相対誤差1×10⁻⁴の許容幅で比べ、誤差の表示とヒストグラムへの集計にも切り替えられる（lc0/src/neural/backends/network_check.cc:183、270-299）。

**バッチ化と対称変換での一致**：KataGoの`runnnbatchingtest`は、棋譜から採った局面を、まず1スレッドで1局面ずつ推論し、次に30スレッドで乱数の待ちを挟みながら並行に推論させてバッチを組ませる。
そのうえで、局面ごとの方策の最大値、勝ちと負けの確率の差、目数の予測、および地の所有の予測が、1スレッドの結果と許容幅の中で一致することを確かめる（KataGo/cpp/tests/testsearchmisc.cpp:183-306）。
許容幅は、勝ちと負けの確率の差なら0.015である。
この試験は、バッチの中で行が入れ替わる誤りや、同じバッチの他の行によって結果が変わる誤りを検出する。
`runnnsymmetriestest`は、同じ局面の8つの対称変換の出力を書き出す（testsearchmisc.cpp:54）。

**既知の局面での健全性の検査**：KataGoのカナリア試験は、手で選んだ局面について、指すべき手の方策が0.95以上であること、勝率が0.30から0.70の範囲にあることなどを、ネットの大きさに応じた許容の幅つきで検査する（testnnevalcanary.cpp:14-86）。
参照の数値がなくても、方策と価値の取り違えや入力の大きな誤りを検出できる。

**入力の符号化の試験と事例**：lc0は、ネットへの入力を盤の升ごとの値の面に並べ、これを入力平面と呼ぶ。
`encoder_test.cc`は、初期局面、50手規則の計数、および終盤の局面を入力平面へ符号化した結果を期待値と比べる（lc0/src/neural/encoder_test.cc:27-399）。
lc0より前に使われていたLeela Chess Zeroのエンジンには、過去の局面を表す履歴の入力平面を現在の手番側の向きへ反転しない不具合があった。
cremは2019年2月27日の投稿で、この不具合を4月中旬に直したこと、直したバイナリに合わせてネットの学習をやり直したこと、および不具合のあるネットも学習はしたが表現力を損なったと考えていることを述べた（CCC t=69881）。
投稿の日付から、修正は2018年4月と読める。
cremは、反転の不具合が、後継のエンジンlc0を書きながら食い違いを調べる過程で見つかったとも述べている。

### 探索の照合と単体試験

探索の照合は、ネットを試験用の評価器に替える方法、正解の分かる小さなゲームを解かせる方法、および探索木の不変条件を検査する方法に分かれる。

**試験用の評価器**：lc0は、ネットの代わりに使える推論器を3つ持つ。
推論器`random`は入力平面のハッシュから方策と価値を決定的に作り、`uniform`を指定すると一様な方策と0の価値を返す（lc0/src/neural/backends/network_random.cc:43-92）。
推論器`trivial`は、駒割りと駒の位置の表などからなる手作りの評価で方策と価値を作る（network_trivial.cc:30-41）。
推論器`recordreplay`は、実際のネットの出力を入力のハッシュとともにファイルへ記録し、後で同じ入力に対して再生するので、GPUを使わずに同じ探索を再現できる（network_record.cc:36-245）。
lc0の`engine_test.cc`は、モックの推論器と探索を使ってUCIの処理を試験する（src/engine_test.cc:33-120）。
KataGoの`debugSkipNeuralNet`はネットを呼ばず、シードを指定した乱数から方策を作る（KataGo/cpp/neuralnet/nneval.cpp:851-877）。
KataGoのCIで走る`runtests`は、この評価器を使った探索の試験を含む（command/runtests.cpp:82、tests/testsearchnonn.cpp）。

**解ける小さなゲーム**：OpenSpielのMCTSの試験は、三目並べの途中局面を探索させ、引き分け、負け、勝ちの各局面で、根の確定した結果と最善手が理論値に一致することを確かめる（open_spiel/python/algorithms/mcts_test.py:109-153）。
同じ試験は、訪問回数、報酬の合計、および確定した結果を手で与えた子の組から、最終手の選択関数が正しい子を選ぶことを、子の順序を乱して確かめる（mcts_test.py:155-210）。
AlphaZero.jlの`test_game`は、ゲームの実装について、乱数の手で到達した局面での複製と元の状態の一致、合法手の表と入力ベクトルの形、着手が元の状態を変えないこと、および対称変換の後の合法手の一致を検査する（AlphaZero.jl/src/scripts/test_game.jl:5-110）。

**探索木の不変条件の検査**：Ceresの`validate-store`は探索グラフの全節点を検査する（Ceres/src/Ceres.MCGS/UCI/UCIManagerMCGS.cs:805-817）。
検査の内容は、子の訪問回数が親を超えないこと、展開済みの子が未展開の子より前に並ぶこと、子から親への参照があること、節点が持つ方策の手の数と子の欄の数が一致すること、探索の根が1つであること、および根から全節点を辿れることである（Ceres/src/Ceres.MCGS/Graphs/GraphStores/GraphStore.Validate.cs:229-367）。
検査は最も新しく加えた節点から根へ向かって進め、不整合の原因に近い葉の側で止まるようにしている（GraphStore.Validate.cs:246-248）。
探索が止まった状態での検査では、評価待ちの訪問が残っていないことも確かめる（GraphStore.Validate.cs:545-549）。
lc0は節点の操作に`assert`を置き、事前確率が0から1の範囲にあることや、親子の参照の一致を確かめる（lc0/src/search/classic/node.cc:162-292）。

### 終局と詰みの扱い

**終端節点の確定値の伝播**：MOBMAT（CCC t=69993、2019年）は、三目並べのMCTSで選択が終端節点に達すると展開も逆伝播もせず、シミュレーションの回数だけが増えて木が育たない不具合を報告した。
返信したnionitaは、終端節点の値は厳密なので、勝ちの子を持つ親を勝ち、全ての子が負けの親を負けとして根まで伝え、確定した節点を再訪しないよう勧めた。
この伝播はMCTS-Solverと呼ばれ、OpenSpielの試験が確かめるのも同じ伝播である。
lc0の`VerboseMoveStats`は、勝敗または引き分けが確定した手に`(T)`を、負けがないことだけが分かった手に`(W)`を、勝ちがないことだけが分かった手に`(L)`を付ける（search.cc:533-543）。

**最終手の選択規則と詰みの報酬**：RedBedHed（CCC t=80947、2022年11月）は、詰まされかけた終盤で駒を捨てる不具合を約1か月追った。
原因は、最終手を訪問回数が最大の子で選ぶRobust Childにあった。
最終手を価値が最大の子で選ぶMax Childへ替えると、この症状は消えた。
同じスレッドでは、詰みの報酬を手数によらず1にすると、勝勢でも詰ませに行かず、50手規則の引き分けまで手を空費する症状も報告され、H.G.Mullerは速い詰みに遅い詰みより高い値を与えるよう勧めた。
dlshogiの履歴には、1手詰めがあるのに3手詰めの手を指す不具合の修正がある（コミットc844187、2021年10月3日、issue #64）。

### 並列化と仮想損失

**展開中の節点の競合**：dlshogiでは、あるスレッドが節点を展開して合法手を設定し終える前に、別のスレッドがその節点を訪れ、合法手が0なので詰みと判定する不具合があった。
修正は、評価済みを示すフラグの確認を詰みの判定より前へ移し、フラグを展開の処理の最後で立てる形をとった（DeepLearningShogi、コミットaf8b5f6、2020年12月10日）。

**評価待ちの訪問の表示と制限**：lc0の`VerboseMoveStats`は各手の評価待ちの訪問数を`(+n)`の形で示す（search.cc:482-488）。
lc0は、選択が評価待ちの同じ葉に重なる衝突について、1回のバッチで許す回数と訪問数をオプションで制限する（params.cc:333-338）。
KataGoの`benchmark`は、探索スレッド数の候補ごとに毎秒の訪問数を測り、スレッドを増やして探索の効率が落ちる分を含めた棋力の差の推定から推奨値を選ぶ（KataGo/cpp/command/benchmark.cpp:438-454）。

### 再現と回帰の検出

**固定条件の探索**：Leela Zeroのヘルプは、`--benchmark`の既定の引数を3,200訪問、ポンダーなし、ランダムな着手0、1スレッド、および乱数のシード1と記す（leela-zero/src/Leela.cpp:168-169）。
コードはこのうちスレッド数を除く設定を行い、さらにノイズと時間管理を切って、決まった局面を探索する（Leela.cpp:467-478、521-529）。
KataGoの`runsearchtests`は、6ブロック96チャネルの小さなネットで固定の訪問回数の探索を行う。
根の訪問回数、推論の行数とバッチ数、PV、および子ごとの統計の全文を`cpp/tests/results/`の期待出力へ書き出し、開発者が変更の前後の差分を読む（KataGo/cpp/runsearchtests.sh）。
期待出力の冒頭にはCUDAとRTX A5000で生成したことが記録されており（tests/results/runSearchTests-s1.txt:3-5）、この照合は機械をまたいだビット単位の一致を前提にしていないと読める。

**benchのノード数署名**：Montyのbenchは、選択経路の平均深さが6に達するまで1スレッドで探索し、CIはコミット本文の「Bench: 数字」と総ノード数を照合する（Monty/src/uci.rs:165-260、src/mcts.rs:278-285、src/chess.rs:126-129、.github/workflows/build.yaml）。
シミュレーションの回数で止めると総ノード数は制限値に等しくなるので、Montyは木の形に依存する停止条件を使って署名を作っている。
署名を機械をまたいで使うには、推論の数値も機械によらず一致しなければならない。
Montyは推論をCPUで行い、価値のネットの一部の層をi8とi16の整数で量子化している（src/networks/value.rs:16-26）。
OpenBenchは、公開エンジンに全ての機械で同じノード数を返すことを求め、AVXの機械とAVX2以上の機械の混在で非決定性が生じうると注意している（OpenBench wiki「Requirements For Public Engines」）。
lc0、Ceres、およびふかうら王のbenchは時間で区切る速度の測定であり、回帰の検出には使われていない（lc0/src/tools/benchmark.cc:64-68、Ceres/src/Ceres.MCGS/UCI/UCIManagerMCGS.cs:717-740、YaneuraOu/source/benchmark.cpp:445-453）。

## minaseへの適用の可否

以上の手法をminaseの現状と照らすと、次の3点が設計の前提になる。

第1に、minaseは自作のコードで`unsafe_code = "forbid"`を宣言しており、AddressSanitizer、valgrind、Miriが主に検出するunsafeなコードのメモリ誤用は、自作のコードからは生じない。
依存ライブラリ（グローバルアロケータのmimallocなど）はこの宣言の対象外である。
共有置換表とLazy SMPは原子操作だけで状態を共有するので、自作のコードのデータ競合は言語が排除しており、残る論理的な競合はサニタイザでは見つからない。

第2に、minaseはperftの数値をテストの正しさの基準にしない方針をとり、perftは`bin/perft`の計測とデバッグの道具として残している（[movegen.md](../plans/archived/movegen.md)の9節）。
perftの既知値照合は適用せず、divideによる絞り込みだけが使える。

第3に、minaseの評価で成り立つ対称性は「段反転と陣営交換」だけであり、この性質は`crates/minase/src/eval/pst/tests.rs`と`crates/minase/src/eval/pst/features.rs`のテストが、手で置いた局面と初期局面について検査している。
左右反転の対称性テストは、中将棋の初期配置が左右非対称であり、学習PSTも筋ごとに異なる重みを持つため適用できない。

PV-MCTSの手法をminaseの現状と照らすと、さらに次の2点が言える。

第4に、ネットの入出力の照合と探索の単体試験については、先行エンジンと同じ役割のものをminaseがすでに持つ。
USIの独自コマンド`nn-eval`は固定の288局面の推論結果を書き出し、その結果をPyTorchの出力と照合する（[ネットの出力の照合の手引き](../guides/alphazero-network-check.md)）。
この照合はCPUとTensorRTのFP16をバッチの大きさ1、7、および256で実行するので、KataGoの`testgpuerror`とバッチ化の試験に近い役割を兼ねる。
`eval`は、MCTSで起動したminaseでは価値と、合法手ごとのlogitおよび事前確率を表示し、KataGoの`kata-raw-nn`やMontyの`policy`にあたる（[USIの独自拡張](../guides/usi-extensions.md)）。
logitは、softmaxで正規化する前の方策の値である。
MCTSの単体試験（`crates/minase/src/search/mcts/tests.rs`）は、衝突と取消しの後の仮想損失の復元、逆伝播での勝率の反転、および確定した勝敗の伝播の試験を含む。
同じ試験は、Gumbel AlphaZeroの選択規則を、DeepMindのMCTSの実装であるmctxの参照値とも照合する。

第5に、探索の後に根の子ごとの統計を表示する道具、探索木の不変条件の検査、およびMCTSの探索結果による回帰の検出は、minaseにない。
MCTSの探索器は終了時に`SearchEvent::Finished`だけを送るので（`crates/minase/src/search/mcts/mod.rs`）、lc0の`VerboseMoveStats`やdlshogiの`DebugMessage`にあたる出力はない。
探索木の実装（`crates/minase/src/search/mcts/tree.rs`）には、Ceresの`validate-store`のように木全体の整合を確かめる関数がない。
`bench`は、MCTSで起動したminaseでもαβ探索を測る。
回帰の検出に試験用の評価器を使う場合、`test-util`フィーチャの評価器（`crates/minase/src/nn/test_util.rs`）は、全ての手のlogitを0にするものと先頭の手のlogitだけを上げるものの2つで、どちらも価値を0.5に固定する。
このため、lc0の`random`のように局面のハッシュから方策と価値を作る評価器と比べると、通る探索の経路が偏ると考えられる。
並列化については、MCTSのオプション`Threads`が1だけを受け付けるので、dlshogiのような並列探索での節点の展開の競合は現状では生じない。
第5点の3つの道具を加える計画は、[PV-MCTSのデバッグ道具の設計書](../plans/archived/mcts-debugging-tools.md)にある。
