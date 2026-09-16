# 秒読みつき時間制御における既存エンジンの持ち時間配分

2026年9月17日に、持ち時間と秒読みを併用する時間制御で既存エンジンが持ち時間をどう配るかを、将棋、チェス、囲碁のエンジン7本のソースコードで確認した。
確認の目的は、[持ち時間の効率的な使用](../plans/time-management-efficiency.md)が採る配分（秒読みの分担を毎手の底とし、持ち時間の分担を序盤から上乗せする方式）と、その代案である持ち時間の温存（持ち時間が十分ある間は秒読みの分担を予算に含めない方式）のどちらを参照実装が支持するかを知ることである。

結論は次の3点である。
第1に、対局全体で持ち時間を温存するエンジンは1本もなく、調査した7本のうち秒読みに対応する6本は、秒読みを毎手の底として持ち時間の分担に上乗せするか、下限として用いる（Aperyの上乗せは20手目以降に限る）。
第2に、持ち時間を早く使い切ることは各エンジンが意図的に選んでおり、対数価値モデルから配分を導くKataGoは、設計書の配分より速い消費を最適としている。
第3に、序盤だけ消費を絞る仕掛けは将棋エンジンの多数派（Apery、技巧、dlshogi）にあり、これは温存とは独立の機構である。
理由を明記するのはYaneuraOuのコメント（序盤は定跡で進み、勝負どころではない）だけで、Apery、技巧、dlshogiは理由を書いていない。確認したソースコードとコメントの範囲では対局測定の記録は見当たらず、Aperyだけが選手権条件で実戦調整した旨をコメントに残す。

## 前提となる時計の規則

以下の比較は、持ち時間が残る間は思考時間の全額を持ち時間から引き、持ち時間が尽きた手から1手ごとに秒読みを与え、未使用の秒読みを持ち越さない時計（持ち時間優先）を前提とする。
lishogiの`Clock.step`、Minaseの対局ハーネスの`Clock::update`、および標準的な将棋GUIはこの規則である。
この規則では、秒読みは持ち時間が尽きるまで使えないので、序盤に節約した持ち時間は「秒読みを超える長考を後で行う」場合にだけ価値を生む。
秒読みを先に消費して超過分だけを持ち時間から引く方式（チェスのブロンシュタイン式遅延に相当）ではこの前提が崩れ、秒読みは毎手無料の枠になるため、以下の比較は当てはまらない。
USIの`go`は残り時間、加算、秒読みの値だけを伝え、時計の規則を伝える欄はないので、エンジンは時計の所有者の規則を仮定するしかない。

## エンジンごとの方式

各エンジンの予算式、持ち時間を配る手数、および序盤の扱いを次の表にまとめる。
「初手の予算」は、issue #7の条件（持ち時間5分、秒読み10秒、加算なし）を各エンジンの式に当てた概算であり、最小思考時間や通信遅延の控除は無視した。

| エンジン | 秒読みの扱い | 持ち時間を配る手数（自分の手番） | 序盤の扱い | 初手の予算 |
|---|---|---|---|---|
| YaneuraOu | `(持ち時間 + 加算·MTG + 秒読み·MTG) / MTG`で毎手加算 | 初形で89手、80手目以降は50手 | なし（定跡が指す前提） | 約13.4 s |
| Apery | 20手目以降だけ秒読みを加算 | 定数の地平線 | 現在の手の重要度に、10手目未満は0.1、16手目未満は0.2、40手目未満は0.4、以後0.89を掛ける | 秒読みなしで持ち時間の分担を絞った値、1 s未満 |
| 技巧 | `(残り + B) / 35 + B`（`B = max(加算, 秒読み)`） | 35手 | 20手目未満は`(ply + 1) / 20`倍 | 約0.94 s、20手目以降は約18.9 s |
| dlshogi | `max(残り / 除数 + 加算, 秒読み)`で秒読みは下限 | 14手、序盤は`14 + (30 − ply)` | 30手目まで除数を大きくし、20手目まで探索延長なし | 約10 s（秒読みが下限） |
| Stockfish | 秒読みの概念なし | 50手 | plyの累乗で中盤へ寄せる | 該当なし |
| KataGo | 対数モデルの最適解を導出し、中盤のために1.75倍に緩和 | `(持ち時間 / 秒読み) / e`手の1〜1.75倍 | なし | 約15.5 s（19路初形、1期間、緩和前の理論値は約27.2 s） |
| Leela Zero | `(持ち時間 + 秒読み·(期間数 − 1)) / 残り手数 + 秒読み` | 盤面から推定 | 交点数の1/6手までは残り手数を大きく見積もり、持ち時間の分担を減らす | 該当なし（囲碁） |

Minaseの設計書の第1段階は、序盤の係数を掛ける前の予算（`remaining / moves_to_go + 0.8·byoyomi`、`moves_to_go`は初形で225手）が初手9,333 msであり、序盤の係数を持つApery、技巧の序盤を除けば最も控えめな部類に入る。係数を掛けた初手は933 msである。

### YaneuraOu

`source/timeman.cpp`は、残り時間の見積りを`remain_estimate = time + inc·MTG + byoyomi·MTG`とし、「秒読み時間も残り手数に付随しているものとみなす」と注記する。
標準予算は`minimumTime + remain_estimate / MTG`、上限は`minimumTime + max_ratio·remain_estimate / MTG`（`max_ratio`は既定5.0）で、かつ`remain_estimate`の30%以下である。
残り手数`MTG`は`min(引き分け手数 − ply + 2, move_horizon) / 2`で、`move_horizon`は`160 + 20 − min(ply, 80)`である。
コメントは、40手目付近までは定跡で進み勝負どころではないので地平線を大きめに取り、「100手時点で残り60手ぐらいのつもりで指していい。これくらいしないと勝負どころを過ぎてからの持ち時間が余ってしまう」「160手目ぐらいで持ち時間を使い切って問題ない」と述べる。
残り時間が秒読みの1.2倍未満なら残り時間と秒読みの全額を1手に使う（`isFinalPush`）。
序盤の係数はなく、`SlowMover`は手数によらない定数である。
[YaneuraOu timeman.cpp](https://github.com/yaneurao/YaneuraOu/blob/master/source/timeman.cpp)

### Apery

`src/timeManager.cpp`は、Stockfish 6系の`moveImportance`で持ち時間の分担を計算する。`Slow_Mover_*`の係数は現在の手の重要度に掛かり、その重要度は配分比の分子と分母の両方に入るので、予算がそのまま係数倍になるわけではない。その後、「秒読み対応」のブロックで秒読み（USIの`byoyomi`は`moveTime`へ読み込まれる）を扱う。
秒読みの加算は`gamePly >= 20`の場合に限り、標準予算と上限の両方へ秒読みを足す。
20手目より前は持ち時間の分担だけで指す。
`src/usi.cpp`の既定値は、`gamePly`が10未満で使う`Slow_Mover_10`が10、16未満の`Slow_Mover_16`が20、20未満、30未満、40未満の`Slow_Mover_20`、`Slow_Mover_30`、`Slow_Mover_40`が40、それ以降の`Slow_Mover`が89であり、コメントは「持ち時間15分、秒読み10秒では10にした（sdt5）」のように第4回および第5回将棋電王トーナメントの条件で調整した経緯を残す。
[Apery timeManager.cpp](https://github.com/HiraokaTakuya/apery/blob/master/src/timeManager.cpp)、[Apery usi.cpp](https://github.com/HiraokaTakuya/apery/blob/master/src/usi.cpp)

### 技巧

`src/time_control.cc`の`ByoyomiTimeControl`は、基礎時間を`remaining_time() / kHorizon + time_per_move()`とする。`kHorizon = 35`、`time_per_move()`は加算と秒読みの大きい方、`remaining_time()`は持ち時間に`time_per_move()`を加えた値である。「序盤の消費時間を削減する」として、初形を0とする`game_ply < 20`の間は`(game_ply + 1) / 20`倍に縮める。
上限は基礎時間の5倍、下限は3分の1である。
加算つきの`FischerTimeControl`も同じ序盤の削減を持つ。
[技巧 time_control.cc](https://github.com/gikou-official/Gikou/blob/master/src/time_control.cc)

### dlshogi

`usi/UctSearch.cpp`の`SetLimits`は、`divisor = 14 + max(0, 30 − gamePly)`として`time_limit = remaining / divisor + inc`を計算し、秒読み（`moveTime`）を下回る場合は秒読みを`time_limit`とする。
秒読みは上乗せではなく下限である。
最善手と次善手の探索回数が拮抗した場合の探索延長（2倍、1回だけ）は`gamePly > 20`かつ残り時間が`time_limit`の2倍を超える場合に限る。
[dlshogi UctSearch.cpp](https://github.com/TadaoYamaoka/DeepLearningShogi/blob/master/usi/UctSearch.cpp)

### Stockfish

`src/timeman.cpp`は加算方式と残り手数指定だけを扱い、秒読みの概念を持たない。
残り手数の既定は50手で、`optScale`はplyの累乗項を含み中盤へ時間を寄せる。
上限は標準予算の数倍で、残り時間の約8割を超えない。
[Stockfish timeman.cpp](https://github.com/official-stockfish/Stockfish/blob/master/src/timeman.cpp)

### KataGo

`cpp/search/timecontrols.cpp`の`divideTimeEvenlyForGame`は、秒読みの分岐で次のように注記する。
「毎手一定の時間を使い、持ち時間が尽きたら秒読みを使い、強さが思考時間の対数に比例すると仮定すると、最適方針は秒読みのe倍を毎手使い、秒読みだけを使う場合の1/eの手数で持ち時間を使い切ることである」。
そのうえで「実際には中盤の十分深い局面のために時間を残す方が重要」として、使い切る手数を理論値の1.75倍まで延ばし、盤面から推定した残り手数を超えないようにする。
さらに、1手の基礎時間が秒読みを下回らないようにし、残り時間が秒読みの1.5倍未満なら残り時間と秒読みを1手で使う。
この理論は設計書の「持ち時間の温存を採用しない理由」の対数価値モデルと同じであり、結論はMinaseの配分より速い消費である。
[KataGo timecontrols.cpp](https://github.com/lightvector/KataGo/blob/master/cpp/search/timecontrols.cpp)

### Leela Zero

`src/TimeControl.cpp`の`max_time_for_move`は、秒読みに入る前は`持ち時間 + 秒読み·(期間数 − 1)`を推定残り手数で割り、そこへ秒読み1期間分を毎手加える。推定残り手数（`get_moves_expected`）は、交点数の1/6手（`opening_moves`）までは大きく見積もり、序盤の持ち時間の分担を減らす。
秒読みに入った後は期間の時間だけを使う。
[Leela Zero TimeControl.cpp](https://github.com/leela-zero/leela-zero/blob/next/src/TimeControl.cpp)

## Minaseへの含意

対局全体で温存する配分を採らないという設計書の決定は、参照実装の全体と一致する。
一方、「最初の数手で秒読み相当の時間を使う」ことへの対処は、温存ではなく序盤の係数として切り出せる。
Apery、技巧、dlshogiの仕掛けはいずれもplyに応じた係数または除数であり、Minaseでは予算式に掛ける係数1つとして実装できる。
中将棋は1局を450手と見込むので、将棋の20〜40手の立ち上げをそのまま使う根拠はなく、Minaseには定跡がないので「定跡で進む」という根拠も当てはまらない。
係数の長さは測定で調整された値ではなく設計上の判断値であり、設計書は1つの値を候補として定め、パラメータ違いを追加しない。
