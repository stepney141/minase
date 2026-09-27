# デバッグの手引き

本書は、minaseの不具合を調べる開発者（人間とエージェント）が、症状を再現し、原因の局面と処理まで絞り込み、修正を確かめるまでの標準手順を定める。
各手順が使う道具の出典と、他のエンジンとの比較は[エンジン開発のデバッグ手法の調査](../research/engine-debugging-survey.md)にあり、道具を追加する計画は[デバッグ機能の整備](../plans/debugging-tools.md)にある。
本書は、現在のmasterにある道具だけを記す。

## 原則

不具合の調査は、再現、絞り込み、確認の3段階で進める。
再現できない不具合は原因を確かめられないので、最初に再現の手段を確保する。
絞り込みでは、同じ計算を2通りに行って最初に食い違う箇所を探す。
minaseの道具の多くは、この照合の形をとる（差分更新と全再計算、最適化した実装と素朴な参照実装、2つのコミットのbench出力など）。
修正の後は、修正を外すと失敗する回帰テストを残す（[教訓](../lessons/confirm-regression-test-fails-without-fix.md)）。

調査を始める前に、[教訓の索引](../lessons/README.md)に目を通す。
過去に同じ症状で詰まった記録があれば、その「以後の規則」から始める。

## 再現の手段

再現の手段は、不具合が現れた場面ごとに異なる。

**局面の再現**：USIの`position sfen`は、盤面と手番の2欄に先獅子の升と手数を加えた4欄、または成り権の保留を加えた5欄の拡張SFENだけを受理し、2欄SFENを拒否する。
先獅子の状態がない局面は、2欄SFENの末尾に`- 1`を付けて4欄にする（例：`position sfen <盤面> b - 1`）。
着手列が分かっている場合は、`position startpos moves ...`または`position sfen ... moves ...`で局面を作る。
`bin/perft`の`--sfen`も、2欄SFENに加えて同じ拡張SFENを受理する。

**ランダム対局**：`random_play`は、ランダムな合法手で対局を進め、毎手`Position::validate`で局面の不変条件（ビットボードと盤配列の一致、Zobristキーと全再計算の一致など）を検査する。
失敗すると、規則、シード、局番号、手数、問題の手、直前の局面のSFEN、および全手順を標準エラーへ出して終了コード1で終わる。
失敗した局は、同じ`--seed`と`--game`で単独に再現できる。

```console
cargo run --release --bin random_play -- --rules engine-default --seed 1 --games 10000
cargo run --release --bin random_play -- --rules engine-default --seed 1 --game 4213 --verify-all --verbose
```

`--verify-all`は、毎手、全合法手を複製した対局に適用して検査する。
規則による差を疑うときは、`--rules`に該当するコード列を与える。

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
2. `random_play`を`--verify-all`付きで多数局走らせ、失敗する局と手数を見つける。
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
`scripts/bench_compare.py`は、作業ツリーのbenchを参照コミットのbenchと全行で照合し、不一致があれば終了コード1で報告する。

```console
scripts/bench_compare.py --reference master --baseline master --parent master --depth 5
```

ノード数が変わった場合は、変更に探索木を変える箇所が含まれている。
探索木を変える変更の採否は、[sprt.md](sprt.md)の棋力測定で決める。

**結果の誤りの調査**：問題の局面を`position`で与え、`go depth N`の`info`行で深さごとの評価値と読み筋を追う。
評価値が特定の深さで急変するなら、その深さで初めて探索される変化に原因がある。
読み筋の手を1手ずつ`position ... moves`へ足して同じ手順を繰り返すと、誤りのある局面まで下りられる。

**探索内部の計測**：静止探索の割合、置換表の一致率、特定の改良の発動回数など、探索の内部で起きた回数は、計測用のカウンタを一時的に足したバイナリで数える。
カウンタはワーカーごとの局所変数に置いて探索の後に合算し、速度は計測コードなしのバイナリで測り直す（[教訓](../lessons/shared-probe-counter-distorts-threads.md)）。
計測コードは、`docs/measurements/<測定名>/diagnostic.patch`としてパッチの形で記録に残し、masterへは入れない。
記録の例は[段階8の発動率の診断](../measurements/strength-stage8-activation-diag.md)にある。
改良を採否測定に出す前の発動率の確認は、[教訓](../lessons/measure-feature-activation-before-sprt.md)に従う。

**置換表**：置換表の手は、使う前に`is_legal_move`で合法性を確かめているので、鍵の衝突で不正な手が指されることはない。
置換表が効いていない疑いがあるときは、置換表を照会するノードの割合を先に数える（[教訓](../lessons/qsearch-dag-without-tt.md)）。

### 評価関数

評価関数の不具合は、差分更新の誤り、対称性の破れ、および重みの誤りに分かれる。

差分更新の誤りは、探索が使う差分更新の値と`pst::evaluate`の全計算の値を比べて見つける。
`src/eval/pst/tests.rs`は、この一致と、段反転と陣営交換による評価の不変性を検査している。
minaseの評価で成り立つ対称性は「段反転と陣営交換」だけであり、左右反転では評価は一致しない（学習PSTは筋ごとに異なる重みを持つ）。

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

1. 修正を一時的に外すと失敗する回帰テストを追加する（[教訓](../lessons/confirm-regression-test-fails-without-fix.md)）。規則に関わるテストは、期待値をRULES.mdの条文から導き、条文の番号をテストに書く。
2. 探索木を変えないはずの修正は、`bench`の局面ごとのノード数が変わらないことを`scripts/bench_compare.py`で確かめる。
3. 探索木を変える修正のうち、棋力に影響し得るものは、[sprt.md](sprt.md)の手順で採否を決める。
