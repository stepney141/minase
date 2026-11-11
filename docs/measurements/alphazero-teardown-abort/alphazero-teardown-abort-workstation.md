# 終了処理の修正の作業機での確認

## 目的

ortクレートをrc.15へ上げ、ONNX Runtimeの環境をプロセスの終了まで保持する変更（[終了処理での異常終了をなくす設計書](../../plans/archived/alphazero-teardown-abort.md)）について、変更の前後の実行ファイルを比べ、設計書の「検証」の4つの確認が通るかを判定する。

## コマンドライン

`<zero>`は導入スクリプト`setup.sh`で準備した`tools/train/zero`、`<前>`と`<後>`は次節の2つの実行ファイル、`<新形式>`と`<旧形式>`は次節の2つのネットである。
次の1つ目の確認以外のコマンドは、`tools/train/zero/env.sh`を読み込んだ環境で、`XDG_CACHE_HOME`を作業用のディレクトリに設定して実行した。

1つ目の確認は、[原因の調査の記録](../adhoc/zero-teardown-abort-root-cause.md)の`probe.py`で行った。
異常終了の原因は、ONNX RuntimeのCUDAの実行プロバイダが静的な`std::shared_ptr`に持つ演算の登録表が、終了時に解放された後でもう一度手放されることである。`probe.py`は、gdbの下で、この`shared_ptr`（`--static`の番地）と、登録表を手放す関数（`--function`の番地）を調べる。`--expect`は、`<後>`では`not-reached`、`<前>`では`reached`である。`--`の後のコマンドを、次の4つに替えた。

```console
python3 probe.py --zero-dir <zero> --ort-version 1.31.1 --function 0x3cfdc0 --static 0x10edb8c0 \
  --expect not-reached --expect-exit 0 -- <後> <下の4つのいずれか>

dev nn-eval --nn-model <新形式> --backend cuda --batch 7 --out out.json
dev nn-eval --nn-model <新形式> --backend tensorrt --cache-dir trt-cache --batch 7 --out out.json
dev nn-eval --nn-model <旧形式> --backend cuda --batch 7 --out out.json
data zero-selfplay --output-dir <出力先> --nn-model <新形式> --nn-backend tensorrt \
  --generation-commit 645d37c400000000000000000000000000000000 --seed 9100000000 \
  --start-position shuffled-setup --games 8 --target-positions 1000000000 --chunk-games 8 \
  --simulations 32 --playthrough-percent 100 --threads 1 --devices 1 --games-per-thread 8
```

3つ目のコマンドは、入出力の形が合わないネットを拒んで終了コード1で終わる経路なので、`--expect-exit 1`とした。TensorRTの実行は、保存済みのTensorRTのエンジンを読む。

2つ目の確認では、[alphazero-teardown-abort-workstation/](alphazero-teardown-abort-workstation/)の`usi_session.py`で、同じディレクトリの`usi-commands.txt`のコマンドをUSIエンジンへ送った。USIエンジンは、実行プロバイダやバッチの大きさの設定が変わるたびに、推論器を破棄して作り直す。

```console
python3 usi_session.py usi-commands.txt -- \
  <後> --protocol usi --rules engine-default --search mcts --nn-model <新形式>
```

3つ目の確認では、1つ目の確認の最初の2つの`dev nn-eval`と、`--backend cpu`の`dev nn-eval`を、gdbを使わずに`<前>`と`<後>`で実行し、出力のJSONを`cmp`で比べた。

4つ目の確認では、上の`data zero-selfplay`の`--games`、`--chunk-games`、および`--games-per-thread`を64にして、`<前>`と`<後>`で2回ずつ実行した。

## エンジン

| 呼び名 | コミット | ortクレート |
|---|---|---|
| 前 | `645d37c4`（変更の前のmaster） | `=2.0.0-rc.13` |
| 後 | `3cd1e93a` | `=2.0.0-rc.15` |

どちらも`cargo build --release --bin minase`に、デバッグ情報を付ける`CARGO_PROFILE_RELEASE_DEBUG=1`を加えてビルドした。
ネットは、原因の調査の記録と同じ2つである。新形式は2回目の強化学習の初期ネット（20ブロック256チャネル、SHA-256の先頭は`7a9f3ec7`）で、旧形式は同じ重みをfloat32の入出力で書き出したもの（SHA-256の先頭は`8be01e2e`）である。今の推論器は入力がuint8のネットだけを受け付けるので、旧形式を渡すと入出力の形の不一致として拒む。

## 環境

作業機（Intel Core Ultra 7 265KF、NVIDIA GeForce RTX 3060 Ti 8 GB、ドライバ615.78.08、Arch Linux、glibc 2.44）で、gdb 18.1、ONNX Runtime 1.31.1のCUDA 13版、およびTensorRT 10.14.1を使った。
確認は2026年11月11日に行った。確認の間、ほかの測定は走っていなかった。

## 結果

### 登録表の2回目の解放

`probe.py`は、gdbが`exit`の入口で止まった時点で登録表の管理ブロックが生きていること、その後に登録表を手放す関数に届いたかどうか、およびプロセスの終了コードを判定する。

| 経路 | 前（rc.13） | 後（rc.15） | 終了コード |
|---|---|---|---|
| `dev nn-eval`、CUDA | 届いた | 届かなかった | 0 |
| `dev nn-eval`、TensorRT | 届いた | 届かなかった | 0 |
| `dev nn-eval`、入出力の形が合わないネット | 届いた | 届かなかった | 1 |
| 自己対局の生成器、8局 | 届いた | 届かなかった | 0 |

8回の実行はどれも、`exit`の入口で管理ブロックが生きており（強い参照1、弱い参照1）、期待した終了コードで終わり、シグナルによる終了はなかった。
gdbの出力は[alphazero-teardown-abort-workstation/probe/](alphazero-teardown-abort-workstation/probe/)にある。

### 同じプロセスでの推論器の作り直し

USIエンジンは、TensorRT、CUDA、TensorRT、バッチの大きさを64に変えたTensorRTの順に、推論器を4回作った。
4回の`isready`はすべて`readyok`を返し、その後の`go nodes 800`はすべて`bestmove`を返して、エンジンは終了コード0で終わった（[usi.log](alphazero-teardown-abort-workstation/usi.log)）。

### 推論の出力

CPU、CUDA、およびTensorRTの`dev nn-eval`の出力のJSONは、3つとも、変更の前後でバイト列が一致した（[nn-eval-sha256.txt](alphazero-teardown-abort-workstation/nn-eval-sha256.txt)）。
`dev nn-eval`は固定の288局面をバッチ7で評価するので、1回の実行は推論を42回呼ぶ。

### 生成器の出力と速さ

| 実行ファイル | 回 | 終了コード | 生成の速さ（毎秒の訓練局面） | 経過時間 |
|---|---|---|---|---|
| 前 | 1（TensorRTのエンジンを新しく作る） | 0 | 57.50 | 40.44秒 |
| 前 | 2 | 0 | 79.50 | 29.25秒 |
| 後 | 1 | 0 | 79.46 | 29.26秒 |
| 後 | 2 | 0 | 79.84 | 29.12秒 |

生成した訓練データ`chunk-00000.mnsd`のSHA-256は、4回とも`fa34f5cd…29edb3a`で一致した（[generation-sha256.txt](alphazero-teardown-abort-workstation/generation-sha256.txt)）。
前の1回目だけは、TensorRTのエンジンを新しく作る時間を含む。保存済みのTensorRTのエンジンを読むだけの3回の速さは、毎秒79.46〜79.84局面の範囲に収まった。

### マージ前の検査

`cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、および`cargo test -p minase --lib --features search-stats,tuning`は、すべて通った。
Rust 1.92のツールチェーンでの`cargo +1.92 check --workspace --all-targets`も通った。

## 結論

4つの確認はすべて通り、変更後の実行ファイルでは、終了時に登録表を手放す関数が呼ばれず、推論と生成の出力と速さは変更の前と変わらなかった。
