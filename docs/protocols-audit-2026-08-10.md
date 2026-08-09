# プロトコル調査文書の独立監査

## 監査範囲

本監査は、現行HEAD `1561f3bf7d22f2a2f985b58e3f3af405101211ad`を基準として、`docs/protocols/README.md`、`docs/protocols/cecp.md`、`docs/protocols/hachu.md`、`docs/protocols/usi-lishogi.md`を対象とした。

監査では3件の独立調査を行い、CECP（Chess Engine Communication Protocol）とUSI（Universal Shogi Interface）の原典、固定コミットのHaChu、shogiops、scalashogi、Lishogi-Bot、および現行のminase実装と照合した。

監査中に文書の修正コミットが加わったため、解消済みの指摘は除外し、上記HEADに残る所見だけを記録する。

## 重大な所見

### HaChuの先獅子状態

`docs/protocols/hachu.md:109`は、`sup0`から`sup2`が「非獅子による獅子捕獲後」の状態を保存すると説明している。

しかし、`MakeMove`が`PROMOTE`を返すのは獅子が中間升の獅子を取る場合だけであり、`MakeMove2`が到達升の捕獲について`PROMOTE`を追加するのも移動駒と被捕獲駒がともに獅子の場合だけである［[board.c 291〜371行](https://github.com/ddugovic/hachu/blob/649ef114dd5fa39d3e1be4112e63ebe2ab3d5d8b/board.c#L291-L371)］［[hachu.c 887〜896行](https://github.com/ddugovic/hachu/blob/649ef114dd5fa39d3e1be4112e63ebe2ab3d5d8b/hachu.c#L887-L896)］。
非獅子による獅子捕獲後のフラグは探索中の一時状態には存在するが、実対局のルート局面には保存されない［[hachu.c 731〜754行](https://github.com/ddugovic/hachu/blob/649ef114dd5fa39d3e1be4112e63ebe2ab3d5d8b/hachu.c#L731-L754)］。

したがって、`docs/protocols/hachu.md:142`のL0およびL1との対応付けも、実対局のルート状態については成立しない。
この記述を拡張SFENの設計根拠にすると、HaChuが実際には保持していない状態を保持していると誤認する。
重大度と確信度はいずれも高い。

### 複数レグ着手の獅子捕獲升

`docs/protocols/usi-lishogi.md:136`は、scalashogiが「動かした駒が獅子でなく、かつ到達升に相手獅子がいた」場合だけ獅子捕獲状態を記録すると説明している。

しかし、scalashogiは移動元を除く全移動升を最終升側から調べ、獅子が捕獲された升を記録する［[Situation.scala 133〜152行](https://github.com/WandererXII/scalashogi/blob/9a1c2c3ae9167da60f47366e922b2f84c8bcda4e/src/main/scala/Situation.scala#L133-L152)］。
shogiopsも中間升と最終升の双方を獅子捕獲升として扱う［[position.ts 350〜374行](https://github.com/WandererXII/shogiops/blob/e295794f792e41c9b0a28aeb30faf9b89c951876/src/position/position.ts#L350-L374)］。

文書どおりに最終到達升だけを検査すると、角鷹や飛鷲が中間升で獅子を捕獲した局面でSFEN第3欄から先獅子状態が失われ、次局面の合法手集合が変わる。
重大度と確信度はいずれも高い。

## 中程度の所見

### `new`コマンドの解釈

`docs/protocols/hachu.md:57`と同文書164行目は、チェスを指せないエンジンが`new`での位置設定を後続の`variant`まで遅延すべきだと説明している。

しかし、[GNU CECP仕様](https://www.gnu.org/software/xboard/engine-intf.html)は、`new`で標準チェスの初期局面へ戻し、その後、標準チェス以外の対局なら`variant`を送ると規定している。
HaChuが`new`で直ちに初期化すること自体は仕様乖離ではなく、`nocastle`の配置に関する問題とは分けて扱う必要がある。

誤った解釈を引き継ぐと、`new`と後続の`variant`がそれぞれ担う位置初期化の責務を誤って設計する可能性がある。
重大度は中で、確信度は高い。

### `Promote on entry`の既定値

`docs/protocols/cecp.md:159`はHaChuの宣言例を`feature option="Promote on entry -check 0"`としている。

しかし、HaChuは`entryProm=1`で初期化し、その値をfeature宣言へ出力する［[hachu.c 88行](https://github.com/ddugovic/hachu/blob/649ef114dd5fa39d3e1be4112e63ebe2ab3d5d8b/hachu.c#L88)］［[hachu.c 1253〜1266行](https://github.com/ddugovic/hachu/blob/649ef114dd5fa39d3e1be4112e63ebe2ab3d5d8b/hachu.c#L1253-L1266)］。
`docs/protocols/hachu.md:27`と同文書44行目も既定値を1としているため、`cecp.md`の0は誤記である。

重大度は中で、確信度は高い。

### 中将棋での`startpos`

`docs/protocols/usi-lishogi.md:36`は、USI原典の`startpos`が標準将棋の初期局面を指すため、中将棋では使用できないと説明している。

しかし、同文書151行目は、Lishogi-BotがAPIの`initialSfen`を`startpos`のまま送る可能性を踏まえ、エンジン側に`startpos`と`sfen`の両対応を推奨している。

`USI_Variant=chushogi`の受信後だけ`startpos`を中将棋初期局面として解釈するのか、原典どおり標準将棋として拒否するのかが未確定である。
重大度は中で、確信度は高い。

### 未対応の`go`への応答

`docs/protocols/cecp.md:186`は、探索部がない段階では`go`に`Error`を返す方針を示している。

しかし、同文書21行目の`Error (unknown command)`は未知コマンド用であり、`go`は既知コマンドである。
USIについても、`docs/plans/protocol-layer.md:40`はエラー応答を決定済みとする一方、同文書96行目は独自エラーではUSIの契約を満たさないため再設計が必要だとしている。

通常の`go`を接続可能な機能から除外するのか、探索を伴わない有効な`bestmove`応答を用意するのかを、実装前に一意に定める必要がある。
重大度は中で、確信度は高い。

## 軽微な所見

### `debug` featureの受理

`docs/protocols/hachu.md:128`は、`feature debug=1`の宣言によって無条件の`#`出力が正当化されると説明している。

しかし、[GNU CECP仕様](https://www.gnu.org/software/xboard/engine-intf.html)ではfeatureの利用にGUI側の受理が必要である。
HaChuは`accepted`と`rejected`を無条件に無視するため、GUIが`debug`を拒否した後も`#`行を出力する挙動は仕様に適合しない。

主要対象のXBoardとの通常接続には直接影響しないが、`debug`を拒否するGUIでは`#`行が無視される保証がない。
重大度は低く、確信度は高い。

### SFEN出力欄数の決定

`docs/protocols/usi-lishogi.md:181`は、`to_sfen`を2欄のままにするか4欄にするかを今後決める未決事項としている。

しかし、同文書187行目は2026年8月10日に2欄形式へ決定したと明記し、`src/sfen.rs:116`以降の現行`to_sfen`も2欄形式を実装している。

181行目は後続決定前の記述が残ったものであり、文書内の時制と状態を不整合にする。
重大度は低く、確信度は高い。

### XBoard式FENの持駒位置

`docs/protocols/cecp.md:115`は、XBoard式FENの持駒をキャスリング欄の位置に書くと説明している。

しかし、XBoardは角括弧付きの持駒を盤面部の直後、手番部の前に出力する［[backend.c 20858行以降](https://git.savannah.gnu.org/cgit/xboard.git/tree/backend.c?id=46b3c1d4ea45529cb2054516ff50feb902628d1c#n20858)］。
中将棋には持駒がないため現在の直接的な影響はないが、共通FEN解析器を設計すると欄位置を誤る。

重大度は低く、確信度は高い。

### `USI_Variant`の宣言要件

`docs/protocols/usi-lishogi.md:157`は、Lishogi-Botへ接続するために`USI_Variant`を宣言して受理する必要があるとしている。

しかし、対象のLishogi-Botはエンジンの`option`宣言を保存または検証せず、宣言の有無にかかわらず`USI_Variant`を送信する［[usi.py 82〜99行](https://github.com/TheYoBots/Lishogi-Bot/blob/17c16bc73b22fa6d56e0a412174c7c44993e619d/engine_ctrl/usi.py#L82-L99)］。
受信した値の受理は必要だが、このブリッジとの接続成立に宣言までは必要ない。

重大度は低く、確信度は高い。

### scalashogiの`_`区切り

`docs/protocols/usi-lishogi.md:139`と同文書180行目は、scalashogiが`_`区切りを受理せず、この構文をshogiops固有の寛容性としている。

scalashogiの直接解析は空白区切りだが、公開された`Sfen.clean`は`_`を空白へ変換する［[Sfen.scala 193行](https://github.com/WandererXII/scalashogi/blob/9a1c2c3ae9167da60f47366e922b2f84c8bcda4e/src/main/scala/format/forsyth/Sfen.scala#L193)］。
実運用の入口が常に`clean`を通るかは確認していないため、「直接解析は非対応だが、正規化関数は対応する」と限定するのが正確である。

重大度は低く、確信度は中である。

### 盤サイズ修飾子の名称

`docs/protocols/hachu.md:35`は`12x12+0_`を変種名の接尾辞と呼んでいるが、構文上は親変種名の前に置く接頭辞である。

例の構文自体は正しいため実装への影響は小さい。
重大度は低く、確信度は高い。

### XBoard参照の再現性

`docs/protocols/cecp.md:214`から216行目はGNU XBoardの「最新版」を参照し、確認したコミットIDを固定していない。

HaChu、shogiops、scalashogi、Lishogi-Botはコミットが固定されているため、XBoardに関する関数単位の確認だけ再現性が低い。
重大度は低く、確信度は高い。

### 文書内参照

`docs/protocols/usi-lishogi.md:5`の`plans/protocol-layer.md`と`plans/random-play.md`は、この文書からの相対パスとしては`../plans/protocol-layer.md`と`../plans/random-play.md`が正しい。
同じ行が参照する「第4章」も本文に存在しない。
`docs/protocols/hachu.md:109`の`protocol-layer.md`も解決可能な相対パスではない。

重大度は低く、確信度は高い。

## 一致を確認した事項

固定コミットのshogiopsとscalashogiは、中将棋の初期SFEN、21文字の駒表、座標変換、3升USI着手、および4欄の正準SFEN出力について一致する。

現行minaseの`to_sfen`が盤面部と手番部だけを出力すること、および`parse_sfen`が同じ2欄基本形式だけを受理することも、コードと文書187行目で一致する。

CECPとlishogi系SFENの手番文字は極性が異なるが、`docs/plans/protocol-layer.md:92`は内部正準を先手`b`とし、CECPアダプタで反転する方針を明記している。

## 検証範囲

本監査は文書、仕様書、およびソースコードの静的照合であり、ビルド、対局接続、lishogi APIの実測は行っていない。

監査対象文書と実装コードは変更せず、本報告書だけを追加した。
