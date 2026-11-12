# 起動引数と診断ツールをUSIへ移す前後の出力の照合

## 目的

[起動引数と診断ツールをUSIへ移す計画](../../plans/archived/cli-to-usi.md)の「検証」節の項目を確かめ、移した機能が移す前と同じ結果を返し、新しい契約どおりに拒否と失敗を知らせるかを判定する。
探索の内容を変えない計画なので、対局による棋力の測定は行っていない。

## コマンドライン

基準は移す前のサブコマンドと起動引数で実行し、変更の後は標準入力からUSIのコマンドを渡して実行した。
変更の後の出力から`info string`の接頭辞と終端の行`info string end`を除き、経過時間と毎秒ノード数の欄以外を全行で突き合わせた。
以下の`<基準>`は基準のコミットの実行ファイル、`<変更後>`は各フェーズのコミットの実行ファイルである。

benchとperftは次の形で実行した。
perftの局面は、初期局面のほかに、保存した対局から採った、先獅子の状態を持つ局面2つと、成りを保留している駒を持つ局面1つを使った。

```console
<基準> dev bench --depth <3|6>
printf 'bench depth <3|6>\nquit\n' | <変更後> --protocol usi

<基準> dev perft <深さ> --rules <規則> --sfen "<拡張SFEN>" [--divide]
printf 'setoption name RuleSet value <規則>\nposition sfen <拡張SFEN>\nperft <深さ> [divide]\nquit\n' | <変更後> --protocol usi
```

benchの速度は、性能コア（論理CPU 3）に固定して、基準と変更の後を交互に5回ずつ測った。

```console
taskset -c 3 <基準> dev bench --depth 9 --repetitions 3
printf 'bench depth 9 repetitions 3\nquit\n' | taskset -c 3 <変更後> --protocol usi
```

ランダム対局は、3つの規則セット（`L0,P0,R1,E0`、`lishogi`、`L1,L3,P0,P5,P6,R2,E1,E2`）のそれぞれで次の形で実行した。

```console
<基準> dev random-play --rules <規則> --seed 7 --games 300
printf 'setoption name RuleSet value <規則>\nrandom-play seed 7 games 300\nquit\n' | <変更後> --protocol usi
```

MCTSとネットの出力は、ONNX Runtimeの環境（`tools/train/zero/env.sh`）を読み込み、実行プロバイダをCPUにして次の形で実行した。

```console
printf 'usi\nsetoption name NnBackend value cpu\nsetoption name NnBatchSize value 16\nisready\nusinewgame\nposition startpos\ngo nodes 64\n' \
  | <基準> --protocol usi --rules engine-default --search mcts --nn-model <ネット>
printf 'usi\nsetoption name RuleSet value JCSA\nsetoption name NnModel value <ネット>\nsetoption name NnBackend value cpu\nsetoption name NnBatchSize value 16\nisready\nusinewgame\nposition startpos\ngo nodes 64\n' \
  | <変更後> --protocol usi --search mcts

<基準> dev nn-eval --nn-model <ネット> --backend cpu --batch <1|7|256> --out <出力>
printf 'usi\nsetoption name RuleSet value JCSA\nsetoption name NnModel value <ネット>\nsetoption name NnBackend value cpu\nsetoption name NnBatchSize value <1|7|256>\nisready\nnn-eval out <出力>\nquit\n' \
  | <変更後> --protocol usi --search mcts
```

対局ハーネスの確認は次の形で行った。
変更の前後をまたぐ対局では、基準の側に起動引数を足した。
ネットのチェックサムの確認では、MCTSとαβ探索を対局させ、ネットのファイルを測定の途中または再開の前に、重みが同じで、説明の欄だけを書き換えたためにバイト列が異なるファイルへ差し替えた。

```console
<変更後> match run --run-dir <出力> --seed 1 --candidate commit:21daee9b --baseline commit:21daee9b \
  --rules JCSA --each depth=3 --concurrency 4 elo --pairs 4
<変更後> match run --run-dir <出力> --seed 2 --candidate commit:21daee9b --baseline commit:e2fa8cd6 \
  --baseline-args '--rules L0,P0,R1,E0' --rules L0,P0,R1,E0 --each depth=3 --concurrency 4 elo --pairs 4
<変更後> match run --run-dir <出力> --seed 11 --rules JCSA --max-ply 80 \
  --candidate "<変更後> --protocol usi --search mcts" --candidate-option NnModel=<ネット> \
  --candidate-option NnBackend=cpu --candidate-option NnBatchSize=8 \
  --baseline "<変更後> --protocol usi" --each nodes=2 --baseline-limit depth=1 \
  --candidate-hash 16 --baseline-hash 16 --concurrency 2 elo --pairs 12
```

CECP（Chess Engine Communication Protocol）は、XBoard 4.9.1を仮想のXディスプレイ（Xvfb）の上で起動して確かめた。

```console
xboard -variant chu -fcp "<変更後> --protocol cecp --io-log <記録>" -firstOptions "RuleSet=JCSA" \
  -scp "<変更後> --protocol cecp" -secondOptions "RuleSet=JCSA" -tc 0:20 -mg 1 -autoCallFlag true -testLegality false
```

## エンジン

基準は、計画のブランチを切った時点のmasterのコミット`e2fa8cd6`である。
変更の後は、計画の5つのフェーズのコミットであり、各フェーズの確認はそのフェーズのコミットの実行ファイルで行った。
フェーズ1（規則セット名の改名）が`a15078af`、フェーズ2（起動引数`--rules`の削除）が`21daee9b`、フェーズ3（起動引数`--nn-model`の削除とネットのチェックサムの照合）が`c4ae030a`、フェーズ4（`perft`と`bench`の移行）が`d336e904`、フェーズ5（`nn-eval`、`nn-fixture`、`random-play`の移行）が`b1df99e1`である。
基準の実行ファイルは、リポジトリの外に作ったworktreeでビルドした。
どちらも通常のrelease設定でビルドした。
ネットは、強化学習の2回目の実行の初期ネット（20ブロック256チャネル、ランダム初期化、SHA-256は`7a9f3ec7…d43d76`）である。
差し替えに使ったファイルは、同じネットの説明の欄だけを書き換えたもので、SHA-256は`5683fc15…fd61b7`である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア）とGeForce RTX 3060 Tiであり、2026年11月12日に測った。
速度の測定のときは、対局の測定が動いていないことを確かめた。負荷平均は約5だった。

## 結果

出力の照合の結果は次のとおりである。

| 項目 | 結果 |
|---|---|
| bench（深さ3と6、15局面） | フェーズ1とフェーズ4で、局面ごとの深さ、ノード数、最善手、評価値と、合計の行が全行一致した。 |
| perft（初期局面の深さ3、深さ0、深さ2のdivide） | 全行一致した。 |
| perft（先獅子の状態を持つ2局面および成りを保留している1局面における、深さ3と深さ2のdivide） | 全行一致した。深さ3の件数は52,516、852,342、972,266である。 |
| ランダム対局（3つの規則セット、各300局） | 局ごとの行と集計が全行一致した。`verify-all`と`verbose`を付けた1局も全行一致した。 |
| 固定局面集（`nn-fixture`） | コミット済みの`crates/minase/tests/fixtures/nn-positions.jsonl`とバイト単位で一致した。 |
| MCTSの`go nodes 64`と`go nodes 100` | 最善手、評価値、読み筋が一致した。 |
| `nn-eval`（CPU、バッチ1、7、256） | 出力のJSONがバイト単位で一致した。 |
| `nn-eval`（TensorRT、バッチ7） | PyTorchの参照出力との照合が、[照合の手引き](../../guides/alphazero-network-check.md)の許容誤差で通った。 |

benchの各回の中央値の毎秒ノード数は次のとおりである。総ノード数は全回で136,697と一致した。

| 版 | 1回目 | 2回目 | 3回目 | 4回目 | 5回目 | 中央値 |
|---|---:|---:|---:|---:|---:|---:|
| 基準 | 1,005,236 | 1,005,556 | 1,007,841 | 998,026 | 1,005,782 | 1,005,556 |
| 変更の後 | 1,007,174 | 1,002,985 | 1,005,310 | 1,009,278 | 1,008,751 | 1,007,174 |

変更の後の中央値は基準の1.0016倍であり、基準の5回の幅（998,026〜1,007,841）に収まる。

規則とネットの指定の契約は、実行ファイルで次のとおり確かめた。

- 規則を設定しない`isready`は、エラーの行を出して終了コード1で終わった。CECPでは、`option RuleSet=`の前の`new`がエラーの行を出して終了コード1で終わった。
- 規則を設定する前の`position`、`go`、`moves`、`state`、`d`、`eval`、`tt`、`perft`、`random-play`は、エラーの1行で拒否され、セッションが続いた。
- 最初の`RuleSet`は現局の規則になり、2回目の`RuleSet`は`usinewgame`の後の対局から反映された。`engine-default`は未知の規則コードとして拒否された。
- XBoard 4.9.1は、`-firstOptions`のオプションを、握手（接続の直後にエンジンが機能とオプションを宣言するやり取り）の`accepted option`の直後に送り、`new`はその後に送った。`-firstOptions`を指定した対局は、着手の行を393行やり取りして時間切れで終わり、`Illegal move`は0件だった。指定しない場合は、最初の`new`でエンジンが終了した。
- `NnModel`を設定しない`isready`は、エラーの行を出して終了コード1で終わった。
- `NnModel`を別のファイルへ変えると、`isready`の前の`go`、`eval`、`nn-eval`が拒否され、`isready`の後のチェックサムの行が変わり、同じ局面の`go nodes 64`が起動直後と同じ結果を返した。`NnModel`を変えずに同じ探索を繰り返した場合は、持ち越した探索木のために結果が変わった。
- 引数の誤りのある診断のコマンドは、エラーの1行で拒否され、終端の行が出ず、セッションが続いた。
- `go infinite`の探索中に送った`perft 1`は、`stop`の後の`bestmove`に続けて実行された。
- ランダム対局の5手目の着手を一時的に不正な着手へ書き換えると、`info string error: random-play invariant failure:`の行と、規則、シード、局番号、手数、問題の着手、直前の局面のSFEN、全手順の行が出て、終了コード1で終わった。書き換えは確認の後に戻した。
- 書き込めないパスを指定した`nn-eval`は、エラーの行を出して終了コード1で終わった。

対局ハーネスでは次のとおりだった。

- 変更の後のコミットどうしの4ペア（1ペアは、同じ開始局面で先後を入れ替えた2局）と、変更の後のコミットと`--baseline-args`を付けた変更の前のコミットとの4ペアが、反則と異常0件で完走した。後者では、各ペアの2局の手数と結果が一致した。
- MCTSのエンジンの`manifest.json`の欄`network_sha256`にネットのSHA-256が記録され、αβ探索のエンジンの欄は`null`になった。
- 3ペアを保存した時点で止めた測定は、ネットのファイルを変えずに再開すると成功し、ペア4〜12だけが実行された。
- 同じパスのネットのファイルを差し替えてから再開すると、`network_sha256 mismatch`のエラーで拒否され、終了コード1で終わった。
- 測定の途中でファイルを差し替えると、次の対局の開始時に、不一致が起きた側（候補）、記録した値、および読んだ値を含むエラーを出して終了コード1で止まった。同時対局数2のこの測定では、保存済みの3ペアが残り、反則負けの記録はなかった。ファイルを元に戻して再開すると、ペア4〜12だけが実行された。

ペアの2局目の開始での不一致、記録が`null`のエンジンがチェックサムの行を出した場合とその逆の場合、チェックサムの行の形式の誤り、および対局ハーネスが対局の前にエンジンを1回ずつ起動して確かめる処理でも`USI_Hash`を送ることは、偽のエンジンを使う自動テストで確かめた。

## 結論

移した機能は移す前と同じ結果を返し、規則とネットの指定、診断の拒否と失敗、およびネットのチェックサムの照合は設計書の契約どおりに動いたので、計画の「検証」節の項目はすべて満たされた。
