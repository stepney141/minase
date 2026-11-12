# AlphaZero型探索で借りる機械の操作の手引き

本書は、AlphaZero型探索の学習と自己対局に使うGPUの機械を、開発に使う作業機から操作して借り、環境を導入し、ファイルを転送し、計算を実行し、終了するまでの手順を定める。
対象は、Google Colab（以下Colab）の有料プランで借りるGPUつきの仮想機械（以下Colab機）である。
vast.aiで時間単位で借りるレンタル機の手順は最後の節に書き、Runpodはまだ実機で確かめていないので、同じ節に使う前の規則だけを書く。
方式を選んだ理由は[Colab機の借り方のADR](../adr/rent-colab-gpus-through-the-cli-and-keep-state-on-the-nas.md)と[実行環境のADR](../adr/install-the-zero-toolchain-with-setup-sh-on-rented-machines-and-build-the-workstation-image-from-it.md)に、手順の根拠になった実測の値は[実現性の実測の記録](../measurements/alphazero-environment/alphazero-feasibility.md)にある。

## 1. 準備

作業機には、Googleが公開するColabのコマンドラインツール`colab`（google-colab-cli。版0.7.5で動作を確かめた）を導入し、`colab usage`で計算単位（compute unit、以下CU）の残高と消費レートが表示されることを確かめる。
認証はOAuthであり、トークンは`~/.config/colab-cli/token.json`に保存される。
`colab`のコマンドが401または403を返したら、利用者が端末で`colab usage`を実行して認証し直す。
`colab auth`と`colab drivemount`は端末での人手の操作を要するので、手順には使わない。

Colab機へのSSHは`colab ssh --proxy-mode`を経由し、`colab`は`~/.ssh/id_ed25519`の公開鍵を接続の要求に添えて送る。
このため、rsyncで転送する前に、対応する秘密鍵がパスフレーズなしで使えるか、ssh-agentに登録されていることを確かめる。

学習データ、チェックポイント、およびネットの保管先は、作業機にNFSでマウントしたNASの`/mnt/synology_nas/minase/alphazero/`とする。

## 2. 機械を借りる

Colab機は、名前を付けて借りる。
名前を省くとランダムな名前が付き、後のコマンドで対象のセッションを取り違えやすいからである。

```console
colab new -s <名前> --gpu L4
colab usage
```

`--gpu`に指定できるのは`T4`、`L4`、`G4`、`H100`、`A100`である。
誤った値を指定すると、`colab`は警告なしにA100を借りる。
2026年10月27日の時点で、利用者のプラン（Google AI Pro）ではL4、A100、G4を借りられ、H100は割当または利用資格がないとして拒否された。
G4のGPUはNVIDIA RTX PRO 6000 Blackwell Server Editionである。
`colab usage`が示す消費レートは、L4が1.54 CU/時、A100が5.30 CU/時、G4が8.90 CU/時であった。
L4、A100、およびG4の3台を同時に借りられた。

借りたら、GPUの種別が指定どおりかを確かめる。
指定と異なるGPUが割り当てられた場合は、そのセッションを止めて借り直す。
`colab new`が失敗した場合は仮想機械がないのでCUを消費しないが、同じ日に3回続けて指定した種別を借りられなければ、利用者へ報告して種別を決め直す。

```console
echo 'import subprocess; print(subprocess.run("nvidia-smi --query-gpu=name,memory.total,driver_version,compute_cap --format=csv; nproc", shell=True, capture_output=True, text=True).stdout)' \
  | colab exec -s <名前> --timeout 60
```

## 3. ソースを転送して環境を導入する

minaseのソースは、Colab機でリポジトリを取得せずに、作業機で固定したコミットから`git archive`で作った書庫を送る。

```console
git -C <worktree> archive --format=tar -o /mnt/synology_nas/minase/alphazero/src/minase-<コミット>.tar <コミット>
```

ファイルの転送には、`colab ssh --proxy-mode`をOpenSSHのProxyCommandにしたrsyncを使う。
`colab upload`は5 GBのファイルの転送でHTTP 500を返して失敗したので、大きなファイルには使わない。
数十MBまでのファイルなら、`colab upload`と`colab download`でも送れる。

```console
RSH="ssh -o ProxyCommand='colab ssh --proxy-mode -s <名前>' -o ControlMaster=auto -o ControlPath=/tmp/colab-cm-<名前> -o ControlPersist=900 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR"
rsync -a -e "$RSH" /mnt/synology_nas/minase/alphazero/src/minase-<コミット>.tar root@colab:/content/minase.tar
```

Colab機は同時に1本の`colab ssh`しか受け付けず、2本目の接続はHTTP 429（「Already-active SSH session」）で拒否される。
このため`RSH`は、OpenSSHのControlMasterで1本の接続を共有する設定にし、同じColab機へのsshとrsyncを同時に2つ起動しない。
2026年10月28日には、借りた直後の最初の接続が2分で切れ、やり直すと通ったことがある。

作業機の回線では、作業機からColab機への転送が約6.2 MB/秒、Colab機から作業機への転送が約9.7 MB/秒であった。
学習データは圧縮したまま送り、Colab機で展開する。

Colab機では、書庫を展開し、`tools/train/zero/setup.sh`で環境を導入してから、学習器の拡張モジュールとminaseをビルドする。
次のコマンドは、`colab ssh -s <名前>`で開いたColab機のシェルで実行するか、4節の方法によりバックグラウンドで起動する。
`setup.sh`は、uv、固定した版のRustのツールチェーン、uvのロックファイルが固定するPythonの依存パッケージ、およびMicrosoftの配布物のONNX Runtimeを導入し、版を表示して終わる。
導入先はすべて`tools/train/zero/`の下の`.runtime/`と`.venv/`であり、ホームのディレクトリには何も置かない。
学習器がMNSDを読む拡張モジュール`minase_mnsd_reader`（`tools/train/zero/mnsd_reader/`）は、`setup.sh`では入らないので、`env.sh`を読み込んだ後に`sync-env.sh`でビルドして導入する。
`setup.sh`を実行し直すと拡張モジュールが仮想環境から外れるので、その後には`sync-env.sh`も実行し直す。
新しいColab機での所要時間は、導入がL4で81秒、A100で214秒、G4で96〜98秒、minaseのビルドが40〜82秒であった。
2026年10月28日から29日に借りた3台のL4では、依存パッケージの取得だけで2分2秒、10分52秒、14分30秒かかり、取得の時間は日と機械によって大きく変わった（[試走の記録](../measurements/alphazero-rl/alphazero-rl-trial.md)）。

```console
mkdir -p /content/src && tar -xf /content/minase.tar -C /content/src
bash /content/src/tools/train/zero/setup.sh
source /content/src/tools/train/zero/env.sh
/content/src/tools/train/zero/sync-env.sh
cd /content/src && cargo build --release --bin minase
```

`env.sh`は、導入したuvとRustのツールチェーンを`PATH`に加え、`cargo`が使うツールチェーンの版を固定する。
あわせて、ONNX Runtimeの共有ライブラリを`ORT_DYLIB_PATH`に、TensorRT、CUDA、およびcuDNNの共有ライブラリを`LD_LIBRARY_PATH`に設定する。
minaseのGPUの推論を使うコマンドは、この設定の下で実行する。

## 4. 計算を実行する

数分を超える計算は、3節の`RSH`のsshで、`setsid`によって端末から切り離して起動し、標準入出力をすべてファイルへ向ける。
切り離さないと、sshが計算の終わるまで戻らない。

```console
eval "$RSH root@colab" "'setsid nohup bash /content/job.sh > /content/job.out 2>&1 < /dev/null &'"
eval "$RSH root@colab" "'tail -n 20 /content/job.out'"
```

`/content/job.sh`は、実行するコマンドを並べたシェルスクリプトであり、3節の方法で前もってColab機へ送っておく。
進み具合は、出力のファイルの末尾を表示する短いsshで確かめ、各呼び出しには`timeout`で時間の上限を付ける。
`colab exec`はColab機のPythonのカーネルでコードを実行する別の経路であり、`subprocess.Popen`で同じように切り離した起動もできるが、2026年10月28日には待機のために繰り返し呼んだ`colab exec`が応答しなくなったので、起動と待機には使わない。

強化学習のサイクルは、学習器の`minase_zero run-cycles`（`tools/train/zero/minase_zero/runner.py`）が、この節と5節の手順で、生成、学習、および出力の照合の起動、確定したファイルの受け取り、ならびにサイクルの記録の追記を順に行う。
引数と、機械を替えたときの再開の規則は、同じモジュールのdocstringにある。

セッションには、借りた時刻からの終了期限を設ける。
期限は、進捗測定を含まないセッションでは8時間、進捗測定を含むセッションでは連続して動作を確かめた時間以内とする。
8時間は、Colabのよくある質問が一般の上限とする12時間に余裕を持たせた値である。
残り時間か残高が次の処理の見込みに足りなければ、新しい処理を始めずに5節の終了手順へ移る。

2026年10月27日の確認では、無人で負荷（GPUの推論とCPUの探索）をかけたL4のセッションが、借りてから12時間8分の間、Colab側に切断されずに動いた。
この間、作業機からは20分ごとに、すぐに終わる`colab exec`を実行して状態を確かめた。
このため、進捗測定を含むセッションの終了期限は、借りた時刻から12時間とする。
Colabは上限を公表しておらず、上限は利用状況によって変わり得るので、期限を延ばす前には確認をやり直す。
一方、2026年10月29日には、実行器が19時11分（UTC）に止まった後、計算も接続もないまま置いたL4のセッションが、22時51分までにColab側に回収されていた。
回収された時刻は分からず、回収までの間の消費は止まらない。
このため、計算を止めた後に利用者の判断などで待つときは、必要なファイルを受け取ってから5節の手順でセッションを止め、待つ間に機械を借りたままにしない。

### 4.1 強化学習のサイクル

強化学習のサイクルを起動し、経過を確かめ、止めて再開する手順は、[深層強化学習を回す手引き](alphazero-training.md)にある。

## 5. 結果を受け取って終了する

結果のファイルは、Colab機で検査和を求めてから、3節で定義した`RSH`を使うrsyncでNASの一時ファイルへ受け取る。
作業機で受け取ったファイルの検査和を照合し、一致したときだけ名前を変えて確定させる。
一致しなければ確定させずに処理を止める。

```console
echo 'import subprocess; print(subprocess.run("sha256sum /content/<ファイル>", shell=True, capture_output=True, text=True).stdout)' \
  | colab exec -s <名前> --timeout 120
rsync -a -e "$RSH" root@colab:/content/<ファイル> /mnt/synology_nas/minase/alphazero/<置き場所>/.<ファイル>.tmp
sha256sum /mnt/synology_nas/minase/alphazero/<置き場所>/.<ファイル>.tmp
mv /mnt/synology_nas/minase/alphazero/<置き場所>/.<ファイル>.tmp /mnt/synology_nas/minase/alphazero/<置き場所>/<ファイル>
```

作業を終えるときは、理由によらず、必要なファイルをすべて受け取って検査和を確かめてから、セッションを止める。
止めた後に`colab usage`で消費レートが減ったことを、`colab sessions`で止め忘れたセッションがないことを確かめる。

```console
colab stop -s <名前>
colab usage
colab sessions
```

止めたセッションの仮想機械はファイルごと破棄され、CUの消費も止まる。

## 6. 1サイクルの所要時間とCUを見積もり直す

1手の探索回数、ネットの大きさ、GPU種別、またはCPUの構成を変えて、その機械の構成ではまだ測っていないときは、`minase_zero run-cycles`で1サイクルを実際に回す試走を行い、かかった時間とCUから見積もる。
処理ごとの速さを別々に測って合計する方法は使わない。
処理ごとの値の合計には、並列化の効率、推論との受け渡し、記録の書き込み、および手数の上限に達して除く対局が含まれないので、合計の値で判定できず、結局は試走が必要になるからである。

試走は本線を汚さないように、`--cycles-dir`に本線以外の名前を指定した別の系列で行う。
先例は、[学習技法の導入の試走](../measurements/alphazero-restart-techniques/alphazero-restart-techniques-trial.md)である。
1サイクルの所要時間は`cycles.jsonl`に相当する記録の`elapsed_seconds`と`interval`から、CUは同じ区間の残高の差から読む。
機械の起動、環境の導入、ビルド、およびTensorRTのエンジンの作成は、1台の機械に1回だけかかるので、1サイクルの時間とは別に数える。
進捗測定の時間は、1手800シミュレーションの200ペアを同じ機械の条件で測った値を、10サイクルに1回の頻度に合わせて10で割り、1サイクルに按分する。
ただし、セッションの終了期限の判定には割らない値を使う。

## 7. レンタル機

### 7.1 vast.ai

vast.aiの機械は、起動スクリプト`docs/measurements/alphazero-multi-gpu/launch-vast.py`で借り、導入し、実行器を走らせ、受け取って削除する。
2回目の学習では、このスクリプトを2回目の設計書の測定記録のディレクトリへ写して使う。
方式を選んだ理由は[vast.aiの実行器のADR](../adr/run-zero-cycles-on-vast-ai-until-the-deadline-and-delete-the-machine-after-a-verified-final-fetch.md)にあり、実測の値は[vast.aiの測定](../measurements/alphazero-multi-gpu/alphazero-multi-gpu-vast.md)にある。
vast.aiのアカウント、CLI、2要素認証、およびコンテナのCPUの割当に関する運用上の注意は、[SPSAの手引き](spsa.md)の8節にある。

**出品を選ぶ**

次の条件で照会し、1時間あたりの料金（`dph_total`）の安い順に選ぶ。
条件には、vast.aiが出品に載せるPCIeの帯域（`pcie_bw`）の下限を入れる。
`pcie_bw`が10.5 GB/sの機械では、推論の入出力の転送を減らした後も、生成が20.9 GB/sの機械の約0.92倍の速さにとどまり（[生成の速さの測定](../measurements/alphazero-inference-transfer/alphazero-inference-transfer-generation-speed.md)）、学習の更新は約0.59倍の速さであった（[vast.aiの測定](../measurements/alphazero-multi-gpu/alphazero-multi-gpu-vast.md)）。
同じ機械では、生成器と`minase dev nn-eval`が、出力を書き終えた後の終了処理で異常終了することもあった。この異常終了は機械の性質ではなく、ortクレートのrc.13が原因であり、rc.15へ上げて直した（[ortの版と環境の寿命のADR](../adr/hold-the-onnx-runtime-environment-until-process-exit-through-ort-rc15.md)）。出品を選ぶ条件にはしない。
`pcie_bw`は出品ごとに測り直されて変わるので、目安として使う。

```console
vastai search offers 'num_gpus=4 gpu_name=RTX_4090 reliability>0.98 verified=true rentable=true direct_port_count>=1 cuda_vers>=13.0 disk_space>=100 cpu_cores_effective>=32 cpu_ram>=128 duration>=1 pcie_bw>=20' \
  --type on-demand --raw --limit 200
```

GPUは1枚、2枚、または4枚の同じ機種に限る（学習のミクロバッチの数4をGPUの枚数で割り切るため）。
照会の結果の`gpu_ram`が、その機種の標準の値（RTX 4090なら24,564 MiB）である出品を選ぶ。
メモリを増設したRTX 4090の出品（`gpu_ram`が49,140 MiB）には、4枚のうち1枚だけメモリの表示が違う機械があり、minaseは同じ機種と同じメモリのGPUであることを起動時に検査するので、その機械では推論器を作れなかった。
NVIDIAのドライバはCUDA 13に対応する版580以上、ディスクは100 GB、貸出期限（`duration`）は回すサイクルの範囲より長くする。
安い出品はすぐに借りられるので、照会と作成の間を空けない。照会の直後でも`no_such_ask`で断られることがあり、その場合は照会し直す。
借りる前に、出品の料金と回すサイクルの範囲の見込みを利用者に示して承認を得る。

**起動する**

```console
python3 docs/measurements/alphazero-multi-gpu/launch-vast.py --offer <出品のID> --label <ラベル> \
  --image nvidia/cuda:13.0.3-base-ubuntu24.04 --hours <自己停止までの時間> --commit <40桁のコミット> \
  --root /mnt/synology_nas/minase/alphazero --salvage-dir /mnt/synology_nas/minase/alphazero/salvage \
  -- <run-cyclesの引数のうち--machine、--vast-instance、--deadline、--rootを除いたもの>
```

起動スクリプトは、作業機で`nohup setsid`により端末から切り離して走らせ、標準出力と標準エラーをファイルへ向ける。
起動スクリプトは、作成時に自己停止を予約し、自己停止の時刻と出品の貸出期限の早いほうを終了期限として実行器へ渡す。
導入は`docs/measurements/alphazero-multi-gpu/vast-setup.sh`で、パッケージの導入、`tools/train/zero/setup.sh`、および`minase`のビルドを行い、測った機械では11〜15分かかった。
GitHubからのONNX Runtimeの書庫の取得が毎秒約80 KBしか出ない機械もあった。その場合は、`env.sh`の`ORT_VERSION`の書庫を作業機で取得して機械へ送り、`env.sh`の`ORT_SHA256`と照合してから`/workspace/src/tools/train/zero/.runtime/`へ展開しておくと、`setup.sh`は取得を省く。この形にした機械の導入は約4分で終わった。
`Dockerfile`から作ったイメージをvast.aiの機械に取得させる方式は、2026年11月12日に試して採らなかった（[実行環境のADR](../adr/install-the-zero-toolchain-with-setup-sh-on-rented-machines-and-build-the-workstation-image-from-it.md)）。

機械の回線は、出品によって大きく違う。
2026年11月12日に借りた中国の機械では、機械から作業機への転送が毎秒約1.25 MBであり、PyPIとcrates.ioからの取得を含む拡張モジュールのビルドに15分かかった（[vast.aiでのイメージの確認](../measurements/docker-environment/docker-environment-vast.md)）。
料金が近い出品が複数あるときは、照会の結果の`inet_down`と`inet_up`が大きく、作業機から近い場所のものを選ぶ。
作業機からvast.aiの機械への転送は毎秒約5.5 MBであり、リプレイバッファの上限の約29 GBを新しい機械へ送るには約90分かかる。

**終わり方**

実行器が終わると、起動スクリプトは、機械の`/workspace/az`の下でNASにないファイルを`--salvage-dir`の下のインスタンスの番号のディレクトリへ受け取り、検査和を照合してから機械を削除する。
受け取りか照合に失敗した機械は削除せずに残し、起動スクリプトは異常で終わる。
この場合は、自己停止の時刻までに受け取り直して検査和を確かめてから、`scripts/vast/common.py`の`destroy`で削除する。
削除が2要素認証の切れで失敗した場合、起動スクリプトはコンテナ専用の鍵で停止を要求してから異常で終わるので、利用者に`vastai tfa login`で認証し直してもらって削除する。
SSHが使えない機械と導入に失敗した機械は、起動スクリプトが削除する。
各機械の出来事と、作成の直後と削除の直前のインスタンスの情報の全体は、NASの`vast-instances.jsonl`に残る。
1サイクルの料金は、この情報の`dph_total`（ディスクの保管料を含む1時間あたりの料金）に稼働した時間を掛け、転送料を加えて求める。

### 7.2 Runpod

Runpodの機械はまだ使っていない。
使う前に、候補のGPU種別の機械を短時間借り、起動から導入とビルドを終えるまでの時間と転送の時間を測り、時間単価を記録する。
それまでは、次の規則で扱う。
事業者が公開するUbuntuとCUDAのイメージの上で、Colab機と同じく`tools/train/zero/setup.sh`と`sync-env.sh`を実行し、作業機からは公式CLI`runpodctl`で操作し、ファイルの転送はSSH越しのrsyncで行う。
公開IPとTCPポートを使う完全なSSH接続ができる機械を選び、rsyncを作業機と機械の両方に導入する。
実行器`minase_zero run-cycles`の機械の種類は`colab`と`vast`だけなので、Runpodでは接続の部分を実行器へ足してから試走する。
作業を終えるときは、必要なファイルを受け取って検査和を確かめてから、terminateで機械を破棄する。停止しただけの機械には記憶領域の保管料がかかり続けるからである。
