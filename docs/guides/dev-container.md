# 開発用のコンテナの手引き

本書は、minaseの開発と確認を、作業機のDockerコンテナの中で行う手順を定める。
作業機とは、開発者が手元で使うArch Linuxの計算機のことであり、コンテナの外側の作業機をホストと呼ぶ。
コンテナは、リポジトリの直下の`Dockerfile`から作るイメージ`minase-dev`を使う。
AlphaZero型探索の学習と自己対局は、Google Colabやvast.aiで借りるGPUつきの機械で動かし、それらの機械には`tools/train/zero/setup.sh`で実行環境を導入する。
イメージは、vast.aiで借りる機械と同じベースのイメージ（Ubuntu 24.04）の上で、同じ`setup.sh`を実行して作る。そのため、コンテナの中で通った確認は、借りる機械と同じOSと同じ版のライブラリの下で通ったことになる。
イメージを使う方式を選んだ理由は、設計判断の記録（ADR）である[実行環境のADR](../adr/install-the-zero-toolchain-with-setup-sh-on-rented-machines-and-build-the-workstation-image-from-it.md)にある。

イメージには、Ubuntu 24.04の上に、uv（Pythonのパッケージ管理ツール）、Rustのツールチェーン、Pythonの依存パッケージを入れた仮想環境、およびONNX Runtimeが入っている。
ONNX Runtimeは、ニューラルネットワーク（以下、ネット）の推論のライブラリである。
これらは`/opt/minase-zero`に置かれ、`/opt/minase-zero/env.sh`を読み込むと使えるようになる。
minase本体と、学習器が教師ファイルを読むための拡張モジュール`mnsd_reader`はイメージに入っていないので、コンテナの中でソースからビルドする。

## 1. 前提

ホストには、Docker、Docker Compose、およびNVIDIA Container Toolkitを導入しておく。
GPUは、CDI（Container Device Interface）の名前`nvidia.com/gpu=all`でコンテナへ渡すので、`docker info`の「Discovered Devices」にこの名前が出ることを確かめる。
動作は、Docker 29.9.2、Docker Compose 5.6.0、およびNVIDIA Container Toolkit 1.20.1で確かめた。

1つのコンテナは、1つの作業ツリーだけを扱う。
作業ツリーとは、リポジトリの本体のディレクトリ、またはgitのworktreeの1つである。
別の作業ツリーで作業するときは、その作業ツリーで別のコンテナを起動する。

## 2. .envを書く

`compose.yml`は、作業ツリーの直下の`.env`から5つの変数を読み、1つでも欠けていれば起動を拒む。
`.env`はgitの管理外なので、作業ツリーごとに書く。

| 変数 | 中身 |
|---|---|
| `COMPOSE_PROJECT_NAME` | composeのプロジェクトの名前。ボリュームの名前がここから付くので、作業ツリーごとに違う名前にする |
| `MINASE_UID`、`MINASE_GID` | ホストの利用者のUIDとGID。コンテナの中の処理はこの利用者で動く |
| `MINASE_TREE` | 扱う作業ツリーの絶対パス |
| `MINASE_GIT_DIR` | リポジトリの本体の`.git`の絶対パス |

作業ツリーの直下で次を実行すると、この5行を`.env`へ書き足せる。
リポジトリの本体の`.env`にはほかの用途の行があるので、上書きせずに書き足す。

```console
cat >> .env <<EOF
COMPOSE_PROJECT_NAME=minase-$(basename "$PWD")
MINASE_UID=$(id -u)
MINASE_GID=$(id -g)
MINASE_TREE=$(git rev-parse --show-toplevel)
MINASE_GIT_DIR=$(git rev-parse --path-format=absolute --git-common-dir)
EOF
```

## 3. イメージを作り、コンテナを起動する

作業ツリーの直下で次を実行する。

```console
docker compose build
docker compose up -d dev
docker compose exec dev bash
```

`docker compose build`は、イメージを作って`minase-dev:local`と名付ける。
初回は、依存パッケージの約7 GBを取得するので、作業機の回線で約4分かかった。
イメージはどの作業ツリーからも同じ名前で使うので、ビルドは1回でよい。

`docker compose up -d dev`は、準備のサービス`dev-init`を先に1回動かしてから、開発用のサービス`dev`を起動する。
`dev-init`は、ビルド出力などを置く4つの名前つきボリューム（中身は7節）の所有者を、ホストの利用者に合わせる。
`dev`は、起動時に、導入するツールとパッケージの版を決める作業ツリーのファイルが、イメージを作ったときのものと同じかを検査し（5節）、違えば起動せずに終わる。
起動しなかったときは、`docker compose logs dev`で理由を読む。

`docker compose exec dev bash`で開いた対話シェルは、`/opt/minase-zero/env.sh`を読み込んだ状態で、作業ツリーの直下から始まる。
`env.sh`は対話シェルの設定ファイルから読み込まれるので、対話シェルを開かずに`bash -c`で1つのコマンドを実行するときは、`env.sh`を自分で読み込む。
次の例の`-T`は、端末を割り当てない指定である。

```console
docker compose exec -T dev bash -c 'source /opt/minase-zero/env.sh && cargo test --workspace'
```

作業ツリーとリポジトリの本体の`.git`は、ホストと同じ絶対パスにマウントされる。
gitのworktreeは本体の`.git`を絶対パスで参照するので、同じパスに置くことで、worktreeのコンテナの中でもgitが動く。
コンテナの中で作ったファイルはホストの利用者の所有になり、ホストからそのまま編集できる。

## 4. コンテナの中でビルドと確認を行う

最初に、`mnsd_reader`をビルドして`/opt/minase-zero`の仮想環境へ入れる。
仮想環境は名前つきボリュームの外にあるので、`docker compose down`などでコンテナを作り直すとイメージの状態へ戻る。コンテナを作り直したら、このスクリプトを実行し直す。
`mnsd_reader`のソースか、それが依存するminaseのクレートを変えたときも、同じスクリプトでビルドし直す。

```console
tools/train/zero/sync-env.sh
```

Rustのビルドと確認は、ホストと同じコマンドで行う。
[テストの手引き](testing-minase-rules.md)のマージ前のローカル検査は、次の4つである。

```console
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p minase --lib --features search-stats,tuning
```

GPUを使う確認も、コンテナの中で実行できる。
αβ探索の評価関数（NNUE）の学習器のGPUの試験（[NNUEの学習の手引き](nnue-training.md)）、AlphaZero型探索の学習器`minase_zero`の試験、および[ネットの出力の照合](alphazero-network-check.md)が、作業機のGPUで通ることを確かめてある。
最初の2つは次のコマンドで実行し、照合はリンク先の手引きの手順で実行する。

```console
cargo test -p minase-trainer -- --ignored gpu_ --test-threads=1
cd tools/train/zero && uv run --frozen python -m unittest discover -s tests -t .
```

エンジン同士を対局させて棋力を測る対局ハーネス`minase match run`の、`commit:<コミット>`の指定も、コンテナの中で使える。
この指定を受けたハーネスは、そのコミットのソースをビルドし、実行ファイルを作業ツリーの`target/match-cache/`に保存して使い回す。

コンテナの中のビルド出力は、ホストのものと分かれている。
作業ツリーの`target/`と`tools/train/zero/mnsd_reader/target/`には名前つきボリュームがかぶさっており、コンテナの中のビルドはボリュームへ書き、ホストの同じパスのディレクトリは変わらない。
分けているのは、ホストでビルドした実行ファイルが、コンテナより新しいglibcに依存し得るからである。
ホストの側にこの2つのディレクトリがないままボリュームをかぶせると、Dockerがホストの側に空のディレクトリをrootの所有で作る。
これを避けるために、ディレクトリがなければ、`dev-init`が先にホストの利用者の所有で作る。

コンテナの中のビルドと試験も作業機のCPUを使うので、重い処理の前には、ホストと同じく[進行中の測定を確かめる](../lessons/check-running-measurements-before-cpu-load.md)。

## 5. 環境の入力を変えたとき

環境の入力とは、導入するツールとパッケージの版を決める`tools/train/zero/`の5つのファイル（`setup.sh`、`env.sh`、`pyproject.toml`、`uv.lock`、`.python-version`）である。
イメージは、作ったときの環境の入力の写しを`/opt/minase-zero/`に持つ。
`tools/train/zero/check-env-inputs.sh`は、作業ツリーの5つのファイルをこの写しとバイト列で比べ、違うファイルがあればそのパスを出して異常で終わる。
この検査は、`dev`の起動時と、`sync-env.sh`の最初に行われる。

検査があるのは、作業ツリーの`uv.lock`がイメージのものと違うまま同期すると、uvがイメージの仮想環境のパッケージを黙って入れ替えるからである。
同じ理由で、コンテナには`UV_NO_SYNC=1`を設定してあり、`uv run`は仮想環境を変えない。
仮想環境を変えるのは`sync-env.sh`だけである。

環境の入力を変えたときと、環境の入力が違うブランチへ切り替えたときは、その作業ツリーでイメージを作り直し、コンテナを作り直す。

```console
docker compose build
docker compose up -d dev
```

コンテナが作り直されるので、その後に4節の`tools/train/zero/sync-env.sh`を実行し直す。

`minase-dev:local`という名前は作業ツリーの間で共用するので、環境の入力が違う2つの作業ツリーのコンテナを同時には使えない。
環境の入力を変える作業は、masterへマージするまで、その作業ツリーだけで確かめる。

## 6. コンテナの中で行わないこと

次の操作は、ホストで行う。

- gitのコミットとpushは、ホストで行う。コンテナにはgitの利用者名とメールアドレスを設定していない。`git status`、`git log`、`git diff`のような読み取りは、コンテナの中でも使える。ただし、コンテナにはホストの利用者のgitの設定がないので、利用者ごとの除外の設定で隠しているファイルは、コンテナの中の`git status`に未追跡として出る。
- worktreeの作成、削除、および`git worktree prune`は、ホストで行う。コンテナからは、マウントした作業ツリーの外が見えないので、コンテナの中で作ったworktreeは、ホストから見えない場所に置かれ得る。コンテナの中のgitには、見えないworktreeの記録を`git gc`が消さない設定（`gc.worktreePruneExpire=never`）を入れてある。
- ほかの作業ツリーのビルドは、その作業ツリーのコンテナで行う。このリポジトリはworktreeを本体の`data/worktrees/`の下に置くので、本体のコンテナからはそれらのworktreeも見える。しかし、そのビルド出力のディレクトリにはボリュームがかぶさっていないので、本体のコンテナの中ではビルドしない。
- 強化学習のサイクルを借りた機械で起動する実行器`minase_zero run-cycles`と、vast.aiの起動スクリプトは、ホストで動かす。どちらも作業機のSSHの鍵、vast.aiの認証、および学習データを置くNASを使い、コンテナにはこれらを渡していない。

## 7. 止めるとき、消すとき

```console
docker compose down       # コンテナを止めて削除する。ボリュームは残る
docker compose down -v    # ボリュームも削除する
```

ボリュームには、ビルド出力（`target/`と`mnsd_reader/target/`）、ホームのディレクトリ（uvのキャッシュを含む）、およびcargoが取得したクレートが入っている。
残しておけば、次にコンテナを起動したときに、クレートの取得とビルドを最初からやり直さずに済む。4節の`sync-env.sh`は、その場合も実行し直す。
worktreeを削除するときは、先にそのworktreeで`docker compose down -v`を実行し、ボリュームを残さない。
