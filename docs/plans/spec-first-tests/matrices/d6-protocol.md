# D6 プロトコル層 挙動マトリクス（USI・CECP・エンジン状態機械・CLI規則引数）

作成日: 2026-08-14。spec-first-tests.md フェーズ2の成果物であり、実装コード（src/）を一切読まずに規範文書だけから作成した。

## 典拠文書の略記

| 略記 | 文書 |
|---|---|
| PL | docs/plans/protocol-layer.md（フェーズ2・フェーズ5の確定設計、実施状況の確定事項を含む） |
| EC | docs/plans/engine-connectivity.md（time=1/memory=1改定、時間正規化、CECP対局進行。feature宣言はPLよりECを正とする） |
| BG | docs/plans/browser-gui.md（moves/state仕様） |
| CE | docs/protocols/cecp.md（CECP調査文書） |
| HA | docs/protocols/hachu.md（HaChu調査文書） |
| UL | docs/protocols/usi-lishogi.md（USI原典・lishogi系調査文書、minase固有拡張の節を含む） |
| R33 | RULES.md 第33条（規則セット名の受理） |
| LS | docs/plans/lazy-smp.md（「プロトコル設定」節） |

規範性の順位はspec-first-tests.mdの原則に従う。設計書（PL・EC・BG・LS）の確定事項は規範文書であり、調査文書（CE・HA・UL）は外部仕様の典拠である。調査文書が「未確認」と明記する事項を期待値の根拠にしてはならない。

## 観測方針（全項共通の指示）

- 期待観測は原則としてプロトコル出力（stdout行）で検証する。台本テストは`Protocol::run`の`input`/`output`をメモリバッファへ差し替えて行う（PL「コマンドenum、応答enumおよびtrait」）。
- 例外はUSIの終局裁定である。USIにはエンジン発の裁定通知手段がないため、PLが明記するとおり「`run`終了後の`Engine`状態（`GameStatus`）の検査」だけを内部観測として許す。それ以外の内部状態（pending規則、置換表、探索ID等）の直接検査は書かず、後続コマンドへの応答差で間接観測する。
- 「台本完全一致」と記した項は、典拠に出力行の完全な文字列が明文であることを意味する。それ以外の`...`部分は完全一致を契約にしない（SPEC_UNCLEAR参照）。
- 探索を含む台本は`depth`または`nodes`（CECPは`sd`）固定で決定的に書く（EC設計判断）。時間制御の実挙動は台本にしない。

---

## 1. USI（D6-USI）

### D6-USI-01 `usi`握手の応答列
- 典拠: PL「プロトコル固有の制御コマンド」（`id name minase <バージョン>`、`id author stepney141`、`option`宣言はRuleSet、USI_Variantの順、`usiok`）。
- 前提: 起動直後（`--protocol usi --rules <有効値>`）。
- 操作と期待観測: `usi`送信 → `id name minase <バージョン>`行、`id author stepney141`行、`option name RuleSet ...`行、`option name USI_Variant ...`行の順、最後に`usiok`。id 2行とusiokは台本完全一致相当（バージョン部を除く）。
- 境界・不正: `USI_Hash`宣言（EC実施状況で追加）の宣言位置はPLの順序規定（RuleSet→USI_Variant）に含まれない → SPEC_UNCLEAR-05。テストはRuleSetがUSI_Variantに先行することと`usiok`が最後であることだけを固定する。
- 性質: 応答列は起動時`--rules`値だけに依存し決定的である。

### D6-USI-02 `isready`→`readyok`
- 典拠: PL「プロトコル固有の制御コマンド」（`isready`には`readyok`。同期実装では即時）。
- 前提: 任意の非探索中状態。
- 操作と期待観測: `isready` → `readyok`1行（台本完全一致）。
- 境界・不正: 探索中の`isready`はD6-USI-22（join後、`bestmove`の後に`readyok`）。
- 性質: `isready`は状態を変更しない（前後で他コマンドへの応答が不変）。

### D6-USI-03 RuleSetオプション宣言の形式
- 典拠: PL「規則オプション」（`option name RuleSet type string default <起動時の--rules値の正準表記>`。正準表記は大文字・L,P,R,E順・同カテゴリ番号昇順）。
- 前提: `--rules l1,r1,e1,l2`のような非正準指定で起動。
- 操作と期待観測: `usi`応答内に`option name RuleSet type string default L1,L2,R1,E1`（正準化された値）が現れる。プリセット起動（`--rules lishogi`）ではdefaultは展開後コード列の正準表記`L1,L2,P3,R1,E1,E3`であり、プリセット名は現れない（PL: プリセットは入力糖衣、宣言・状態には展開後だけが現れる）。
- 境界・不正: なし（宣言は起動値の関数）。
- 性質: 宣言default値は`--rules`値の正準化写像として決定的。

### D6-USI-04 RuleSet値文法（コンマ区切り・大小非区別・重複拒否）
- 典拠: PL「規則オプション」（コンマ区切り空白なし、大文字小文字を区別せず受理、重複コードは拒否）。UL（setoption値は空白を含められない）。
- 前提: `usi`/`usiok`後の待機中。
- 操作と期待観測: `setoption name RuleSet value l1,r2` → エラー出力なし（受理、pending更新）。次局開始後の観測（D6-USI-07経由、または`state`のrules欄）で`L1,R2`が反映される。
- 境界・不正: `value L1,l1,R1`（重複、大小違いも同一コード）→ `info string error: ...`行、pending不変。空白入り値はUSI仕様上送れない（構文上トークンが分かれ、未知トークン無視により意味不正となる）。
- 性質: 受理判定は大文字小文字の正規化後の集合として決まる。

### D6-USI-05 プリセット名の受理と併記拒否
- 典拠: PL「規則オプション」（プリセット名は`engine-default`と`lishogi`、単独指定のみ、規則コードまたは他プリセットとの併記は拒否、`lishogi,P1`への専用エラー）。R33第5・6項（大文字小文字非区別、併記を認めない）。
- 前提: 待機中。
- 操作と期待観測: `setoption name RuleSet value LISHOGI` → 受理（大小非区別）。次局の`state`のrules欄が`L1,L2,P3,R1,E1,E3`（展開後正準表記）。`value engine-default` → 次局rules欄`R1`。
- 境界・不正: `value lishogi,P1`・`value lishogi,engine-default` → `info string error: ...`、pending不変。`value standard`は受理しない（PL 2026-08-11追記: `standard`という名前は受理しない）→ エラー行。
- 性質: プリセットは受理時にコード列へ展開され、以後の観測にプリセット名は現れない。

### D6-USI-06 無効なRuleSet値の受信時拒否とpending維持
- 典拠: PL「規則オプション」（受信時に`SetRules`検証、不正ならpendingを変えずエラー通知。USIは`info string error: ...`）。
- 前提: 起動時`--rules R1`。`setoption name RuleSet value L1,R2`で正当なpendingを作った後。
- 操作と期待観測: `setoption name RuleSet value XX9`（未知コード）→ `info string error: ...`行。その後`usinewgame`→`position startpos`→`state` → rules欄は`L1,R2`（直前の正当なpendingが生きている）。
- 境界・不正: 反復規則を含まない列（例`value L1,E1`）も受信時に拒否される（D6-ENG-02）。矛盾組合せ（`L0,L1`、`R1,R2`、`E2,E3`）は`Rules::from_codes`検証で拒否（R33第9項、RULES.md第29〜32条の排他規定）。
- 性質: 拒否はpending・active・ライフサイクルのいずれも変更しない。

### D6-USI-07 規則latch（対局中は不反映、次局から反映）
- 典拠: PL設計判断「規則指定」・「規則オプション」（active/pendingの2状態、変更は次の対局開始時に反映）。PL実施状況フェーズ3（`InGame`中の`SetPosition`はactive規則で現局を原子的に再構成し、pending規則は次局まで反映しない）。
- 前提: `--rules R1`で起動し対局中（`position startpos moves ...`適用済み）。
- 操作と期待観測: `setoption name RuleSet value R2` → エラーなし。直後の`state` → rules欄は依然`R1`（active不変）。続けて同一対局の`position startpos moves <延長列>`を適用しても`state`のrulesは`R1`のまま。`gameover ...`後に新しい`position startpos` → `state`のrulesが`R2`。
- 境界・不正: 起動時は`--rules`値がactiveとpendingの両方に入り、未確定状態は存在しない（PL）。
- 性質: activeの変化点はcommit（D6-USI-08）だけである。

### D6-USI-08 commit点（`usinewgame`とAwaitingStartでの`position`）
- 典拠: PL「コマンドenum…」（commit点は`NewGame`受信時と、`AwaitingStart`状態での`SetPosition`受信時。lishogi-botは`usinewgame`を送らないため後者が実質のcommit経路）。
- 前提: pendingにactiveと異なる規則がある状態。
- 操作と期待観測: 経路A: `usinewgame`→`position startpos`→`state`でpending規則が反映。経路B: `usinewgame`を送らず、`gameover`後（AwaitingStart）に直接`position startpos ...`→`state`で同じく反映。両経路で同一の結果。
- 境界・不正: `InGame`での`position`はcommitしない（D6-USI-07）。
- 性質: `usinewgame`の有無は次局規則の反映結果に影響しない（lishogi-bot互換の要）。

### D6-USI-09 `USI_Variant`（`chushogi`のみ受理）
- 典拠: PL「規則オプション」（string型で宣言、値`chushogi`だけを受理、他はエラー通知）。UL（lishogi-botは宣言の有無にかかわらず`setoption name USI_Variant value chushogi`を送る）。
- 前提: 待機中。
- 操作と期待観測: `setoption name USI_Variant value chushogi` → エラーなし、以後の挙動不変。`usi`応答に`option name USI_Variant type string ...`宣言がある。
- 境界・不正: `value shogi`・`value standard` → `info string error: ...`行。状態は不変。
- 性質: 本エンジンは中将棋専用であり、受理される変種値は1つに固定される。

### D6-USI-10 `position startpos`の中将棋初期局面読み替え
- 典拠: PL「規則オプション」末尾（`position startpos`は中将棋初期局面と解釈する。原典の標準将棋読みからの意図的な読み替えで、仕様として明記）。UL（初期局面SFEN文字列、shogiopsも`startpos`を受理）。
- 前提: 待機中または対局中。
- 操作と期待観測: `position startpos`→`state` → board欄がRULES.md第5条初期配置の2欄SFEN（`lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL b`）。`position sfen <初期4欄SFEN>`と同一の内部状態になる（`state`・`moves`の応答一致で観測）。
- 境界・不正: なし。
- 性質: `startpos`と初期SFENの等価性。

### D6-USI-11 `position`の原子的適用（失敗時は前局面維持）
- 典拠: PL「コマンドenum…」（commitは複製上で全適用し成功時のみ交換、途中失敗で全状態不変）・「思考開始指示と終局裁定の通知」（USIは失敗時点で`info string error: ...`を出力し当該`position`全体を適用せず直前の有効状態を保持）。
- 前提: `position startpos moves <正当列A>`適用済み。
- 操作と期待観測: `position startpos moves <正当列A> <不合法手>` → `info string error: ...`行1行。直後の`state`・`moves` → 列A適用後の局面のまま（board・rules・status不変）。
- 境界・不正: 不正なSFEN（`position sfen <壊れた文字列>`）も同じ経路（`InvalidPosition`）でエラー行、状態不変。moves列の先頭手が不合法な場合も同様（部分適用は起きない）。
- 性質: `position`の効果は全適用か無効果かの2値である。

### D6-USI-12 `position`失敗後の`go`拒否と正当な`position`による回復
- 典拠: EC「Lishogi-Bot経路」（検証失敗後は局面同期失敗状態へ入り、次に正当な`position`を受理するまで`go`はエラー情報行だけを返し、旧局面から`bestmove`を生成しない）。
- 前提: D6-USI-11の失敗直後。
- 操作と期待観測: `go depth 1` → `info string error: ...`行のみ、`bestmove`行なし。次に`position startpos moves <正当列>` → エラーなし。続く`go depth 1` → `bestmove <手>`が返る。
- 境界・不正: 同期失敗状態でも`Engine.game`自体は直前の有効状態を保持している（`state`は旧局面を返す）。失敗状態の解除はエラー行の再出力ではなく正当な`position`受理だけによる。
- 性質: 「サーバから見て不正な着手を黙って返す経路」が存在しないこと（安全性）。

### D6-USI-13 Lishogi-Bot実送信系列互換（`usinewgame`なしの毎手全列再送）
- 典拠: PL「コマンドenum…」（Lishogi-Botは`usinewgame`を送信せず、対局ごとに`setoption`（USI_Variant含む）→`position`→`go`の順で送る。ソース確認済み）。EC「局面同期」（毎手`position <初期局面> moves <全着手列>`、`InGame`では現局の原子的再構成）。UL（B1典拠）。
- 前提: `--rules lishogi`で起動（EC: Lishogi-Bot経路は`--rules lishogi`を必須とする）。
- 操作と期待観測: `usi`→`isready`→`setoption name USI_Variant value chushogi`→`position startpos`→`go depth 1`→（`bestmove`受領）→`position startpos moves <1手>`→`go depth 1`→… の系列が、`usinewgame`なしで全手正常応答する。各`go`に対し`bestmove`がちょうど1行。
- 境界・不正: 着手列は毎回全列であり増分ではない。途中で`position`列が過去手を含んだまま伸びることが正常系である。
- 性質: 反復履歴・先獅子状態は着手列の再適用で復元される（PL: SFENは履歴を運ばない）。

### D6-USI-14 `go`時間引数の受理とミリ秒正規化・手番側選択
- 典拠: EC実施状況フェーズ1（`btime`・`wtime`・`binc`・`winc`・`byoyomi`・`movetime`・`depth`・`nodes`・`infinite`を受理してミリ秒の`SearchLimits`へ正規化し、残り時間と加算は現局面の手番側から選ぶ）。USIの時間単位はミリ秒（EC「責務分担」）。
- 前提: 対局中。先手番の局面と後手番の局面をそれぞれ用意する。
- 操作と期待観測: `go btime 1000 wtime 2000 binc 10 winc 20 byoyomi 0` → `bestmove`が返る（時間予算式そのものはsearch.mdの責務であり本領域では検証しない）。先手番局面ではb系、後手番局面ではw系の値が使われることを、台本では手番別の2系列が同様に完走することで固定する（予算の数値検証はD7領域）。
- 境界・不正: 正規化の単位（USIはミリ秒のまま）はCECPの1/100秒（D6-CECP-20）と対比する。`movetime`と時計の併用時の優先順位は明文がない → SPEC_UNCLEAR-02。同一引数の重複（`go depth 1 depth 2`）も明文がない → SPEC_UNCLEAR-03。
- 性質: wire解析と単位正規化はプロトコル側、予算計算は探索側という所有境界（EC）。

### D6-USI-15 裸の`go`・未知引数`go`のエラー
- 典拠: EC実施状況フェーズ1（裸の`go`と未知引数は従来どおりエラーとし、`go depth|nodes`→`bestmove`のwire契約は不変）。
- 前提: 対局中。
- 操作と期待観測: `go`（引数なし）→ `info string error: ...`行、`bestmove`なし。`go foobar 3` → 同様。
- 境界・不正: エラー本文の完全形は明文にない（完全一致を契約にしない）。
- 性質: 探索制限が定まらない`go`は暗黙の既定値でフォールバックしない（絶対規則: 暗黙フォールバック禁止。CECP側D6-CECP-15と同型）。

### D6-USI-16 `go depth`/`go nodes`の決定的探索と`bestmove`
- 典拠: EC実施状況フェーズ1（`go depth|nodes`→`bestmove`のwire契約は前倒し実装から変更しない）。EC設計判断（台本は`depth`または`nodes`固定で決定的に書く）。
- 前提: 対局中。
- 操作と期待観測: `go depth 1` → 0行以上の`info`行に続いて`bestmove <合法手>`1行。返った手は同一局面への`moves`応答の集合に含まれる（合法性の外形検証）。同一局面・同一引数で再実行すると同一の`bestmove`（決定性）。
- 境界・不正: `depth`の上限256は文書補修対象であり現時点で規範文書に明文がない → SPEC_UNCLEAR-01。補修完了までは境界値（256受理・257拒否）のテストを書かない。
- 性質: `bestmove`は各`go`に対し高々1回（D6-USI-20と併せて一意性を成す）。

### D6-USI-17 `go mate`→`checkmate notimplemented`
- 典拠: PL「思考開始指示と終局裁定の通知」（`go mate`には原典が未実装応答`checkmate notimplemented`を定めるためこれを返す）。EC（`go mate`への`checkmate notimplemented`応答は現行のまま維持）。UL（`checkmate [... | notimplemented]`はUSI原典のコマンド）。
- 前提: 対局中。
- 操作と期待観測: `go mate 10` → `checkmate notimplemented`1行（台本完全一致）。`bestmove`なし。
- 境界・不正: 引数の有無・値によらず同一応答。
- 性質: 状態は変化しない。

### D6-USI-18 `go ponder`・`ponderhit`の拒否
- 典拠: EC「探索とbestmove」（`go ponder`と`ponderhit`は対象外であり、受信した場合は`info string error: ...`を出力し`bestmove`を返さない）。EC適用範囲（`USI_Ponder`は宣言しない）。
- 前提: 対局中。
- 操作と期待観測: `go ponder` → エラー行のみ。`ponderhit` → エラー行のみ。`usi`応答に`USI_Ponder`宣言が現れない。
- 境界・不正: なし。
- 性質: ponder系は完全な非対応であり部分動作しない。

### D6-USI-19 `go infinite`は`stop`まで`bestmove`を保留
- 典拠: EC「思考情報」（`go infinite`では停止指示（`stop`）まで`bestmove`を出さない）。EC実施状況フェーズ1煙試験（`go infinite`→`stop`の単一bestmove）。
- 前提: 対局中。
- 操作と期待観測: `go infinite`→（`bestmove`が出ないことを確認可能な後続入力）→`stop` → `bestmove`1行。台本ではチャネル駆動の順序保証に依存し、`stop`前に`bestmove`行が現れないことを出力順で検証する。
- 境界・不正: `go infinite`中の`quit`は`bestmove`なしで終了（D6-USI-23）。
- 性質: `bestmove`はちょうど1回（一意性）。

### D6-USI-20 `stop`と`bestmove`一意性
- 典拠: EC「探索中に届くコマンド」（探索中の`stop`は探索を停止し、現時点の最善手を`bestmove`として返す）。EC実施状況（停止済み探索の遅延結果は探索IDの不一致で破棄）。
- 前提: `go depth <大>`または`go infinite`で探索中。
- 操作と期待観測: `stop` → `bestmove`1行。以後、同じ探索から追加の`bestmove`が出力されない（遅延結果破棄の外形観測: 台本末尾まで`bestmove`が計1行）。
- 境界・不正: 探索外の`stop`の扱いは明文がない → SPEC_UNCLEAR-06。
- 性質: 1回の`go`に対する`bestmove`は最大1行（`gameover`/`quit`破棄時は0行）。

### D6-USI-21 探索中の重複`go`拒否
- 典拠: EC「探索中に届くコマンド」（探索中の重複した`go`はエラー情報行とする）。
- 前提: 探索中。
- 操作と期待観測: 2つ目の`go depth 1` → `info string error: ...`行。元の探索の`bestmove`は正常に1行返り、2つ目の`go`への`bestmove`は存在しない。
- 境界・不正: なし。
- 性質: 同時に走る探索は高々1つ。

### D6-USI-22 探索中コマンドのキュー順序（`bestmove`が`readyok`に先行）
- 典拠: EC「探索中に届くコマンド」（その他のコマンドは探索のjoin後（`bestmove`送出後）に適用する）。
- 前提: 探索中。
- 操作と期待観測: 探索中に`isready`を送る → 出力順は`bestmove ...`の後に`readyok`。探索中の`position`・`setoption`も同様にjoin後適用（適用結果は`bestmove`後の`state`で観測）。
- 境界・不正: 停止を指示するコマンド（`stop`・`gameover`・`quit`）はキューではなく即時に停止系の効果を持つ。
- 性質: 出力順序の全順序性（受信順の因果が出力に保存される）。

### D6-USI-23 探索中の`gameover`・`quit`による探索破棄
- 典拠: EC「探索中に届くコマンド」（探索中の`gameover`と`quit`は探索を停止して結果を破棄し、`bestmove`を返さない）。EC実施状況煙試験（探索中`quit`の結果破棄）。
- 前提: 探索中。
- 操作と期待観測: `gameover win` → `bestmove`が出力されない。エンジンはAwaitingStartへ（後続の`moves`が`info string error: moves requires an active game`を返すことで観測）。`quit`の場合は無応答で終了し、以後の出力が一切ない。
- 境界・不正: `gameover`の引数（win|lose|draw）の意味論への言及は規範文書にない → SPEC_UNCLEAR-07。テストは引数値によらない同一挙動を仮定しない（win固定で書く）。
- 性質: 破棄後に遅延`bestmove`が漏れない（D6-USI-20と同じ一意性の系）。

### D6-USI-24 `bestmove`を`Engine.game`へ適用しない
- 典拠: EC「探索とbestmove」（探索は複製上で行い、返った着手を`bestmove`として送出するだけで`Engine.game`へは適用しない。送出後も`InGame`のまま次の`position`で同期）。EC完了条件（不適用が検証されていること）。
- 前提: 対局中、`position startpos moves <列A>`適用済み。
- 操作と期待観測: `go depth 1`→`bestmove <m>`受領後、`state` → board欄は列A適用後の局面のまま（`<m>`が指されていない）。`moves` → 列A局面の合法手集合のまま。
- 境界・不正: なし。
- 性質: `go`は`Engine.game`に対して読み取り専用である。

### D6-USI-25 `InGame`以外での`go`拒否
- 典拠: EC「探索とbestmove」（`go`が`InGame`以外（AwaitingStart・Finished）で届いた場合はエラー情報行を出力し`bestmove`を返さない。契約外入力として扱う）。EC設計判断「`go`のライフサイクル契約」。
- 前提: (a) 起動直後（AwaitingStart）。(b) 着手適用で終局した局面（Finished）。
- 操作と期待観測: 両状態で`go depth 1` → `info string error: ...`行のみ、`bestmove`なし。
- 境界・不正: Finishedの作り方は`position startpos moves <終局に至る列>`（王駒捕獲手を含む列）による。USI原典は`go`に`bestmove`を要求するが、本設計はこの系列を契約外と明記している（意図的逸脱の固定）。
- 性質: `bestmove`が返るのは`InGame`かつ同期成功状態だけである。

### D6-USI-26 `info`行の形式
- 典拠: EC「思考情報」（`info depth <depth> score <cp|mate> <value> nodes <nodes> nps <nps> pv <move1> <move2> ...`。PVは既存のlishogi系USI表記。深さ1完了前の打ち切りでは`info`を省略し、事前確保した合法手を`bestmove`とする）。
- 前提: 対局中、`go depth 2`等。
- 操作と期待観測: `bestmove`に先行する各`info`行が上記トークン順に適合する（depth・nodes・npsは非負整数、scoreは`cp <整数>`または`mate <整数>`、pvの各手はUSI指し手構文）。
- 境界・不正: 行数（イテレーション数）は契約にしない。数値の値自体も契約にしない（形式のみ）。
- 性質: `info`は`bestmove`より前にだけ現れる。

### D6-USI-27 `USI_Hash`の受理
- 典拠: EC実施状況フェーズ1（`USI_Hash`（既定256MB、探索中でないときのリサイズ）を実装）。EC適用範囲（`USI_Hash`オプションのwire受理は本設計書の責務）。
- 前提: 待機中。
- 操作と期待観測: `setoption name USI_Hash value 64` → エラーなし。以後の`go depth N`が正常に`bestmove`を返す（リサイズの外形無害性）。`usi`応答に`USI_Hash`の宣言行がある。
- 境界・不正: 宣言の型・default表記・min/max、不正値（0、負、非数、極大）への応答は明文がない → SPEC_UNCLEAR-04。探索中の`USI_Hash`はD6-USI-22のキュー契約に従いjoin後適用（「探索中でないときのリサイズ」）。置換表内容の検査は行わない（観測方針）。
- 性質: リサイズは対局状態（`state`・`moves`の応答）を変えない。

### D6-USI-28 `gameover`受理とUSIに終局通知経路がないこと
- 典拠: PL「思考開始指示と終局裁定の通知」（USIにはエンジン発の裁定通知手段がないため出力せず、台本は`run`終了後の`Engine`状態で裁定一致を検証する）。EC「終局責任」（minaseは`gameover`の受理（AwaitingStartへの復帰）だけを行う）。PLコマンドenum（`EndGame`はAwaitingStartへ戻る）。
- 前提: (a) 対局中。(b) 王駒捕獲手の適用で内部的にFinishedへ遷移した局面。
- 操作と期待観測: (b)の`position ... moves <王駒捕獲列>`適用時、stdoutに終局を示す自発出力が一切現れない（USI経路の無通知）。裁定の検証は`run`終了後の`GameStatus`（Finishedと勝者・理由）または`state`のstatus欄で行う。(a)(b)いずれでも`gameover win` → 無応答でAwaitingStartへ（後続`moves`のエラーで観測）。
- 境界・不正: `gameover`後の`state`は応答しない（D6-USI-33）。
- 性質: 3経路（USI無通知・CECPのRESULT・自作GUIのstate応答）の通知独立性（EC「終局状態の通知経路の対比」）。

### D6-USI-29 未知コマンド・未知トークンの無視と`quit`の無応答
- 典拠: PL「USIの未知入力は原典準拠」（未知コマンド行と既知コマンド内の未知トークンは無視。意味的な不正だけを`info string error: ...`で通知）。UL（未知のコマンド・トークンを無視して残りの解釈を続ける原典規則）。PL（`quit`は無応答で終了）。
- 前提: 任意の状態。
- 操作と期待観測: `foobar baz` → 出力なし、状態不変。`position startpos unknown_token`のような既知コマンド内未知トークン → トークンを無視して解釈が続く（結果は`position startpos`と同一。`state`で観測）。`quit` → 何も出力せず`run`が正常終了する。
- 境界・不正: `debug`・`register`（原典の既知コマンド）への具体的挙動は規範文書に明文がない → SPEC_UNCLEAR-14。テスト化するなら「無視」を実装契約として明示する。
- 性質: wire上の寛容はプロトコルモジュールに閉じ、fail-fastはエンジン境界（`Rejected`）で保つ（PL）。

### D6-USI-30 `moves`照会（合法手集合、順序非固定）
- 典拠: BG「movesコマンド」（`moves` → `moves <move1> <move2> ...`の1行。`Game::legal_moves()`の全要素を既存lishogi系USI表記で。指し手の順序は契約に含めず受信側は集合として扱う）。UL「minase固有のUSI拡張」。
- 前提: `position startpos`適用済み（InGame）。
- 操作と期待観測: `moves` → `moves `で始まる1行。空白分解した集合が当該局面の合法手集合のUSI表記と一致する（初期局面の合法手はD1/D2領域の期待値を参照。本領域では「1行・集合比較・表記がD5のUSI構文」だけを契約とする）。
- 境界・不正: 列挙順を固定するassertを書いてはならない（BGが明示的に順序を契約から除外）。空集合を進行中の正常状態として表す形式は存在しない（合法手ゼロは審判層が終局させるため`InGame`と両立しない）。
- 性質: `moves`は読み取り専用（前後で`state`不変）。

### D6-USI-31 `moves`の対局開始前・終局後エラー
- 典拠: BG（AwaitingStartまたはFinishedでは`info string error: moves requires an active game`）。UL同節。
- 前提: (a) 起動直後。(b) 終局裁定後（Finished）。
- 操作と期待観測: 両状態で`moves` → `info string error: moves requires an active game`1行（台本完全一致）。
- 境界・不正: Finishedでは`moves`でなく`state`を使うのが契約（非対称はBGが意図的と明記）。
- 性質: エラー応答は状態を変更しない。

### D6-USI-32 `state`の行形式とstatus語彙
- 典拠: BG「stateコマンド」（`state rules <rules> board <board-sfen> <side> status <status>`の1行。rulesは正準順コンマ区切り、boardは`to_sfen`の2欄SFEN。statusは`ongoing`／`win black|white royal-capture|repetition|piece-exhaustion|bare-king|stalemate|mate`／`draw repetition|piece-exhaustion|bare-king`）。UL同節。
- 前提: `position startpos`適用済み、`--rules lishogi`。
- 操作と期待観測: `state` → `state rules L1,L2,P3,R1,E1,E3 board <初期2欄SFEN> status ongoing`（単一行・完全一致。BGテスト方針が完全一致比較を明記）。終局局面では`status win black royal-capture`等、裁定に対応する語。
- 境界・不正: `resignation`と`agreement`は文法に含めず、遭遇時はstatusを出さず`info string error: ...`（BG。現行USIには到達経路がないため通常台本では検証不能。実装契約としての防御的テストのみ可）。`piece-exhaustion`と`bare-king`は採用規則により排他。
- 性質: 2欄SFENは先獅子状態・成り権保留・反復履歴を運ばない（GUIは`board`欄から状態を復元しない、という利用制約が文書化されている）。

### D6-USI-33 `state`のライフサイクル（Finished応答・AwaitingStartエラー・次局はactive規則）
- 典拠: BG（`state`は`InGame`と`Finished`で応答。起動直後と`gameover`受信後のAwaitingStartでは`info string error: state requires an active or finished game`。`gameover`後は終局済み局面を内部に保持していても応答しない）。BGテスト方針（`gameover`後にRuleSetを変更して次局を開始し、rulesが新しい規則を返す）。
- 前提: 終局裁定に至る`position`適用済み。
- 操作と期待観測: Finishedで`state` → status欄が終局裁定（`moves`はエラーになる非対称と対で検証）。`gameover win`後に`state` → `info string error: state requires an active or finished game`（台本完全一致）。続けて`setoption name RuleSet value R2`→`position startpos`→`state` → rules欄`R2`（次局のactive規則）。
- 境界・不正: 起動直後の`state`も同じエラー行。
- 性質: GUIは`state`で終局を確認してから`gameover`を送る、という利用順序が契約の前提。

### D6-USI-34 王駒捕獲終局を含むUSI台本の裁定検証方式
- 典拠: PL検証の節（USIの台本テストは出力照合に加えて`run`終了後の`Engine`状態（GameStatus）の検査で裁定一致を確認する）。PL実施状況フェーズ4（lishogiリプレイ照合が王駒捕獲・裸玉・反復・引き分け・投了の10局で裁定一致済み。これはtests/lishogi_replay.rsとして保全対象）。
- 前提: `--rules lishogi`、終局に至る実棋譜断片。
- 操作と期待観測: `position <初期> moves <全列>`適用後、stdoutに終局出力がないこと（D6-USI-28）と、`run`後のGameStatusがlishogiの実裁定（勝者・理由）と一致すること。
- 境界・不正: 本項は既存統合テスト（リプレイ照合）が担う領域と重なるため、ユニット側では代表1局面（王駒捕獲1手前→捕獲手適用）に縮約してよい。
- 性質: 裁定の正は外部オラクル（lishogi実裁定）である。

### D6-USI-35 `Threads`オプションの宣言
- 典拠: LS「プロトコル設定」（`option name Threads type spin default 1 min 1 max 256`を宣言し、既定値は探索層の`DEFAULT_THREADS`から表示する）。
- 前提: 起動直後。
- 操作と期待観測: `usi`送信 → 応答列に`option name Threads type spin default 1 min 1 max 256`が1行現れ、最後に`usiok`が現れる。
- 境界・不正: 利用可能なCPU数を既定値や上限の表示へ反映しない。
- 性質: 宣言は実行環境に依存せず決定的である。

### D6-USI-36 `Threads`の受理・拒否と設定維持
- 典拠: LS「プロトコル設定」（`setoption name Threads value N`は1以上256以下の10進整数だけを受理し、値の欠落、非整数、0、上限超過は固定した`info string error`を返して直前の設定を保持する）。
- 前提: 待機中。
- 操作と期待観測: `value 1`と`value 256`は無応答で受理され、値の欠落、`value nope`、`value 0`、`value 257`は固定した`info string error: ...`を各1行返す。
- 境界・不正: 不正値をCPU数や境界値へ丸めず、直前に受理した設定を保持する。
- 性質: 設定値は常に`NonZeroUsize`として1以上256以下である。

### D6-USI-37 探索中の`Threads`変更
- 典拠: LS「プロトコル設定」（探索中の設定変更は既存の入力待機列へ載せ、実行中探索を変更せず次の探索から適用する）。
- 前提: `go infinite`による探索中。
- 操作と期待観測: `setoption name Threads value 2`→`stop`→次の`position`→`go depth 1`を送ると、最初の`bestmove`後に設定が処理され、次の探索も単一の`bestmove`を返して完走する。
- 境界・不正: 実行中探索のワーカー数は変更せず、設定処理のために別の並列探索経路を作らない。
- 性質: 設定の反映境界は探索のjoinである。

### D6-USI-38 採用結果の条件付き最終`info`
- 典拠: LS「設計判断」「探索チーム」第2版（`Finished.depth`が最後に出力した`info depth`を超える場合だけ、`bestmove`の前に採用結果の最終`info`を出す）。
- 前提: 対局中。`Threads=1`または2以上で、1件以上の`info`を生成できる固定深さ探索。
- 操作と期待観測: `Finished.depth`が最後に出力済みの深さを超える場合、`Finished`のdepth、score、nodes、elapsed、およびpvを使った`info`を1行出し、その直後に`bestmove`を1回だけ出す。超えない場合は追加の`info`を出さない。
- 境界・不正: `Threads=1`では`Finished.depth`が最後の`Progress.depth`を超えないため、出力列は従来と同一である。`Threads>=2`の探索順は非決定的なので、統合テストは最終`info`の追加有無や深さの値を固定せず、`info`行の後に`bestmove`が1回だけ続く構造を確認する。
- 性質: 最終`info`を出す場合もD6-USI-26と同じ書式を使い、`bestmove`より後には`info`を出さない。

---

## 2. CECP（D6-CECP）

### D6-CECP-01 feature宣言の全文
- 典拠: PL「CECPのfeature宣言」（`myname`、`variants="chu"`、`setboard=1`、`usermove=1`、`ping=1`、`colors=0`、`sigint=0`、`sigterm=0`、`analyze=0`、`draw=0`、`option`（RuleSet）、`done=1`。文字列値は二重引用符）＋PL2026-08-14注記とEC（`time=1`へ改定、`memory=1`を追加。現行はECを正とする）＋LS「プロトコル設定」（`smp=1`を`done=1`の前に追加）。CE第4章（featureの型と既定値、done=1の意味）。
- 前提: 起動直後。
- 操作と期待観測: `xboard`→`protover 2` → feature宣言列が出力され、集合として {myname="…", variants="chu", setboard=1, usermove=1, ping=1, colors=0, sigint=0, sigterm=0, analyze=0, time=1, draw=0, memory=1, option="RuleSet -string <正準表記>", smp=1} を含み、`smp=1`の次が`done=1`である。`myname`・`variants`・`option`の値は二重引用符で囲む（PL実施状況フェーズ5のレビュー修正が明記）。
- 境界・不正: `debug`・`highlight`・`san`・`reuse`は宣言しない（PL: debugは#出力を行わないため宣言しない、highlightは対象外）。宣言の行分割・行内順序は仕様が任意とするため（CE第4章）、LSが定める`smp=1`直後の`done=1`以外の順序を契約にしない。
- 性質: 宣言は起動時`--rules`値だけに依存する。

### D6-CECP-02 必須featureの拒否で終了、他の拒否は無視
- 典拠: PL「プロトコル固有の制御コマンド」（`accepted`と`rejected`は記録し、必須feature（`setboard`、`usermove`、`ping`）が拒否された場合は`tellusererror`を出して終了）。PL台本テスト範囲（この経路を台本に含める）。
- 前提: `protover 2`後のfeature交渉中。
- 操作と期待観測: `rejected setboard`（または`usermove`、`ping`）→ `tellusererror ...`行を出力して`run`が終了する。一方`rejected time`・`rejected memory`等の非必須は無視され、セッション継続（後続の`new`が正常動作）。`accepted <任意>`は常に無視。
- 境界・不正: 3種それぞれの拒否で同じ終了経路になることを個別に検証する。
- 性質: 終了は必須3種に限る（他featureの拒否で挙動が変わらない）。

### D6-CECP-03 `variant chu`の厳密受理
- 典拠: PL「プロトコル固有の制御コマンド」（`variant`は`chu`だけを受理し、他は`Error (unsupported variant): ...`とする）。CE第6章（chuは正典の定義済み変則）。
- 前提: `new`後（CE第3章: variantはnewの直後、最初の指し手・局面設定より前に送られる）。
- 操作と期待観測: `variant chu` → エラーなし、対局続行可能。`variant shogi`・`variant dai` → `Error (unsupported variant): `で始まる1行、状態不変。
- 境界・不正: `...`部分の完全形は明文がない → SPEC_UNCLEAR-12（前方一致で検証）。大文字`CHU`等の受理は明文がない（値照合の大小非区別はRuleSet系にだけ規定がある）。厳密一致`chu`のみを契約とする。
- 性質: 受理変種は1つに固定。

### D6-CECP-04 白=先手の内部手番対応
- 典拠: PL「内部正準と手番文字の変換責任」（CECPはWhiteが先手。手番文字の反転はCECPモジュール内に閉じる。同一局面をCECPの`setboard`（手番`w`）とUSIの`position sfen`（手番`b`）で設定したとき同じ内部手番になる検証を台本に含める）。PL「setboardの受理契約」（手番部は`w`を先手（内部Color::Black）、`b`を後手とし、それ以外は拒否）。CE第6章（白が先に指す）。
- 前提: 同一盤面のSFEN盤面部。
- 操作と期待観測: CECPセッションで`setboard <盤面部> w`、USIセッションで`position sfen <盤面部> b - 1`を適用し、両者の後続挙動（合法手・裁定）が一致する。観測は、両セッションに同一の指し手列を与えたときの受理・拒否の一致による（CECP側は`usermove`の受理、USI側は`position ... moves`の成功）。
- 境界・不正: `setboard <盤面部> x`（不正手番文字）→ 拒否（D6-CECP-24の失敗経路）。
- 性質: 手番反転の閉じ込め（エンジン内部は常に先手=Black）。

### D6-CECP-05 結果コードの白視点
- 典拠: PL「内部正準と手番文字の変換責任」（CECPの`1-0`は先手勝ち。先手勝ちの局面でCECPが`1-0`を出力する検証を台本に含める）・「思考開始指示と終局裁定の通知」（先手勝ち`1-0`、後手勝ち`0-1`、引き分け`1/2-1/2`）。CE第6章（resultの結果コードは白から見た値）。
- 前提: 先手勝ちで終局する指し手列、後手勝ちで終局する指し手列。
- 操作と期待観測: 先手（White）が後手の最後の王駒を取る`usermove`列 → `1-0 {royal capture}`のRESULT行。後手勝ちなら`0-1 {royal capture}`。理由文字列はPLの対応表（royal capture／checkmate／no legal moves／repetition／bare king／piece exhaustion系はD3の裁定に対応）で固定（台本完全一致）。
- 境界・不正: 引き分け（R1反復等）では`1/2-1/2 {repetition}`。
- 性質: RESULT行の視点変換はCECPモジュールに閉じる。

### D6-CECP-06 `new`の意味論
- 典拠: PL「newの意味論」（`new`受信時に`NewGame`（pending規則のcommit）に続けて初期局面の`SetPosition`を適用し`InGame`へ入る。後続の`setboard`は`InGame`での`SetPosition`として現局を置換）。EC「状態機械」（`new`はforce状態を解除し、担当手番を後手（CECPのBlack）とする。探索中の`new`は停止・破棄）。
- 前提: feature交渉済み。
- 操作と期待観測: `new`→`variant chu` → エラーなし。直後の`usermove <初期局面の合法手>`が受理される（初期局面から`InGame`である証拠）。`new`の時点でエンジンは着手を自発しない（担当手番=後手、手番は先手=White側にあるため）。pendingに規則変更がある場合、`new`でcommitされる（次のRuleSet観測で確認: D6-CECP-25）。
- 境界・不正: 探索中の`new` → 探索停止・結果破棄（`move`行が出ない）後に新規対局へ。
- 性質: `new`後の状態は（active規則, 初期局面, force解除, 担当=後手）の4組に固定される。

### D6-CECP-07 `usermove`の複数レグ受信とレグ連続性
- 典拠: PL「Move文字列表記2形式」（受信はコンマ区切りの単一文字列、第2レグの始点が第1レグの終点と一致しない入力を拒否）。CE第5章（コンマ区切り単一行、レグ連続性はXBoard/HaChu実装で確認）。HA第7節。
- 前提: `new`→`variant chu`後、獅子の2段階手が合法な局面（初期局面から数手で作る）。
- 操作と期待観測: `usermove <leg1>,<leg2>`（連続する2レグ、例: 2段階捕獲・居喰い）→ 受理され局面が進む（後続手の受理で観測）。
- 境界・不正: 第2レグ始点≠第1レグ終点の入力 → `Illegal move: ...`（Movement分類、D6-CECP-11）。3レグ以上は中将棋の合法手に対応しないため拒否。
- 性質: 受信（コンマ区切り1行）と送信（レグ分割複数行、D6-CECP-10）の非対称。

### D6-CECP-08 成り接尾辞の解釈
- 典拠: PL「CECP指し手表記の関数契約」（成り接尾辞`+`は最終レグの末尾だけに認め、`=`は不成として受理。送信は不成に接尾辞なし）。CE第5章（+はMuller版付録Eに明文、=は実装慣行）。
- 前提: 成りが選択できる着手が合法な局面。
- 操作と期待観測: `usermove <sq><sq>+` → 成りとして適用（以後その駒が成駒として動けることで観測）。`usermove <sq><sq>=`と`usermove <sq><sq>` → どちらも不成として適用。エンジン送信の`move`行は成り時のみ`+`、不成は無印。
- 境界・不正: 非最終レグへの`+`（`<leg1>+,<leg2>`）→ 拒否。成れない着手への`+` → `Illegal move: ...`。
- 性質: 受信は`=`許容の寛容、送信は正準（無印/`+`）のみという受送信非対称。

### D6-CECP-09 `@@@@`の受理と代表選択
- 典拠: PL「Move文字列表記2形式」（受信の`@@@@`は現在局面の合法手のうち「移動元=移動先、中間升なし、不成」の正準じっとだけに照合。居喰いは候補に含めない。複数候補では移動元の内部密番号が最小の手を代表として選ぶ。代表規則は仕様として固定し、R1採用時の既知の制限として記録）。HA第8節（`@@@@`転用はHaChuの実装判断であり仕様に明文なし）。
- 前提: じっとが合法な駒が1つ以上ある局面。
- 操作と期待観測: `usermove @@@@` → 受理。複数の駒がじっと可能な局面では、移動元の内部密番号が最小の駒のじっとが適用される（観測は後続局面の同値性、またはR1攻撃的着手判定への影響を避けた局面設計で行う）。じっと不能局面（獅子系の第1段階到達升が全て塞がる等）では拒否。
- 境界・不正: 居喰い（移動元=移動先で中間升あり）は`@@@@`では入力できず、明示レグ（`f6g7,g7f6`型）で送る必要がある。
- 性質: 全じっと候補は同一の後続局面と反復キーを生む（代表選択が局面遷移に影響しないことの検証はD5表記領域と分担。本領域は受理・拒否の外形のみ）。

### D6-CECP-10 エンジン着手の`move`行送信形式
- 典拠: PL「Move文字列表記2形式」（送信はレグ分割の複数`move`行、非最終レグの末尾にコンマ、成りの`+`は最終レグだけ、正準じっとは`@@@@`の1行）。EC「着手と結果の順序」（複数レグは既存のレグ分割、じっとは`@@@@`）。CE第5章（正典の双方向規定）。
- 前提: `new`→`variant chu`→`sd 1`→`go`等でエンジンに着手させる。
- 操作と期待観測: 単レグ手は`move <leg>`1行。2段階手は`move <leg1>,`行と`move <leg2>`行の2行（1行目末尾コンマ）。じっとは`move @@@@`1行。
- 境界・不正: 決定的台本では`sd`固定で手が定まる局面を選ぶ。
- 性質: `legs`出力の連結を`parse`へ渡すと元のMoveに一致する往復性（D5領域が所有。本領域はwire上の行形式のみ）。

### D6-CECP-11 `Illegal move`の2形式
- 典拠: PL「思考開始指示と終局裁定の通知」（`Movement`は省略形`Illegal move: MOVE`、`Repetition`は`Illegal move (repetition): MOVE`）。CE第3章・第9章（正典の2形式。R2/R3の反復禁止手はIllegal move応答に対応）。
- 前提: (a) 任意規則で駒の動きに反する`usermove`。(b) R2またはR3採用時に既出局面を再現する`usermove`。
- 操作と期待観測: (a) → `Illegal move: <MOVE>`1行（受信したMOVE文字列を反響。台本完全一致）。(b) → `Illegal move (repetition): <MOVE>`1行。どちらも状態不変（同じ局面で別の合法手が引き続き受理される）。
- 境界・不正: HaChuの波括弧形式`Illegal move {理由}`（指し手反響なし）はHaChuの仕様乖離であり追随しない（HA第9章第1項）→ SPEC_UNCLEAR-10に非追随の根拠を登録。
- 性質: 理由分類は`IllegalMoveCause`（D6-ENG-03）からの決定的写像。

### D6-CECP-12 `usermove`後の自動応手
- 典拠: EC「状態機械」（`usermove`は`Game`へ適用。適用後、force状態でなく、対局が継続中で、手番が担当手番と一致するなら探索を開始する）。EC実施状況フェーズ2（`usermove`後の自動応手を実装、台本で検証）。
- 前提: `new`→`variant chu`→`sd 1`（担当手番=後手）。
- 操作と期待観測: 先手の`usermove <合法手>` → エンジンが自動で探索し`move <手>`行を返す。続けて先手が指すと再び`move`が返る、の交互進行。
- 境界・不正: force中の`usermove`は適用のみで`move`が返らない（D6-CECP-13）。担当手番でない側へ手番が渡った時点（=自分が指した直後）では探索しない。終局した`usermove`の後は探索せずRESULTのみ（D6-CECP-16）。
- 性質: 自動応手の条件は（¬force ∧ 継続中 ∧ 手番=担当）の連言。

### D6-CECP-13 `force`
- 典拠: EC「状態機械」（`force`は探索中なら停止して結果を破棄。以後は着手を出さず`usermove`を両陣営分適用するだけ。担当手番は解除）。PL（探索導入前は無視と規定していたが、ECの状態機械が現行の正）。
- 前提: 対局中。
- 操作と期待観測: `force`後、両陣営の`usermove`を交互に送ってもエンジンは`move`行を出力しない（棋譜再生モード）。不合法手には引き続き`Illegal move`応答。探索中の`force` → 進行中探索の`move`行が出ない。
- 境界・不正: `force`後の`go`で解除（D6-CECP-14）。
- 性質: force中もエンジンの合法性判定・裁定（RESULT）は生きている。

### D6-CECP-14 `go`の状態機械とRESULT後の拒否
- 典拠: EC「状態機械」（`go`は対局が継続中の場合に限り、force状態を解除し、現在の手番を担当手番として直ちに探索を開始する。`Finished`または`AwaitingStart`ではエラーを返し探索を開始しない）。EC実施状況フェーズ2（`RESULT`後の`go`拒否を台本で固定）。
- 前提: (a) force中の対局中局面。(b) RESULT送出済みのFinished。(c) `new`前（AwaitingStart）。
- 操作と期待観測: (a) `go` → エンジンが現在手番を担当して`move`行を返す。(b)(c) `go` → エラー応答（探索なし・`move`なし）。
- 境界・不正: エラーの具体形（`Error (...)`か`tellusererror`か）はECが「エラーを返し」とだけ規定 → 完全一致は契約にしない。
- 性質: 担当手番は`go`受信時の手番へ動的に付け替わる。

### D6-CECP-15 探索制限が未設定の`go`は`tellusererror`
- 典拠: EC実施状況フェーズ2の実装判断（探索制限が一つも設定されていない`go`は、暗黙の既定値でフォールバックせず`tellusererror`を返す）。
- 前提: `new`→`variant chu`直後、`level`・`st`・`sd`・`time`をいずれも受信していない。
- 操作と期待観測: `go` → `tellusererror ...`行、探索なし、`move`なし。`sd 1`受信後の`go`は正常。
- 境界・不正: なし。
- 性質: 暗黙フォールバック禁止（USIの裸`go`エラーD6-USI-15と同型の原則）。

### D6-CECP-16 着手適用→`move`行→`RESULT`1回の順序
- 典拠: EC「着手と結果の順序」（1. ApplyMove、2. `move`行送信、3. この着手で`newly_finished`が値を持つ場合に限り`RESULT {comment}`行を1回だけ。`usermove`の適用で終局した場合も`newly_finished`から1回）。PL（既にFinishedの対局への後続コマンドの応答からRESULTを再生成しない）。
- 前提: エンジン着手または`usermove`で終局する直前の局面（`sd 1`で決定的に）。
- 操作と期待観測: エンジンの王駒捕獲着手 → `move ...`行の後に`RESULT`行（例`1-0 {royal capture}`）がちょうど1行。以後、`ping`等の後続コマンドを送ってもRESULTが再出力されない。`usermove`で終局した場合は`move`行なしでRESULT1行。
- 境界・不正: 王駒捕獲での`RESULT`出力が全セッションを通じて1回だけであることを台本末尾までの出力走査で固定する。
- 性質: RESULT生成の唯一のトリガは`newly_finished`（D6-ENG-04の外形観測）。

### D6-CECP-17 `result`の受理
- 典拠: EC「状態機械」（`result`は時間切れ・切断など外部要因を含むGUIからの確定通知として受理。探索中なら停止して結果を破棄し、`EndGame`でAwaitingStartへ戻る。エンジンは自らの裁定と食い違っても異議を唱えない）。
- 前提: 対局中（自エンジンは未終局と認識）。
- 操作と期待観測: `result 1-0 {time forfeit}` → 無応答（異議・エラーなし）でAwaitingStartへ。後続の`usermove`は対局外入力として拒否系応答、`new`で次局開始可能。探索中の`result` → 進行中の`move`行が出ない（破棄）。
- 境界・不正: エンジンがRESULTを送った後にGUIから届く`result`も同じ確定通知として処理（EC）。
- 性質: 裁定の正はGUI側（外部要因を含むため）という片方向の信頼。

### D6-CECP-18 `?`（move now）
- 典拠: EC「状態機械」（`?`は探索中なら停止を指示し、その時点の最善手で通常の着手処理を行う。探索中でなければ無視する）。
- 前提: (a) 探索中。(b) 非探索中。
- 操作と期待観測: (a) `?` → 探索が打ち切られ`move`行が返る（通常の着手処理=適用→move→必要ならRESULT）。(b) `?` → 出力なし・状態不変。
- 境界・不正: 台本では探索を決定的に長くできないため、(a)は探索キュー機構のある`run_channel`系での検証か、`sd`大きめ＋即時`?`の順序契約として書く（実装到達手段はフェーズ3で確定）。
- 性質: `?`は着手の内容を変えず時期だけを早める。

### D6-CECP-19 `ping`/`pong`の順序（探索中はmove行の後）
- 典拠: PL（`ping N`は先行コマンドの処理完了後に`pong N`。同期実装では受信順処理で自動的に満たされる）。EC実施状況フェーズ2（探索中の`ping`への`pong`はmove行の後に返す順序契約を台本テストで固定）。CE第3章。
- 前提: (a) 非探索中。(b) 探索中（自動応手の思考中）。
- 操作と期待観測: (a) `ping 1` → `pong 1`（台本完全一致）。(b) `usermove <手>`→`ping 2` → 出力順が`move ...`（＋RESULTがあればその後）→`pong 2`。
- 境界・不正: Nは受信値をそのまま反響する（`ping 42`→`pong 42`）。
- 性質: pongは「それ以前の全コマンド処理完了」の同期点。

### D6-CECP-20 `time`/`otim`の単位正規化（1/100秒）
- 典拠: EC（feature `time=1`へ変更し、`time`・`otim`（1/100秒）を受理してミリ秒へ正規化する）。EC「責務分担」（CECPのtime・otimは1/100秒）。CE第4章（time featureの意味）。
- 前提: `new`→`variant chu`後。
- 操作と期待観測: `time 6000`（=60秒）・`otim 6000` → 無応答で受理され、後続の`go`が時間予算つき探索として動作する（予算値の検証はD7領域。本領域は「受理されエラーにならない」「センチ秒→ミリ秒の写像がSearchLimitsへ渡る」ことを、決定的でない時間対局を避けつつ引数解析の単体レベルで検証する）。
- 境界・不正: 非数引数の扱いは明文がない（未知トークン無視はUSIの規則でありCECPには対応規定がない）→ 完全一致契約にしない。
- 性質: 単位はUSI（ミリ秒）と異なりセンチ秒であり、両プロトコルの正規化先は同じミリ秒のSearchLimits。

### D6-CECP-21 `level`の単位正規化
- 典拠: EC実施状況フェーズ2（`level`（分・`分:秒`・加算秒）をミリ秒へ正規化。`level`第1引数（区切り手数）は構文検証のみ行い予算へは渡さない）。EC「責務分担」（levelは分）。
- 前提: `new`後。
- 操作と期待観測: `level 40 5 0`（40手/5分/加算0）→ 受理。`level 0 0:30 1`（30秒+加算1秒）→ 受理（`分:秒`形式）。正規化値の検証は引数解析の単体レベル（5分→300000ms、0:30→30000ms、加算1→1000ms）。
- 境界・不正: 第1引数は構文上整数であることだけ検証され、値は予算に影響しない（SearchLimitsに残り手数欄がないため。ECの実装判断として文書化済み）。
- 性質: 3形式（分、分:秒、加算秒）の網羅。

### D6-CECP-22 `st`/`sd`の正規化
- 典拠: EC（`st`（秒）、`sd`（深さ）を受理し正規化した値をSearchLimitsへ写す。EC「責務分担」: stは秒）。
- 前提: `new`後。
- 操作と期待観測: `st 5` → 受理（5000msの1手時間として写る）。`sd 3` → 受理（深さ上限3）。`sd 1`→`go` → 深さ1探索の`move`が決定的に返る（台本の決定性担保はこの経路）。
- 境界・不正: `sd`の上限（USIのdepth上限256相当）の明文がない → SPEC_UNCLEAR-11。
- 性質: `sd`固定はCECP台本の決定性の基盤（EC設計判断）。

### D6-CECP-23 `memory`コマンド
- 典拠: EC（feature `memory=1`。`memory <MB>`は探索中でないときの置換表リサイズとして受理する（既定256MB、探索間で再利用、`new`とRuleSet変更でクリア））。CE第4章（memory featureの意味: ハッシュ等の総メモリ量）。
- 前提: feature交渉済み。
- 操作と期待観測: `memory 64` → 無応答で受理。後続の`sd 1`→`go`が正常に`move`を返す（外形無害性）。
- 境界・不正: 探索中の`memory`はpendingキュー（D6-CECP-31）でjoin後適用。リサイズ・クリアの内部効果（TT内容）は直接観測できず、観測手段の明文もない → SPEC_UNCLEAR-13（本領域では受理の外形のみをテストし、クリア意味論はD7の置換表領域に委ねる）。不正値（0・非数）への応答は明文がない。
- 性質: `USI_Hash`（D6-USI-27）と同じ既定256MB・非探索中リサイズの対。

### D6-CECP-24 `setboard`の受理契約
- 典拠: PL「setboardの受理契約」（「盤面部 手番部」の2欄必須、3欄目以降は無視、盤面部はSFEN盤面部と同一構文・同一パーサ、手番`w`=先手・`b`=後手・他は拒否、解析失敗には`tellusererror Illegal position`を返し状態を変更しない。先獅子状態と成り権保留は設定できない）。EC（setboard局面からの対局は反復判定が空履歴から始まる制約つき）。CE第7章（XBoardの余剰フィールドをHaChuと同じ割り切りで無視）。
- 前提: `new`→`variant chu`後（InGame）。
- 操作と期待観測: `setboard <正当盤面部> w` → 無応答で現局を置換（後続`usermove`の合法判定が新局面基準になることで観測）。`setboard <正当盤面部> w - 0 1`（余剰欄つき）→ 同一結果（余剰無視）。
- 境界・不正: `setboard <壊れた盤面>` → `tellusererror Illegal position`1行（台本完全一致）、状態不変（直前局面の合法手が引き続き受理される）。手番文字`x` → 同じ失敗経路。2欄未満 → 拒否。
- 性質: 失敗時の状態保持はエンジン境界の原子性（D6-ENG-01・05）の外形。

### D6-CECP-25 RuleSetオプション（宣言・latch・不正値）
- 典拠: PL「規則オプション」（CECPは`feature option="RuleSet -string <起動値の正準表記>"`で宣言。`option NAME=VALUE`受信で検証し、正当ならpendingのみ更新、不正なら`Error (invalid option value): <受信行>`。commitは`new`）。CE第8章（feature optionの構文、GUIは`option NAME=VALUE`を送る、最初のnewより前に届く規定）。R33。
- 前提: feature交渉済み。
- 操作と期待観測: `option RuleSet=L1,R2` → 無応答で受理。現局（あれば）の裁定は不変で、次の`new`から反映（反映の観測は反復裁定の差など規則依存挙動で行うか、R2禁止手への`Illegal move (repetition)`の出現で行う）。`option RuleSet=lishogi` → 受理（プリセット、大小非区別）。
- 境界・不正: `option RuleSet=XX9`・`option RuleSet=lishogi,P1`・反復規則欠如列 → `Error (invalid option value): <受信行>`1行（受信行を反響。台本完全一致相当）、pending不変。
- 性質: USI側（D6-USI-04〜07）と同一の値文法・同一のlatch意味論をwireだけ変えて再検証する（共通解析関数の契約）。

### D6-CECP-26 無視するコマンド群
- 典拠: PL「コマンド対応の残余」（時間制御・思考出力制御・通知系のコマンドは無視。無視しても状態が変わらないため）。ただし`time`・`otim`・`level`・`st`・`sd`はECが処理へ昇格済みであり、現行の無視リストは`easy`、`hard`、`post`、`nopost`、`random`、`computer`、`name`、`hint`、`draw`である（PLのリストからEC昇格分を除いた集合）。
- 前提: 任意の状態。
- 操作と期待観測: `easy`・`hard`・`post`・`nopost`・`random`・`computer`・`name foo`・`hint`・`draw`の各行 → 出力なし、状態不変（前後の`ping`/`pong`と局面応答が不変）。
- 境界・不正: `draw`の無視は`feature draw=0`宣言（D6-CECP-01）と整合（宣言しても届き得る運用ノイズへの防御）。
- 性質: 無視は「応答なし・状態変化なし」の両方を含む。

### D6-CECP-27 `undo`・`remove`・`analyze`へのエラー
- 典拠: PL「コマンド対応の残余」（undo、remove、analyzeは無視すると盤面状態またはモードの不整合を生むため`Error (command not supported): <コマンド>`を返す）。EC「現状維持の範囲」（この扱いを変更しない）。
- 前提: 対局中。
- 操作と期待観測: `undo` → `Error (command not supported): undo`1行（台本完全一致）。`remove`・`analyze`も同型。状態不変。
- 境界・不正: なし。
- 性質: 「黙って無視すると不整合を生むもの」だけがエラーになるという分類基準（PLが根拠を明文化）。

### D6-CECP-28 未知コマンドへの`Error (unknown command)`
- 典拠: PL「コマンド対応の残余」（明示対応にも無視リストにも該当しない行は`Error (unknown command): <第1トークン>`）。CE第3章（正典の未知コマンド応答）。
- 前提: 任意の状態。
- 操作と期待観測: `foobar baz qux` → `Error (unknown command): foobar`1行（第1トークンのみ反響。台本完全一致）。状態不変。
- 境界・不正: USIが未知行を無視する（D6-USI-29）のと対照的な、プロトコル別の意図的な差。
- 性質: 3分類（明示対応／無視／unknown）の全域性。

### D6-CECP-29 `xboard`の無視と`quit`の無応答終了
- 典拠: PL「プロトコル固有の制御コマンド」（`xboard`は無視、`quit`は無応答で終了）。
- 前提: 起動直後／任意の状態。
- 操作と期待観測: `xboard` → 出力なし。`quit` → 何も出力せず`run`が正常終了。
- 境界・不正: 探索中の`quit`は探索停止・破棄を伴う（EC: 停止を指示するコマンドに含まれる）。
- 性質: セッション終了後の入力は処理されない。

### D6-CECP-30 エンジン着手の適用拒否時の退避
- 典拠: EC「着手と結果の順序」（審判層確定済みのルート合法手から選んだ手なので拒否は契約違反。拒否された場合は`tellusererror`を出してforce状態へ退避する）。
- 前提: 正常系では到達不能（防御的契約）。
- 操作と期待観測: 通常台本では検証不能。変異検証（spec-first-tests.mdフェーズ4）で探索が不正手を返す変異を適用した際、`tellusererror`行とforce退避（以後`move`を出さない）が観測されることをもって固定する。
- 境界・不正: 本項をユニットテスト化する場合は到達手段が実装依存になるため、「実装契約」と明示する。
- 性質: 契約違反時にも不正な`move`行を外へ出さない安全性。

### D6-CECP-31 探索中コマンドのpendingキュー
- 典拠: EC実施状況フェーズ2（探索中のコマンドは停止を指示するもの（`?`・`force`・`result`・`new`・`quit`）を除きpendingキューへ積んで探索join後に適用する。停止済み探索の遅延結果は探索IDの不一致で破棄）。
- 前提: 自動応手の探索中。
- 操作と期待観測: 探索中に`ping N`（D6-CECP-19）や`option RuleSet=...`を送る → 出力・適用が`move`行の後になる。停止指示5種は即時に効く（`?`は着手前倒し、`force`/`result`/`new`は破棄、`quit`は終了）。
- 境界・不正: 破棄後に遅延`move`行が漏れない（出力走査で`move`行数を固定）。
- 性質: USIのキュー契約（D6-USI-22）と同型の順序保存。

### D6-CECP-32 `smp=1`の宣言
- 典拠: LS「プロトコル設定」（CECPで`feature smp=1`を宣言し、`feature done=1`の前に置く）。
- 前提: 起動直後。
- 操作と期待観測: `protover 2`送信 → `feature smp=1`が1行現れ、その直後に`feature done=1`が現れる。
- 境界・不正: `smp=1`は対応能力の宣言であり、既定のワーカー数1を変更しない。
- 性質: 宣言順は`smp=1`、`done=1`の順で固定する。

### D6-CECP-33 `cores`の受理・拒否と設定維持
- 典拠: LS「プロトコル設定」（`cores N`は1以上256以下の10進整数だけを受理し、不正値は固定したCECPエラーを返して丸めや縮退を行わない）。
- 前提: 待機中。
- 操作と期待観測: `cores 1`と`cores 256`は無応答で受理され、値の欠落、`cores nope`、`cores 0`、`cores 257`は`Error (invalid command): cores`を各1行返す。
- 境界・不正: 不正値をCPU数や境界値へ丸めず、直前に受理した設定を保持する。
- 性質: USIの`Threads`と同じ`NonZeroUsize`の範囲を使う。

### D6-CECP-34 探索中の`cores`変更
- 典拠: LS「プロトコル設定」（探索中の`cores`は既存の入力待機列を経由し、次の探索から反映する）。
- 前提: `go`による探索中。
- 操作と期待観測: 探索中に`cores 2`を送り、`?`で探索を完了してから次局の`go`を送ると、設定は最初の`move`行後に処理され、次の探索も単一の論理着手を返して完走する。
- 境界・不正: 実行中探索のワーカー数は変更せず、CECP固有の並列探索経路を作らない。
- 性質: 設定の反映境界は探索のjoinであり、USIのD6-USI-37と同型である。

---

## 3. エンジン状態機械（D6-ENG）

本節の挙動は原則としてUSI・CECP両wireの外形（上記D6-USI／D6-CECP項）で観測する。エンジン型を直接叩くテストを書く場合も、assert対象は`EngineReply`（公開境界）までとし、内部フィールドへ触れない。

### D6-ENG-01 commitの原子性
- 典拠: PL「コマンドenum…」（commitは「pending規則で新しいGameを構築し、SetPositionの場合は局面と着手列を複製上で全適用し、成功した場合に限りactive規則・Game・ライフサイクルを同時に交換する」原子的操作。途中で失敗した場合は全状態を変更しない）。
- 前提: activeとpendingが異なる状態で、失敗するSetPosition（不正SFENまたは途中不合法手）。
- 操作と期待観測: 失敗するcommit経路（AwaitingStartでの不正`position`／`new`直後の不正`setboard`相当）の後、(1) 局面が直前の有効状態のまま、(2) active規則が旧値のまま、(3) ライフサイクルが遷移していない、の3点をwire応答（`state`／後続`usermove`の判定基準）で確認する。
- 境界・不正: moves列の最後の1手だけが不合法な場合も先頭からの部分適用が残らない（D6-USI-11）。
- 性質: 交換は全か無か。active規則・Game・ライフサイクルの3つ組は常に整合する。

### D6-ENG-02 `SetRules`の受信時検証と反復規則欠如の拒否
- 典拠: PL「コマンドenum…」（SetRulesの検証にはRules::from_codesの成功に加えて反復規則の存在（Game構築可能性）を含める。from_codesはR1〜R3を含まない列も受理するがGame::newが拒否するため、commit時ではなく受信時に弾く）。R33第5項（反復規則は常にいずれか1つ）。
- 前提: 待機中。
- 操作と期待観測: USI `setoption name RuleSet value L1,E1`（R欠如）→ `info string error: ...`、pending不変。CECP `option RuleSet=L1,E1` → `Error (invalid option value): ...`。その後のcommit点（usinewgame等）が旧pendingで成功する（エラーが遅延しない証拠）。
- 境界・不正: R0は選択可能コードとして提供されない（R33第5項: MinaseはR0を提供せずR1〜R3の明示を必須とする）→ `value R0`は拒否。
- 性質: 「未確定のまま対局開始」という状態が存在しない（起動時から常に規則確定、拒否は受信時点）。

### D6-ENG-03 `IllegalMoveCause`の分類
- 典拠: PL「コマンドenum…」（IllegalMoveCauseはMovement（駒の動き・獅子規則等の違反）とRepetition（R2またはR3の反復禁止手））・「思考開始指示と終局裁定の通知」（CECPの2形式への写像）。RULES.md第27条第4項（R2/R3の禁止手は第26条11項の不合法な着手）。
- 前提: R2採用の対局で、(a) 動きに反する手、(b) 既出局面を再現する手。
- 操作と期待観測: CECP経由で(a)→`Illegal move: MOVE`、(b)→`Illegal move (repetition): MOVE`（D6-CECP-11の再掲。分類の正しさはCECP出力形式が唯一のwire観測点）。USI経由では両方とも`info string error: ...`となり分類は行文言に依存するため、USI側では分類の完全一致を契約にしない。
- 境界・不正: R1採用時は反復が裁定（終局）であってIllegal moveにならない（第27条第4項の前段。R1の4回反復はRESULT経路）。
- 性質: 分類は規則コード（R1系かR2/R3系か）と違反種別から決定的に定まる。

### D6-ENG-04 `newly_finished`の一回性
- 典拠: PL「コマンドenum…」（`Accepted`の`newly_finished`は、この応答でOngoingからFinishedへ遷移した場合だけ裁定結果を持つ）・「思考開始指示と終局裁定の通知」（既にFinishedの対局に対する後続コマンドの応答からはRESULTを再生成しない）。
- 前提: 終局に至る着手適用。
- 操作と期待観測: wire観測はD6-CECP-16（RESULTがセッション全体で1回）。USI側は、Finished局面への同一`position`全列の再送（lishogi-bot互換で起き得る）が再度の終局通知や重複出力を生まないことで観測する。
- 境界・不正: Finished局面を含む`position`の再構成（AwaitingStartでのcommit）では、その応答が「新たに終局を確定した」応答となる。同じ局面でも応答の文脈（遷移か再確認か）で`newly_finished`の有無が変わるのが契約である。
- 性質: 終局通知は状態遷移イベントであって状態述語ではない。

### D6-ENG-05 `Rejected`時の状態不変
- 典拠: PL「コマンドenum…」（`Rejected(RejectReason)`: 拒否。エンジンの状態は一切変化しない。RejectReasonはInvalidRules／InvalidPosition／IllegalMove／GameAlreadyOver）。
- 前提: 各RejectReasonに対応する不正入力（不正規則列・不正SFEN・不合法手・終局後の着手）。
- 操作と期待観測: 各拒否の直後に、拒否前と同一の応答が得られる（`state`不変、同じ合法手が受理される、pending不変）。4種のRejectReasonを1つずつwire経由で発火させる。
- 境界・不正: `GameAlreadyOver`はFinished局面への`ApplyMove`（CECPの`usermove`）で発火し、RULES.md第26条第12項（対局終了後の着手は不成立）に対応する。
- 性質: 拒否は冪等（同じ不正入力の再送は同じ拒否を返す）。

### D6-ENG-06 ライフサイクル（AwaitingStart／InGame／Finished）
- 典拠: PL「コマンドenum…」（EngineはAwaitingStart（対局未開始）、InGame、Finishedのライフサイクルを持つ。EndGameはAwaitingStartへ戻る）。
- 前提: 起動→対局→終局→次局の全周回。
- 操作と期待観測: 状態別のコマンド受理表をwireで固定する: AwaitingStartでは`moves`/`state`エラー・`go`エラー・`position`はcommit経路。InGameでは`moves`/`state`応答・`go`受理。Finishedでは`moves`エラー・`state`応答・`go`エラー・着手はGameAlreadyOver。`gameover`/`result`でAwaitingStartへ戻り、次局が開始できる（周回性）。
- 境界・不正: FinishedからAwaitingStartへの唯一の経路はEndGame（USI `gameover`、CECP `result`）と次のcommit（AwaitingStartでのSetPosition／new）である。
- 性質: 3状態の遷移全域性（到達不能な組合せがwire上に現れない）。

### D6-ENG-07 観測方式の契約（内部検査の限定）
- 典拠: PL「コマンドenum、応答enumおよびtrait」（台本テストはinput/outputをメモリバッファへ差し替え。USIには裁定の外部出力がないため、USIの台本テストは出力照合に加えてrun終了後のEngine状態（GameStatus）の検査で裁定一致を確認する）。
- 前提: 全D6テストの実装方針。
- 操作と期待観測: テスト実装（フェーズ3）への指示として固定する: (1) 期待値の第一観測点はstdout行、(2) 内部検査は「run後のGameStatus」だけを許す、(3) pending規則・置換表・探索ID・force状態などは後続コマンドへの応答差で間接観測する、(4) privateフィールドの直叩き（棚卸しでIMPL分類された手法）を再導入しない。
- 境界・不正: なし（メタ契約）。
- 性質: テストの実装カップリング排除（spec-first-tests.mdの原則の本領域への具体化）。

---

## 4. CLI規則引数（D6-CLI）

### D6-CLI-01 `--protocol`と`--rules`の明示必須
- 典拠: PL「モジュールと名称」・完了条件（単一バイナリが`--protocol usi|cecp`と`--rules`の明示指定を必須とし、未指定時に起動を拒否する。既定値と自動判別は設けない）。EC（明示指定を維持）。
- 前提: minaseバイナリの起動。
- 操作と期待観測: `minase`（引数なし）・`minase --protocol usi`（--rulesなし）・`minase --rules R1`（--protocolなし）→ いずれも非0終了しエンジンとして起動しない。`minase --protocol usi --rules R1` → 起動して`usi`に応答する。
- 境界・不正: `--protocol`の値は`usi`と`cecp`の2値のみ（他値は拒否）。
- 性質: 既定値・フォールバック不在。

### D6-CLI-02 `--rules`の値文法とRuleSetオプションの同一性
- 典拠: PL「規則オプション」（同じ値文法を`--rules`起動フラグにも適用し、解析はコア層の共通関数（`parse_rule_set`）が担う。`engine-default`と`Rules::engine_default()`の一致はコア層の単体テストが保証）。R33第5項（engine-defaultはR1へ展開、大文字小文字非区別、併記拒否）。
- 前提: minase起動。
- 操作と期待観測: `--rules engine-default`で起動 → `usi`応答のRuleSet default値が`R1`。`--rules r1,l1` → default値`L1,R1`（正準化）。wireオプションで受理される値と`--rules`で受理される値の集合が一致することを、代表値（正当・不正各数件）の突き合わせで固定する。
- 境界・不正: `--rules XX9`・`--rules L1,E1`（R欠如）・`--rules R0` → 起動拒否（非0終了）。エンジンは起動時点から規則確定状態でなければならないため、不正値での起動成功はあり得ない。
- 性質: 値文法の単一定義（wireとCLIで同一の解析関数）。

### D6-CLI-03 `lishogi`プリセットの展開
- 典拠: R33第6項（`L1＋L2＋P3＋R1＋E1＋E3`の組合せを規則セット名lishogiとして受理）。PL（プリセット表と`parse_rule_set`、リプレイ照合テストが`parse_rule_set("lishogi")`の解決を経由し、名前が指す組合せと検証済み組合せの一致をテストが保証）。
- 前提: `--rules lishogi`で起動。
- 操作と期待観測: `usi`応答のRuleSet default値が`L1,L2,P3,R1,E1,E3`。`state`のrules欄も同値。プリセット名自体はいかなる出力にも現れない。
- 境界・不正: `--rules lishogi,P1` → 起動拒否（専用エラー。PLがこの併記への専用エラーを明記）。
- 性質: 名前→組合せの写像はコード内の契約であり、リプレイ照合（統合テスト）が実裁定との一致で裏づける。

### D6-CLI-04 大文字小文字非区別と併記拒否
- 典拠: R33第5項（規則セット名の照合は大文字小文字を区別せず、規則コードまたは他の規則セット名との併記は認めない）。PL「規則オプション」。
- 前提: minase起動。
- 操作と期待観測: `--rules LISHOGI`・`--rules Engine-Default` → 受理（小文字正準へ照合）。`--rules lishogi,engine-default`・`--rules engine-default,R1` → 拒否。
- 境界・不正: `standard`は受理しない（PL 2026-08-11: R0非実装の現状でstandardという名前は受理しない）。
- 性質: 照合の正準は小文字（PL: 名前はlishogi、大文字小文字非区別・正準は小文字）。

### D6-CLI-05 バイナリ間の同一契約と共通化の注記
- 典拠: PL（解析はコア層の共通関数が担う）。docs/sprt.md（match_runnerの`commit:`specは起動引数`--protocol usi --rules <マッチ規則>`を自動付与。minase本体は`--protocol`と`--rules`が必須。`usi_random`は同一ビルドの校正用エンジン）。R33第5・6項（「本規則を実装するプログラム（Minase）」として名前受理を規定しており、バイナリを限定しない）。
- 前提: minase・usi_random・match_runner・random_playの各バイナリ。
- 操作と期待観測: 4バイナリの規則引数が同じ値集合を受理・拒否することを、代表値（`engine-default`、`lishogi`、明示コード列、`lishogi,P1`拒否、R欠如拒否）で突き合わせる。共通解析関数`parse_rule_set`の単体テスト1系列と、各バイナリの受け口が同関数を経由することのwire確認で構成し、4重の全数重複テストは書かない（共通化の検討: 値文法のテストは共通関数側へ集約し、バイナリ側は接続確認1件ずつに留めることを推奨）。
- 境界・不正: usi_randomとrandom_playの規則引数の存在・形式は規範文書に明文がない → SPEC_UNCLEAR-09。テスト化するなら実装契約と明示する。
- 性質: 規則文法の定義は1箇所（コア層）であり、バイナリはすべてそれを参照する。

### D6-CLI-06 usi_randomの`Seed`オプションと決定性
- 典拠: docs/sprt.md（`usi_random`は合法手から一様ランダムに着手する校正用エンジン、真のelo差0の煙試験用。同じ`--seed`と引数で出力・度数・LLR・判定・停止ペア番号が完全再現）。spec-first-tests.md引き継ぎ資産5（決定的シード群と決定性契約、`usi_random`のシード42導出）。
- 前提: usi_randomバイナリ。
- 操作と期待観測: 同一シード・同一入力系列（`position`→`go`の反復）で2回実行した出力が完全一致する（決定性）。異なるシードでは着手系列が変わり得る。
- 境界・不正: `Seed`オプションの宣言形式・値0の拒否は規範文書に明文がない → SPEC_UNCLEAR-08。テスト化する場合は「実装契約」とテスト名に明示し、根拠は引き継ぎ資産（意図的挙動の符号化と確認済みの既存テスト）とする。
- 性質: 校正用エンジンの再現性はmatch_runnerの完全再現契約（sprt.md）の前提である。

---

## 5. SPEC_UNCLEAR登録簿（D6分）

規範文書から期待挙動を確定できない事項。テスト化する場合は「実装契約」であることをテスト名またはコメントで明示し、根拠を捏造したassertを書かない。文書補修対象（spec-first-tests.mdフェーズ1）はその旨を記す。

| ID | 事項 | 現状と方針 |
|---|---|---|
| SU-01 | `go depth`の上限256 | spec-first-tests.mdが文書補修対象に列挙済み。補修（所有文書への規範文追記）を待って境界値テスト（256受理・257の扱い）を書く。補修前はテスト化しない。 |
| SU-02 | `movetime`と時計引数の併用時の優先順位 | 同じく文書補修対象。補修後にテスト化。 |
| SU-03 | `go`の同一引数重複（`go depth 1 depth 2`等）の扱い | どの文書にも明文がない。テスト化するなら実装契約（現行挙動の凍結）と明示。 |
| SU-04 | `USI_Hash`の宣言細目（型・default表記・min/max）と不正値（0・負・非数）への応答 | ECは「既定256MB、探索中でないときのリサイズ」だけを規定。宣言行の完全一致・受理範囲のテストは実装契約と明示。 |
| SU-05 | USI `option`宣言列における`USI_Hash`の位置 | PLは「RuleSet、USI_Variantの順」だけを規定し、EC追加のUSI_Hashの位置は未規定。テストは相対順（RuleSet<USI_Variant）と`usiok`終端のみを契約とする。 |
| SU-06 | 探索外での`stop`受信の扱い | ECは探索中の`stop`だけを規定。無視かエラーかは明文なし。実装契約として凍結するか、無応答（未知トークン扱い）を仮定しない。 |
| SU-07 | `gameover`引数（win/lose/draw）の意味論 | PLの`EndGame`はAwaitingStart復帰のみを規定し、引数値の利用有無は明文なし。テストは引数値に依存しない挙動だけを固定する。 |
| SU-08 | usi_randomの`Seed`オプション（値0の拒否、既定シード42） | 規範文書に明文なし。引き継ぎ資産（spec-first-tests.md資産5）として実装契約テスト化を許すが、その旨を明示する。 |
| SU-09 | usi_random・random_playの規則引数（`--rules`）の存在と契約 | sprt.md・PLはminase本体とmatch_runnerしか規定しない。バイナリ間同一契約の全数検証は実装契約。共通解析関数への集約（D6-CLI-05）を推奨。 |
| SU-10 | HaChuの波括弧`Illegal move {理由}`形式への非追随 | HA第9章第1項がHaChu側の仕様乖離として記録。minaseは正典2形式（PL確定）だけを出力し、波括弧形式を出力も受理もしない。minase側の出力はPLに明文があるため通常テストで固定できるが、「HaChu形式を受理しない」ことのテストは実装契約（minaseはGUI側でないため受信場面が原理的にない）。 |
| SU-11 | CECP `sd`の深さ上限 | USIのdepth上限（SU-01）に対応する規定がCECP側にない。SU-01の文書補修時に併せて確定するのが望ましい。 |
| SU-12 | `Error (unsupported variant): ...`と各`info string error: ...`の本文完全形 | PLは形式の先頭部だけを規定（`...`）。完全一致契約は、明文のある行（moves/stateのエラー2種、`checkmate notimplemented`、`pong N`、`Illegal move`2形式、`Error (unknown command): <第1トークン>`、`Error (command not supported): <コマンド>`、`Error (invalid option value): <受信行>`、`tellusererror Illegal position`、RESULT理由表）に限り、他は前方一致とする。 |
| SU-13 | `memory`/`USI_Hash`によるTTリサイズ・クリアの観測手段 | 効果は置換表内部にあり、wire観測手段の明文がない。本領域では受理の外形（エラーなし・後続go正常）のみをテストし、リサイズ・クリアの意味論はD7（search.mdの置換表規範）へ委ねる。 |
| SU-14 | USI原典の既知コマンド`debug`・`register`への挙動 | ULはコマンドの存在を記すが、minaseの応答方針（無視か）はPLの「未知入力は無視」に包含されるか明文でない。テスト化するなら「無視」を実装契約と明示。 |

---

## 集計

- 仕様化した挙動: 85件（USI 38、CECP 34、ENG 7、CLI 6）
- SPEC_UNCLEAR: 14件（うち文書補修対象2件: SU-01、SU-02。SU-11は補修時の併合候補）
