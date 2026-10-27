# ortの依存によるビルド時間と実行ファイルの増加

## 目的

[ネットの入出力と推論の設計書](../../plans/archived/alphazero-network.md)のフェーズ2として、ONNX Runtimeのバインディングortの依存を加える前後で、`cargo build --release --bin minase`のクリーンビルドの時間と実行ファイルの大きさを比べ、依存を常に含める方針（cargoのfeatureで切り替えない）を見直す必要があるかを判定する。
あわせて、ortを含むビルドが`ORT_DYLIB_PATH`もGPUのライブラリもない環境で通ること、および最小対応版のツールチェーンで全ターゲットを検査できることを確かめる。

## コマンドライン

```console
# 変更前: ortを加える前のコミット1467ffe3を git archive で展開したディレクトリ
# 変更後: ortを加えた作業ツリー（コミット982d772dと同じ依存）
rm -rf <target>
nice -n 19 env -u ORT_DYLIB_PATH -u LD_LIBRARY_PATH CARGO_TARGET_DIR=<target> \
  /usr/bin/time -f "%e %U" cargo build --release --bin minase --manifest-path <dir>/Cargo.toml
stat -c %s <target>/release/minase
cargo +1.88 check --workspace --all-targets
```

変更前と変更後を交互に3回ずつ、毎回空のターゲットディレクトリからビルドした。

## エンジン

変更前はブランチ`alphazero-network`のコミット1467ffe3、変更後は同じブランチのortの依存（`ort = "=2.0.0-rc.13"`、`default-features = false`、機能は`std`、`load-dynamic`、`tensorrt`、`cuda`、および`api-28`）を加えた作業ツリーであり、後者の依存とソースはコミット982d772dと同じである。
ortの追加で`Cargo.lock`に増えたパッケージは16個（`ort`、`ort-sys`など）である。

## 環境

Intel Core Ultra 7 265KF（物理20コア、論理20コア）、メモリ31 GB、Arch Linux、Rust 1.98.0の作業機で測った。
測定のあいだ、lishogiのBotが通信対局を指しており、1分間の平均負荷は1.8〜2.6であった。

## 結果

| 回 | 変更前の経過時間 | 変更後の経過時間 | 変更前のユーザー時間 | 変更後のユーザー時間 |
|---|---|---|---|---|
| 1 | 30.17秒 | 31.37秒 | 51.32秒 | 55.47秒 |
| 2 | 29.73秒 | 31.21秒 | 50.66秒 | 55.55秒 |
| 3 | 29.80秒 | 31.33秒 | 50.90秒 | 55.26秒 |

経過時間の中央値は、変更前が29.80秒、変更後が31.33秒で、増加は1.53秒（5.1%）であった。
実行ファイルの大きさは、変更前が7,872,936バイト、変更後が8,138,048バイトで、増加は265,112バイト（3.4%）であった。
変更後のビルドは、`ORT_DYLIB_PATH`と`LD_LIBRARY_PATH`を外した環境で成功した。
`cargo +1.88 check --workspace --all-targets`は、ortとその依存を含めて成功した。
ただし、ortと無関係に、masterにあった`crates/minase/src/search/alphabeta/tests/time.rs`のテストが1.88で一時値の寿命の延長を受けられずに失敗したので、コミット637d5b96で値を変数へ束縛してから検査した。

## 結論

ortの依存によるクリーンビルドの時間の増加は約5%、実行ファイルの大きさの増加は約3%にとどまり、許容できない増加ではないので、依存を常に含めてcargoのfeatureで切り替えない方針を維持する。
