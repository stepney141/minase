# 規則書の読みが分かれる5つの箇所に関する典拠と先行実装の調査

本書は、規則書 [chu-shogi.md](../rules/chu-shogi.md) の第17版のうち、条文から読みが一意に決まらない4つの箇所と、条文を改めるかどうかが論点になる詰みの定義について、規則書の典拠と先行実装が何を定めているかを一次資料で調べた結果をまとめる。
5つの箇所は、[規則実装の形式検証の実現可能性と検証を強める候補](../proposals/rules-formal-verification.md)の検討で見つかった。
調査日は2026年11月25日であり、本書が引用する原文とソースの行は、同月26日に該当箇所を開いて確かめ直した。
調べた事実から、どの読みを勧めるかは、[規則書の読みが分かれる箇所の読みの候補](../proposals/rulebook-readings.md)に書く。
調査の時点のminaseの実装の挙動は、[測定記録](../measurements/adhoc/rulebook-readings-behavior-20261126.md)にある。

資料の記号（［J1］、［E4］など）は、規則書の第11章に従う。
足、先獅子、付け喰い、じっとなどの規則の用語と、L0やP2などの規則コードも、規則書の定義に従う。
「連盟」は日本中将棋連盟をいう。
先行実装として、オンライン対局サイトLishogiが使う規則のライブラリscalashogiと、H. G. Muller作の中将棋エンジンHaChuを調べた。

## 結論

5つの箇所のうち、典拠に直接の定めがあるのは詰みの定義だけである。

| 箇所 | 典拠の定め | 先行実装の挙動 |
|---|---|---|
| 同一局面の条件としての先獅子（第24条第1項c） | 定めはない。連盟の規則は同一局面を「駒の配置及び手順が同じ」と書き、英語の資料は「同じ手番の同じ局面」と書くだけである。 | scalashogiは、獅子が取られた升の記録を反復の判定のハッシュに含めるが、ハッシュに入るのは1手前の着手の記録である。HaChuは、駒の配置だけで比べる。 |
| 不成を選んだ直後の成りの待機（P2）と、最奥段の救済の併用（第30条） | 定めはない。P2の系統の資料に待機と救済の優先の記述はなく、救済を定める連盟の資料に待機はない。 | HaChuは、待機中の香車を最奥段で成らせない。ただしHaChuの設定は香車の救済（P3）を含まない。scalashogiは不成の履歴を持たない。 |
| 付け喰いの優先が及ぶ範囲（第15条第8項、第16条第7項） | 連盟の補足編は、獅子を添えて取る付け喰いを認める事例と、先獅子の下での付け喰いを認める事例を別々に挙げており、両者が重なる局面の定めはない。 | HaChuとscalashogiは、獅子が動く着手に先獅子の禁止を適用しないので、重なる局面での2つの読みを区別する手がかりにならない。 |
| 終局条件の優先順位 | 定めはない。 | scalashogiは、王駒の全滅、裸玉、合法手なし、反復、駒の不足による引き分けの順に判定する。HaChuに順位を定める箇所はない。 |
| 詰みが王手を要件とするか（第21条第3項） | 連盟の初心者向けの解説は、王手を受けていることを詰みの要件に含める。英語版Wikipediaも同じである。 | scalashogiとHaChuは、詰みを判定しない。 |

## 調べた資料と版

ウェブの資料は2026年11月25日と26日に取得した。

| 記号 | 資料 | 版または取得元 |
|---|---|---|
| ［J1］ | 日本中将棋連盟「中将棋連盟競技規則・総則編（2019年度版）」 | [rule2007.htm](https://www.chushogi-renmei.com/kouza/rule2007.htm)（2019年9月14日訂正の表示がある）。初心者向けの解説は[rule.htm](https://www.chushogi-renmei.com/kouza/rule.htm) |
| ［J2］ | 日本中将棋連盟「中将棋対局規定　補足編」 | [rule_hosoku2004.htm](https://www.chushogi-renmei.com/kouza/rule_hosoku2004.htm)（2004年6月6日更新の表示がある） |
| ［J3］［H1］ | 岡崎史明「中将棋の指し方」と『中将棋指南抄』の翻刻 | [Okazaki.htm](https://www.chushogi-renmei.com/kouza/Okazaki.htm) |
| ［E1］ | English Wikipedia “Chu shogi” | [2026年9月8日の版](https://en.wikipedia.org/w/index.php?title=Chu_shogi&oldid=1373876688) |
| ［E2］ | H. G. Muller, “Chu Shogi,” The Chess Variant Pages | Wayback Machineの2026年1月1日の[写し](https://web.archive.org/web/20260101072321/https://www.chessvariants.com/rules/chu-shogi)（最終更新は2025年12月1日） |
| ［E3］ | H. G. Muller, “The Chu Shogi promotion rule” | [ChuPromoRule.html](https://hgm.nubati.net/MSM/ChuPromoRule.html) |
| ［E4］ | scalashogi | [コミット`9a1c2c3`](https://github.com/WandererXII/scalashogi/tree/9a1c2c3ae9167da60f47366e922b2f84c8bcda4e)の手元の複製。以下のパスは`src/main/scala/`からの相対パスである |
| ［E5］ | HaChu | [Debianのパッケージのソース](https://salsa.debian.org/debian/hachu)が収録する、開発元のコミット`df26f4a`に当たる`hachu.c`の手元の複製 |
| ［E6］ | Eric Silverman, “Chu Shogi, Part I: How to Play” | [drericsilverman.com](https://drericsilverman.com/2020/04/05/chu-shogi-part-i-how-to-play/) |

類例として、チェスと本将棋の規則と実装も調べた。

| 資料 | 版または取得元 |
|---|---|
| 国際チェス連盟（FIDE）のLaws of Chess | 2023年1月1日施行の版（[handbook.fide.com](https://handbook.fide.com/chapter/E012023)） |
| 日本将棋連盟「対局規則」 | [shogi.or.jp](https://www.shogi.or.jp/match/taikyoku_rules)（2025年10月1日改定の表示がある） |
| python-chess（Pythonのチェスのライブラリ） | [タグ`v1.11.2`の`chess/__init__.py`](https://github.com/niklasf/python-chess/blob/v1.11.2/chess/__init__.py) |
| Stockfish（チェスエンジン） | [タグ`sf_17.1`の`src/position.cpp`](https://github.com/official-stockfish/Stockfish/blob/sf_17.1/src/position.cpp) |

## 同一局面の条件としての先獅子

規則書の第24条第1項cは、同一局面の条件を「先獅子による直後の捕獲禁止の有無と対象」と書く。
獅子以外の駒が獅子を取ったが、取った側に獅子が残っていないなどの理由で禁止が生じない場合に、局面を区別するかどうかが論点である。

### 連盟の資料

［J1］六1は、千日手を「同一局面（駒の配置及び手順が同じ）が４回出現する前に、仕掛けた側が別な手を指さなければならない」と定める。
［J1］八の末尾は、双方が獅子などのじっとを繰り返して「同一局面がおきたときは千日手となる」と書く。
［J3］は「千日手は仕掛けた方より変えなければならない」と書くだけで、同一局面を定義しない。
［J2］には「千日手」の語がない。
先獅子と千日手の関係、および獅子の取り返しの制限を同一局面の条件に含めるかどうかに触れる記述は、［J1］［J2］［J3］と［H1］のどれにもなかった。

### 英語の資料

［E1］のRepetitionの節は、“In principle a player may not make a move if the resulting position is one that has previously occurred in the game with the same player to move.”と書く。
［E2］は、“Historic descriptions of Chu Shogi mention that repeating a position that occurred previously (with same side to move) is forbidden.”と書く。
どちらも、局面を構成する要素を手番のほかに定義せず、獅子の取り返しの制限の状態に触れない。

### scalashogi

scalashogiは、獅子が取られた升の記録を反復の判定のハッシュに含めるが、ハッシュに入るのは1手前の着手の記録である。

- ハッシュは、局面を空白で区切った欄で表す文字列（SFEN）から計算したMD5の値の先頭3バイトである（`Hash.scala`の7行目から12行目）。中将棋のSFENでは、第3欄が`history.lastLionCapture`の升を表し、記録がなければ`-`になる（`format/forsyth/Sfen.scala`の86行目、112行目から113行目）。
- 記録の条件は、動かした駒が獅子でなく、着手の経路の升に相手の獅子がいたことだけである（`Situation.scala`の141行目から152行目）。取った側に獅子が残っているか、その獅子に足があるかは条件にない。
- 記録は着手のたびに上書きされるので、獅子を取った着手の直後の1局面にだけ残る。合法手の生成は、この記録を参照する。
- 着手nの後の局面の履歴は、n手目の記録を持つ。一方、保存する反復のハッシュは、新しい履歴を付ける前の局面から計算するので、n−1手目の記録が入る（`Situation.scala`の138行目で着手前の履歴を持つ局面を作り、163行目でそのハッシュを計算し、165行目で新しい履歴を付ける）。
- 4回の反復の判定は、ハッシュが12個を超えることを前提にする（`History.scala`の19行目から20行目）。

この読解は実行で確かめていない。
読解のとおりなら、獅子以外の駒が獅子を取った着手の直前の着手に記録がない場合、取った直後の局面のハッシュには記録が入らず、その1手後の局面のハッシュに記録が入る。

### HaChu

HaChuは、反復を駒の配置のハッシュだけで判定し、先獅子の状態を含めない。

- 反復の判定は、2手おきに`hashKeyH`を比べる（`hachu.c`の2287行目）。
- `hashKeyH`を更新するのは、駒のキーと升のキーの積の排他的論理和だけである（1499行目から1587行目）。
- 獅子以外の駒が獅子を取ったことを表すフラグは、変数`defer`を通して次の探索へ渡され（2318行目）、獅子以外の駒が獅子を取る着手の合法性の判定に使われる（2315行目から2316行目）。このフラグは`hashKeyH`に入らない。

したがってHaChuでは、獅子以外の駒が獅子を取った直後の局面と、後に現れる同じ駒の配置と手番の局面は、禁止が効いている場合も含めて同一である。

### 類例

FIDEの規則は、同一局面を可能な着手で定義する。
9.2.3は“Positions are considered the same if and only if the same player has the move, pieces of the same kind and colour occupy the same squares and the possible moves of all the pieces of both players are the same.”と定め、9.2.3.1は、局面が同じでない場合として“at the start of the sequence a pawn could have been captured en passant”を挙げる。
アンパッサンで取れる駒が実際にはない場合を名指しで扱う文言は、条文になかった。

チェスの実装は、アンパッサンの升を同一局面の判定のキーに含める条件が分かれる。
python-chessは、合法なアンパッサンの捕獲がある場合に限って升をキーに含める（`_transposition_key`、3797行目から3802行目）。
Stockfishの`sf_17.1`は、相手のポーンがその升に利いていれば升をキーに含め、捕獲が合法かどうかは調べない（813行目から819行目）。

日本将棋連盟の対局規則の第8条第1項は、千日手を「盤面（盤上の駒の配置）・双方の持ち駒・手番のすべてが同一となる局面（同一局面）が4回発生した場合」と定める。

## P2の待機と最奥段の救済の併用

P2は、敵陣へ入る着手で不成を選んだ駒が、その側の直後の1手番に限り、非捕獲の着手で成れないと定める。
第19条第1項は歩兵に、P3は香車に、P4は仲人に、最奥段で非捕獲による追加の成る機会を与える。
待機中の駒が最奥段へ非捕獲で進むときに、どちらが優先するかが論点である。

### 連盟の資料

救済を定めるのは連盟の資料であり、連盟の資料に1手の待機という考え方はない。

- ［J1］四3は「歩兵が不成で進んだ場合、敵駒を取るかあるいは敵陣の最後段まで進んだときに再び成る機会が与えられる」と定め、歩兵の救済措置と注記する。
- ［J1］四4は「歩兵および香車が最後段まで不成で進んだ場合、その場で死に駒(行かずの駒)として扱う」と定める。
- ［J1］四5は「通信での対局に限り、香車の不成後の扱いも3.の歩兵に順ずるものとする」と定める。
- ［J2］の1は、歩兵の救済の理由を「歩兵はその性能上、後退できないので」と書く。香車については「性能上後退の出来ない駒」であり、「歩兵と同様の規定をあてることでいいのではないか」として、「現時点では通信対局上でのみ、救済措置としてこのルールを適用する」と書く。

### 英語の資料

1手の待機を記録するのは［E3］であり、［E3］は待機と最奥段の救済の優先を書かない。

- ［E3］は、“In the MSM this can be done by moving the piece (a forced deferral if it is a non-capture). In the Wikipedia the promotability returns automatically after a one-turn delay.”と書く。MSMはGeorge Hodgesの『Middle Shogi Manual』であり、“the Wikipedia”は当時の英語版Wikipediaである。
- ［E3］は、連盟の方式の説明の中で、歩兵を最奥段で成らせる方式に触れ、香車について“no such rule is needed, as there never can be any reason to defer promotion of a Lance”と書く。
- ［E1］の現在の版は1手の待機を記述せず、歩兵について“If a pawn reaches the furthest rank, it gets a second opportunity to promote on a non-capture.”と書く。

### HaChu

HaChuは、P2に当たる設定（“Promote on entry”を無効にした設定）で、待機中の駒の非捕獲の成りを、最奥段でも生成しない。

- 非捕獲の着手を生成する処理は、待機の対象の升にいる駒について、成りの条件を表すフラグを0にする（`hachu.c`の1304行目）。最奥段を除外する分岐はない。
- フラグが0の駒には、不成の手だけが生成される（1208行目から1219行目）。
- 入陣の着手で不成の手も生成される駒では、生成された不成の手に待機の印が付く（1212行目から1215行目）。香車と歩兵は、成駒が元の駒の動きをすべて含むので、不成の手が生成されない。生成されなかった不成を利用者の入力で受け付ける経路では、不成への置き換えが許される場合に、歩兵は成りの条件が最奥段での成りだけに絞られて待機は付かず、ほかの駒は、敵陣の外から入る着手であれば待機が付く（2713行目から2727行目）。敵陣の外から最奥段へ直接進む香車の不成の入力は、不成への置き換えが許されないので、待機は付かない。

したがって、待機中の香車が非捕獲で最奥段へ進む手は不成だけであり、その香車は以後動けない。
歩兵には待機が付かないので、待機中の歩兵が最奥段へ進む状況は生じない。
規則書の第33条第7項が記すとおり、HaChuのこの設定は「P2＋P5＋P6」に当たり、香車の救済（P3）を含まない。

### scalashogi

scalashogiは、不成を選んだ履歴を持たない。
`canPromote`は、駒、移動元、移動先、および捕獲の有無だけから成りの可否を決め、歩兵と香車は到達升が最奥段であれば成れる（`variant/Chushogi.scala`の263行目から272行目）。

### 見つからなかったもの

P2の待機と、P3またはP4の救済を併用する規則を記した資料は、見つからなかった。

## 付け喰いの優先が及ぶ範囲

第15条第8項と第16条第7項は、付け喰いを先獅子より優先すると定める。
先獅子の禁止が成立している直後の1手で、獅子が第1段階で足のある隣接する相手の獅子Aを取り、第2段階で隣接しない別の相手の獅子Bを取る着手について、Aへの禁止まで解かれて着手が合法になるかが論点である。

### 連盟の資料

優先を定めるのは［J2］であり、［J2］は2つの条件が重なる局面を扱わない。

- ［J2］の3④は「先獅子と付け喰いは、付け喰いを優先する。先獅子の状態であっても付け喰いが出来る状態なら相手の獅子を付け喰いの駒とともに捕獲出来る」と定める。
- ［J2］の3⑤は「付け喰いが出来ない駒は、従来同様、歩兵と仲人の２種類」と定める。
- ［J2］は、3の①から⑥について「④のみ連盟が補足する形となりましたが、それ以外は中将棋全集に基づく改定となっています」と注記する。優先の理由は書いていない。
- ［J2］のケース2は、獅子が隣接する獅子を取ってから別の獅子を取る着手を、「隣接している獅子は無条件で喰うことが出来ますので、１二▽獅子も２二▽獅子を付け喰いするという形で取ることが出来ます。（付け喰いの駒は歩兵と仲人以外なら何でも可です）」と認める。この事例に先獅子は関わらない。
- ［J2］のケース3は、「先手が後手の獅子を飛車で取った時点で先獅子が成立していますが、後手は付け喰いが出来ます」と認める。この事例で第1段階に取る駒は銀将である。
- ［J1］七・四は先獅子を「敵側は直後の一手で獅子を取り返すことが出来なくなる」と定め、七・五は付け食いを「獅子と獅子の間に敵の駒がある場合、この駒を取った上で相手の獅子も一緒に取ることが出来る」と定める。両者の関係には触れない。
- ［J3］の（Ｃ）は「獅子と獅子の間に敵の駒があれば、その駒と共に獅子を獅子で取ることが出来る」と書き、歩兵と仲人だけを除く。

### 英語の資料

［E1］と［E2］は、先獅子に当たる禁止を獅子以外の駒による捕獲に限るので、獅子による付け喰いとの優先関係を書かない。

- ［E1］は、“A non-lion cannot capture a lion when in the immediately preceding move a lion was captured by a non-lion on another square.”と書く。
- ［E1］は、獅子による獅子の捕獲の制限の例外を“unless it captures something substantial (i.e. other than pawn or go-between) together with the lion in a double capture”と書く。
- ［E2］は、“If you capture another piece together with the opponent Lion, you can even do it when your Lion can be recaptured. This additionally captured piece must not be a Pawn or Go Between, though.”と書く。

どちらも、添えて取る駒から獅子を除く文言を持たず、1手で獅子を2枚取る場合に触れない。

### HaChuとscalashogi

どちらの実装も、獅子が動く着手に先獅子の禁止を適用せず、添えて取る駒が獅子であることを理由に扱いを変える分岐を持たない。

- HaChuは、獅子が獅子を取る着手を、到達升が隣接せず、着手の後に到達升へ相手の利きがあり、第1段階で取った駒の価値が50以下である場合に限って禁じる（`hachu.c`の2311行目から2313行目）。価値が50以下の駒は歩兵と仲人である（222行目、223行目）。先獅子のフラグを調べるのは、獅子以外の駒が獅子を取る分岐だけである（2315行目から2316行目）。
- scalashogiの`lionMoveFilter`は、獅子が動く場合に、経由升で取る駒が歩兵でも仲人でもなければ到達升の獅子の捕獲を認め、獅子の捕獲の記録を参照しない（`variant/Chushogi.scala`の298行目から318行目）。記録を参照するのは、獅子以外の駒が動く場合だけである（319行目から326行目）。

どちらの実装でも、Aだけを取る着手と、AとBを取る着手は、ともに合法になる。
標準規則のL0は、先獅子の下でAだけを取る着手を禁じる。
その場合にAとBを取る着手まで禁じるかどうかについて、2つの実装は手がかりにならない。

## 終局条件の優先順位

規則書は、王駒の捕獲、反復（R1）、駒枯れまたは裸玉、合法手なし、および詰みが、同じ着手の後に同時に成立する場合の順位を定めていない。

### 連盟の資料

［J1］二は、勝ち方として、詰みと突き落とし（王駒の捕獲）、投了、駒枯れ、合意による持将棋、および反則負けを挙げる。
条件どうしの順位は定めていない。
［J1］九には、複数の条件が関わる事例がある。
駒枯れの条件が成立する着手の直後に、相手の太子がその駒を取り返せる局面について、駒枯れの勝ちが「成立するように思えますが、すぐに太子が獅子を取り返せるのでこの場合は持将棋となります」と書く。
千日手と、詰み、捕獲、または駒枯れとの関係に触れる記述はなかった。

### scalashogi

scalashogiは、着手の後の局面の状態を、最初に成立した1つに決める（`variant/Chushogi.scala`の402行目から410行目）。
順は、手番側の王駒の全滅、いずれかの側の裸玉、合法手なし、4回の反復、および駒の不足による引き分けである。
裸玉は、相手に2枚以上の価値ある駒があるなどの条件を満たして負けになる状態をいう（353行目から382行目）。
駒の不足による引き分けは、死に駒を除いて双方が王駒1枚ずつになり、どちらの王駒にも相手の利きがない状態をいう（384行目から400行目）。
scalashogiは詰みを判定しない。

### HaChu

HaChuに、終局条件の順位を定める箇所はない。
探索の処理は、相手の唯一の王駒へ利きがあれば勝ちの値を返す（`hachu.c`の2071行目から2075行目）。
合法手がない場合を引き分けにする処理は、チェス系の変種にだけ働く（1079行目、2404行目）。

### 英語の資料

［E1］は、裸玉について“a bare king immediately loses against any other material, unless the player can bare the opponent on the following move (in which case the game is a draw), or the player can capture the opponent's sole remaining king or prince on the following move (in which case the opponent loses)”と書く。
［E2］は、“you can win by King baring, unless in the immediately following move your King is bared too (or captured to leave you without royal)”と書く。
これらが、終局条件どうしの関係を述べる記述のすべてである。

### 類例

FIDEの規則は、5.1.1で、詰みが“immediately ends the game”と定める。
9.6は、同じ局面の5回の出現（9.6.1）と75手の規則（9.6.2）による引き分けを定める。
9.6.2の末尾には“If the last move resulted in checkmate, that shall take precedence.”とある。
この文が9.6.1にも及ぶかどうかは、条文に明記がない。

## 詰みが王手を要件とするか

第21条第3項は、詰みを、逃れとなる合法手がないことで定義する。
最後の王駒が王手を受けていることを要件にしていない。
同項は、典拠に明文がない事項を補った補充規定である。

### 連盟の資料

［J1］の総則編は、詰みを定義しない。
連盟の初心者向けの解説（rule.htm）は、「詰めるというのは、王将が王手をかけられてしまい、なおかつ他に逃げる場所が無くなってしまった状態のことを言います」と定義する。

［J1］九の第2図の解説は、動かせる駒が盲虎しかない後手について「盲虎が動いた時点で負けが決まります」と書き、その後の先手の着手を「▲２三獅子で合い駒きかずの詰めです」と書く。
続けて「先手がバカな手をして盲虎と相討ちにでもなってしまえば勝敗は変わります」と書く。
この解説は、盲虎を動かさざるを得ない時点を負けの決まる時点と呼び、その後の王手を詰みと呼んでいる。

### 英語の資料

［E1］は、“If a player's king or prince is in check and no legal move by that player will get it out of check, the checking move is also mate, and effectively wins the game.”と書く。
［E6］は、“in Chu Shogi you must capture the King, rather than checkmate it”と書き、詰みによる終局を採らない。
［E2］は、合法手がない場合について“For definiteness we can assume that stalemate is a win.”と書き、詰みを定義しない。

### 類例

日本将棋連盟の対局規則の第2条第5項は、「詰みとは、次にどのように応対しても玉が取られてしまうことを防げない状態をいう」と定め、王手を要件に書かない。
本将棋は、中将棋と異なり、王手を放置する着手を禁じる。

## 確認できなかった事項

- scalashogiの反復のハッシュに1手前の記録が入ることは、コードの読解に基づくものであり、実行して確かめていない。
- ［E3］が“the Wikipedia”と呼ぶ当時の英語版Wikipediaの本文は取得していない。
- George Hodgesの『Middle Shogi Manual』の原著は確認していない。
