# プロトコル層の設計書

## 先に読む要約

minaseを外部のGUIや対局サービスから呼び出すには、エンジンが標準的な通信規約を話す必要がある。
本マイルストーンは、将棋GUIで広く使われるUSI（Universal Shogi Interface）と、対局GUIのWinBoardおよびXBoardが使うCECP（Chess Engine Communication Protocol、XBoardプロトコルとも呼ぶ）の2つをエンジン側に実装した。
想定する接続先は、CECPではWinBoard、USIでは中将棋のオンライン対局サイトlishogiへエンジンを接続する仲介プログラムlishogi-botである。
方式は、Rust製のオープンソースチェスエンジンRusticの通信設計に倣い、エンジン本体を標準入出力から切り離してプロトコル非依存のコマンドと応答だけを扱わせ、文字列の解析と生成をプロトコルごとのモジュールに閉じ込める構造とした。
このためコードは、指し手と局面の文字列表記を変換する`src/notation/`と、通信を担う`src/protocol/`の2層に分け、局面はlishogiが使うSFEN（盤面と手番などを1行の文字列で表す局面表記）に、SFENが運べない2つの中将棋固有の状態、すなわち先獅子状態（獅子を取られた直後の1手だけ相手獅子を取り返せない状態。RULES.md第15条）と成り権保留（成れる着手で成らなかった駒の記録。RULES.md第18条）を加えた拡張形式で受け渡す。
規則セットは起動フラグ`--rules`とプロトコル内オプション`RuleSet`の両方で指定でき、対局中に受けた変更は次の対局から反映する。
完了の判定は、入力列と期待出力列を照合する台本テストと、lishogiの実対局10局を再生して各手の合法性と終局裁定の一致を確かめるリプレイ照合で行った。
思考開始指示（USIの`go`など）は本マイルストーンの対象外であり、その実装は探索部の完成後に後続の別マイルストーン[外部対局接続](engine-connectivity.md)が担った。
HaChu（H. G. Muller作の中将棋エンジン）との互換規則セットの検証は、検証に使った派生版のHaChu自体に欠陥があり、基本的な移動規則に反する手を出力したため不成立となった。そのため、検証済みの組合せだけに与えるHaChu用のプリセット名は導入していない。

## 状態

完了。
2026年8月10日に起案し、同日に着手して、2026年8月10日に完了した。
仕様調査、拡張SFENと指し手表記2形式、USIとCECPの両モジュール、および規則セットのオプション化を実装し、台本テストとlishogi棋譜10局のリプレイ照合（`tests/lishogi_replay.rs`）が全手の合法性と終局裁定で一致したため、完了条件をすべて満たしたと判定した。
棋力を比較する測定は本マイルストーンの対象ではないため測定記録はなく、根拠は「検証」節の確認手順と統合テストである。
HaChu互換規則セットの検証は不成立であり、フェーズ5の規定どおりプリセット名の導入は見送った（プリセットの導入自体は完了条件に含まれない。詳細と再検討条件は [protocols/hachu.md](../protocols/hachu.md) 第11章）。
本マイルストーンで保留した思考開始指示と実対局接続は、[外部対局接続](engine-connectivity.md)として独立した設計書へ分離した。

## 目的

本マイルストーンは、外部のGUIおよび対局サービスからminaseを呼び出せるようにするため、USIとCECPの2プロトコルをエンジン側で実装する。対象ユースケースは、WinBoardとのCECP接続とlishogiへのUSI系接続の2つであり、自作CUI対局管理マネージャは本マイルストーンに含めない。

実装に先立ち、既存の中将棋関連ソフトウェアのプロトコル仕様を調査し、結果を`docs/protocols/`配下に永続的な参照資料として残す。中将棋のプロトコル仕様は標準将棋やチェスと異なり一次資料が分散しているため、RULES.mdと同様に典拠と参照日を明記した調査文書を設計の正とする。

アーキテクチャはRusticチェスエンジンの通信設計を踏襲する。エンジン本体をプロトコルから完全に隔離することで、2プロトコルの実装を重複なく共存させ、将来のプロトコル追加も局所的な変更で済むようにする。

## 適用範囲

本マイルストーンでは、次の作業を行う。

- 既存ソフトウェアの仕様調査と`docs/protocols/`の整備。
- 先獅子状態や成り権状態を含む拡張SFEN形式の設計。
- Move文字列表記2形式（lishogi系拡張USI形式、CECP複数レグ形式）の設計と実装。
- USIおよびCECPのプロトコル本体の実装と台本テスト。規則セットをプロトコル内オプションとして公開する機能を含む。
- lishogi棋譜のリプレイ照合の導入。

探索部、評価関数、思考開始指示への着手応答、CUI対局管理マネージャ、実GUI接続による端到端検証は対象外とする。思考開始指示と実GUI接続の検証は、着手決定器の完成後に外部対局接続マイルストーンで行う。

## 依存関係

ランダム対局検証ハーネス（plans/random-play.md）は2026年8月10日に完了しており、次の依存関係はすべて解消済みである。フェーズ3の着手前提は満たされている。

- 本マイルストーンのフェーズ1調査のうちusi-lishogi調査の完了は、random-playフェーズ2（`to_sfen`）の前提である。`to_sfen`の表記規範を、未検証の「現行`parse_sfen`の規約」ではなく、一次資料で検証済みのshogiops仕様に置くためである。
- 本マイルストーンのフェーズ3以降（実装）は、`to_sfen`を利用するため、random-playの完了後に着手する。
- フェーズ1の調査はコードに触れないため、random-playの実装と並行して実施できる。

## 設計判断

| 項目 | 決定 |
|---|---|
| アーキテクチャ | Rusticからは「プロトコル非依存の状態機械と表記分離」の構造だけを踏襲する。エンジン本体はstdin/stdoutに触れず、プロトコル非依存のコマンドenumだけを扱う。スレッド構成は採らない（フェーズ2の確定設計で決定）。 |
| バイナリ構成 | 既存クレートに単一バイナリ`src/bin/minase.rs`を追加する。workspace分割やプロトコル別バイナリは行わない。 |
| プロトコル選択 | `--protocol usi\|cecp`の明示指定を必須とし、既定値と自動判別は設けない。 |
| 規則指定 | `--rules`起動フラグでの明示指定を必須とし、既定値を設けない。加えて、同一の規則セットをUSI `setoption`とCECP `feature option`のオプションとして公開し、GUIがエンジンを再起動せずに変更できるようにする。変更は次の対局開始時に反映するlatch方式とし、不正なコード列は受信時点でエラー応答する。この構成により、エンジンは起動時点から常に規則確定状態を保ち、「未確定のまま対局開始」という状態が存在しない。 |
| 拡張SFEN | shogiopsの中将棋SFENを基底形式として採用し、lishogiが表現できない状態だけを追加フィールドで拡張する。 |
| 思考開始指示 | USIの`go`相当は未対応とし、エラー応答を返す。着手決定器は探索部、プロトコルへの接続は外部対局接続で導入する。 |
| 実装順序 | USIを先行し、CECPを後続とする。拡張SFENの基底がlishogi互換であり、リプレイ照合との相乗効果を先に得るためである。 |
| 完了判定 | 台本テスト（セッショントランスクリプト照合）とlishogi棋譜リプレイ照合で行う。 |
| モジュール配置 | 表記変換層を`src/notation/`、通信層を`src/protocol/`に置く2層分離とする。`sfen.rs`は`src/notation/`へ移設し、指し手文字列表記2形式も同層に実装する。依存は`protocol`から`notation`への一方向とし、通信を経由しないperftやrandom_playは`notation`だけに依存する。移設はrandom-playフェーズ2との編集衝突を避けるため、フェーズ3の冒頭で行う。 |
| 調査成果物 | `docs/protocols/`配下に調査対象別のファイルと索引を置き、参照日付き典拠を明記する。 |

## アーキテクチャ

Rusticの設計から踏襲するのは、エンジン本体が外部との入出力を一切行わず、プロトコルモジュールが共通インターフェース（Rusticでは`IComm`に相当）を実装して起動時の指定に応じてインスタンス化される、という責務分離である。Rustic自体は各プロトコルモジュールに入力用と出力用の2スレッドを持たせるが、minaseはこのスレッド構成を採用しない（フェーズ2の確定設計の「コマンドenum、応答enumおよびtrait」の節参照）。

エンジンが知るのは「プロトコル非依存のコマンドenumを受信し、ハンドラで処理し、応答enumを送信する」ことだけである。stdinの読み取り、行のパース、プロトコル固有の文字列生成はすべてプロトコルモジュール側に閉じる。この隔離により、2つ目のプロトコル追加はモジュールの追加だけで済み、エンジン本体の変更を要しない。

起案時は、探索部導入時に「探索中の`stop`受信」へ改造なしで対応するため最初から2スレッド構成を採る方針だったが、フェーズ2の設計レビューを受けて撤回した。思考開始指示が未対応の現段階でスレッドは用途を持たず、CLAUDE.mdのYAGNI方針に反するためである。スレッド化は探索導入時に、境界enumの拡張と併せて再設計する。

minaseにおける各部の名称、コマンドenumの変種、traitの署名は、フェーズ2でプロトコル調査の結果と突き合わせて確定し、本設計書へ追記する。フェーズ2の結果は「フェーズ2の確定設計」の章にある。同章のスレッド構成の決定により、本章の入出力2スレッド構成は探索部導入時まで先送りへ変更した。

## フェーズ2の確定設計

本章は、フェーズ1の調査3文書（docs/protocols/）、設計レビュー、独立監査（docs/protocols-audit-2026-08-10.md）の指摘を踏まえた確定設計である。2026年8月10日に確定した。

### 内部正準と手番文字の変換責任

内部正準は先手を`Color::Black`とする現行実装のままとする。lishogi系のSFENとUSIは`b`が先手であり変換を要しない。CECPはWhiteが先手であるため、手番文字と結果コードの反転はCECPモジュール内に閉じる。CECPの`1-0`は先手勝ちを意味する。台本テストには、同一局面をCECPの`setboard`（手番`w`）とUSIの`position sfen`（手番`b`）で設定したとき同じ内部手番になる検証と、先手勝ちの局面でCECPが`1-0`を出力する検証を含める。

### 拡張SFEN

基底はshogiopsの4欄「盤面部 手番部 獅子捕獲升部 手数部」とする。第3欄の意味論は独立監査の訂正後の条件、すなわち「直前の着手で獅子以外の駒が、移動元を除く経由升または到達升で相手獅子（麒麟が成った獅子を含む）を取った場合のその升、なければ`-`」とする。

拡張は第5欄に成り権保留升の列を置く。構文は、保留升の升名（筋数字＋段英字）を内部密番号の昇順でコンマ区切りにした列とし、保留がなければ`-`とする（例: `... b - 1 7f,12c`）。`Position`の保留集合はBitboardであり複数升を保持できるため、列形式が必要である。解析は重複と非昇順を拒否する。保留升は、盤外・空升・成駒・成れない駒など`PositionBuilder::mark_promotion_deferred`の不変条件に反する入力を`InvalidPosition`として拒否する。成り権保留はP1・P2・P5だけが参照する状態であるため（RULES.md第13版でP2の待機とP5の歩兵状態が加わった）、適用対象の規則にこの3つのいずれも含まれない場合、`-`以外の第5欄は拒否する。これらを含まない対局で非空の保留集合を許すと、合法手に影響しないのにR1の同一局面キーだけが分断され、第24条第1項dと不整合になるためである。

局面設定の解析結果は次の型で運ぶ。

```text
SetupPosition
  position: Position            盤面・手番・成り権保留（第1・2・5欄）
  lion_capture: Option<Square>  第3欄。獅子が捕獲された升
  next_move_number: u32         第4欄。次の着手が第何手目か
```

第3欄から先獅子状態を復元する際、L2の判定に必要な「麒麟が成った獅子による捕獲かどうか」は、第3欄の升にいる直前着手側（非手番側）の駒が麒麟由来の成獅子であるかどうかから導出する。minaseもlishogi系も駒の出自（`+o`と`n`の区別）を盤面表記に保存し、麒麟は必ず捕獲升へ着地して成るため、この導出は一意である。第3欄の升が空である入力は受理し、麒麟由来ではない捕獲として復元する。飛鷲または角鷹が第1段階の経由升で獅子を取り第2段階で別の升へ進んだ場合、捕獲升は空になり、この状態はエンジン自身の`LionTrigger`（捕獲升を記録する）からも生成されるためである。第3欄の升に手番側の駒がある入力は、直前の着手の結果として成立しないため拒否する。確定設計の初版は「直前着手側の駒が存在しない入力は拒否する」としていたが、経由升捕獲の往復出力と矛盾するため、フェーズ3の着手時（2026年8月10日）に本契約へ訂正した。第4欄は`Position`も`Game`も保持しない値であるため`SetupPosition`が保持し、用途は表記の保持（往復出力と手数表示）に限る。裁定には影響しない。手数部が1以上9999以下の整数でない入力は拒否する。

関数は次の4つに分ける。2欄基本形の`to_sfen`と`parse_sfen`は現行のまま維持し、用途を検証ハーネスとperftに限定する。拡張形式は`to_extended_sfen`（`SetupPosition`相当の情報から常に5欄を出力）と`parse_extended_sfen`（5欄と、第5欄を`-`とみなす4欄を受理し、それ以外の欄数は拒否する）を新設する。lishogiへ渡す文字列が必要な場合は拡張5欄から第5欄を落とした4欄を使う。第3欄を`Position`へ反映する経路は、獅子捕獲状態の注入API `Position::set_lion_capture` である。

反復の履歴はSFENが運ばない。lishogi-botは毎手`position <初期局面> moves <着手列>`を送るため、履歴は着手列の再適用で再構成される。着手列なしで途中局面だけを設定した場合、R1からR3までの反復判定は空の履歴から始まる。この制限は仕様として明記し、コード側で補わない。

### Move文字列表記2形式

前提として、内部の正準じっとは`{from, mid: None, to: from, promote: false}`であり、中間升を保持しない（指し手正準化マイルストーンの決定）。居喰いは`mid: Some(捕獲升)`で中間升を保持する。両表記の設計はこの正準形を基準とする。

lishogi系拡張USI形式は`src/notation/usi.rs`に置く。構文は`<移動元>[<中間升>]<移動先>[+]`であり、升名は筋数字（1〜12）＋段英字（a〜l）、変換式は「筋番号 = 12 − 内部file」「段英字 = 'a' + (11 − 内部rank)」とする。解析は末尾の`=`と`?`を不成として受理し、出力は成りの`+`だけを付ける。じっとと居喰いは3升連結で表す。正準じっとは中間升を持たないため、じっとの生成は局面を引数に取り、第1段階として合法な空の隣接升のうち内部密番号が最小の升を中間升に補って3升連結を出力する。受信した3升連結は、中間升に相手駒がなければ終点にかかわらず`mid: None`の正準形へ正規化して返す。移動元と移動先が等しければ正準じっと、異なれば直接の二段移動になる。当初の契約はじっとだけを正規化していたが、lishogi系は経由升で捕獲のない二段移動にも実経路の3升連結を送るため、フェーズ4のリプレイ照合を受けて2026年8月10日に本契約へ訂正した。往復一致は文字列単位ではなく正規化後の`Move`単位（`parse(position, text(position, m)) == m`）で全合法手に要求する。じっと以外の指し手は文字列単位でも往復一致する。

CECP形式は`src/notation/cecp.rs`に置く。升名は筋英字＋段番号であり、変換式は「筋英字 = 'a' + 内部file」「段番号 = 内部rank + 1」とする。受信はコンマ区切りの単一文字列を解析し、第2レグの始点が第1レグの終点と一致しない入力を拒否する。送信はレグ分割の複数`move`行とし、非最終レグの末尾にコンマ、成りの`+`は最終レグだけに付け、不成は接尾辞なしとする（受信では`=`も不成として許容する）。

CECPのじっとは送信を`@@@@`とする。受信の`@@@@`は、現在局面の合法手のうち「移動元＝移動先、中間升なし、不成」の正準じっとだけに照合する。移動元＝移動先で中間升を持つ手は居喰いであり、駒を取って後続局面を変えるため候補に含めない。異なる駒のじっとが複数ある場合は、移動元の内部密番号が最小の手を代表として選ぶ。全候補は同一の後続局面と反復キーを生むが、R1の攻撃的着手判定は動かした駒の到達升を参照するため、代表の選択がR1の裁定へ影響し得る。`@@@@`が移動元を運ばない以上この曖昧さはwire形式に固有であり、代表規則を仕様として固定したうえで、R1採用時のCECP対局における既知の制限として記録する。表記の往復一致はじっと以外の全合法手に要求し、じっとは局面遷移の同値性と代表規則の決定性で検証する。受信側の解析は合法手リストという局面文脈を要するため、送信用と受信用を別APIとする。

### コマンドenum、応答enumおよびtrait

エンジン本体は、`Game`と規則latchを持つプロトコル非依存の状態機械`Engine`とし、`src/protocol/engine.rs`に置く。`Engine`は`AwaitingStart`（対局未開始）、`InGame`、`Finished`のライフサイクルを持つ。コマンドと応答は次の変種で確定する。

```text
EngineCommand
  SetRules(Vec<RuleCode>)   from_codesによる4群の意味検証に通ればpendingへ格納する
  NewGame                   pendingをcommitしてAwaitingStartへ遷移する
  SetPosition { setup: SetupPosition, moves: Vec<Move> }
                            局面設定と着手列の適用を1コマンドで原子的に行う
  ExtendPosition { moves: Vec<Move> }
                            現局への着手列の追加を1コマンドで原子的に行う
  ApplyMove(Move)           1手を適用する（CECPのusermove）
  EndGame                   GUI発の終局通知（USI gameover、CECP result）。AwaitingStartへ戻る
  Quit                      セッションを終了する

EngineReply
  Accepted { status: GameStatus, newly_finished: Option<GameResult> }
                            受理。newly_finishedは、この応答でOngoingからFinishedへ
                            遷移した場合だけ裁定結果を持つ
  Rejected(RejectReason)    拒否。エンジンの状態は一切変化しない

RejectReason
  InvalidRules(String)、InvalidPosition(String)、
  IllegalMove { mv: Move, cause: IllegalMoveCause }、GameAlreadyOver

IllegalMoveCause
  Movement（駒の動き・獅子規則等の違反）、Repetition（R2またはR3の反復禁止手）
```

`SetRules`は受信時に`Rules::from_codes`を呼び、同一コードの重複、同群の複数指定、獅子・成り・反復・駒枯れの各群の欠落を検証する。不正ならpendingを維持し、検証済みの`Rules`だけを`Game`へ渡す。commitは「pending規則で新しい`Game`を構築し、`SetPosition`の場合は局面と着手列を複製上で全適用し、成功した場合に限りactive規則・`Game`・ライフサイクルを同時に交換する」原子的操作とし、途中で失敗した場合は全状態を変更しない。これにより`position ... moves`の途中に不合法手があっても直前の有効状態が保持される。`ExtendPosition`は`InGame`でだけ受理し、現局の複製へ着手列を適用して、全手が成功した場合に限り`Game`とライフサイクルを更新し、基底局面までの絶対手数は変更しない。commit点は`NewGame`受信時（CECPの`new`、USIの`usinewgame`）と、`AwaitingStart`状態での`SetPosition`受信時である。参照したLishogi-Bot（コミット17c16bc7）は`usinewgame`を送信せず、対局ごとに`setoption`（`USI_Variant`を含む）→`position`→`go`の順で送ることをソースで確認した。したがってUSIでは`AwaitingStart`での`SetPosition`が実質のcommit経路であり、`usinewgame`に依存しない設計が必須である。`InGame`中の`SetPosition`は、lishogi-botが毎手`position`の全列を再送する挙動に合わせ、active規則で現局を原子的に再構成し、pending規則は次局まで反映しない。

プロトコルモジュールのtraitは次の同期署名とする。

```text
trait Protocol {
    fn run(&mut self, engine: &mut Engine,
           input: &mut dyn BufRead, output: &mut dyn Write) -> io::Result<()>;
}
```

台本テストは`input`と`output`をメモリ上のバッファへ差し替えて実現する。USIには裁定の外部出力がないため、USIの台本テストは出力照合に加えて、`run`終了後の`Engine`状態（`GameStatus`）の検査で裁定一致を確認する。

スレッド構成は、起案時の「最初から入出力2スレッド＋チャネル」の決定を撤回し、同期実装とする。探索部が存在しない現在、スレッドは用途を持たない未使用機構であり、YAGNI方針に反するためである。現在のenumとtraitは同期フェーズ専用の境界であり、探索導入時には探索開始・`stop`・思考情報・`bestmove`の変種追加とスレッド化を含めて再設計する。「探索導入後も境界を変更しない」ことは要件としない。

プロトコル固有の制御コマンドは`EngineCommand`へ変換せず、プロトコルモジュール内で処理する。対応は次のとおりとする。USIでは、`usi`に`id name minase <バージョン>`、`id author stepney141`、`option`宣言（`RuleSet`、`USI_Variant`の順）、`usiok`を返す。`isready`には`readyok`を返す（同期実装では即時）。`go mate`には`checkmate notimplemented`を返し、その他の`go`は後述のエラー情報行とする。`quit`は無応答で終了する。CECPでは、`xboard`は無視、`protover`にfeature宣言列を返す。`accepted`と`rejected`は記録し、必須feature（`setboard`、`usermove`、`ping`）が拒否された場合は`tellusererror`を出して終了する。`variant`は`chu`だけを受理し、他は`Error (unsupported variant): ...`とする。`ping N`は先行コマンドの処理完了後に`pong N`を返す（同期実装では受信順の処理により自動的に満たされる）。`force`は無視する（探索がなく自発着手しないため、モードの区別が存在しない）。`quit`は無応答で終了する。

USIの未知入力は原典準拠とする。未知のコマンド行と既知コマンド内の未知トークンは無視する。既知コマンドの意味的な不正（不正なSFEN、不合法手、不正なオプション値）は`info string error: ...`で通知し、当該コマンドを適用しない。fail-fastの厳格性は`EngineReply::Rejected`としてエンジン境界で保ち、wire上の寛容はプロトコルモジュールに閉じる。

### 規則オプション

オプション名は両プロトコル共通で`RuleSet`とする。値は4群を明示した規則コードのコンマ区切り（`setoption`の値は空白を含められないため空白なし、例: `L1,L2,P0,P3,R1,E1,E3`）、または検証済みプリセット名の単独指定である。値は大文字小文字を区別せずに受理し、重複、排他違反、群の欠落を拒否する。プリセット名は`engine-default`（L0+P0+R1+E0）と`lishogi`（L1+L2+P0+P3+R1+E1+E3）であり、規則コードまたは他のプリセット名との併記（例: `lishogi,P1`）は専用のエラーで拒否する。プリセット名の目的は誤指定の防止であり、検証済みの組合せをコード内の契約として保持する。`engine-default`は、R0を実装しない現状で規則書上の標準規則と実行可能な既定構成を区別するための名前であり、`Rules::ENGINE_DEFAULT`へ展開する。`standard`という名前は受理しない。プリセットは`Rules`定数からコード列へ戻して展開し、正準表記・オプション宣言のdefault値・エンジン状態には展開後のコード列だけが現れる。同じ値文法を`--rules`起動フラグにも適用し、解析はコア層の共通関数（`src/core/rules.rs`の`parse_rule_set`）が担う。正準表記は大文字で、カテゴリをL、P、R、Eの順、同カテゴリ内を番号昇順に整列した形とする。宣言は、USIが`option name RuleSet type string default <起動時の--rules値の正準表記>`、CECPが`feature option="RuleSet -string <同>"`とする。

latchはactive（対局に適用中）とpending（次局から適用）の2状態とし、起動時は`--rules`の値を両方へ入れる。`setoption`と`option`は受信時に`from_codes`で4群を意味検証し、正当ならpendingだけを更新し、不正ならpendingを変えずにエラーを通知する。エラー通知は、USIが`info string error: ...`、CECPが`Error (invalid option value): <受信行>`とする。commitの時点と原子性は前節のライフサイクルの定義に従う。

`USI_Variant`はstring型で宣言し、値`chushogi`だけを受理する。他の値は上記と同じエラー通知とする。`position startpos`は中将棋初期局面と解釈する。原典の`startpos`は標準将棋を指すが、本エンジンは中将棋専用であり標準将棋の局面を扱えないため、読み替えがlishogi-bot互換にとって安全側である。この読み替えは仕様として明記する。

### 思考開始指示と終局裁定の通知

思考開始指示（USIの`go`各種、CECPの`go`と`analyze`）は本マイルストーンの接続可能機能から除外する。USI原典は通常の`go`に必ず`bestmove`を要求するため、独自エラー行では契約を満たせず、`bestmove resign`は投了を意味するため代用できない。ただし`go mate`には原典が未実装応答`checkmate notimplemented`を定めるため、これを返す。その他の`go`は着手も`bestmove`も返さず、USIは`info string error: go is not supported (search is not implemented)`、CECPは`Error (command not supported): go`を出力するだけとする。この挙動では思考を求めるGUIと正常な対局にならないことを完了条件に明記し、実対局の接続は外部対局接続マイルストーンで扱う。CECPは`feature analyze=0`を宣言する。

終局裁定はエンジン内部で`GameStatus::Finished(GameResult)`に一元化する。CECPは、`EngineReply`の`newly_finished`が値を持つ場合に限り`RESULT {comment}`行へ変換して通知する。既にFinishedの対局に対する後続コマンドの応答からは`RESULT`を再生成しない。結果コードは先手勝ちが`1-0`、後手勝ちが`0-1`、引き分けが`1/2-1/2`であり、理由文字列は次の対応表で固定する。

| 変種 | 理由文字列 |
|---|---|
| WinReason::RoyalCapture | royal capture |
| WinReason::Mate | checkmate |
| WinReason::Stalemate | no legal moves |
| WinReason::Repetition | repetition |
| WinReason::PieceExhaustion | bare king |
| WinReason::Resignation | resignation |
| DrawReason::Repetition | repetition |
| DrawReason::PieceExhaustion | bare kings |
| DrawReason::Agreement | agreement |

USIにはエンジン発の裁定通知手段がないため出力せず、USIの台本テストは`run`終了後の`Engine`状態で裁定の一致を検証する。

不合法手の扱いは、CECPが`Illegal move (REASON): MOVE`応答とし、REASONは`IllegalMoveCause`から生成する（`Movement`は省略形の`Illegal move: MOVE`、`Repetition`は`Illegal move (repetition): MOVE`）。USIは`position`コマンドの原子的適用が不合法手で失敗した時点で`info string error: ...`を出力し、当該`position`全体を適用せず直前の有効状態を保持する。USIでは、開始局面のトークン列が前回受理時と一致し、新しい着手トークン列が前回の列を接頭辞として含む場合に限り、追加分を現局へ原子的に適用する。それ以外の場合は、開始局面から着手列全体を再生する。

### CECPのfeature宣言

起動時のfeature宣言は`myname`、`variants="chu"`、`setboard=1`、`usermove=1`、`ping=1`、`colors=0`、`sigint=0`、`sigterm=0`、`analyze=0`、`time=0`、`draw=0`、`option`（RuleSet）、`done=1`の最小セットとする。`time=0`と`draw=0`は、既定値1のまま届く`time`・`otim`・引き分け提案に対する処理を持たないための抑止である。`debug`は`#`出力を行わないため宣言しない。featureの拒否への対応は前述のとおり、必須3種（`setboard`、`usermove`、`ping`）の拒否で終了し、他は記録にとどめる。`highlight`は宣言せず、人間の対話入力支援として外部対局接続マイルストーンの対象外に置く（hachu.mdの調査どおり、highlightは人間の対話入力に必要であり、台本テストとエンジン間対局には不要である）。XBoard内蔵のVariantChu駒文字表とHaChu表は、基底21文字の完全一致と内蔵初期配置のFEN一致を確認済みであり、照合の記録はcecp.md第7章にある。
なお本節の`time=0`は起案時の値である。外部対局接続マイルストーン（engine-connectivity.md）が時間コマンド処理の実装に伴い`time=1`へ改定し、`memory=1`を追加したため、現行のfeature宣言はengine-connectivity.mdの記述を正とする（2026年8月14日注記）。

### モジュールと名称

表記変換層は`src/notation/`（`sfen.rs`移設、`usi.rs`、`cecp.rs`）、通信層は`src/protocol/`（`mod.rs`にtrait、`engine.rs`、`usi.rs`、`cecp.rs`）とする。単一バイナリ`src/bin/minase.rs`は`--protocol usi|cecp`と`--rules`の明示指定を必須とする。

## フェーズ5の確定設計

本章は、フェーズ2の確定設計がCECPについて残した実装細部を確定する。2026年8月10日に確定した。フェーズ2で確定済みの事項（座標変換式、送受信の非対称性、`@@@@`の代表規則、feature宣言、`RESULT`行、`Illegal move`形式、エラー応答形式）は再掲しない。

### CECP指し手表記の関数契約

`src/notation/cecp.rs`に受信用の`parse`と送信用の`legs`を置く。`parse(position, input)`はコンマ区切りの1レグまたは2レグを解析し、2レグでは第2レグの始点が第1レグの終点と一致しない入力を拒否する。成り接尾辞`+`は最終レグの末尾だけに認め、`=`は不成として受理する。中間升の扱いはUSI形式と同一契約とし、中間升に相手駒がなければ`mid: None`へ正規化し、移動元と移動先が等しい入力は正準じっとへ正規化する。`legs(position, mv)`は`move`行の本文列を返す。`mid`を持たない指し手は単一レグ、`mid`を持つ指し手は2レグ（非最終レグの末尾にコンマ）、正準じっとは`@@@@`の1行とする。じっと以外の全合法手について、`legs`の出力をそのまま連結した文字列（非最終レグが末尾コンマを持つため区切りの追加は不要）を`parse`へ渡すと元の`Move`へ一致することを検証する。`@@@@`の受信解析は合法手リストという局面文脈を要するため、notation層ではなくCECPプロトコルモジュールが行う。

### setboardの受理契約

`setboard`は「盤面部 手番部」の2欄を必須とし、3欄目以降は無視する。盤面部はSFEN盤面部と同一構文であり、同一のパーサを使う（HaChuの初期局面FENとlishogiの初期SFENの盤面部が一致することはフェーズ1調査で確認済みである）。手番部はCECPの慣例に従い`w`を先手（内部`Color::Black`）、`b`を後手とし、それ以外の手番文字は拒否する。XBoardが生成するFENは変則でもキャスリング欄などの余剰フィールドを含み得るため、余剰の無視はHaChuと同じ割り切りをwire寛容として採る。先獅子状態と成り権保留はCECPのsetboardでは設定できない。この制限は調査（cecp.md第7章、hachu.md第12節）が裏づける仕様上の制約であり、独自拡張はしない。解析に失敗した局面には、調査どおり`tellusererror Illegal position`を返し状態を変更しない。

### newの意味論

CECPでは`new`の後にsetboardを経ずに着手が直接届き得るため、`new`受信時に`NewGame`（pending規則のcommit）に続けて初期局面の`SetPosition`を適用し、`InGame`へ入る。後続の`setboard`は`InGame`での`SetPosition`として現局を置換する。適用規則は直前にcommit済みのactive規則であり、フェーズ3で確定したライフサイクルと整合する。エンジン本体（`Engine`とコマンドenum）の変更は要しない。

### コマンド対応の残余

明示的な対応（xboard、protover、accepted、rejected、variant、new、setboard、usermove、ping、force、result、option、go、quit）に含まれない入力は次の2群に分ける。時間制御・思考出力制御・通知系のコマンド（time、otim、level、st、sd、easy、hard、post、nopost、random、computer、name、hint、draw）は無視する。対応featureを0で宣言しても届き得る運用上のノイズであり、無視しても状態が変わらないためである。undo、remove、analyzeは、無視すると盤面状態またはモードの不整合を生む未対応機能であるため、`Error (command not supported): <コマンド>`を返す。前記のいずれにも該当しない行は`Error (unknown command): <第1トークン>`とする。

### 台本テストの範囲

CECPの台本テストは次を含む。握手とfeature交渉（必須feature`setboard`・`usermove`・`ping`の`rejected`で`tellusererror`を出して終了する経路を含む）、`variant chu`の受理と他変則の拒否、同一局面をCECPの`setboard`（手番`w`）とUSIの`position sfen`（手番`b`）で設定したときの内部手番一致、`new`直後の着手適用、多レグ指し手（2段階捕獲・居喰い）と`@@@@`じっとの適用、成り接尾辞の解釈、`Illegal move`の2形式（Movement・Repetition）、先手勝ち局面での`RESULT`行`1-0`の出力と`newly_finished`限定の再生成禁止、RuleSetオプションのlatchと不正値の`Error`応答、`go`のエラー応答、undoと未知コマンドの`Error`応答。

## 実装フェーズ

### フェーズ1　仕様調査

次の3対象を並列に調査し、`docs/protocols/`配下へ対象別のファイルとして執筆する。各ファイルには、RULES.mdと同じ様式で参照日付きの典拠を明記する。

| ファイル | 対象 | 主な調査項目 |
|---|---|---|
| `cecp.md` | CECPプロトコル仕様とWinBoardの変則将棋拡張 | engine-intf.htmlのコマンド体系、H. G. Mullerによる複数レグ指し手表記、12×12盤の指定方法、中将棋対局に必要な`feature`群、`feature option`によるエンジンオプション公開の機構、局面設定の形式 |
| `hachu.md` | HaChuのソースコード | CECPでの中将棋対局の実装例、指し手表記の実際の送受信、局面表記、仕様文書と実装の乖離 |
| `usi-lishogi.md` | USI原典仕様とlishogi系拡張 | USI原典のコマンド体系、`option`宣言と`setoption`の仕様、shogiops/scalashogiの中将棋指し手表記とSFEN形式、lishogiへのエンジン接続経路（lishogi-bot等）と接続経路からのオプション設定可否、現行`parse_sfen`の規約との乖離の有無 |

索引として`docs/protocols/README.md`を置き、各ファイルの範囲と本設計書への対応を示す。usi-lishogi.mdには、random-playフェーズ2が参照する規範であることを明記する。

### フェーズ2　設計確定

調査結果に基づき、次の項目を本設計書へ追記して確定する。確定した内容は「フェーズ2の確定設計」の章にある。

- 拡張SFENの具体形式（基底部分と追加フィールドの構文）。
- Move文字列表記2形式の構文と、内部`Move`型との相互変換の仕様。
- コマンドenum、応答enum、プロトコルモジュールtraitの設計。
- 規則オプションの名称、値の構文、および両プロトコルでの公開形式（USI `option`宣言とCECP `feature option`）。
- 思考開始指示に対するエラー応答の具体的な形式。

2026年8月10日の設計レビューと独立監査を受け、次の論点もフェーズ2の対象へ追加した。いずれも「フェーズ2の確定設計」の章で決定済みである。

- 手番文字の変換責任。CECPは`w`が先手、lishogi系SFENは`b`が先手であり、内部正準を先手`b`に固定したうえでCECP側アダプタが反転を担う構成と、同一局面が両プロトコルで同じ内部手番になる台本テストを定める。
- CECPのじっと表記`@@@@`の扱い。`@@@@`は始点と経由升を運ばないため、経由升を明示する内部`Move`との厳密な往復一致は成立しない。受信は現在局面の合法手集合に対する文脈依存解析とし、候補が複数ある場合の選択規則または拒否規則を定め、検証項目の「往復一致」はCECPのじっとに限り局面遷移の同値性検証へ差し替える。
- 拡張SFENの分担の精密化。先獅子（獅子捕獲升）は基底4欄の第3欄で表せるため、追加フィールドが必要なのは成り権保留などlishogi基底が運ばない状態に限る。獅子捕獲状態を`Position`へ反映する経路の設計を含む。
- 規則オプションのlatchの精密化。active/pendingの2状態、初回起動時の扱い、反映のcommit点（CECPは`new`、USIは`usinewgame`または`position`のいずれか）をプロトコル別に定め、lishogi-botの`USI_Variant`送信順との整合を実測または実装確認で確定する。不正値の応答は、CECPでは`Error`形、USIでは標準のエラー応答が存在しないことを踏まえた方針を定める（`rejected option`はGUIがエンジンのfeature宣言を拒否する応答であり、この用途には使えない）。
- USIの`position startpos`の扱い。原典では標準将棋の初期局面を指すが、`USI_Variant`で変種通知を受けた後の`startpos`を当該変種の初期局面として読み替えるかどうかを、lishogi-botの実送信内容と合わせて確定する。
- USIの`go`の扱い。USI原典は通常の`go`に必ず`bestmove`を要求し、未実装応答の明文は`go mate`の`checkmate notimplemented`だけであるため、独自エラー行では契約を満たさない。通常`go`を本マイルストーンの接続可能機能から明示的に除外するか、応答方式を再設計する。
- 終局裁定の通知経路の分岐。CECPはエンジン発の`RESULT {comment}`行があるが、USIの`gameover`はGUI発でありエンジンから裁定を通知する標準手段がない。内部裁定イベントは共通に保ち、通知はCECPのみ、USIは台本内の内部照合とする分岐を定める。
- CECP対話入力の範囲。WinBoardでの獅子2段階手の対話入力にはhighlight機構が事実上必須であり、これをフェーズ5に含めるか実GUI接続マイルストーンへ明示的に先送りするかを決める。XBoard内蔵VariantChu駒文字表とHaChu表の一致確認も実装前に行う。
- スレッド構成の導入時期。将来の`stop`対応だけを理由とする先行スレッド化はYAGNI方針と緊張関係にあるため、メッセージ境界（コマンドenumとtrait）だけを固定して同期実装から始める案と比較し、採否と根拠を確定する。

### フェーズ3　USI実装

既存の`sfen.rs`を`src/notation/`へ移設し、lishogi系拡張USI形式の指し手文字列表記と拡張SFENを同層へ実装する。続いて`src/protocol/`に`Protocol` trait、`Engine`、USIモジュールを同期構成で実装する。コア側の変更として、`GameError::IllegalMove`への原因分類（`Movement`と`Repetition`の区別）の追加と、獅子捕獲状態を`Position`へ注入するAPIを含む。stdinへの入力列とstdoutの期待出力列をフィクスチャ化した台本テストを併せて追加する。

### フェーズ4　リプレイ照合

厳選したlishogi棋譜を圧縮フィクスチャとして同梱し、L1+L2+P0+P3+R1+E1+E3の設定で各手の合法性と終局裁定の一致を確認するテストを導入する。対象は、終局理由の多様性（詰み、投了、王駒捕獲、反復、駒枯れ）を可能な範囲で網羅する10局程度とし、lishogiの棋譜IDで固定する。フィクスチャは棋譜ID、初期SFEN、USI着手列、期待結果（勝者と終局status）を持つ圧縮NDJSONとし、期待結果はlishogi APIの値を出典とする。比較項目は各手の合法性と、終局時の勝者および理由の対応である。具体的な棋譜IDは取得時に確定してフィクスチャとともにコミットする。取得スクリプトは同梱するが、実行はコーパス更新時に限る。

導入済みの照合は次の構成である。フィクスチャは`tests/fixtures/lishogi_replays.ndjson.gz`、取得スクリプトは`scripts/fetch_lishogi_replays.py`（棋譜ID固定、標準ライブラリのみ）、統合テストは`tests/lishogi_replay.rs`であり、規則セットは直書きせず`parse_rule_set("lishogi")`の解決を経由して、名前が指す組合せと検証済みの組合せの一致をテストが保証する。10局は上位8プレイヤーの695局から終局statusの多様性を基準に選び、王駒捕獲（royalsLost）3局、裸玉（bareKing）2局、反復（repetition）1局、合意引き分け（draw）2局、投了（resign）2局からなり、10局の合計は2,462手である。lishogiは王駒実捕獲制（E1、詰みでは終局せず王駒が実際に取られるまで対局を続ける規則）を採るため、終局statusがmateの中将棋棋譜は存在せず、詰みの多様性はroyalsLostが担う。shogiopsの成り判定は香車にも最奥段救済を与えるため、lishogi互換の規則セットにはP3（香車の最奥段救済）を含める。gzip圧縮NDJSONの読み取りのため、dev-dependenciesとして`flate2`と`serde_json`を追加した（テスト専用であり実行時依存は増えない）。

### フェーズ5　CECP実装

CECP複数レグ形式の指し手文字列表記を`src/notation/`へ追加し、CECPモジュールを実装して台本テストを追加する。2つ目のプロトコルの追加がエンジン本体の変更なしで完了することをもって、trait抽象の妥当性を検証する。

フェーズ5では、HaChu互換の規則セットの検証も行う。docs/protocols/hachu.mdの調査時点で、HaChuの既定の成り規則（敵陣入域時のみ）に対応する規則コードは存在せず、反復のR2との厳密一致と先獅子の実対局挙動も未確認である。CECP経由のHaChuとの実対局照合（lishogiにおけるリプレイ照合に相当する検証）で対応を確定し、成立した組合せに限りプリセット名を与える。必要な場合は新Pコードの規則書改版を先行させる。検証が成立しない場合、HaChuプリセットは導入しない。

HaChu互換検証は不成立であり、プリセットは導入していない。検証に用いたハーネスは`scripts/hachu_selfplay.py`（CECP経由の自己対局生成）と`scripts/hachu_replay.py`（minaseによるリプレイ照合）である。検証対象のHaChu 0.23（コミット649ef114）は`memory`未受信時の`go`で異常終了するほか、正常終局した全局で基本的な移動規則に反する手（自駒がある升への移動、走り駒の経路上の自駒の無視、手番でない側の駒の移動）を自発出力し、規則セットR2+E1とL1+R2+E1のどちらでも全手合法の局は0局であったため、先獅子・反復・成りの規則差の照合には到達していない。欠陥の詳細と再検討条件は [protocols/hachu.md](../protocols/hachu.md) 第11章にあり、生成棋譜は壊れた実装の産物であるため同梱しない。

## 検証

単体テストと台本テストで、次の性質を検査する。

- 各プロトコルについて、台本テストがセッション開始から局面設定、指し手適用、終局裁定の確認までを再現する。裁定の確認は、CECPでは`RESULT`行の照合、USIではエンジン応答（`EngineReply`の`GameStatus`）の照合による。
- 規則オプションの変更が対局中には反映されず、次の対局開始時に反映される。不正なコード列は受信時点でエラー応答となる。
- 思考開始指示に対して、設計で確定したエラー応答が返る。
- Move文字列表記2形式のそれぞれについて、内部`Move`型との相互変換が往復で一致する。ただしCECPのじっと（`@@@@`）は表記が中間升を運ばないため、往復一致に代えて局面遷移の同値性で検証する。
- lishogi棋譜のリプレイ照合が、全対局の各手の合法性と終局裁定で一致する。lishogiの裸玉裁定（bareKing）は第22条の駒枯れと条件が異なるため、照合の規則セットにはRULES.md第10版で新設したE3（Lishogi式裸玉即時裁定）を含める。

最終確認では、次のコマンドを実行する。

```console
cargo test
cargo clippy --all-targets
cargo fmt --all -- --check
git diff --check
cargo run --quiet --bin perft -- 4
```

## 完了条件

次の条件をすべて満たした時点で、本マイルストーンを完了とする。

- `docs/protocols/`に3対象のファイルと索引が典拠付きで存在する。
- 本マイルストーンの接続可能範囲は、握手、オプション設定、局面設定、棋譜再生、および終局裁定の確認（CECPは`RESULT`行、USIは内部状態）までである。思考開始指示には応答せず、思考を求めるGUIとの実対局は成立しない。この制限が文書化されている。
- 拡張SFEN、Move文字列表記2形式、コマンドenumとtraitの設計が本設計書に確定記載されている。
- USIとCECPの両モジュールが、台本テストとともに実装されている。
- 単一バイナリが`--protocol`と`--rules`の明示指定を必須とし、未指定時に起動を拒否する。
- 規則セットが両プロトコルのオプション機構で変更でき、次の対局開始時に反映される。
- 表記変換層が`src/notation/`、通信層が`src/protocol/`に配置され、`protocol`から`notation`への一方向依存になっている。
- lishogi棋譜のリプレイ照合が導入され、L1+L2+P0+P3+R1+E1+E3で全対局の各手の合法性と終局裁定が一致する。
- 本節の検証コマンドがすべて成功する。

## 参考資料

- Marcel Vanthoor, "Design," Creating the Rustic chess engine. アーキテクチャの参照元。2026年8月9日に参照した。
  <https://rustic-chess.org/communication/design.html>

フェーズ1で作成する`docs/protocols/`配下の各文書が、プロトコル仕様の典拠一覧を保持する。
