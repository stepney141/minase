# AlphaZero型探索の実行環境は、作業機とvast.aiでは1つのコンテナイメージで用意し、Colab機では同じ導入スクリプトを実行する

## ステータス

承認済み。決定日は2026年11月12日。

## コンテキスト

AlphaZero型探索の学習と自己対局は、Google Colabで借りる機械（以下、Colab機）、vast.aiで借りる機械、および開発に使う作業機で動かす。
[前の決定](archive/install-the-zero-toolchain-from-one-lock-on-every-machine.md)は、独自のコンテナイメージを作らず、どの機械でも`tools/train/zero/setup.sh`を実行して、uvのロックファイルとスクリプトが固定する版のツールとパッケージを導入すると決めていた。
イメージを作る案は、Colab機で使えないことを理由に採らなかった。

この方式には2つの問題があった。
1つ目は、vast.aiの機械の導入の結果と時間が、機械や借りた日によって変わることである。
導入のたびに、apt、PyPI、NVIDIAのパッケージインデックス、GitHub、およびrustupの配布元から大きなパッケージを取得していた。
[複数GPU化の測定](../measurements/alphazero-multi-gpu/alphazero-multi-gpu-vast.md)では、導入に3分52秒かかった機械と11分50秒かかった機械があり、ONNX Runtimeの書庫の取得が毎秒約80 KBしか出ない機械もあった。
aptのパッケージは版を固定していなかったので、借りた日によって中身が変わり得た。
2つ目は、作業機（Arch Linux、glibc 2.44）で通った試験や照合が、借りた機械（Ubuntu 24.04、glibc 2.39）で通るとは限らないことである。
作業機の環境は手で更新していたので、版のずれも起きていた。

エンジンの開発者は2026年11月11日に、作業機のdocker composeとvast.aiの両方で同じイメージを使う方針と、イメージをGitHubのコンテナレジストリ（GHCR）に非公開で置く方針を決めた。
本書は、この方針を作業機、vast.aiの機械、およびColab機で確かめたうえで、決定として記録する。
設計と確認の経過は[Dockerイメージの設計書](../plans/archived/docker-environment.md)にある。

## 決定

リポジトリの直下の`Dockerfile`から1つのイメージ`minase-dev`を作り、作業機ではdocker composeの開発用のコンテナとして、vast.aiでは借りる機械のイメージとして使う。
イメージは、固定したダイジェストのベースのイメージの上で`tools/train/zero/setup.sh`を実行して作り、Colab機では今までどおり同じ`setup.sh`を実行する。
導入するツールとパッケージの版は、環境の入力（`tools/train/zero/`の`setup.sh`、`env.sh`、`pyproject.toml`、`uv.lock`、`.python-version`）と、`Dockerfile`が指定するベースのイメージのダイジェストだけに書く。

この決定には、次の4つの取り決めが伴う。

- `setup.sh`は、uv、rustup、Rustのツールチェーン、Pythonの本体、仮想環境、およびONNX Runtimeを、すべて`env.sh`が置かれたディレクトリの下に導入する。このディレクトリは、イメージでは`/opt/minase-zero`であり、ソースから`setup.sh`を実行するColab機と作業機のホストでは、作業ツリーの`tools/train/zero`である。導入先と環境変数は、`env.sh`を読み込めば決まる。
- minase本体と、学習器が教師ファイルを読む拡張モジュール`mnsd_reader`は、イメージに入れず、動かす機械で対象のコミットのソースからビルドする。
- イメージは、作ったときの環境の入力の写しを持つ。作業ツリーの環境の入力がこの写しとバイト列で一致することを`check-env-inputs.sh`で検査し、仮想環境の同期は、この検査を先に行う`sync-env.sh`だけで行う。開発用のコンテナでは、`uv run`が仮想環境を自動で書き換えないように`UV_NO_SYNC=1`を設定する。
- イメージはGHCRに非公開で置き、vast.aiの機械にはダイジェストで指定して、読み取りの権限だけを持つトークンで取得させる。

## 検討した代案

どの機械でも`setup.sh`を実行し続ける案は、コンテキストに書いた2つの問題が残るので採らなかった。

開発用と実行用の2つのイメージを作る案は、Dockerfileの保守が2倍になり、作業機で確かめた環境と借りた機械の環境が別物になるので採らなかった。

Dockerfileに版を書き、`setup.sh`とは別の手順で導入する案は、Colab機の`setup.sh`と版が食い違い得るので採らなかった。

uvやRustのツールチェーンを今までどおり`$HOME`の下に導入する案は、採らなかった。
開発用のコンテナはホストの利用者のUIDで動くので、イメージを作ったときの`$HOME`と使うときの`$HOME`が同じにならない。
導入先をイメージの環境変数で指す形も、vast.aiのSSHのセッションがイメージの環境変数を引き継ぐとは限らないので使えない。

minaseと`mnsd_reader`をイメージに入れる案は、2つの理由で採らなかった。
`.cargo/config.toml`が`target-cpu=native`を指定しているので、実行ファイルはビルドした機械のCPUに特化する。
また、`mnsd_reader`はminaseのクレートにパスで依存するので、イメージがソースの特定のコミットに結び付く。

環境の入力を検査しない案は、作業ツリーの`uv.lock`がイメージのものと違うときに、uvがイメージの仮想環境のパッケージを黙って入れ替えるので採らなかった。

イメージを公開のレジストリに置く案は、イメージに入るTensorRTとcuDNNの再配布の条件をNVIDIAのライセンスで確かめていないので採らなかった。

開発用のコンテナは作業ツリーをホストからマウントするので、ホストでビルドした実行ファイルをコンテナで使わないように、ビルド出力の`target/`をホストと分ける必要がある。
これを`CARGO_TARGET_DIR`で分ける案は、採らなかった。エンジン同士を対局させる対局ハーネスが、指定されたコミットをビルドした後、そのソースの直下の`target/release/minase`を読むからである。
代わりに、作業ツリーごとの名前つきボリュームを`target/`と`mnsd_reader/target/`にかぶせる。

Colab機でもイメージを使う案は、Colabの規約がコンテナ化を禁じ、コンテナの実行環境もないので採れない（[Colab機の借り方のADR](rent-colab-gpus-through-the-cli-and-keep-state-on-the-nas.md)）。

## 結果

vast.aiの機械の準備は、イメージの取得と、`mnsd_reader`とminaseのビルドだけになる。
aptのパッケージを含むユーザーランドは、イメージのダイジェストで固定される。
ビルドのときには、PyPIが配布するmaturinと、crates.ioが配布するクレートの取得が残る。
実測の時間は[vast.aiでの確認の記録](../measurements/docker-environment/docker-environment-vast.md)にある。

作業機では、借りた機械と同じユーザーランドで、マージ前の検査、GPUの試験、およびネットの推論の出力の照合を実行できる（[作業機での確認の記録](../measurements/docker-environment/docker-environment-workstation.md)）。
手順は[開発用のコンテナの手引き](../guides/dev-container.md)にある。

環境の入力を変えたときは、そのコミットでイメージを作り直してGHCRへ登録し直し、vast.aiの機械に指定するダイジェストを改める。
作業ツリーの環境の入力がイメージのものと違えば、開発用のコンテナは起動せず、vast.aiの準備スクリプトは止まる。

借りた機械で強化学習のサイクルを起動する実行器`minase_zero run-cycles`は、vast.aiの機械では`/opt/minase-zero/env.sh`を読み込む。
そのため、vast.aiで学習と自己対局に使う機械は、このイメージで借りる。ベースのイメージで借りて`setup.sh`で導入する方式は、vast.aiでは使えない。

作業機のホストで`env.sh`を読み込んで学習器や推論を動かすには、その作業ツリーで`setup.sh`を実行し直す必要がある。
`env.sh`が指すuvとRustのツールチェーンが、`$HOME`の下のものから、作業ツリーの`tools/train/zero/.runtime/`の下のものに変わったからである。

GPUドライバは機械ごとに異なるので、推論の結果が機械によって変わらないことは、今までどおり出力の照合で確かめる（[ネットの出力の照合の手引き](../guides/alphazero-network-check.md)）。
