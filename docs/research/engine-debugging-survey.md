# エンジン開発のデバッグ手法の調査

本書は、Chess Programming Wiki（以下CPW）の「Debugging」頁とその関連頁および引用投稿、ならびにStockfish、YaneuraOu、Fairy-Stockfish、HaChuのソースを調べ、エンジン開発で使われるデバッグ手法を整理した調査メモである。
[デバッグ機能の整備](../plans/debugging-tools.md)の設計書が方式の根拠として参照し、[デバッグの手引き](../guides/debugging.md)が手法の出典として参照する。
調査日は2026年9月27日である。

## 結論

調べた手法は、同じ計算を2通りに行って一致を確かめる照合、内部状態を人が読める形で表示する道具、探索の挙動を数える統計、および入出力と再現の記録の4種類に分かれる。
3つの主流エンジン（Stockfish、YaneuraOu、Fairy-Stockfish）に共通する最小限の組は、局面の表示（`d`）、評価値の表示（`eval`）、合法手の列挙、入出力のログファイル、決定的なbenchのノード数による回帰検出、および実行と取消しの後の局面の不変条件検査である。
minaseは、合法手の列挙（USIの`moves`）、不変条件検査（`Position::validate`）、benchのノード数照合（`scripts/bench_compare.py`）を持つ一方、局面と評価値の表示、入出力のログ、置換表の照会、および探索統計を持たない。

置換表を照会して探索後の木を辿る方法と、指し手順序の品質統計は、CPWの「Debugging」頁がTord Romstadの提案として挙げるが、3つの主流エンジンはいずれも常設していない。
3エンジンの統計関数（Stockfishの`dbg_hit_on`など）は呼び出し箇所を1つも持たず、開発者が一時的に差し込む道具として扱われている。
局面の完全な再計算照合は、速度と衝突して既定では無効にされる傾向がある。
Stockfishは2023年に局面状態全体の再計算照合を外し、Fairy-Stockfishは`Fast = true`で既定の検査を軽量版に限り、YaneuraOuはハッシュキーの照合を実行時の検査ではなく単体テストのランダム対局へ移した。

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

## minaseへの適用の可否

以上の手法をminaseの現状と照らすと、次の3点が設計の前提になる。

第1に、minaseは自作のコードで`unsafe_code = "forbid"`を宣言しており、AddressSanitizer、valgrind、Miriが主に検出するunsafeなコードのメモリ誤用は、自作のコードからは生じない。
依存ライブラリ（グローバルアロケータのmimallocなど）はこの宣言の対象外である。
共有置換表とLazy SMPは原子操作だけで状態を共有するので、自作のコードのデータ競合は言語が排除しており、残る論理的な競合はサニタイザでは見つからない。

第2に、minaseはperftの数値をテストの正しさの基準にしない方針をとり、perftは`bin/perft`の計測とデバッグの道具として残している（[movegen.md](../plans/movegen.md)の9節）。
perftの既知値照合は適用せず、divideによる絞り込みだけが使える。

第3に、minaseの評価で成り立つ対称性は「段反転と陣営交換」だけであり、この性質は`src/eval/pst/tests.rs`と`src/eval/pst/features.rs`のテストが、手で置いた局面と初期局面について検査している。
左右反転の対称性テストは、中将棋の初期配置が左右非対称であり、学習PSTも筋ごとに異なる重みを持つため適用できない。
