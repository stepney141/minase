# 開発環境とvast.aiの機械の実行環境を、1つのDockerイメージで用意する設計書

## 先に読む要約

本書は、中将棋の対局プログラムであるminaseの開発環境と、AlphaZero型探索の学習と自己対局を行うvast.aiの機械の実行環境を、1つのDockerイメージにまとめるマイルストーンの設計書である。
vast.aiは、GPUつきの機械を時間単位で貸し出すサービスである。
ここで作業機とは、開発者が手元で使い、対局の測定にも使うArch Linuxの計算機のことである。
今は、借りた機械ごとに`tools/train/zero/setup.sh`を実行し、uv（Pythonのパッケージ管理ツール）、Rustのツールチェーン、Pythonの依存パッケージ、およびONNX Runtimeをその場で導入している。
ONNX Runtimeは、ニューラルネットワーク（以下、ネット）の推論のライブラリである。vast.aiの機械では、`setup.sh`の前にaptで10個のパッケージを、版を固定せずに入れている。
本書では、これらの外部の依存を入れたイメージを作り、作業機ではdocker composeで起動する開発用のコンテナとして、vast.aiでは借りる機械のイメージとして使う。
Google Colabで借りる機械（以下、Colab機）は、Colabの規約がコンテナ化を禁じ、コンテナの実行環境もないので、今と同じく`setup.sh`で導入する。
版を二重に管理しないため、イメージは同じ`setup.sh`を実行して作り、版は`setup.sh`、`env.sh`、`uv.lock`などの「環境の入力」と、Dockerfileが指定するベースのイメージだけに書く。
ネットを学ぶPythonのプログラムを学習器、自己対局で作った学習データのファイルを教師ファイルと呼ぶ。
minase本体と、学習器が教師ファイルを読むための拡張モジュール`mnsd_reader`はイメージに入れず、動かす機械で、対象のコミットのソースからビルドする。
イメージはGitHubのコンテナレジストリ（GitHub Container Registry、以下GHCR）に非公開で置き、vast.aiの機械は読み取り専用のトークンで取得する。
完了条件は、作業機の開発用のコンテナ、vast.aiの機械、およびColab機で「検証」の節の確認がすべて通り、手引きと設計判断の記録（ADR、[docs/adr/](../adr/README.md)）が新しい手順に改まることである。
強化学習の[2回目の実行](alphazero-rl-2.md)は、20ブロック256チャネルのネットでやり直す2回目の学習であり、自己対局の生成と学習の1回分をサイクルと呼ぶ。
2回目の実行をvast.aiで回すなら、そのサイクル1は本書の完了を待つ。

## 状態

進行中。本書は2026年11月11日に書き、11月12日に着手した。
利用者は11月11日に、composeとvast.aiの両方で同じイメージを使うこと、GHCRに非公開で置くこと、および作業機に残っていた古いイメージ`minase-zero:dev`を削除することを決め、古いイメージはその日のうちに削除した。
完了したフェーズはまだない。次の一手は、フェーズ1の環境の入力と実行器の変更である。

## 目的

本書の目的は2つある。

1つ目は、vast.aiの機械の導入を、機械や借りた日によらず同じ結果にすることである。
今の導入は、apt、PyPI（Pythonのパッケージの配布元）、NVIDIAのパッケージインデックス（pypi.nvidia.com）、GitHub、およびrustupの配布元から、毎回大きなパッケージを取得する。どれか1つでも遅くなったり止まったりすると、導入が長引くか失敗する。
aptの10個のパッケージは版を固定していないので、借りた日によって中身が変わり得る。
イメージにすれば、容量の大きい実行時の依存はイメージに入り、aptのパッケージを含むユーザーランド全体がイメージのダイジェスト（イメージの内容から計算するSHA-256の値）で固定される。
ただし、`mnsd_reader`とminaseを初めてビルドするときには、PyPIが配布するmaturinと、crates.ioが配布するRustのクレートの取得が残る。これらはソースのコミットごとに変わり得るので、イメージには入れない。
[複数GPU化のvast.aiの測定](../measurements/alphazero-multi-gpu/alphazero-multi-gpu-vast.md)では、RTX 4090を4枚積んだ2台の機械（測定記録の機械Hと機械B）で、導入（パッケージ、学習器の環境、およびminaseのビルド）に3分52秒と11分50秒かかった。
イメージにしたときの導入の時間は、イメージの取得に加え、ビルドのときに残るパッケージの取得とビルドの時間も含めて、本書の測定で記録する。

2つ目は、作業機でも、借りた機械と同じユーザーランドで開発と確認をできるようにすることである。
作業機はArch Linux（glibc 2.44）、借りた機械はUbuntu 24.04（glibc 2.39）であり、作業機で通った確認が借りた機械で通るとは限らない。
作業機の環境は手で更新しているので、版のずれも起きる。
たとえば2026年11月11日の時点で、作業機の`tools/train/zero/.runtime`にはONNX Runtime 1.28.3しかなく、`env.sh`が指す1.31.1が入っていない。
開発用のコンテナはイメージの版から始まり、ソースの版とのずれは「環境の入力の検査」の節の検査で検出する。

## 適用範囲

本書で使う名前は次のとおりである。

| 名前 | 中身 |
|---|---|
| イメージ | 本書で作るDockerイメージ。GHCRでの名前は`ghcr.io/stepney141/minase-dev` |
| ホスト | コンテナの外側の作業機。作業機のOSで直接動かすことを「ホストで動かす」という |
| 開発用のコンテナ | 作業機でdocker composeが起動する、イメージのコンテナ |
| 作業ツリー | リポジトリの本体のディレクトリ、またはgitのworktree（同じリポジトリの別のブランチを置くディレクトリ）の1つ。このリポジトリは設計書ごとにworktreeを作り、`data/worktrees/`などに置く |
| 環境の入力 | 導入の版を決めるファイル。`tools/train/zero/`の`setup.sh`、`env.sh`、`pyproject.toml`、`uv.lock`、および`.python-version` |
| 環境のディレクトリ | 導入したuv、Rustのツールチェーン、Pythonの本体と仮想環境、およびONNX Runtimeを置くディレクトリ。`env.sh`が置かれたディレクトリであり、Colab機では今と同じ`/content/src/tools/train/zero`、イメージでは`/opt/minase-zero` |
| 実行器 | 借りた機械でサイクルの生成、学習、および照合を順に起動する`minase_zero run-cycles`（`tools/train/zero/minase_zero/runner.py`）。作業機のホストで動き、借りた機械をSSHで操作する |
| 照合 | 学習器が書き出したネットに同じ288局面を与え、minaseの推論器と学習器のPyTorchとでそれぞれ計算して、出力の差を比べる処理。バッチ1、7、256で比べ、許容誤差は実行器が`nn_compare.py`へ渡す値とする。ONNX RuntimeのCPUでは、方策のlogit（確率に変換する前の値）と価値の差を1×10⁻⁴以下とする。TensorRTでは、確率の差を1×10⁻²以下、価値の差を5×10⁻²以下とする |
| 起動スクリプト | vast.aiの機械を借り、準備し、実行器を走らせ、結果を受け取って機械を削除する`launch-vast.py`。作業機のホストで動く |
| 準備スクリプト | 起動スクリプトがvast.aiの機械へ送って実行するシェルスクリプト。今は`docs/measurements/alphazero-multi-gpu/vast-setup.sh`が、aptの導入、`setup.sh`の実行、およびminaseのビルドを行う |
| 対局ハーネス | エンジン同士を対局させて棋力を測る`minase match run`。`commit:<コミット>`の指定では、そのコミットを`git archive`で展開してビルドし、実行ファイルを`target/match-cache/`に保存して使い回す |
| NAS | 作業機にNFSでマウントした保存先。学習データ、チェックポイント、およびネットを置く |

本書が変えるものは次のとおりである。

- リポジトリの直下に置く`Dockerfile`、`.dockerignore`、および`compose.yml`。
- 環境の入力のうち`setup.sh`と`env.sh`。導入先を環境のディレクトリに閉じる（「導入先を環境のディレクトリに閉じる」の節）。
- `tools/train/zero/`に新しく置く、環境の入力を検査するスクリプトと、検査してからPythonの仮想環境をロックファイルに同期するスクリプト。
- 実行器の`RemotePaths`。機械の種類ごとに、読み込む`env.sh`の場所を決める。
- `scripts/vast/common.py`の`create`と、その試験。非公開のレジストリへのログインの引数を加え、失敗したときの例外にトークンが入らないようにする。
- 本書の測定記録のディレクトリ`docs/measurements/docker-environment/`に置く、起動スクリプトの写しと新しい準備スクリプト。
- 手引き（[借りる機械の操作の手引き](../guides/alphazero-machines.md)の3節と7.1節、および開発用のコンテナの新しい手引き）とADR。

次のものは対象外とする。

- 探索の係数の調整（SPSA）と、αβ探索の評価関数（NNUE）の教師生成でvast.aiを使うスクリプト。Rustの公式のイメージをそのまま使い続け、`create`の引数の追加に合わせる以外は変えない。
- lishogi Bot。別のリポジトリ`minase-lishogi-bot`が独自のイメージを持つ。
- 外部のサービスとAIエージェントを操作するコマンドラインツール（vastai、colab、agy、codex）、リリースのツール（git-cliff、cargo-release）、および比較の相手にする外部の中将棋エンジンHaChu。最初の4つは作業機の認証情報を使い、残りは今のところコンテナで使う予定がない。
- 実行器と起動スクリプトの実行。作業機のSSHの鍵、vast.aiの認証、およびNASを使うので、今と同じくホストで動かす。
- NVIDIAのドライバ。ホストのものを、NVIDIA Container Toolkitがコンテナへ渡す。
- Runpod。まだ使っていないので、使うときに同じイメージを指定できるかを確かめる。
- イメージを自動でビルドするCI。必要になるまで作らない。

## 依存関係

本書は、[複数GPU化](archived/alphazero-multi-gpu.md)とONNX Runtime 1.31.1への更新の後のmasterから始める。
[推論の転送の削減](archived/alphazero-inference-transfer.md)とは変えるファイルが重ならない。
どちらかが環境の入力を変えた場合は、変えた後のコミットでイメージを作り直す。
2回目の実行をvast.aiで回すなら、そのサイクル1は本書の完了を待つ。

本書の完了時に、[実行環境の導入のADR](../adr/install-the-zero-toolchain-from-one-lock-on-every-machine.md)を新しいADRで置き換える。
このADRは、vast.aiとRunpodの機械でも事業者が公開するUbuntuとCUDAのイメージの上で`setup.sh`を実行すると決めており、独自のコンテナイメージを作る案は、Colab機で使えないことを理由に棄却していた。
本書は、Colab機では今と同じく`setup.sh`を使い、vast.aiの機械と作業機では独自のイメージを使うので、vast.aiの機械についての決定が変わる。

GHCRへの登録には、作業機での`docker login ghcr.io`と、書き込みの権限（`write:packages`）を持つトークンが要る。
作業機の`~/.docker/config.json`にはGHCRの認証の項目があるが、まだ有効かは確かめていない。
vast.aiの機械が取得するときは、読み取りの権限（`read:packages`）だけを持つ別のトークンを使う。どちらのトークンも利用者がGitHubで発行する。

## 設計判断

| 判断 | 採る方式 | 棄却した代案 |
|---|---|---|
| イメージの数 | 1つにし、作業機の開発用のコンテナとvast.aiで共用する | 開発用と実行用の2つ（Dockerfileの保守が2倍になり、作業機で確かめた環境と借りた機械の環境が別物になる） |
| 版を書く場所 | Dockerfileが`setup.sh`を実行し、版は環境の入力と、Dockerfileが指定するベースのイメージのダイジェストだけに書く | Dockerfileに版を書いて別の手順で導入する（Colab機の`setup.sh`と版が食い違い得る） |
| ベースのイメージ | `nvidia/cuda:13.0.3-base-ubuntu24.04`をダイジェストで固定する | cuDNNつきのランタイムのイメージ（PyPIから入るCUDAとcuDNNのライブラリと重複する）、Rustの公式のイメージ（Debianであり、Colab機のUbuntuと違う） |
| 導入先 | uv、rustup、Rustのツールチェーン、Pythonの本体、仮想環境、およびONNX Runtimeをすべて環境のディレクトリに置く | 今のように一部を`$HOME`に置く（コンテナの利用者とSSHのセッションによって`$HOME`が変わる） |
| minaseと`mnsd_reader` | イメージに入れず、動かす機械で対象のコミットからビルドする | イメージに入れる（`target-cpu=native`なのでビルドした機械のCPUに特化し、コミットごとに作り直しになる） |
| 環境の入力の検査 | イメージに写した環境の入力とソースのものをバイト列で比べ、違えば止める。開発用のコンテナではuvの自動の同期を止め、同期は検査を先に行うスクリプトだけで行う | 検査しない（ソースの`uv.lock`が違うと、uvが黙ってイメージの仮想環境を入れ替える） |
| レジストリ | GHCRに非公開で置き、vast.aiは読み取り専用のトークンで取得する | 公開（TensorRTとcuDNNの再配布の条件をまだ確かめていない）、借りた機械でイメージを作る（今の導入と同じ取得が要る） |
| 開発用のコンテナが扱う作業ツリー | 1つのコンテナは1つの作業ツリーだけを扱い、その作業ツリーの`target/`と`mnsd_reader/target/`を、作業ツリーごとの名前つきボリュームで覆う | リポジトリ全体を1つのコンテナで扱い、直下の`target/`だけを覆う（worktreeと`mnsd_reader`のビルド出力がホストと共有されたまま残る）、`CARGO_TARGET_DIR`を設定する（対局ハーネスが`<ソース>/target/release/minase`を読むので壊れる） |
| マウント先 | 作業ツリーとリポジトリの本体の`.git`を、ホストと同じ絶対パスにマウントする | `/workspace`などの別のパス（worktreeが絶対パスで記録した場所とずれ、gitが動かない） |
| ボリュームの所有者 | rootで1回だけ動く準備用のサービスが、ボリュームの所有者をホストの利用者に合わせてから、開発用のサービスをホストの利用者で起動する | 開発用のサービスをrootで動かす（ホストにrootのファイルができる）、イメージに利用者を作る（UIDがイメージに焼き付き、vast.aiと共用できない） |
| GPUの渡し方 | CDI（Container Device Interface）の名前`nvidia.com/gpu=all`で渡す | DockerにNVIDIAのランタイムを登録して`--gpus`で渡す（作業機のDockerの設定を変える必要がある） |

### イメージは1つにし、同じsetup.shを実行して作る

Dockerfileは、ベースのイメージの上で次の順に処理する。

1. aptで、今の準備スクリプトが入れている10個のパッケージ（ca-certificates、curl、rsync、python3、build-essential、pkg-config、git、xz-utils、procps、time）を入れる。
2. 環境の入力を`/opt/minase-zero/`へ写す。
3. `/opt/minase-zero/setup.sh`を実行する。
4. uvのキャッシュとONNX Runtimeの書庫を消し、開発用のコンテナの利用者が環境のディレクトリへ書き込めるように権限を与える。

ベースのイメージには、今vast.aiで使っている`nvidia/cuda:13.0.3-base-ubuntu24.04`を使い、タグではなくダイジェストで指定する。
CUDAのランタイム、cuDNN、およびTensorRTの共有ライブラリは、PyPIの配布形式であるwheelから入る。具体的には、uvのロックファイルが固定する`tensorrt-cu13-libs`と、PyTorchが依存するNVIDIAのパッケージから入り、`env.sh`の`LD_LIBRARY_PATH`がそれらを指す。
そのため、CUDAのライブラリを含む大きなベースのイメージは要らない。
2026年10月に作業機で作った古いイメージは、CUDAのライブラリ（2.52 GB）とcuDNN（877 MB）を含むベースを使っており、wheelと重複していた。

作業機の仮想環境は8.5 GB（TensorRTが3.9 GB、NVIDIAのCUDAのライブラリが2.7 GB、PyTorchが1.2 GB）あるので、イメージの大きさは10 GB前後になる見込みである。
vast.aiの機械のディスクは100 GBで借りているので、収まる。

### 導入先を環境のディレクトリに閉じる

今の`setup.sh`は、uvを`$HOME/.local/bin`へ、rustupとRustのツールチェーンを`$HOME/.cargo`と`$HOME/.rustup`へ入れる。uvはPythonの本体を`$HOME/.local/share/uv/python`へ入れる。
環境のディレクトリに入るのは、仮想環境`.venv`とONNX Runtimeの`.runtime`だけである。
この配置では、イメージを作ったときの`$HOME`（`/root`）と、使うときの`$HOME`が違うと、ツールが見つからない。
開発用のコンテナはホストの利用者のUID（利用者の番号）で動くので、`/root`を読めない。
vast.aiの機械ではrootでSSHのセッションに入るが、vast.aiの文書は、SSHの起動方式ではコンテナの起動時に`-e`で渡した環境変数が既定ではSSHのセッションから見えないとし、機械の起動時に動くonstartのスクリプトで`/etc/environment`へ書き出すよう勧めている。DockerfileのENVで設定した環境変数が見えるかどうかは、文書に書かれていない。
そのため、導入先と環境変数は、イメージの`ENV`にも`$HOME`にも頼らず、`env.sh`を読み込めば決まる形にする。

`setup.sh`と`env.sh`は、すべての導入先を環境のディレクトリの下に置く形に改める。
uvの実行ファイルは`.runtime/bin`、rustupの`RUSTUP_HOME`と`CARGO_HOME`は`.runtime/rustup`と`.runtime/cargo`、uvの`UV_PYTHON_INSTALL_DIR`は`.runtime/python`とする。
uvの公式のインストーラには`UV_UNMANAGED_INSTALL`で導入先を指定し、シェルの設定ファイルを書き換えさせない。
今の`setup.sh`は`command -v uv`と`command -v rustup`で既存の導入を見つけると導入を省くので、別の場所の既存のツールを使ってしまう。改めた`setup.sh`は、環境のディレクトリの中の決まったパスにある実行ファイルとその版を確かめ、なければ入れる。
`env.sh`は、これらの変数と`PATH`を設定し、加えて`UV_PROJECT_ENVIRONMENT`を環境のディレクトリの`.venv`に設定する。
`env.sh`は、さらに2つの変数を設定する。`RUSTUP_TOOLCHAIN`は固定した版とし、版を指定しない`cargo`と、`mnsd_reader`をビルドするときにmaturinが起動するcargoが、同じツールチェーンを使うようにする。`UV_MANAGED_PYTHON`は1とし、uvが機械にもとからあるPythonを使わず、`.runtime/python`に入れたPythonだけを使うようにする。
`UV_PROJECT_ENVIRONMENT`を設定すると、`uv run --project <ソース>`はソースの中の`.venv`ではなく、環境のディレクトリの仮想環境を使う。
Colab機では環境のディレクトリがソースの中にあるので、今の`uv run`と同じ仮想環境を指す。
これで、`env.sh`を読み込んだシェルは、利用者と`$HOME`によらず同じツールを使う。

Rustのツールチェーンには、開発用のコンテナでマージ前のローカル検査（[テストの手引き](../guides/testing-minase-rules.md)）を実行するために、clippyとrustfmtを加える。
Colab機とvast.aiの機械でも同じものが入るが、追加で取得する容量は小さい。
作業機のホストで`setup.sh`を実行し直すと、ホストのrustupとは別のツールチェーンが`.runtime`に入る。ホストの`~/.rustup`は変わらない。

### minaseとmnsd_readerは動かす機械でビルドする

minase本体をイメージに入れないのは、`.cargo/config.toml`が`target-cpu=native`を指定しており、ビルドした機械のCPUに特化した実行ファイルになるからである。
また、対局ハーネスの`commit:`の指定と実行器の`--commit`は、測るたびに違うコミットをビルドする。

`mnsd_reader`は、`tools/train/zero/mnsd_reader/Cargo.toml`でminaseとminase-coreのクレートにパスで依存するので、イメージに入れると、イメージがソースの特定のコミットに結び付く。
そこで、`setup.sh`は、パスで指定した依存を入れない`uv sync --frozen --no-install-local`で外部のパッケージだけを入れる。
`setup.sh`は最後に、PyTorchの版とGPUを認識しているかを`uv run`で表示している。`uv run`は既定で仮想環境をロックファイルに同期し、`mnsd_reader`のビルドを始めてしまうので、この表示は同期を止めた`uv run --no-sync`で行う。
`mnsd_reader`は、動かす機械で作業ツリーを指定した`uv sync --frozen --project <作業ツリー>/tools/train/zero`を実行し、そのコミットのソースからビルドして入れる。
vast.aiの準備スクリプトとColab機の手順は、導入の直後にこの同期を実行し、ビルドの失敗を最初のサイクルの前に検出する。

イメージを作る時点では`mnsd_reader/`をイメージへ写さない。
uv 0.13.4は、`tools/train/zero/pyproject.toml`からパスで参照している`mnsd_reader/`のディレクトリがない状態でも、`--no-install-local`の同期を受け付ける。2026年11月12日に作業機で、`pyproject.toml`、`uv.lock`、および`.python-version`だけを置いたディレクトリで同期を実行して確かめた。

### 環境の入力の検査

イメージは、作ったときの環境の入力の写しを`/opt/minase-zero/`に持つ。
検査のスクリプト`check-env-inputs.sh`は作業ツリーの`tools/train/zero/`に置き、`env.sh`を読み込んだシェルから引数なしで実行する。
スクリプトは、自分と同じディレクトリにある環境の入力の各ファイルを、環境のディレクトリの写しとバイト列で比べ、違うファイルがあればそのパスを出して異常で終わる。環境のディレクトリは、`env.sh`が設定した`UV_PROJECT_ENVIRONMENT`の親のディレクトリとする。
検査を置くのは、作業ツリーの`uv.lock`がイメージのものと違うまま同期すると、uvが作業ツリーの`uv.lock`に合わせてイメージの仮想環境のパッケージを入れ替え、イメージで固定したはずの版が知らないうちに変わるからである。
検査に失敗したら、そのコミットでイメージを作り直す。

vast.aiの機械では、ソースは起動時に送った1つのコミットから変わらないので、準備スクリプトが最初に1回検査すればよい。
開発用のコンテナでは、作業ツリーはホストから書き換えられ、コンテナは常駐するので、起動時の検査だけでは足りない。
そこで、開発用のコンテナでは`UV_NO_SYNC=1`を設定して`uv run`の自動の同期を止め、仮想環境を変える操作を、検査してから`uv sync --frozen`を実行する同期のスクリプト`sync-env.sh`だけに限る。`sync-env.sh`も作業ツリーの`tools/train/zero/`に置く。
`mnsd_reader`のソースやminaseのクレートを変えたときも、開発者はこの同期のスクリプトで`mnsd_reader`をビルドし直す。
起動時の検査は、開発用のサービスを起動するときにも行い、失敗したらサービスを起動しない。

### レジストリと認証

イメージは、GHCRに非公開で置く（2026年11月11日の利用者の決定）。
タグには、イメージを作ったコミットの40桁のハッシュを使う。
vast.aiの機械には、タグではなく`ghcr.io/stepney141/minase-dev@sha256:<ダイジェスト>`の形でダイジェストを指定する。
タグは後から別の内容へ付け替えられるが、ダイジェストは内容そのものから決まるので、同じ指定が別の内容を指すことがない。
起動スクリプトは、作成時に指定したイメージをNASの`vast-instances.jsonl`に記録するので、どの機械がどの内容で動いたかも後から分かる。

vast.aiは、`vastai create instance`の`--login`に`'-u <GitHubの利用者名> -p <トークン> ghcr.io'`の形で渡したログインの情報で、非公開のイメージを取得する。
トークンは読み取りの権限だけを持つものとし、リポジトリの外のファイル（権限600）に置き、起動スクリプトは必須の引数でそのファイルの場所を受け取る。
`scripts/vast/common.py`の`create`に、レジストリ、利用者名、およびトークンを持つログインの引数を加え、ログインの要らない公開のイメージを使う既存のスクリプトは、ログインなしを明示して呼ぶ。
ログインなしの`create`は、`vastai`が失敗すると、コマンドの全体を含む`CalledProcessError`か`TimeoutExpired`を投げ、呼び出し元はその文字列を表示する。
`--login`を加えたままでは、この表示にトークンが入る。そこで、ログインつきの`create`はこれらの例外を、ログインの情報を含まない例外へ変換して投げ、`vastai`の標準エラーも、トークンを置き換えてから表示する。
ログインなしの`create`の例外は変えない。探索の係数の調整のスクリプトなどが、この例外から終了コードを決めているからである。
トークンはvast.aiにも渡るが、読み取りの権限だけなので、漏れても書き換えや削除はできない。

公開にしない理由は、イメージに入るTensorRTとcuDNNの再配布の条件を、NVIDIAのライセンスでまだ確かめていないからである。

### vast.aiでの起動と準備

起動スクリプトは、複数GPU化の測定で使った`launch-vast.py`を本書の測定記録のディレクトリへ写して改め、`--image`でイメージのダイジェストを、新しい必須の引数`--registry-token-file`でトークンのファイルのパスを受け取る。
SSHの接続には今と同じく、vast.aiがコンテナにSSHのサーバを用意する起動方式と、機械の公開IPへ直接つなぐ方式を使う。
ベースのイメージが今と同じなので、vast.aiのSSHの用意も今と同じく働く見込みだが、実際に借りて確かめる。
これは、起動の経路は設定を読むだけで済ませず、配備と同じ環境で実際に起動して確かめるという教訓（[ブリッジ経由の起動の教訓](../lessons/verify-bridge-launch-path.md)）に従うためである。

新しい準備スクリプトは、次の順に処理し、最後に今と同じく完了を示す`setup-done`の行を標準出力へ出す。起動スクリプトは、この行が記録に現れるのを待つ。

1. ソースの書庫を`/workspace/src`へ展開する。
2. 検査のスクリプトで、ソースの環境の入力がイメージのものと同じことを確かめる。
3. `/opt/minase-zero/env.sh`を読み込み、作業ツリーを指定した`uv sync --frozen`で`mnsd_reader`をビルドする。
4. `cargo build --release --bin minase`でminaseをビルドする。
5. 今の準備スクリプトと同じく、GPU、CPU、コンテナに割り当てられたCPUの上限（cgroupの割当て）、メモリ、ディスク、および共有メモリ（`/dev/shm`）の大きさを表示する。

aptの導入と`setup.sh`の実行は、イメージが済ませているので行わない。
共有メモリは、vast.aiの文書によれば、借りたGPUの割合に応じて自動で割り当てられる。複数GPUの学習はプロセスの間の通信に共有メモリを使い得るので、大きさを記録しておく。

実行器の`RemotePaths`は、読み込む`env.sh`の場所を機械の種類で決める。
Colab機では今と同じ`/content/src/tools/train/zero/env.sh`、vast.aiでは`/opt/minase-zero/env.sh`とする。
vast.aiの機械は、このイメージで借りることを前提にする。ベースのイメージで借りて`setup.sh`で導入する今の借り方は、vast.aiの強化学習の機械には残さない。

### 開発用のコンテナ

`compose.yml`は、1回だけ動く準備のサービス`dev-init`と、開発用のサービス`dev`の2つを持つ。
`dev`は、Dockerfileからビルドしたイメージに`minase-dev:local`と名付けて使い、起動したら`sleep infinity`で待つ。
開発者は作業ツリーの直下で`docker compose up -d dev`を実行して起動し、`docker compose exec dev bash`でシェルに入る。イメージの`/etc/bash.bashrc`に1行を足してあるので、対話シェルは起動時に`/opt/minase-zero/env.sh`を読み込む。
GHCRへ登録するときは、`minase-dev:local`にコミットのタグを付けて送る。

1つのコンテナは、1つの作業ツリーだけを扱う。
対局ハーネスは実行した場所の作業ツリーの`target/match-cache/`に実行ファイルを置き、`mnsd_reader`は独立したCargoのワークスペースとして`tools/train/zero/mnsd_reader/target/`へビルドする。どちらの出力先も、作業ツリーごとにある。
これらをホストと共有すると、ホストでビルドしたものをコンテナで使ってしまう。ホストでビルドした実行ファイルはglibc 2.44に依存し得るので、glibc 2.39のコンテナでは動かないおそれがある。
対局ハーネスは実行ファイルをコミットだけを鍵として使い回すので、ホストでビルドしたかどうかを区別しない。
そこで、扱う作業ツリーの`target/`と`tools/train/zero/mnsd_reader/target/`を、名前つきのボリュームで覆う。
ボリュームの名前はcomposeのプロジェクトの名前から付くので、プロジェクトの名前を作業ツリーごとに変え、作業ツリーの間でボリュームを共有しない。
`CARGO_TARGET_DIR`で出力先を変える方法は、対局ハーネスがビルドしたソースの`target/release/minase`を読む（`crates/minase/src/harness/commit.rs`）ので使えない。
リポジトリの本体を扱うコンテナからは`data/worktrees/`の下のworktreeも見えるが、本体のコンテナの中ではそれらのworktreeをビルドせず、対象のworktreeのコンテナを起動して扱う。

マウントするのは、扱う作業ツリーと、リポジトリの本体の`.git`の2つであり、どちらもホストと同じ絶対パスに置く。
gitは、本体の`.git/worktrees/<名前>/`と各worktreeの`.git`ファイルに、互いの場所を絶対パスで記録しているので、別のパスに置くとworktreeのgitが動かない。
対局ハーネスの`commit:`の指定も、実行した場所の作業ツリーで`git archive`を実行する。
扱わない作業ツリーはコンテナから見えないので、gitはそれらのworktreeの記録を、`git gc`のときに一定の期間を過ぎると消す（`gc.worktreePruneExpire`）。
そこで、`compose.yml`は、環境変数からgitの設定を渡す`GIT_CONFIG_COUNT`などの仕組みを使い、コンテナの中のgitに限って`gc.worktreePruneExpire=never`を設定する。
worktreeの作成、削除、および`git worktree prune`はホストだけで行い、コンテナの中では行わない。コンテナの中で作ったworktreeは、ホストから見えない場所に置かれ得るからである。
コンテナにはgitの利用者名とメールアドレスを設定しないので、コミットとpushもホストで行う。

`compose.yml`は、作業ツリーの直下の`.env`（git管理外）から、次の変数を`${変数:?}`の形で読み、変数がなければ起動を拒む。

- ホストの利用者のUIDとGID（`MINASE_UID`、`MINASE_GID`）。
- 扱う作業ツリーの絶対パス（`MINASE_TREE`）と、リポジトリの本体の`.git`の絶対パス（`MINASE_GIT_DIR`）。
- composeのプロジェクトの名前（`COMPOSE_PROJECT_NAME`）。トップレベルの`name`に使う。

リポジトリの本体の`.env`には、2026年10月の試みで書いた`GHCR_IO_TOKEN`がすでにある。composeはこのファイル全体を読むので、既存の行は残したまま上の変数を書き足す。

`dev-init`はrootで動き、新しく作られたボリューム（`target/`、`mnsd_reader/target/`、ホームのディレクトリ、およびcargoのレジストリ）の所有者を、ホストの利用者のUIDとGIDに合わせて終わる。
作業ツリーの下に覆いかぶせるボリュームは、イメージの中に同じパスがないので、Dockerがrootの所有で作る。そのままでは、ホストの利用者で動く`dev`が書き込めない。
`dev-init`は、ボリュームをかぶせる先の2つのディレクトリ（作業ツリーの`target/`と`tools/train/zero/mnsd_reader/target/`）も、ホストの利用者の所有で作る。ホストの側にこれらがないまま`dev`を起動すると、Dockerがマウント先として空のディレクトリをrootの所有でホストに作るからである。
`dev`は、`dev-init`が成功して終わってから、ホストの利用者のUIDとGIDで起動する。
`HOME`は、名前つきのボリュームに置く`/home/dev`とし、uvのキャッシュもこの下に置く。
cargoのレジストリ（`/opt/minase-zero/.runtime/cargo/registry`）も名前つきのボリュームに置き、コンテナを作り直しても取得し直さないようにする。

ほかに、次の設定を置く。

- GPUは、CDIの名前`nvidia.com/gpu=all`で、composeの`devices`に指定して渡す。作業機のDocker 29.9.2は、NVIDIA Container Toolkit 1.20.1が作った`/run/cdi/nvidia.yaml`を読み、`docker info`に`nvidia.com/gpu=all`を表示する。
- 共有メモリ（`/dev/shm`）を8 GBにする。Dockerの既定は64 MBであり、2026年10月の作業機での試みでも、PyTorchを動かすために8 GBを指定した。
- 開発用のコンテナでは`UV_NO_SYNC=1`を設定する（「環境の入力の検査」の節）。
- NASはマウントしない。NASを使う実行器と起動スクリプトはホストで動かすからである。

作業機は、対局の測定やlishogi Botの実行にも使っている。
コンテナの中のビルドと試験も作業機のCPUを使うので、[進行中の測定を確かめる教訓](../lessons/check-running-measurements-before-cpu-load.md)をホストと同じく守る。

## 実装フェーズ

コードの変更は、実装を担うAIエージェントであるcodexへ委任する（[codexへの委任の手引き](../guides/codex-delegation.md)）。
codexのサンドボックスにはネットワーク、GPU、およびDockerがないので、イメージのビルド、GPUの試験、vast.aiとColab機での確認は委任元が行う。

### フェーズ1: 環境の入力と実行器の変更

`setup.sh`と`env.sh`を「導入先を環境のディレクトリに閉じる」と「minaseとmnsd_readerは動かす機械でビルドする」の節のとおりに改め、検査のスクリプトと同期のスクリプトを`tools/train/zero/`に置く。
実行器の`RemotePaths`に読み込む`env.sh`の場所を加え、`scripts/vast/common.py`の`create`にログインの引数と例外の変換を加え、それぞれの試験を改める。
`create`の試験では、ダミーのトークンで`vastai`の異常終了と時間切れを模擬し、例外の文字列と標準エラーにトークンが出ないことを確かめる。
完了条件は、`tools/train/zero/tests`、`scripts/vast/`、および`scripts/vast-spsa/`の試験が通ることである。

### フェーズ2: イメージと開発用のコンテナ

`Dockerfile`、`.dockerignore`、および`compose.yml`を置き、作業機でイメージを作る。
`.dockerignore`は、最初にすべてを除外したうえで環境の入力だけを除外から外す形にし、ビルドのときにDockerへ送るファイル（ビルドコンテキスト）を環境の入力だけに絞る。リポジトリには`data/`や`target/`のような大きなディレクトリがあり、除外するものを列挙する形では、漏れたときにそれらを送ってしまうからである。
完了条件は、「検証」の節の作業機の確認がすべて通ることである。

### フェーズ3: GHCRとvast.ai

イメージをGHCRへ非公開で登録し、起動スクリプトと準備スクリプトを本書の測定記録のディレクトリに置く。
同じ機種のGPUを2枚積んだ機械を1台借り、本線（2回目の実行の正式な系列）とは別の名前の系列で、短い1サイクルを回す。
GPUを2枚にするのは、複数GPUの学習のプロセスの間の通信が、イメージの中でも動くことを確かめるためである。
借りる前に、出品（vast.aiで貸し手が機械を貸し出す1件）の1時間あたりの料金と見込みの時間を利用者に示して承認を得る。
完了条件は、「検証」の節のvast.aiの確認がすべて通り、測定記録を書くことである。

### フェーズ4: Colab機と文書

ColabでGPUがNVIDIA L4の機械を短時間借り、改めた`setup.sh`で導入から照合までが通ることを確かめる。
L4の消費レートは、Colabの課金の単位である計算単位（compute unit、CU）で1.54 CU/時であり、導入、ビルド、および照合が1時間に収まれば、消費は1.54 CU以下になる。
あわせて、手引き、ADR、および[ROADMAP](../ROADMAP.md)を改める。
完了条件は、「完了条件」の節を満たすことである。

## 検証

作業機の開発用のコンテナでは、次を確かめる。
ボリュームを1つも作っていない状態から始め、`dev-init`がボリュームの所有者を合わせる経路を通す。

1. `docker compose build`と`docker compose up -d dev`が通り、コンテナの中の`nvidia-smi`が作業機のGPUを表示する。
2. コンテナの中で、同期のスクリプトが`mnsd_reader`をビルドし、テストの手引きのマージ前のローカル検査（`cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、および`cargo test -p minase --lib --features search-stats,tuning`）が通る。cargoのクレートの取得とuvの同期が、新しいボリュームで権限のエラーなしに通ることも、この項目で確かめる。
3. NNUEの学習器のGPUの試験（`#[ignore = "requires a CUDA GPU and NVRTC"]`の付いた試験）が`--ignored`で通る。この学習器は、GPUで動かすCUDAのプログラムを起動時にNVRTC（NVIDIAの実行時のコンパイラ）でコンパイルし、NVRTCの共有ライブラリをcudarcというクレートで読み込む。cudarc 0.19.10は、`libnvrtc.so`などのほかに`libnvrtc.so.13`という名前でも探す。仮想環境の`nvidia/cu13/lib/`には`libnvrtc.so.13`があるので、CUDAのツールキットを入れなくても読めるはずであり、この項目でそれを確かめる。
4. `tools/train/zero/tests`の試験が通り、2回目の実行の初期ネット（NASの`cycles-rl2/cycle-0000/`を`data/`へ写したもの）の照合で、出力の差が許容誤差に収まる。
5. リポジトリの本体と、`data/worktrees/`の下にあるworktreeの1つで、それぞれコンテナを起動し、対局ハーネスの`commit:`の指定でコミットをビルドできる。実行ファイルと`mnsd_reader`のビルド出力がそれぞれのコンテナのボリュームに置かれ、ホストの`target/match-cache/`と`mnsd_reader/target/`は変わらない。worktreeのコンテナの中でも`git status`と`git log`が動く。
6. 起動したコンテナの作業ツリーで、ホストから`uv.lock`を1バイト変えると、同期のスクリプトが仮想環境を変える前に異常で終わって`uv.lock`の名前を出し、`uv run`も仮想環境を変えない。変えた`uv.lock`は確認の後に戻す。
7. コンテナの中でバインドマウントの下に作ったファイルの所有者が、ホストの利用者である。

vast.aiでは、同じ機種のGPUを2枚積んだ機械を1台借りて、次を確かめる。

8. イメージのダイジェストとログインの情報で機械を作り、`running`になるまでの時間と、準備スクリプトの時間を記録する。準備スクリプトの時間には、maturinとクレートの取得と、`mnsd_reader`とminaseのビルドが含まれる。これらの時間は、機械Hの3分52秒と機械Bの11分50秒と比べて記録するが、採否の条件にはしない。共有メモリの大きさも記録する。
9. 実行器の引数`--cycles-dir`（系列の記録とネットを置くNASのディレクトリ）に本線とは別の名前を指定し、実行器が短い1サイクルの生成、GPU 2枚の学習、および照合を終え、ファイルをNASへ受け取り、機械を削除する。
10. 起動スクリプトの標準エラー、NASの`vast-instances.jsonl`、および導入の記録に、トークンが出ていない。

Colab機では、次を確かめる。

11. 改めた`setup.sh`、作業ツリーを指定した`uv sync --frozen`、minaseのビルド、および照合が通る。

## 完了条件

本書は、次をすべて満たしたときに完了（採用）とする。

- 「検証」の節の11項目がすべて通り、vast.aiとColab機の確認の結果を測定記録`docs/measurements/docker-environment/`に書いた。
- 新しいADRに、作業機とvast.aiではイメージを使い、Colab機では`setup.sh`を使うことと、その理由を書き、実行環境の導入のADRを置き換え済みにして`docs/adr/archive/`へ移した。
- 借りる機械の操作の手引きの3節と7.1節を新しい手順に改め、開発用のコンテナの手引きを`docs/guides/`に置いて索引に加えた。
- 利用者の許可を得て、ブランチをmasterへマージした。

## 参考資料

- [実行環境の導入のADR](../adr/install-the-zero-toolchain-from-one-lock-on-every-machine.md)と[Colab機の借り方のADR](../adr/rent-colab-gpus-through-the-cli-and-keep-state-on-the-nas.md)：Colab機でコンテナを使わない理由。
- [複数GPU化のvast.aiの測定](../measurements/alphazero-multi-gpu/alphazero-multi-gpu-vast.md)：今の導入の時間と、起動スクリプトと準備スクリプトの元。
- [借りる機械の操作の手引き](../guides/alphazero-machines.md)：Colab機とvast.aiの今の手順。
- [vast.aiのDocker Execution Environment](https://docs.vast.ai/guides/instances/docker-environment)：SSHの起動方式で、`-e`で渡した環境変数がSSHのセッションから既定では見えないこと、onstartのスクリプトで`/etc/environment`へ書き出す方法、および共有メモリがGPUの割合に応じて割り当てられること（2026年11月11日参照）。
- vast.aiのCLI 1.10.0の`vastai create instance --help`：`--image`と`--login`の引数（2026年11月11日に作業機で確認）。
- uv 0.13.4の`uv help sync`と`uv help run`：`--no-install-local`（パスで指定した依存を入れない）と`UV_NO_SYNC`（`uv run`の同期を止める）。uvの公式のインストーラ（0.13.4）の`UV_UNMANAGED_INSTALL`（2026年11月11日に作業機で確認）。
- [uvのDockerの手引き](https://docs.astral.sh/uv/guides/integration/docker/#intermediate-layers)：外部の依存を先に入れ、プロジェクトの中のパッケージを後から入れる構成。
- 作業機の`docker info`（Docker 29.9.2、2026年11月11日）：CDIの仕様のディレクトリ`/etc/cdi`と`/run/cdi`、および`nvidia.com/gpu=all`の表示。
