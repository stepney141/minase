# CECPプロトコルとWinBoard変則将棋拡張の調査

## 第1章　目的と範囲

本文書は、minaseのプロトコル層マイルストーン（docs/plans/protocol-layer.md）のフェーズ1調査として、CECP（Chess Engine Communication Protocol、XBoardプロトコルとも呼ばれる）の仕様と、WinBoard系GUIが持つ変則将棋向け拡張を一次資料で調査した結果を記録する。調査項目は、コマンド体系、複数レグ指し手表記、12×12盤の指定方法、中将棋対局に必要なfeature群、feature optionによるエンジンオプション公開の機構、および局面設定の形式である。各記述には章末または段落末に典拠を示し、一次資料で確認できなかった事項は「未確認」と明記する。参照日はすべて2026年8月10日である。

HaChu（H. G. Mullerによる中将棋エンジン）の実装詳細は、姉妹文書hachu.mdの対象である。本文書では、仕様書に明文がない事項の傍証としてXBoardおよびHaChuのソースコードを参照した箇所に限り、実装を典拠として引く。

## 第2章　資料の性格

CECPの正典は、Tim Mann原著のgnu.org掲載版仕様書である［C1］。同文書は「Version 2」（2009年9月3日付の注記あり）と称するが、H. G. Mullerによる追記（memory、smp、egt、option、setup、highlightなど）を含む形で保守されており、Muller自身は追記済みの版を便宜的に「version 2f」と呼んでいる。H. G. Mullerは同じ内容を再構成した仕様書CECP.htmlを自サイトに置いており、付録（指し手形式、標準変則一覧、feature一覧、PieceToCharTable）は正典より整理されている［C2］。

これらとは別に、WinBoardには「Alien Edition」と呼ばれる分岐版が存在し、複数レグ指し手や手番パスなどの拡張はもともとこの版で導入された［C3］。ただし、複数レグ表記・ヌルムーブ・setup・highlightは現在では正典［C1］に取り込まれており、XBoard本家のソースコードも12×12盤の変則chuを実装している［C5］。大将棋以上の変則（dai、tenjikuなど）だけがAlien Edition固有として残る［C2］［C4］。WinBoardの大型将棋対応を解説するWB-Chu.htmlは利用者向け文書であり、プロトコル構文の記述を含まない［C4］。

## 第3章　通信モデルとセッションの流れ

CECPは、GUIとエンジンが標準入出力を介して改行終端のテキスト行を交換する行指向プロトコルである。GUIからエンジンへのすべてのコマンドは改行で終端し、エンジンからの応答も行単位で送る［C1］。

典型的なセッションは次の順で進む。GUIは起動直後に「xboard」を送り、続けて「protover 2」でプロトコル版数を通知する。エンジンは「feature」コマンド列で自分の対応機能を宣言し、最後に「feature done=1」を送る。GUIは各featureに「accepted FEATURE」または「rejected FEATURE」で応答する。その後、GUIは「new」で対局を初期化し、標準チェス以外の場合は「variant VARNAME」を「new」の直後、最初の指し手または局面設定より前に送る。対局中は、相手の指し手が「usermove MOVE」（feature usermove=1のとき。既定では裸のMOVE行）で届き、エンジンは自分の指し手を「move MOVE」で返す。「force」はエンジンを両側の指し手を受理するだけの状態にし、「go」は現在の手番側をエンジンに持たせる。「ping N」には、それ以前に受けた処理の完了後に「pong N」で応答する。対局終了はGUIからは「result RESULT {REASON}」で通知され、終了後は「quit」でエンジンを終了させる［C1］［C2］。

エンジンが自発的に送る主な行には、指し手「move MOVE」、規則による終局の主張「RESULT {comment}」、投了「resign」、引き分け提案および受諾「offer draw」、不合法手の通知「Illegal move: MOVE」または「Illegal move (REASON): MOVE」、未知コマンドへの「Error (unknown command): COMMAND」がある。feature debug=1を宣言したエンジンは、「#」で始まる行をデバッグ出力としてGUIに無視させることができる［C1］。

**主な典拠：［C1］［C2］。**

## 第4章　feature交渉

### 交渉の機構

featureは「feature 名前=値」の形式で宣言し、値には真偽値（0または1）、整数、二重引用符で囲む文字列がある。1行に複数のfeatureを並べても、複数行に分けてもよい。GUIは起動時に2秒のタイムアウトでfeature列を待ち受けるため、エンジンは宣言の最後に「done=1」を送ってタイムアウトを打ち切る。初期化に時間がかかるエンジンは、先に「done=0」を送ってタイムアウトを1時間に延長できる。宣言していないfeatureは、旧版プロトコルと互換になるよう定められた既定値を保つ［C1］。

### 中将棋対局に関係する主なfeature

次の表は、正典［C1］とMuller版の一覧［C2］から、中将棋エンジンに関係するものを抜粋したものである。推奨値は［C2］の記述に従う。

| feature | 型と既定値 | 内容 |
| --- | --- | --- |
| done | 整数、既定なし | 宣言終了の合図。最後に必ずdone=1を送る。 |
| myname | 文字列 | エンジン名。ウィンドウ表示とPGNタグに使われる。 |
| variants | 文字列、既定は全変則 | 対応する変則名のコンマ区切りリスト。正しい値の設定が推奨される。中将棋は「chu」。 |
| setboard | 真偽、既定0 | 1でGUIは局面設定にsetboardを使う。0では旧式のeditを使う。常に1が推奨される。 |
| usermove | 真偽、既定0 | 1で相手の指し手に「usermove」前置詞が付く。 |
| ping | 真偽、既定0 | 1でGUIがping/pongによる同期を使う。常に1が推奨される。 |
| colors | 真偽、既定1 | 0で旧式のwhite/blackコマンドを抑止する。0が推奨される。 |
| sigint、sigterm | 真偽、既定1 | 0でPOSIXシグナルの送付を抑止する。［C2］は常に0を推奨する。 |
| time | 真偽、既定1 | 0でtime/otimによる時計更新を抑止する。 |
| draw | 真偽、既定1 | 0で相手の引き分け提案の通知を抑止する。 |
| reuse | 真偽、既定1 | 0で1対局ごとにエンジンプロセスを使い捨てる。 |
| analyze | 真偽、既定1 | 0で解析モードを拒否する。探索部を持たない段階のminaseは0にする必要がある。 |
| memory、smp | 真偽、既定0 | 1でそれぞれmemoryコマンド（ハッシュ等の総メモリ量）、coresコマンド（使用コア数）を受け取る。 |
| nps | 真偽、既定は未定義 | ノード数基準の時間管理。未対応なら「Error (unknown command): nps」を返すべきである。 |
| option | 文字列 | エンジン定義オプションの宣言。第8章で詳述する。 |
| highlight | 真偽、既定0 | 1でGUIがlift、put、hoverを送り、エンジンがhighlight応答で着手可能升を示す。複数レグ入力に関係する（第5章）。 |
| debug | 真偽、既定0 | 1で「#」始まりのデバッグ行の無視をGUIに保証させる。 |

**主な典拠：［C1］［C2］。**

GUIの合法手検査を切って対局する場合（第5章参照）、highlight=1はGUI上での獅子の2段階移動の入力に実質的に必要となる。エンジン対エンジンの自動対局だけならhighlightは不要である。この判断は［C1］のhighlight仕様と［C4］の解説から導いたものであり、「中将棋に必須のfeature一式」を列挙した明文は一次資料にない（未確認）。参考として、HaChuは「feature variants=...（対応変則列）」「feature ping=1 setboard=1 colors=0 usermove=1 memory=1 debug=1 sigint=0 sigterm=0」「feature myname=... highlight=1」に続けてoption群とdone=1を宣言する［C6］。

## 第5章　指し手の表記

### 座標表記の基本

GUIからエンジンへの指し手は、既定で座標表記（coordinate algebraic notation）で送られる。正典は次の形を例示する。通常の指し手はe2e4、チェス式のポーン成りは成り先駒種の小文字を後置したe7e8q、キャスリングはキングの2升移動（e1g1など）、持駒打ち（クレイジーハウス系）はP@h3、複数レグ指し手はコンマで区切ったc4d5,d5e4、ヌルムーブは@@@@である。段数がちょうど10の盤では、段の数え始めが0になる［C1］。feature san=1を宣言するとSAN（Standard Algebraic Notation、標準代数式表記）へ切り替えられるが、［C2］はSANの使用を推奨していない。

11段以上の盤における段番号の書式は、仕様書に明文がない（未確認）。実装上は、XBoardが11段以上の盤で段番号を2桁の10進数に変換して送信し（SendMoveToProgramのBOARD_HEIGHT > 10分岐）、HaChuも2桁の段番号を読み取る（ReadSquareがatoiで解析）。したがって中将棋の升は筋a〜l、段1〜12で表され、指し手はたとえばg5g6のようになる［C5］［C6］。段1は白（先手）側の最下段である。

筋aの左右の向きは、次の照合から導ける。HaChuが内蔵する中将棋初期局面FEN（第7章）は各段を筋aから筋lへ書き、lishogiの初期SFEN（usi-lishogi.md）は各段を12筋から1筋へ書くが、両者の盤面文字列は完全に一致する。したがって筋aはlishogiの12筋、すなわち先手から見て左端に対応し、minase内部file（0が12筋）との変換式は「筋英字 = 'a' + 内部file」である。これは2つの確認済み事実からの導出であり、XBoardの描画やGUI送信文字列での直接の検証は行っていない（未確認）。

### 将棋式の成りと不成

チェス式の駒種後置とは別に、将棋式の「固定の成り先への成り」には成り文字「+」を後置する。この規定は、PieceToCharTableで成駒を定義した駒について「SANまたはFENでは+Lのように参照され、成る指し手は成り文字'+'を使う」と定めるMuller版付録Eに明文がある［C2］。したがって中将棋の成りを伴う指し手は、たとえばc4c5+のようになる。

不成を明示する接尾辞は、仕様書に明文がない（未確認）。実装上は、XBoardが接尾辞「=」を不成として解釈し（MakeMoveがpromoChar '='を成り抑止として扱う）、HaChuも接尾辞なしまたは「=」を不成、それ以外を成りとして解析する。エンジンからの送信では、HaChuは成りのときだけ「+」を付け、不成には接尾辞を付けない［C5］［C6］。minaseは、受信では「=」と接尾辞なしの両方を不成として受理し、送信では接尾辞なしを不成とするのが安全である。

### 複数レグ指し手

獅子の2段階移動のような複数レグ指し手は、方向によって表記が非対称である。GUIからエンジンへは、コンマで区切ったレグ列を1行で送る（例：c4d5,d5e4）。エンジンからGUIへは、各レグを別々の「move」コマンドで送り、最終レグ以外の末尾にコンマを付けて手番交代を抑止する（例：「move f2h4,」の行に続けて「move h4f6」）。この双方向の規定は正典に明文がある［C1］。この拡張の由来はAlien Editionである［C3］。

各レグは前のレグの到達升から始まらなければならない。この連続性は仕様書に明文がないが（未確認）、XBoardは獅子型の指し手を内部表現から「出発升→経由升、経由升→到達升」の2レグへ展開して送信し、HaChuの解析器は第2レグの始点が第1レグの終点と一致しない入力をINVALIDとして拒否する［C5］［C6］。居喰い（第1レグで取って元の升へ戻る）は、到達升が出発升に一致する2レグ表記（例：f6g7,g7f6）で表現でき、HaChuが実際にこの形で送受信する［C6］。一方、じっと（空の隣接升へ出て元の升へ戻る非捕獲手）については、仕様書に表記の明文がなく（未確認）、HaChuは往復2レグ表記を生成も受理もせず、ヌルムーブ@@@@をじっとに転用して送受信する。この実装判断の詳細はhachu.mdに記す。なお、HaChuの解析器は大将棋以上の獅子犬などのために3レグ（例：直線上の1+1+1）も受理するが、中将棋では2レグまでで足りる［C6］。

### highlightによる入力支援と合法手検査

WinBoardは駒の性能表から着手可能升を強調表示できるが、獅子の捕獲制限のような細部の規則は知らないため、中将棋では合法手検査を切り、エンジンに強調表示を委ねる運用が推奨されている［C4］。feature highlight=1を宣言したエンジンは、利用者が駒をつかむと「lift SQUARE」を受け、着手可能升を色文字で示す「highlight COLORFEN」を返す。色にはR（赤、捕獲）、Y（黄）、M（マゼンタ、成り）、C（シアン、複数レグの中間升）などがあり、シアンの升へ動かすと指し手入力が完了せず、続きのレグの入力が求められる。GUIはhighlightの結果を合法手検査にも使い、無印の升への着手を不合法として扱う［C1］。

**主な典拠：［C1］［C2］［C5］［C6］。**

## 第6章　12×12盤と変則の指定

### 定義済み変則chu

正典のvariantコマンドの変則名表には「chu　Chu Shogi: Edo-period Japanese Chess on a 12x12 board」が含まれており、中将棋は定義済み変則である。GUIは「new」の直後に「variant chu」を送ることで12×12盤の中将棋対局を開始する［C1］。XBoard本家のソースコードもVariantChuを盤幅12、盤高12、キャスリング権なしとして実装しており、初期配置と駒文字表を内蔵する［C5］。Muller版の標準変則一覧は、chuを「他変則よりはるかに多くの駒種を持つ」変則として親変則（エンジン定義変則の規則継承元）に使える群に挙げる［C2］。大将棋（dai）以上の変則はAlien Edition限定である［C2］。

### 盤サイズの上書き構文

variants featureでは、変則名に盤サイズと持駒枠サイズの接頭辞を付けた「幅x高さ+持駒枠_親変則」形式で、標準と異なる盤面構成を宣言できる（例：8x8+0_capablanca、10x8+7_capablanca）。任意サイズに対応するエンジンは変則名として「boardsize」を列挙でき、最大サイズがあるなら「12x10+0_boardsize」のように接頭辞で示す［C1］。利用者がGUI側で盤サイズを上書きした場合も、同じ形式（例：7x7+6_shogi）でエンジンに通知される［C2］。中将棋は定義済み変則なので、この構文は必須ではないが、ローカルルール差を別変則として見せる場合などの応用がある。

### エンジン定義変則とsetupコマンド

XBoard 4.8以降、GUIが知らない変則名もエンジン定義変則として利用者に提示される。エンジンは対応するvariantコマンドへの応答として「setup」コマンドを送り、変則の内容を定義する。setupには「setup FEN」「setup (PIECETOCHAR) FEN」「setup (PIECETOCHAR) WxH+S_PARENTVARIANT FEN」の3形式があり、3番目の形式は盤の幅・高さ・持駒枠サイズと、残りの規則を継承する親変則を指定する［C1］。さらに「piece ID 駒動作記述」コマンドで、個々の駒の動きをBetza記法で定義できる［C1］。中将棋の規則細部（獅子の捕獲制限など）はBetza記法では表現できないため、GUIの合法手検査を切ってエンジン側で判定する前提となる（第5章参照）。

### 先後と白黒の対応

CECPでは白が先に指し、resultの結果コード（1-0、0-1、1/2-1/2）は白から見た値である［C1］。したがって中将棋の先手はCECP上のWhiteに対応する。lishogi系SFENが先手を「b」（Black）と書くのと逆の対応であり、表記変換層での取り違えに注意する。この対応関係自体は仕様の一般規定からの帰結であり、中将棋に即した明文はない（未確認）。実装上は、HaChuの初期局面文字列で大文字（白）側が段1側に置かれ、白が先に指すことと整合する［C6］。

**主な典拠：［C1］［C2］［C5］。**

## 第7章　局面設定の形式

### setboardとedit

局面設定の推奨手段は「setboard FEN」であり、feature setboard=1を宣言したエンジンにだけ送られる。宣言しない場合、GUIは旧式の「edit」コマンド（駒を1枚ずつ置くサブコマンド列）を使う。［C2］は、setboardの拒否を想定しないエンジンが実際には多いことから、GUI側にとってsetboard対応は事実上必須と述べ、editを廃止扱いにしている。変則エンジンは「その変則で一般に使われているFEN形式」を理解すべきものとされ、扱えない局面には「tellusererror Illegal position」を返し、以後の指し手を「Illegal move」で拒否する対処が推奨される［C1］［C2］。

### 変則向けFENの拡張

FEN（Forsyth-Edwards Notation、局面の1行表記）は変則に応じて拡張される。正典は、8×8以外の盤や標準外の駒がある変則では「その変則にとって標準的または適切なFEN形式」を使うと定める［C1］。将棋式の成駒は、非成駒の文字に接頭辞「+」を付けて表す（例：+L）。この規定はMuller版付録Eに明文がある［C2］。空升の連続数が10以上になり得る盤では2桁の数字を使う。この点は仕様書に明文がなく（未確認）、XBoardのPositionToFENが2桁出力を実装していることを確認した［C5］。持駒は角括弧で書かれ（例：[PPPRQ]）、XBoardの実装では盤面部の直後、手番部の前に置かれる（PositionToFEN）。中将棋には持駒がないため使わない［C2］［C5］。

中将棋の初期局面の実例として、HaChuは次のFENを内蔵する［C6］。

```text
lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/
3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL
```

（実際は1行。改行は紙面の都合である。）段12（後手最下段）から段1へ、各段は筋aから筋lへ並び、小文字が後手（Black）、大文字が先手（White）の駒である。

### 駒文字の割当

中将棋FENの駒文字と駒種の対応には、仕様レベルの標準割当がない（未確認）。対応はGUIとエンジンの間でPieceToCharTable（setupコマンドの括弧内、またはGUIの内蔵表）によって決まる。PieceToCharTableは、GUIが持つ駒画像の既定順序の各位置に駒文字を割り当てる文字列であり、不使用の画像位置にはピリオドを書き、「+」はその画像を対応する非成駒の成駒として扱わせ、「^ID」は任意の画像をID駒の成形として定義し、「ID=別文字」はエイリアスを定義する［C2］。XBoard本家はVariantChu用の駒文字表を内蔵する。

XBoard内蔵表とHaChu表の照合は、フェーズ5の着手前条件として2026年8月10日に完了した。XBoardのmasterブランチHEAD（コミット46b3c1d4、2026年8月4日付）のbackend.cにあるVariantChu用SetCharTableEsc文字列（6863〜6876行）を、common.hのChessSquare enum順（289〜434行）とSetCharTableEscの構文（`.`は未割当、`^Y`は基底駒Yの成駒、`/`は書き込み位置の区切り）に従って展開し、次の結果を得た。第1に、基底21文字の駒種対応はhachu.md第11節のchuIDs表と完全に一致し、J、U、W、Y、Zが未割当である点、成駒を`+`接頭辞で表す点、成らない駒がK、Q、Nの3種である点も一致する。第2に、初期配置ChuArray（backend.c 662〜676行と仲人の補正配置）を同表で展開したFENは、第7章冒頭に掲げたHaChu内蔵FENと、王と醉象・獅子と奔王・麒麟と鳳凰の左右非対称も含めて完全に一致する。したがってXBoardとHaChuの間で駒文字の取り違えは生じず、minaseのCECP setboardがSFEN盤面部と同一のパーサを使う設計は両者と互換である。リリース版tarballでの同一性は確認していない（未確認）。

**主な典拠：［C1］［C2］［C5］［C6］。**

## 第8章　feature optionによるオプション公開

### 宣言の構文

エンジンは「feature option="..."」でエンジン定義オプションを宣言し、GUIのメニューに設定項目を追加させる。option featureは他のfeatureと異なり累積され、done=1の後でも追加できる。型は10種類あり、正典は次の構文を定める［C1］。

```text
feature option="NAME -button"
feature option="NAME -save"
feature option="NAME -reset"
feature option="NAME -check VALUE"
feature option="NAME -string VALUE"
feature option="NAME -spin VALUE MIN MAX"
feature option="NAME -combo CHOICE1 /// CHOICE2 ..."
feature option="NAME -slider VALUE MIN MAX"
feature option="NAME -file VALUE"
feature option="NAME -path VALUE"
```

NAMEは空白を含み得る英数字列である。VALUEは現在値（-checkは1が有効、0が無効）、MINとMAXは-spinおよび-sliderの範囲、-comboの選択肢はスラッシュ3つで区切り、現在値には前置アスタリスクを付ける。-fileと-pathは-stringと同様の文字列だが、GUIが参照ボタンを付けられる。-saveは送信前に全オプション設定のフラッシュを保証するボタンであり、-resetはGUI側のオプション一覧を消去してからエンジンに通知するボタンで、エンジンはその後feature option列を送り直してオプション群を再定義できる［C1］。

### 設定変更の受信

GUIは「option NAME=VALUE」でエンジンに設定変更を伝える。VALUEの型はオプション型に従い、-spinと-checkでは10進整数、-comboと-stringでは文字列、-buttonと-saveでは値なしの「option NAME」となる。GUIがこのコマンドを送るのは、利用者がメニューで値を変更したときと、エンジン起動時（最初のcoresコマンド、それがなければ最初のnewコマンドより前）にコマンドラインオプションの設定を反映するときである［C1］。なお「rejected option 値文字列」は、エンジンの「feature option="..."」宣言の値の構文をGUIが拒否するときの応答であり、GUIから届いたoptionコマンドの値をエンジンが拒否する規定ではない［C1］。エンジン側が不正な値を拒否する標準の書式は定められておらず、「Error (ERRORTYPE): COMMAND」の一般形を使う具体形はフェーズ2で定める。

### 規則セット公開への適用

minaseは規則セット（L、P、R、Eの各コード列）をこの機構で公開する予定である。起動直後の設定は「最初のnewより前」に届くという上記の規定により、対局開始前の規則確定と整合する。対局中の変更要求は任意の時点で届き得るため、設計書どおり次の対局開始時に反映するlatch方式で受ける。エンジンオプションとして規則を公開する先例には、HaChuの「feature option="Okazaki rule -check 0"」（先獅子の岡崎方式の切替）や「Promote on entry -check 1」（成り規則の切替、既定は有効）がある［C6］［C7］。

**主な典拠：［C1］。**

## 第9章　終局裁定と不合法手の通知

エンジンは、盤上の出来事によって対局が確定的に終了したと判定したとき、「RESULT {comment}」形式の行（例：「1-0 {White mates}」「1/2-1/2 {Draw by repetition}」）を送らなければならない。この行は継続拒否とみなされるため、まだ確定していない局面で送ると敗北扱いになり得る。指し手の後に成立する引き分けの主張には、着手前に「offer draw」を使う。投了は「resign」または理由文字列に「resign」を含むRESULT行で行う［C1］。

受信した指し手が不合法な場合、エンジンは「Illegal move: MOVE」または理由付きの「Illegal move (REASON): MOVE」を返し、GUIがその指し手を取り消す［C1］。中将棋への適用では、R2またはR3が禁止する反復着手（RULES.md第27条第4項の不合法な着手）はIllegal move応答に対応し、R1の4回反復の裁定や駒枯れなどの終局はRESULT行に対応する。この対応付けはminase側の設計判断であり、変則の裁定をどう通知するかについて中将棋固有の明文は一次資料にない（未確認）。

**主な典拠：［C1］。**

## 第10章　未確認事項の一覧

次の各事項は一次資料の仕様書に明文がなく、実装の確認または本文書の解釈で補った。

1. 不成の接尾辞「=」は仕様書に明文がない。XBoardとHaChuの実装が「=」および接尾辞なしを不成として扱うことを確認した（第5章）。
2. 11段以上の盤の段番号の書式は仕様書に明文がない。仕様書は10段の盤で段が0始まりになることだけを定める。XBoardとHaChuが2桁10進・1始まりで送受信することを確認した（第5章）。
3. 複数レグ指し手の各レグが前レグの到達升から始まるという連続性は仕様書に明文がなく、実装の挙動から確認した（第5章）。
4. 中将棋対局に必須のfeature一式を列挙した明文はなく、本文書の表は仕様と実装からの整理である（第4章）。
5. 中将棋FENの駒文字の標準割当はなく、PieceToCharTableによる合意事項である。ただしXBoard内蔵表（master HEAD、コミット46b3c1d4）とHaChuのchuIDs表は21文字すべてで一致することを2026年8月10日に確認した（第7章）。
6. 先手とWhiteの対応は仕様の一般規定からの帰結であり、中将棋に即した明文はない（第6章）。
7. 変則固有の終局裁定（反復、駒枯れ）をRESULT行とIllegal move応答のどちらで通知すべきかの明文はない（第9章）。
8. じっとの表記は仕様書に明文がない。HaChuは往復2レグでなくヌルムーブ@@@@を転用しており、この確認はhachu.mdの第8節による（第5章）。

## 第11章　minase実装への含意

本調査から、フェーズ2の設計確定に対して次の含意が得られる。第一に、CECPモジュールの台本テストは、xboard、protover 2、feature交渉（accepted/rejected）、option、new、variant chu、setboard、usermove、moveの系列を再現すればよく、必要なコマンド群は本文書の範囲で閉じる。第二に、指し手表記はレグ連続性のあるコンマ区切り座標表記（2桁段、成り「+」、不成は受信「=」許容・送信接尾辞なし）とし、送信側では複数レグをmoveコマンド分割と末尾コンマで表す。第三に、規則セットのオプションは-comboまたは-stringで宣言し、起動時反映と対局中latchの区別は「optionは最初のnewより前に届く」という仕様の規定に載せる。第四に、探索部がない段階ではanalyze=0を宣言する。`go`は既知コマンドであるため「Error (unknown command)」の形は使えず、思考開始系コマンドへの具体的な応答形式（別のエラー種別の選定、または当該機能の接続対象からの除外）はフェーズ2で確定する。

## 第12章　参考文献

以下の資料は、2026年8月10日に参照した。

### 仕様書

**［C1］Tim Mann and H. G. Muller, "Chess Engine Communication Protocol"**
CECPの正典仕様書。コマンド体系、feature交渉、option構文、座標表記と複数レグ指し手、変則名表（chuを含む）、盤サイズ上書き構文、setboardとedit、setupとpieceコマンド、highlight機構を定める。H. G. Mullerによる追記を含む版である。
[Chess Engine Communication Protocol](https://www.gnu.org/software/xboard/engine-intf.html)

**［C2］H. G. Muller, "Chess-Engine Communication Protocol v2," WinBoard**
正典を再構成した仕様書。付録に指し手形式、標準変則と親変則の一覧、feature一覧と推奨値、PieceToCharTableの定義（成駒の+接頭辞と成り文字+の明文を含む）を持つ。
[Chess-Engine Communication Protocol v2](http://hgm.nubati.net/CECP.html)

**［C3］H. G. Muller, "WinBoard Alien Edition"**
複数レグ指し手（コンマ区切りとmoveコマンド分割）、ヌルムーブ@@@@、setup、highlight、infoboardなどの拡張の由来を記す。これらの多くは現在の正典に取り込まれている。
[WinBoard Alien Edition](http://hgm.nubati.net/alien.html)

### 解説資料

**［C4］H. G. Muller, "WinBoard and Large Shogi"**
WinBoardの大型将棋対応の利用者向け解説。定義済み変則（Chu 12×12ほか）、合法手検査を切ってエンジンに強調表示を委ねる推奨運用、Ctrlキーまたはシアン升による複数レグ入力を説明する。プロトコル構文の記述はない。
[WinBoard and Large Shogi](http://hgm.nubati.net/WB-Chu.html)

### ソースコード

**［C5］GNU XBoardソースコード（backend.c、common.h）**
GNU Savannahのgitリポジトリ最新版を参照した。SendMoveToProgramによる複数レグ展開と2桁段番号の送信、MakeMoveによる成り文字+と不成=の解釈、PositionToFENによる+接頭辞と2桁空升数の出力、VariantChuの盤サイズと駒文字表の定義を確認した。参照コミットを固定していないため、関数単位の確認の再現性は他のソースコード典拠より限定的である。
[XBoard git repository](https://git.savannah.gnu.org/cgit/xboard.git)

**［C6］H. G. Muller, "HaChu"ソースコード（hachu.c、move.c、variant.c）**
ddugovicによるGitHubミラーを参照した。feature宣言列とエンジン定義オプション、MoveToTextとParseMoveによる複数レグ・成り・不成の送受信、中将棋初期局面FENを確認した。詳細な検証はhachu.mdで行う。
[HaChu GitHub repository](https://github.com/ddugovic/hachu)

**［C7］H. G. Muller, "HaChu, an AI for playing Chu Shogi," WinBoard**
HaChuの紹介ページ。エンジンオプション（Allow repeats、Promote on entryなど）の存在を記すが、プロトコル構文の記述はない。
[HaChu, an AI for playing Chu Shogi](http://hgm.nubati.net/HaChu.html)
