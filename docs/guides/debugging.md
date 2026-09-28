# デバッグの手引き

本書は、minaseの不具合を調べる開発者（人間とエージェント）が、症状を再現し、原因の局面と処理まで絞り込み、修正を確かめるまでの標準手順を定める。
各手順が使う道具の出典と、他のエンジンとの比較は[エンジン開発のデバッグ手法の調査](../research/engine-debugging-survey.md)にあり、道具を追加する計画は[デバッグ機能の整備](../plans/debugging-tools.md)にある。
本書は、リポジトリに実装済みの道具だけを記す。

## 原則

不具合の調査は、再現、絞り込み、確認の3段階で進める。
再現できない不具合は原因を確かめられないので、最初に再現の手段を確保する。
絞り込みでは、同じ計算を2通りに行って最初に食い違う箇所を探す。
minaseの道具の多くは、この照合の形をとる（差分更新と全再計算、最適化した実装と素朴な参照実装、2つのコミットのbench出力など）。
修正の後は、修正を外すと失敗する回帰テストを残す（[教訓](../lessons/confirm-regression-test-fails-without-fix.md)）。

調査を始める前に、[教訓の索引](../lessons/README.md)に目を通す。
過去に同じ症状で詰まった記録があれば、その「以後の規則」から始める。

## 調査用の道具

minaseは、通常の対局と測定には使わない調査用の道具を、USIの独自コマンド、起動引数、およびCargoフィーチャとして持つ。
各道具の契約は[デバッグ機能の整備](../plans/debugging-tools.md)が定める。

| 道具 | 使い方 | 用途 |
|---|---|---|
| `d` | USIで`position`の後に送る | 駒を漢字で書いた盤面、拡張SFEN、手番、手数、対局の状態、先獅子と成り権の保留、2つのZobristキーを表示する。表示の前に局面の不変条件を検査する。 |
| `eval` | USIで`position`の後に送る | 手番側から見た静的評価と、升ごとの寄与を12行12列の表で表示する。 |
| `tt` | USIで`go`の後に送る | 置換表に残る現局面と全子局面の項目（深さ、境界の種類、評価値、最善手）を表示する。 |
| `--io-log <パス>` | `minase`の起動引数 | 受け取った行と送った行を、起動からの経過ミリ秒付きでファイルへ記録する。 |
| フィーチャ`invariants` | `cargo test`では常に有効。他のビルドでは`--features invariants` | 着手の実行と取消しのたびに局面を全検査し、探索中の評価のたびに差分更新の値を全計算と照合する。失敗すると診断を出してパニックする。 |
| フィーチャ`search-stats` | `--features search-stats`で`bench`をビルドする | ノードの種類、置換表の照会と打ち切り、βカット、最善手の順位を数えて`bench`が出力する。 |

`eval`と`tt`の出力は、各行が`info string`で始まり、`info string end`の行で終わる。
`d`の出力は、StockfishとYaneuraOuに合わせて接頭辞のない素の行であり、`rights-zobrist`の行で終わる。
盤面の各升は、後手の駒に`^`、成駒に`+`を付けた漢字1文字で書き、空升を`・`とする。
成駒は成った後の駒種の漢字で書くので、金将の成駒は`+飛`、歩兵の成駒は`+金`と表示される。
3つのコマンドとも、失敗した場合は`info string error:`で始まる1行だけが返る。
探索中に送ったコマンドは、探索が終わってから処理される。

## 再現の手段

再現の手段は、不具合が現れた場面ごとに異なる。

**局面の再現**：USIの`position sfen`は、盤面と手番の2欄に先獅子の升と手数を加えた4欄、または成り権の保留を加えた5欄の拡張SFENだけを受理し、2欄SFENを拒否する。
先獅子の状態がない局面は、2欄SFENの末尾に`- 1`を付けて4欄にする（例：`position sfen <盤面> b - 1`）。
着手列が分かっている場合は、`position startpos moves ...`または`position sfen ... moves ...`で局面を作る。
着手列で作った局面を他の道具へ渡すときは、`d`が表示する拡張SFENを写す。
`bin/perft`の`--sfen`も、2欄SFENに加えて同じ拡張SFENを受理する。

**ランダム対局**：`random_play`は、ランダムな合法手で対局を進め、毎手`Position::validate`で局面の不変条件（ビットボードと盤配列の一致、Zobristキーと全再計算の一致など）を検査する。
失敗すると、規則、シード、局番号、手数、問題の手、直前の局面のSFEN、および全手順を標準エラーへ出して終了コード1で終わる。
失敗した局は、同じ`--seed`と`--game`で単独に再現できる。

```console
cargo run --release --bin random_play -- --rules engine-default --seed 1 --games 10000
cargo run --release --bin random_play -- --rules engine-default --seed 1 --game 4213 --verify-all --verbose
```

`--verify-all`を付けると、ランダムに選んだ1手だけでなく、各手番の全合法手を1手ずつ対局の複製へ適用し、生成した合法手が着手として拒否されないことを確かめる。
規則による差を疑うときは、`--rules`に該当するコード列を与える。
`--features invariants`を付けてビルドすると、着手の実行と取消しの内部でも毎回局面を検査する。
この場合は、不変条件が壊れた最初の着手の直後に、理由、手、差分更新と全再計算の両方のZobristキー、および升ごとの駒コードの生データを出して止まる。

```console
cargo run --release --features invariants --bin random_play -- --rules engine-default --seed 1 --games 1000
```

**自己対局の測定**：`match_runner`の実行ディレクトリの`pairs/NNN.json`は、各局の開始局面の着手列と全着手を保存している。
両エンジンが`Threads=1`で、思考制限が`depth`または`nodes`の対局は完全に再現できる（[sprt.md](sprt.md)の「ペア対局と再現性」）。
時間制御の対局は着手時間の実測に依存するので、同じ対局は再現できない。
その場合は、保存された着手列を`position ... moves ...`へ与えて問題の局面を作り、局面単位で調べる。

**lishogiの対局**：lishogiの対局は、棋譜の着手列を`position ... moves ...`へ与えて再現する。
途中の局面から始まる対局の開始局面は、lishogi APIの`initialSfen`欄にある（[lishogi Bot接続](../plans/lishogi-bot.md)）。
規則の裁定がlishogiと食い違う対局は、`scripts/fetch_lishogi_replays.py`の対局一覧へ加えて固定データを作り直すと、`tests/lishogi_replay.rs`が合法性と終局の裁定を照合する回帰テストになる。

**並列探索**：`Threads`が2以上の探索は、共有置換表へのアクセス順がスケジューリングに依存するので、固定深さでも再現しない。
まず`Threads=1`で同じ症状が出るかを確かめ、出れば単一スレッドの問題として調べる。
出なければ並列の問題であり、修正の確認には回帰テストを10回以上連続で通す（[教訓](../lessons/repeat-concurrency-regression-tests.md)）。

## 症状ごとの絞り込み

### 合法手生成と着手の実行

合法手の過不足、不正な局面への遷移、Zobristキーの不一致は、次の順で調べる。

1. RULES.mdの該当条文を読み、期待する挙動を条文から決める。実装の出力を正としない。
2. `random_play`を`--verify-all`付きで多数局走らせ、失敗する局と手数を見つける。Zobristキーや盤面の不整合が疑われるときは、`--features invariants`付きでビルドして、壊れた最初の着手で止める。
3. 失敗した局面を`bin/perft`へ与え、`--divide`で根の手ごとの件数を出す。
4. 条文から数えた期待値と食い違う手を1手進め、深さを1減らして`--divide`を繰り返し、誤りのある局面と手まで絞り込む。

```console
cargo run --release --bin perft -- 3 --rules engine-default --sfen "<拡張SFEN>" --divide
```

`--rules`は必須であり、不具合が出た対局と同じ規則セットを与える。
`--sfen`には、USIの独自コマンド`d`が表示する拡張SFENをそのまま与えられ、先獅子の状態と成り権の保留も復元される。
perftの数値は、テストの正しさの基準にしない（[movegen.md](../plans/movegen.md)の9節）。
perftはdivideによる絞り込みと速度の計測に使う。

HaChuの規則との食い違いは、`scripts/hachu_replay.py`でHaChuの自己対局の棋譜をminaseへ再生して照合する。

### 探索

探索の不具合は、結果の誤り（明らかに悪い手、詰みの見落とし、評価値の異常）と、挙動の意図しない変化に分かれる。

**挙動の変化の検出**：探索木を変えないはずの変更（高速化、リファクタリング）は、`bench`の局面ごとのノード数、最善手、評価値が変更前と完全に一致することで確かめる。
`bench`は、`Threads=1`の固定深さの探索であり、同じバイナリでは実行のたびに同じノード数を返す。
`scripts/bench_compare.py`は、作業ツリーのbenchを参照コミット（`--reference`）のbenchと全行で照合し、不一致があれば終了コード1で報告する。
`--baseline`と`--parent`は速度の比を求める比較先であり、挙動の一致だけを確かめるときは3つとも同じ比較先のコミットを与える。

```console
scripts/bench_compare.py --reference master --baseline master --parent master --depth 5
```

ノード数が変わった場合は、変更に探索木を変える箇所が含まれている。
探索木を変える変更の採否は、[sprt.md](sprt.md)の棋力測定で決める。

**結果の誤りの調査**：問題の局面を`position`で与え、`go depth N`の`info`行で深さごとの評価値と読み筋を追う。
評価値が特定の深さで急変するなら、その深さで初めて探索される変化に原因がある。
読み筋の手を1手ずつ`position ... moves`へ足して同じ手順を繰り返すと、誤りのある局面まで下りられる。

**置換表による木の閲覧**：ある手が選ばれなかった理由は、探索の後に置換表を辿って調べる。
置換表は探索の後もエンジンが保持しているので、同じプロセスで次の手順を繰り返す。

1. `usinewgame`と`position`で局面を与えて、`go depth N`で探索する。`Threads`は既定の1のままにする。2以上でも閲覧はできるが、表の内容がワーカーの実行順に依存し、同じ手順を繰り返しても同じ表にならない。
2. `tt`を送り、現局面の項目と、合法手ごとの子局面の項目（深さ、境界の種類、子局面の手番側から見た評価値、最善手）を読む。選ばれなかった手の子局面の値と深さを、選ばれた手のものと比べる。
3. 調べたい手を`position ... moves`の末尾に足して1手進め、再び`tt`を送る。子局面の最善手（反駁手）を辿ると、その手の評価を決めた変化まで下りられる。

`position`を送り直しても置換表は消えないが、`usinewgame`と規則の変更は置換表を空にし、別の`go`は内容を上書きするので、閲覧の途中ではこれらを送らない。
表示される値は照会の時点で表に残っている項目であり、直前の探索が最後に保存した値とは限らない（深い項目は浅い探索の結果で上書きされない）。
項目の最善手は評価値と別の語に保存されるので、評価値と同じ探索に由来するとは限らない。

**探索内部の計測**：静止探索の割合、置換表の一致率、βカットの割合など、探索の健全性を示す回数は、`--features search-stats`でビルドした`bench`で数える。
統計は局面ごとの行とサマリの後に`stats:`の行で出力され、回数と、分母が0でない比が並ぶ。
測定用の`target/release`を上書きしないよう、別のターゲットディレクトリでビルドする。

```console
CARGO_TARGET_DIR=target/stats cargo build --release --features search-stats --bin bench
target/stats/release/bench --depth 5
```

変更の前後で統計を比べるときは、同じ深さと`Threads=1`で両方の`bench`を実行し、回数の比だけを読む。
速度は計測コードなしのバイナリで測り直す（[教訓](../lessons/shared-probe-counter-distorts-threads.md)）。
特定の改良の発動回数のように固定項目にない回数は、`src/search/stats.rs`の項目の宣言へ項目を1つ足し、該当箇所に加算を書いたパッチで数える。
合算と出力は宣言から自動で生成される。
パッチは`docs/measurements/<測定名>/diagnostic.patch`として記録に残し、masterへは入れない。
記録の例は[段階8の発動率の診断](../measurements/strength-stage8-activation-diag.md)にある。
改良を採否測定に出す前の発動率の確認は、[教訓](../lessons/measure-feature-activation-before-sprt.md)に従う。

**置換表**：置換表の手は、使う前に`is_legal_move`で合法性を確かめているので、鍵の衝突で不正な手が指されることはない。
置換表が効いていない疑いがあるときは、置換表を照会するノードの割合を先に数える（[教訓](../lessons/qsearch-dag-without-tt.md)）。

### 評価関数

評価関数の不具合は、差分更新の誤り、対称性の破れ、および重みの誤りに分かれる。

差分更新の誤りは、探索が使う差分更新の値と`pst::evaluate`の全計算の値を比べて見つける。
フィーチャ`invariants`は探索中の評価のたびにこの照合を行うので、`cargo test`と、`--features invariants`でビルドした`bench`が照合の失敗を報告する。
失敗の診断は、局面のZobristキー、手数、差分更新と全計算の両方の中間値を含む。

対称性の破れは、段反転と陣営交換を施した局面で評価が一致するかで調べる。
`src/eval/pst/tests.rs`は、手で置いた局面と、固定シードのランダム対局から採った局面について、この不変性を検査している。
minaseの評価で成り立つ対称性は「段反転と陣営交換」だけであり、左右反転では評価は一致しない（学習PSTは筋ごとに異なる重みを持つ）。

ある局面の評価値が不自然なときは、`eval`で升ごとの寄与を読み、寄与の大きい駒から原因を探す。
表示される評価値は`pst::evaluate`の戻り値と一致する。
升ごとの寄与は0.1センチポーン単位に丸めて表示されるので、寄与の和は評価値と丸めの分だけ異なり得る。

重みの誤りは、`pst_probe`の出力をPythonの参照評価と照合して見つける。
手順は[PSTの学習手順](pst-training.md)にある。
学習した評価関数の駒価値の歪みは、検証損失からは分からないので、[教訓](../lessons/validation-loss-hides-material-distortion.md)の点検を行う。

### プロトコルと外部接続

プロトコルの不具合は、エンジンを手で起動してコマンドを送り、応答を読んで調べる。

```console
printf 'usi\nisready\nposition startpos\nmoves\nstate\ngo depth 4\n' \
  | cargo run --release --bin minase -- --protocol usi --rules engine-default
```

探索中に`quit`を受けると、minaseは探索の結果を捨てて終了する。
探索の結果を読むときは、上の例のように`quit`を送らずに入力を閉じるか、`bestmove`を受け取ってから`quit`を送る。

独自コマンド`moves`は現局面の全合法手を返し、`state`は規則、局面、対局の状態を返す（[ブラウザGUI向けUSI照会](../plans/browser-gui.md)）。
エラーは`info string error:`で始まる行で返り、探索の停止理由は`info string stop`の行で返る。
未知のコマンドには`info string error: unknown command <語>`が返る。
ただしUSI原典の`debug`と`register`は応答なしで無視される。

対局ハーネスは子エンジンの標準エラーを端末へそのまま流すが、プロトコルの送受信は保存しない。
GUI、lishogiのBotブリッジ、または対局ハーネスとの間でだけ起きる不具合は、`minase`を`--io-log <パス>`付きで起動させて、エンジン側の送受信を記録する。
ログの各行は`<経過ミリ秒> <向き> <行>`の形であり、向きはエンジンが受け取った行が`>`、送った行が`<`である。
ログのファイルは新規に作られ、同じパスのファイルが既にあると起動はエラーになるので、起動ごとに別のパスを与える。
対局ハーネスは局ごとにエンジンを起動し直すので、起動コマンドに固定のパスを書くと2局目の起動が失敗する。
ハーネスの対局で起きた不具合は、保存された着手列から問題の局面を作り、エンジンを手で起動して`--io-log`付きで再現する。
外部との接続の不具合は、次の教訓の手順で切り分ける。

- 外部エンジンを`cecp:`で接続する前の検証は、[外部エンジンの検証の教訓](../lessons/verify-external-engine-before-cecp-match.md)に従う。
- lishogiのBotブリッジからの起動は、[ブリッジ経由の起動の教訓](../lessons/verify-bridge-launch-path.md)に従って配備と同じ環境で確かめる。
- xboardの時間切れ検出の引数は、[xboardの教訓](../lessons/xboard-autoflag-argtrue.md)に従う。

### 時間切れと時間管理

時間切れは、保存済みの実行ディレクトリから時計を再構成して、手数帯ごとの残り時間と思考時間を数えて調べる（[教訓](../lessons/reconstruct-clock-from-match-records.md)）。
`scripts/clock_profile.py`がこの集計を行う。

```console
scripts/clock_profile.py data/matches/<測定名> --budget quadruple-soft
```

`--budget`は、再構成で使う時間予算の式を`quadruple-soft`、`quadruple-main`、`target`、`total`、`byoyomi-opening`から選ぶ（定義は`scripts/clock_budget_stats.py`）。

対局ハーネスの外（lishogiやGUI）で起きた時間切れは、`--io-log`の記録から`go`を受けた時刻と`bestmove`を送った時刻の差を読み、エンジン側の思考時間を確かめる。
この差が`go`で渡された残り時間に収まっていれば、遅れはエンジンの外（通信やブリッジ）にある。

超長手数の対局でだけ起きる時間切れは、1手あたりの固定費が手数とともに増えていないかを確かめる（[教訓](../lessons/per-move-overhead-grows-with-game-length.md)）。
時間管理を変更したときの確認は、[打ち切り上限の教訓](../lessons/cap-hard-limit-by-remaining-fraction.md)と[総思考時間の教訓](../lessons/compare-total-time-usage-before-sprt.md)に従う。

### パニックと異常終了

パニックは、`RUST_BACKTRACE=1`を付けて再現させ、呼び出し履歴を得る。
releaseプロファイルはデバッグ情報を持たないので、行番号が必要なときは、デバッグ情報付きのビルドを別のターゲットディレクトリに作る。
別のディレクトリにするのは、測定に使う`target/release`のバイナリを上書きしないためである。

```console
CARGO_PROFILE_RELEASE_DEBUG=line-tables-only CARGO_TARGET_DIR=target/debuginfo \
  cargo build --release --bin minase
RUST_BACKTRACE=1 target/debuginfo/release/minase --protocol usi --rules engine-default
```

対局ハーネスは、エンジンのクラッシュと応答タイムアウトを`engine_failures:`の行に理由別に数える（[sprt.md](sprt.md)の「異常時の裁定」）。
0件でない測定は、採否に使う前に当該局の着手列を再現して原因を調べる。

## 修正の確認

修正は、次の3点を確かめてから採用する。

1. 修正を一時的に外すと失敗する回帰テストを追加する（[教訓](../lessons/confirm-regression-test-fails-without-fix.md)）。規則に関わるテストは、期待値をRULES.mdの条文から導き、条文の番号をテストに書く。`cargo test`は整合検査（フィーチャ`invariants`）を常に有効にして走るので、テストの中の着手と探索も全検査を受ける。
2. 探索木を変えないはずの修正は、`bench`の局面ごとのノード数が変わらないことを`scripts/bench_compare.py`で確かめる。
3. 探索木を変える修正のうち、棋力に影響し得るものは、[sprt.md](sprt.md)の手順で採否を決める。
