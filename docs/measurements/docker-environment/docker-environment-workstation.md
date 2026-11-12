# 作業機でのイメージと開発用のコンテナの確認

## 目的

[Dockerイメージの設計書](../../plans/archived/docker-environment.md)のフェーズ2の記録である。
作業機で`Dockerfile`からイメージを作り、docker composeで起動した開発用のコンテナの中で、設計書の「検証」の節の項目1〜7が通るかを確かめた。

## コマンドライン

確認日は2026年11月12日である。
作業ツリーの直下で、次のようにイメージを作り、コンテナを起動した。

```console
docker build --progress=plain -t minase-dev:local .
docker compose build
docker compose up -d dev
```

コンテナの中のコマンドは、`docker compose exec -T dev bash -c 'source /opt/minase-zero/env.sh && <コマンド>'`の形で実行した。
項目ごとのコマンドは次のとおりである。

```console
# 項目1（ビルド、起動、GPU）
nvidia-smi --query-gpu=name,memory.total,driver_version --format=csv
# 項目2（同期とマージ前の検査）
tools/train/zero/sync-env.sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p minase --lib --features search-stats,tuning
# 項目3（NNUEの学習器のGPUの試験）
cargo test -p minase-trainer -- --ignored gpu_ --test-threads=1
# 項目4（AlphaZero型探索の学習器の試験。照合のコマンドは下の段落）
cargo build --release --bin minase
cd tools/train/zero && uv run --frozen python -m unittest discover -s tests -t .
# 項目5（gitと対局ハーネス）
git status --short && git log --oneline -2 && git worktree list
target/release/minase match run --run-dir data/docker-environment-check/match-smoke --seed 1 \
  --candidate commit:ffe28348 --baseline commit:947e6417 --rules JCSA --each time=1000+10 gsprt --max-pairs 1
```

項目4の照合は、ニューラルネットワーク（以下、ネット）の同じ288局面の出力を、学習器のPyTorchの出力を基準にして、minaseの推論器の出力と比べる処理である。
minaseの出力は、ONNX RuntimeのCPUとTensorRTのFP16の2つの実行プロバイダについて、バッチの大きさ1、7、256で書き出すので、比較は6つになる。
コマンドは[ネットの出力の照合の手引き](../../guides/alphazero-network-check.md)の3節と5節のものを使った。
ネットは、強化学習の2回目の実行の初期ネット（NASの`cycles-rl2/cycle-0000/model.onnx`、SHA-256は`7a9f3ec7…d436d43d76`）であり、基準は同じディレクトリの`check/pytorch.json`である。どちらも作業ツリーの`data/docker-environment-check/`へ写して使った。
項目6は、作業ツリーの`uv.lock`がイメージのものと違うときに、仮想環境が書き換わらないことを確かめる項目である。ホストから`tools/train/zero/uv.lock`の末尾に空白を1バイト足し、コンテナの中で`sync-env.sh`と`uv run`を実行してから、ファイルを元へ戻した。
項目7では、コンテナの中で`touch`と`mkdir`を実行し、ホストから所有者を調べた。

## エンジン

イメージとコンテナの定義はコミット`ffe28348`のものである。
項目5で対局ハーネス`minase match run`にビルドさせたコミットは、`ffe28348`と、ブランチを切った時点のmasterの`947e6417`である。
ベースのイメージは`nvidia/cuda:13.0.3-base-ubuntu24.04@sha256:7c7413a5…e927a5c1`である。
イメージに入った版は、uv 0.13.4、Rust 1.98.0（clippy 0.1.98、rustfmt 1.9.0）、Python 3.13.16、PyTorch 2.11.0+cu130、およびONNX Runtime 1.31.1である。

## 環境

作業機のCPUはIntel Core Ultra 7 265KF（20コア20スレッド）、GPUはNVIDIA GeForce RTX 3060 Ti（8,192 MiB、ドライバ615.78.08）、OSはArch Linuxである。
Dockerは29.9.2、Docker Composeは5.6.0、NVIDIA Container Toolkitは1.20.1である。
確認の間、作業機でほかの対局の測定は動いていなかった。

項目5は、リポジトリの本体と、`data/worktrees/`の下のworktreeの両方で、コンテナの中のgitと対局ハーネスが動くことを確かめる項目である。
worktreeは`.git`がファイルであり、本体の`.git`のディレクトリを絶対パスで参照するので、2つを別々に確かめる。
本体のチェックアウトはまだ`compose.yml`を持たないので、ブランチを`git clone`した複製を本体に見立てた。複製は、作業ツリーの直下に`.git`のディレクトリを持つ点が本体と同じである。
worktreeには、このブランチのworktreeそのものを使った。

## 結果

7項目はすべて通った。

| 項目 | 結果 |
|---|---|
| 1. ビルド、起動、GPU | イメージのビルドは3分41秒で、圧縮後の大きさは7.12 GBであった。`docker compose up -d dev`は、ボリュームのない状態から約1秒で、準備のサービス`dev-init`と開発用のサービス`dev`を起動し、コンテナの中の`nvidia-smi`はRTX 3060 Tiを表示した |
| 2. 同期とマージ前の検査 | `sync-env.sh`は、学習器が教師ファイルを読む拡張モジュール`mnsd_reader`を、11秒でビルドして導入した。4つの検査は、クレートの取得を含めて2分25秒で通った（clippyが11秒、`cargo test --workspace`が1分31秒、フィーチャつきの試験が43秒） |
| 3. NNUEの学習器のGPUの試験 | `gpu_`の35件が通った。NVRTCの共有ライブラリは、仮想環境の`nvidia/cu13/lib/libnvrtc.so.13`から読み込まれた |
| 4. 学習器の試験と初期ネットの照合 | 学習器の試験は316件（うち1件は実行を省く指定の試験）が6分35秒で通った。照合の6つの比較はすべて許容誤差に収まった |
| 5. 対局ハーネスとgit | worktreeのコンテナでも複製のコンテナでも、`commit:`の指定で2つのコミットをビルドして1ペアを対局できた。実行ファイルは`target/match-cache/`のボリュームに置かれ、ホストの`target/`と`mnsd_reader/target/`のファイルの一覧、更新時刻、および大きさは、対局の前後で変わらなかった。worktreeのコンテナの中で`git status`、`git log`、`git worktree list`が動いた |
| 6. 環境の入力の検査 | `sync-env.sh`は`uv.lock`のパスを出して終了コード1で終わり、uvを呼ばなかった。`uv run`は動いたが、仮想環境のファイルの一覧、更新時刻、および大きさは変わらなかった。`uv.lock`を変えたまま`dev`を起動し直すと、`dev`は同じメッセージを出して終了コード1で終わった |
| 7. 所有者 | コンテナの中で作ったファイルとディレクトリは、ホストの利用者の所有であった。ホストになかった`target/`は、`dev-init`がホストの利用者の所有で作った |

照合での基準との差の最大は、次のとおりである。
方策は、CPUでは合法手のlogitで、TensorRTでは合法手の確率で比べる。

| 実行プロバイダ | バッチ | 方策の差の最大 | 価値の差の最大 |
|---|---|---|---|
| ONNX RuntimeのCPU | 1、7、256 | logitで1.2×10⁻⁴ | 8.8×10⁻⁸ |
| TensorRTのFP16 | 1 | 確率で3.2×10⁻⁵ | 2.6×10⁻⁴ |
| TensorRTのFP16 | 7 | 確率で3.2×10⁻⁵ | 2.6×10⁻⁴ |
| TensorRTのFP16 | 256 | 確率で3.4×10⁻⁵ | 2.7×10⁻⁴ |

CPUの欄のlogitの差は、PyTorchの出力との差そのものである。照合は、ONNXが返すlogitがFP16へ丸められていることを見込んで、この差からFP16の1目盛りを引いた値を許容誤差1×10⁻⁴と比べる。

項目2の同期では、uvが「キャッシュと仮想環境のファイルシステムが違うので、ハードリンクの代わりに複製する」という警告を出した。
キャッシュはホームのディレクトリのボリュームに、仮想環境はイメージの層にあるためであり、導入は成功する。

あわせて、導入先を`tools/train/zero/`の下に閉じる形へ改めた`setup.sh`を、作業機のホストでも実行した。
uv、rustup、Rustのツールチェーン、Python、およびONNX Runtimeが作業ツリーの`tools/train/zero/.runtime/`（1.1 GB）に入り、ホームのディレクトリのシェルの設定ファイルは変わらなかった。
`setup.sh`は、仮想環境をロックファイルの外部のパッケージだけに揃えるので、仮想環境に入っていた`mnsd_reader`は外れた。ホストでも、`setup.sh`の後に`sync-env.sh`を実行して`mnsd_reader`を入れ直す。

### 層を分けたイメージ

7項目を確かめた後、vast.aiの機械がイメージを取得できなかったので、導入したファイルを8つの層に分ける`Dockerfile`（コミット`b2e2cd2f`）も作った（経過は[vast.aiでの確認の記録](docker-environment-vast.md)）。
このイメージの`/opt/minase-zero`は、分ける前のイメージと、ファイル、ディレクトリ、およびシンボリックリンクの一覧（種類、権限、所有者、ファイルの大きさ、リンク先）が26,066行とも一致し、全ファイルのSHA-256の一覧のSHA-256も一致した。
その後、vast.aiではイメージを使わないと決まったので、`Dockerfile`は7項目を確かめたコミット`ffe28348`の内容へ戻した。

## 結論

作業機では、イメージのビルドから開発用のコンテナの中の検査、GPUの試験、照合、および対局ハーネスまでが通り、ホストのビルド出力とコンテナのビルド出力は分かれていた。
本体のチェックアウトそのものでの確認は、ブランチをmasterへマージした後に行う。
