# USI原典仕様とlishogi系拡張の調査

## 本文書の位置づけ

本文書は、プロトコル層マイルストーン（plans/protocol-layer.md）フェーズ1の調査成果物であり、USI原典仕様とlishogi系の中将棋表記を一次資料で確認した結果を記録する。ランダム対局検証ハーネス（plans/random-play.md）のフェーズ2は、`to_sfen`の表記規範として本文書を参照する。つまり、第4章に記す座標系、駒種文字およびSFEN盤面部の構文が、`to_sfen`と`parse_sfen`の正である。

調査対象のソースコードは、shogiops（lishogiのTypeScript規則ライブラリ）、scalashogi（lishogiサーバのScala規則ライブラリ）およびLishogi-Bot（USIエンジンをlishogiへ接続するブリッジ）である。設計書はshogiopsとscalashogiのリポジトリを`lishogi`組織配下として言及するが、実際の所在はどちらも`WandererXII`アカウント配下であった。各記述の末尾に典拠コードを付し、一次資料で確認できなかった事項は「未確認」と明記する。参照日と対象コミットは末尾の典拠一覧に示す。

## USI原典のコマンド体系

### 仕様の出自

USI（Universal Shogi Interface）は、Tord Romstadがチェスの UCI プロトコルを基礎として起草した将棋エンジン用プロトコルである。原典の掲載元は失われており、本調査ではH. G. Mullerのサイトに保存されている版を参照した［U1］。仕様自身が「The USI protocol, as well as the textual description on the protocol below, is based on the UCI protocol used in computer chess」と述べる［U1］。

### 通信の一般規則

通信は標準入出力を介したテキスト行で行い、各コマンドは改行で終端する。トークン間の空白は任意の個数を許す。未知のコマンドまたはトークンを受信した側は、それを無視して同じ行の残りの解釈を続けなければならない。エンジンは常に受動（forced mode）であり、`go`を受信するまで思考を開始してはならない［U1］。この「未知トークンの無視」規則は、minaseの「不正入力を握りつぶさない」方針と衝突し得るため、フェーズ2でエラー応答方針との整合を確定する必要がある。

### GUIからエンジンへのコマンド

| コマンド | 構文と意味 |
|---|---|
| `usi` | プロトコル開始の合図。エンジンは`id`と`option`宣言を送り、`usiok`で締める。 |
| `debug [on \| off]` | デバッグ出力の切替。既定はoff。 |
| `isready` | 同期用の応答要求。エンジンは初期化完了後に`readyok`を返す。 |
| `setoption name <id> [value <x>]` | エンジンパラメータの設定。エンジンが待機中のときだけ送られる。 |
| `register [later \| name <x> code <y>]` | エンジン登録の処理。 |
| `usinewgame` | 新規対局の開始通知。 |
| `position [sfen <sfenstring> \| startpos] moves <move1> ... <movei>` | 局面設定。`startpos`は標準将棋9×9の初期局面を意味する。 |
| `go` | 思考開始。`searchmoves`、`ponder`、`btime`、`wtime`、`binc`、`winc`、`byoyomi`、`movestogo`、`depth`、`nodes`、`mate`、`movetime`、`infinite`の副引数を持つ。 |
| `stop` | 思考の停止要求。 |
| `ponderhit` | 先読み対象の相手着手が実際に指されたことの通知。 |
| `gameover [win \| lose \| draw]` | 対局終了と結果の通知。 |
| `quit` | エンジンの終了要求。 |

`position`の`startpos`が標準将棋の初期局面（`lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1`）を指すと仕様に明記されているため、中将棋では`startpos`を使えず、`position sfen ...`形式が必要になる［U1］。

### エンジンからGUIへのコマンド

| コマンド | 構文と意味 |
|---|---|
| `id name <x>` / `id author <x>` | エンジン名と作者名の申告。 |
| `usiok` | `usi`への応答の終端。一定時間内に返さないとGUIに強制終了される。 |
| `readyok` | `isready`への応答。 |
| `bestmove <move1> [ponder <move2>]` | 思考結果の着手。`bestmove resign`と`bestmove win`も許される。 |
| `info ...` | 思考情報。`depth`、`time`、`nodes`、`pv`、`score`、`nps`などの副引数を持つ。 |
| `option ...` | 設定可能パラメータの宣言。構文は次節に示す。 |
| `checkmate [<moves> \| nomate \| timeout \| notimplemented]` | 詰将棋探索（`go mate`）の結果。 |
| `copyprotection` / `registration` | コピー保護と登録の状態通知。 |

## option宣言とsetoptionの仕様

### 宣言の構文

エンジンは`usi`への応答の中で、設定可能なパラメータを1行ずつ宣言する［U1］。

```text
option name <id> type <t> [default <x>] [min <x>] [max <x>] [var <value>]
```

型は`check`（真偽値）、`spin`（整数範囲）、`combo`（`var`で列挙した選択肢）、`button`（動作の起動）、`string`（文字列）、`filename`（ファイルパス）の6種である。仕様は`USI_Hash`（spin）、`USI_Ponder`（check）、`USI_OwnBook`（check）、`USI_MultiPV`（spin）、`USI_AnalyseMode`（check）などの予約オプション名を定めるが、`USI_Variant`という名前は原典のどこにも現れない［U1］。`USI_Variant`は後述するとおり、lishogi-botが送信する事実上の拡張である。

### setoptionの構文

GUIは`setoption name <id> [value <x>]`でパラメータを変更する。1回の送信で1パラメータを設定し、エンジンが待機中のときだけ送信される。オプション名と値は大文字小文字を区別せず、空白を含められない［U1］。この空白禁止は、minaseの規則セットオプションの値の構文（たとえば`R1,L1,P3`のようなコンマ区切り）が空白なしで表現できることを要求する。

## lishogi系の中将棋表記

本章の内容が`to_sfen`の表記規範である。lishogiの中将棋表記は、クライアント側規則ライブラリshogiops［S1］とサーバ側規則ライブラリscalashogi［S2］の2実装で一致していることを確認した。

### 座標系

盤上の升は「筋の数字＋段の英字」で表記する。筋は1から12までの10進数字（1桁または2桁）であり、先手から見て右端が1筋、左端が12筋である。段はaからlまでの英字であり、後手側最奥段（RULES.md第5条の一段目）がa、先手側最奥段（十二段目）がlである。たとえば`12a`は後手から見て自陣最奥段の、先手から見て左上隅の升を指す。筋名の並びはshogiopsの`FILE_NAMES`（`'1'`から`'16'`）と`RANK_NAMES`（`'a'`から`'p'`）で定義され、中将棋はその先頭12要素だけを使う［S1: src/constants.ts、src/util.ts `makeSquareName`］。scalashogiも`Pos.key`が同じ「筋数字＋段英字」を生成する［S2: src/main/scala/Pos.scala］。

shogiopsの内部升番号は`file + 16 * rank`（fileは筋番号−1、rankは段aを0とする通し番号）である［S1: src/util.ts `parseCoordinates`］。minaseの内部表現（`rank << 4 | file`）とは16升幅の点で同型だが、軸の向きが異なる。minaseの`parse_sfen`はSFEN先頭行を内部rank 11、行内左端を内部file 0に対応づけるため、変換式は「筋番号 = 12 − 内部file」「段英字 = 'a' + (11 − 内部rank)」である［M1: src/sfen.rs、src/core/square.rs］。

### 指し手の文字列表記

指し手は「移動元＋（中間升）＋移動先＋（成り記号）」を連結した文字列である。shogiopsの構文正規表現は次のとおりである［S1: src/util.ts `usiMoveRegex`］。

```text
^(\d\d?[a-p])(\d\d?[a-p])?(\d\d?[a-p])(\+|=|\?)?$
```

scalashogiは段の範囲をa〜lに限定した同型の正規表現を持つ［S2: src/main/scala/format/usi/Usi.scala `MoveRegex`］。構文の要点は次のとおりである。

1. 通常の着手は「移動元＋移動先」の2升連結であり、末尾`+`が成りを表す。成らない場合は無印である。出力時に`=`や`?`が付くことはなく、`makeUsi`は`+`だけを生成する［S1: src/util.ts `makeUsi`］［S2: Usi.scala `promotionString`］。
2. 解析時には末尾`=`と`?`も受理され、どちらも「成らない」と解釈される［S1: src/util.ts `parseUsi`］。`=`は人間向け表記（western記法など）で不成の明示に使う記号であり、`?`の用途は一次資料からは確認できなかった（未確認）。
3. 獅子、角鷹および飛鷲の2段階移動は「移動元＋中間升＋最終升」の3升連結で表す。たとえば`6f5e6d`は6fの獅子が5eを経て6dへ達する着手である。居喰いは最終升を移動元と同じ升にして`6i5h6i`のように書き、じっとは中間升を経て元の升へ戻る`6f6g6f`のように書く。1歩だけ動いて止まる着手と2升先への直行（跳び）は、どちらも2升連結（`6f5e`、`6f7d`）で書き、中間升を書かない［S1: test/rules/chushogi.test.ts］。
4. 距離0の2升連結（`6f6f`）は不合法である。じっとは必ず3升連結で表す［S1: test/rules/chushogi.test.ts］。
5. 中将棋には持ち駒がないため、駒打ち表記（`P*3d`形式）は使わない。shogiopsのchushogiは`dropDests`が常に空を返す［S1: src/position/rules/chushogi.ts］。
6. 2段階移動の成りは着手全体の末尾に`+`を置く構文が許されるが、中将棋で2段階移動を行う駒（獅子、角鷹、飛鷲）はいずれも成れないため、実際の合法手には現れない。

### 駒種の文字表記

SFENの盤面部と駒種表記は、次の21文字と成駒接頭辞`+`の組合せで29種の駒（成駒の動きの種類として数えた場合）を表す。大文字が先手、小文字が後手である。shogiopsとscalashogiの対応表は完全に一致する［S1: src/sfen.ts `chushogiRoleToForsyth`］［S2: src/main/scala/format/forsyth/SfenUtils.scala `toForsythChushogi`］。

| 文字 | 駒 | 成駒表記 | 成駒 |
|---|---|---|---|
| `p` | 歩兵 | `+p` | 金将と同じ動き |
| `i` | 仲人 | `+i` | 醉象と同じ動き |
| `l` | 香車 | `+l` | 白駒 |
| `a` | 反車 | `+a` | 鯨鯢 |
| `f` | 猛豹 | `+f` | 角行と同じ動き |
| `c` | 銅将 | `+c` | 横行と同じ動き |
| `s` | 銀将 | `+s` | 竪行と同じ動き |
| `g` | 金将 | `+g` | 飛車と同じ動き |
| `t` | 盲虎 | `+t` | 飛鹿 |
| `e` | 醉象 | `+e` | 太子 |
| `x` | 鳳凰 | `+x` | 奔王と同じ動き |
| `o` | 麒麟 | `+o` | 獅子と同じ動き |
| `m` | 横行 | `+m` | 奔猪 |
| `v` | 竪行 | `+v` | 飛牛 |
| `b` | 角行 | `+b` | 龍馬と同じ動き |
| `r` | 飛車 | `+r` | 龍王と同じ動き |
| `h` | 龍馬 | `+h` | 角鷹 |
| `d` | 龍王 | `+d` | 飛鷲 |
| `k` | 王将・玉将 | なし | 成らない |
| `n` | 獅子 | なし | 成らない |
| `q` | 奔王 | なし | 成らない |

shogiopsとscalashogiは、成駒を出自別の独立ロール（たとえば猛豹が成った角行相当は`bishoppromoted`）として保持し、合計39ロールを区別する。SFEN上では出自が`+f`と`b`のように異なる文字列で表れるため、盤面文字列だけで出自を復元できる。minaseは「駒種＋成りフラグ」の対で同じ区別を保持しており、表現力は一致する［S1: src/constants.ts `ROLES`、src/position/util.ts `chuushogiPromote`］［M1: src/sfen.rs テスト`parses_every_promoted_piece_and_rejects_unpromotable_pieces`］。

### SFENの全体構文

中将棋のSFENは、空白で区切られた4欄「盤面部 手番部 獅子捕獲升部 手数部」からなる。標準将棋で持ち駒欄が占める第3欄を、中将棋では獅子捕獲状態が置き換える点が最大の相違である［S1: src/sfen.ts `makeSfen`］［S2: src/main/scala/format/forsyth/Sfen.scala `situationToString`］。初期局面のSFENは両実装で同一である［S1: src/sfen.ts `initialSfen`］［S2: src/main/scala/variant/Chushogi.scala `initialSfen`］。

```text
lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL b - 1
```

この初期配置はRULES.md第5条と一致し、王将（`K`）が先手、玉将（`k`）が後手にある。各欄の構文は次のとおりである。

1. 盤面部は、段aから段lへ向かって12段を`/`で区切り、各段は12筋から1筋へ向かって走査する。空升の連続は10進数で圧縮し、中将棋では`12`までの2桁が現れる。数字の解析は桁を累積する方式であり、`1`の直後の`2`は12と解釈される［S1: src/sfen.ts `parseBoardSfen`、`makeBoardSfen`］［S2: Sfen.scala `boardToString`］。
2. 手番部は`b`が先手、`w`が後手である［S1: src/sfen.ts `parseColorLetter`、src/util.ts `toBW`］。
3. 獅子捕獲升部は、直前の着手で獅子以外の駒が相手の獅子（麒麟が成った獅子を含む）を取った場合に、その捕獲が起きた升の名前（たとえば`7f`）を書き、それ以外は`-`を書く［S1: src/sfen.ts `lastLionCapture`、src/types.ts `Setup.lastLionCapture`］。scalashogiは着手適用時に「動かした駒が獅子でなく、かつ到達升に相手獅子がいた」場合だけこの状態を記録する［S2: src/main/scala/Situation.scala `finalizeAfterUsi`、History.scala `lastLionCapture`］。この欄はRULES.mdのL1（足条件なし先獅子）に対応する一時状態であり、合法手生成では「非獅子駒はこの升にいる獅子だけを取り返せる」制限（麒麟成獅子の同一升例外、RULES.mdのL2に相当）として使われる［S1: src/position/rules/chushogi.ts `moveDests`のコメント「can't recapture lion on another square (allow capturing lion on the same square from kirin promotion)」］。
4. 手数部は次の着手が第何手目かを表す整数であり、初期局面は1である。shogiopsは書き出し時に、scalashogiは解析時に、それぞれ値を1以上9999以下へ丸める［S1: src/sfen.ts `makeSfen`］［S2: Sfen.scala `toSituationPlus`］。

解析側の寛容性は実装で異なる。shogiopsの`parseSfen`は区切りとして空白と`_`の両方を受理し、`startpos`という文字列も初期局面として受理し、手番以降の欄の省略を許す（手番省略時は先手番）［S1: src/sfen.ts `parseSfen`］。scalashogiは欄を位置で取り出すため省略に寛容だが、`_`区切りと`startpos`は受理しない［S2: Sfen.scala］。書き出しはどちらも常に4欄である。

### 成りの可否判定

指し手表記の`+`が合法になる条件として、shogiopsの中将棋の成り判定を記録する。成れるのは、(1) 敵陣外から敵陣（各色の奥4段）へ入る非捕獲または捕獲の着手、(2) 移動元か移動先が敵陣にある捕獲の着手、(3) 歩兵と香車が相手側最奥段へ達する着手、のいずれかである［S1: src/position/util.ts `pieceCanPromote`］。この(3)はRULES.mdのP3（香車最奥段救済）を含み、lishogi互換の対局にP3の採用が必要であることを裏づける。強制成りは存在しない［S1: src/position/util.ts `pieceForcePromote`］。

## lishogiへのエンジン接続経路

### lishogi-botブリッジ

lishogiはlichess由来のBot APIを備え、USIエンジンはブリッジプログラムを介して接続する。調査時点で参照できた実装はTheYoBots/Lishogi-Bot（Python製、lichess-botの移植）である［B1: README.md］。ブリッジはlishogiのAPIから対局イベントを受け取り、ローカルのUSIエンジンプロセスと標準入出力で通信する。対応プロトコルは`usi`のみである［B1: config.yml.default］。

局面は毎手`position <初期局面> moves <着手列>`で送られる。初期局面はAPIの`initialSfen`フィールドから取り、`startpos`でなければ`sfen `を前置する［B1: engine_ctrl/usi.py `position`、model.py］。中将棋の対局で`initialSfen`が常に完全なSFEN文字列になるかどうかはAPIの実測を行っておらず未確認だが、エンジン側は`startpos`と`sfen`両形式への対応が安全である。

### 接続経路からのオプション設定

ブリッジ経由のオプション設定には2つの経路がある。第一に、ブリッジの設定ファイル`config.yml`の`usi_options`節に書いた各項目が、起動時のハンドシェイク後に`setoption name <key> value <value>`として送信される［B1: config.yml.default、engine_ctrl/usi.py `setoption`］。したがって、minaseが規則セットをUSIオプションとして公開すれば、bot運用者は`config.yml`から規則を指定できる。lishogiサーバ自身がエンジンへ`setoption`を送る経路はない。

第二に、ブリッジは対局ごとに変種名を`setoption name USI_Variant value <variant>`で通知する。変種名はlishogiのAPI上のキーを小文字化したものであり、中将棋は`chushogi`である。エンジン名に`fairy-stockfish`を含む場合だけ`UCI_Variant`が使われ、標準将棋は例外的に`shogi`という値になる［B1: engine_ctrl/usi.py `set_variant_options`］。`USI_Variant`はUSI原典にない事実上の拡張であるため、minaseがlishogi-bot接続を想定するなら、この名前のオプションを宣言して受理する必要がある。

`go`の時間引数（`btime`、`wtime`、`byoyomi`など）はブリッジが対局条件から生成し、`config.yml`の`go_commands`節で`nodes`、`depth`、`movetime`の上書きもできる［B1: config.yml.default］。

## minase現行parse_sfenとの照合

### 一致している点

現行`parse_sfen`［M1: src/sfen.rs］は、次の点でshogiops仕様と一致していることを確認した。

- 盤面部の走査順序。先頭行を内部rank 11（段a）とし、行内左端を内部file 0（12筋）とする対応は、shogiopsの段・筋の向きと一致する。
- 駒種の21文字と大文字小文字の帰属（大文字が先手）。
- 成駒の`+`前置と、成れない駒（`k`、`n`、`q`）への`+`の拒否。
- 空升数の複数桁解析（桁の累積方式）。
- 手番部の`b`と`w`の解釈。
- 成駒を「駒種＋成りフラグ」で出自別に区別する表現力。shogiopsの39ロールと相互に単射で対応する。

### 乖離している点

次の乖離を確認した。いずれも実装の欠落または過剰な厳格さであり、文字の割当や走査順序の食い違いはない。

1. 現行`parse_sfen`は「盤面部 手番部」の2欄だけを受理し、3欄目以降を`UnexpectedFields`エラーにする。lishogiの正準SFENは4欄であり、初期局面の`... b - 1`すら受理できない。獅子捕獲升部（第3欄）と手数部（第4欄）の解析が欠けている。
2. 獅子捕獲状態を`Position`へ反映する経路がない。第3欄が`-`以外の局面はL1とL2の裁定に影響するため、切り捨てると合法手が変わる。拡張SFEN設計（プロトコル層フェーズ2）での対応が必要である。
3. shogiopsが受理する`startpos`キーワード、`_`区切り、欄の省略（手番省略時は先手番）に対応していない。これらはshogiops固有の寛容性であり、scalashogiも`startpos`と`_`区切りは受理しないため、採否はminaseの方針判断でよい。
4. 手数部がないため、対局再現時の手数照合ができない。書き出し側（`to_sfen`）を2欄のままにするか、`- 1`を補った4欄にするかは、random-playフェーズ2の設計で確定する必要がある。lishogiとの互換を厳密にするなら書き出しは常に4欄が正準である。

### to_sfenへの要求事項

以上から、`to_sfen`の表記規範を次のとおり確定する。盤面部は段a（内部rank 11）から段l（内部rank 0）へ、各段は12筋（内部file 0）から1筋（内部file 11）へ走査し、空升連続を10進数で圧縮し、先手を大文字で書く。手番部は先手`b`、後手`w`とする。駒種文字は本文書第4章の表に従い、成駒は出自の駒の文字に`+`を前置する。獅子捕獲升部と手数部を含む完全形式を出すかどうか、および`parse_sfen`の欄数拒否を緩めるかどうかは、random-playフェーズ2の着手時にプロトコル層設計と突き合わせて決定する。

## 典拠一覧

以下のウェブ資料およびソースコードは、2026年8月10日に参照した。

**［U1］Tord Romstad「The Universal Shogi Interface」（H. G. Mullerサイト掲載版）**
USIプロトコルの原典仕様。コマンド体系、option宣言、setoption、position構文、SFENの4欄構成と予約オプション名を定める。原典の掲載元は失われており、保存版を参照した。
<http://hgm.nubati.net/usi.html>

**［S1］WandererXII/shogiops（lishogiのTypeScript規則ライブラリ、version 0.21.1、コミットe295794f792e41c9b0a28aeb30faf9b89c951876）**
中将棋のSFEN構文と駒種文字（`src/sfen.ts`）、升名と指し手正規表現（`src/util.ts`、`src/constants.ts`）、成り判定と成駒対応（`src/position/util.ts`）、獅子捕獲制限（`src/position/rules/chushogi.ts`）、指し手表記の実例（`test/rules/chushogi.test.ts`、`test/util.test.ts`）の典拠とした。
<https://github.com/WandererXII/shogiops>

**［S2］WandererXII/scalashogi（lishogiサーバのScala規則ライブラリ、コミット9a1c2c3ae9167da60f47366e922b2f84c8bcda4e）**
USI指し手の構文（`src/main/scala/format/usi/Usi.scala`）、SFENの4欄構成と獅子捕獲升部（`src/main/scala/format/forsyth/Sfen.scala`、`SfenUtils.scala`）、獅子捕獲状態の更新規則（`src/main/scala/Situation.scala`、`History.scala`）、初期局面（`src/main/scala/variant/Chushogi.scala`）の典拠とした。shogiopsとの相互照合に用いた。
<https://github.com/WandererXII/scalashogi>

**［B1］TheYoBots/Lishogi-Bot（lishogi Bot APIとUSIエンジンのブリッジ、コミット17c16bc73b22fa6d56e0a412174c7c44993e619d）**
接続経路、`usi_options`による`setoption`送信、`USI_Variant`の送信規則（`engine_ctrl/usi.py`）、初期局面の受け渡し（`model.py`、`engine_wrapper.py`）、対応変種の設定（`config.yml.default`）の典拠とした。
<https://github.com/TheYoBots/Lishogi-Bot>

**［M1］minase `src/sfen.rs`および`src/core/square.rs`（コミット6ad0ef6abeacc124e093635039d6e2830fe1d395）**
現行`parse_sfen`の規約と内部座標表現の照合対象とした。
