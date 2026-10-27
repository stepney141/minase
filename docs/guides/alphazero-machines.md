# AlphaZero型探索で借りる機械の操作の手引き

本書は、AlphaZero型探索の学習と自己対局に使うGPUの機械を、開発に使う作業機から操作して借り、環境を導入し、ファイルを転送し、計算を実行し、終了するまでの手順を定める。
対象は、Google Colab（以下Colab）の有料プランで借りるGPUつきの仮想機械（以下Colab機）である。
vast.aiとRunpodで時間単位で借りるレンタル機の手順は、まだ実機で確かめていないので、最後の節に現状と使う前に行うことだけを書く。
方式を選んだ理由は[実行環境と実現性の実測の設計書](../plans/alphazero-environment.md)に、手順の根拠になった実測の値は[実現性の実測の記録](../measurements/alphazero-environment/alphazero-feasibility.md)にある。

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
指定と異なるGPUが割り当てられた場合は、そのセッションを止めて借り直す（設計書の「Colab機」の節）。

```console
echo 'import subprocess; print(subprocess.run("nvidia-smi --query-gpu=name,memory.total,driver_version,compute_cap --format=csv; nproc", shell=True, capture_output=True, text=True).stdout)' \
  | colab exec -s <名前> --timeout 60
```

## 3. ソースを転送して環境を導入する

minaseのソースは、Colab機でリポジトリを取得せずに、作業機で固定したコミットから`git archive`で作った書庫を送る。

```console
git -C <worktree> archive --format=tar -o /mnt/synology_nas/minase/alphazero/minase-<コミット>.tar <コミット>
```

ファイルの転送には、`colab ssh --proxy-mode`をOpenSSHのProxyCommandにしたrsyncを使う。
`colab upload`は5 GBのファイルの転送でHTTP 500を返して失敗したので、大きなファイルには使わない。
数十MBまでのファイルなら、`colab upload`と`colab download`でも送れる。

```console
RSH="ssh -o ProxyCommand='colab ssh --proxy-mode -s <名前>' -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR"
rsync -a -e "$RSH" /mnt/synology_nas/minase/alphazero/minase-<コミット>.tar root@colab:/content/minase.tar
```

作業機の回線では、作業機からColab機への転送が約6.2 MB/秒、Colab機から作業機への転送が約9.7 MB/秒であった。

Colab機では、書庫を展開し、`tools/train/zero/setup.sh`で環境を導入してから、minaseをビルドする。
次のコマンドは、`colab ssh -s <名前>`で開いたColab機のシェルで実行するか、4節の方法によりバックグラウンドで起動する。
`setup.sh`は、uvのロックファイルからPythonの依存パッケージを、rustupから固定した版のRustのツールチェーンを、Microsoftの配布物からONNX Runtimeを導入し、版を表示して終わる。
新しいColab機での所要時間は、導入がL4で81秒、A100で214秒、G4で96〜98秒、minaseのビルドが40〜82秒であった。

```console
mkdir -p /content/src && tar -xf /content/minase.tar -C /content/src
bash /content/src/tools/train/zero/setup.sh
source /content/src/tools/train/zero/env.sh
cd /content/src && cargo +"$RUST_TOOLCHAIN" build --release --bin minase
```

`env.sh`は、ONNX Runtimeの共有ライブラリを`ORT_DYLIB_PATH`に、TensorRT、CUDA、およびcuDNNの共有ライブラリを`LD_LIBRARY_PATH`に設定する。
minaseのGPUの推論を使うコマンドは、この設定の下で実行する。

## 4. 計算を実行する

`colab exec`はColab機のPythonのカーネルでコードを実行し、既定では30秒で打ち切る。
数分を超える計算は、カーネルからPythonの`subprocess.Popen`で、カーネルとは別のプロセスグループ（`start_new_session=True`）として起動し、出力をファイルへ書き出させる。
起動したプロセスは`colab exec`の終了後も動き続ける。

```python
import subprocess
log = open('/content/job.out', 'w')
subprocess.Popen(['bash', '/content/job.sh'], stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
```

`/content/job.sh`は、実行するコマンドを並べたシェルスクリプトであり、3節の方法で前もってColab機へ送っておく。
上のPythonのコードを作業機にファイルとして保存し、`colab exec -s <名前> -f <ファイル> --timeout 120`で送って実行する。
進み具合は、出力のファイルの末尾を表示する短い`colab exec`で確かめる。

```console
echo 'import subprocess; print(subprocess.run("tail -n 20 /content/job.out", shell=True, capture_output=True, text=True).stdout)' \
  | colab exec -s <名前> --timeout 60
```

セッションには、借りた時刻からの終了期限を設ける。
期限は、進捗測定を行わないサイクルでは8時間、進捗測定を行うサイクルでは連続して動作を確かめた時間以内とする（設計書の「Colab機」の節）。
残り時間か残高が次の処理の見込みに足りなければ、新しい処理を始めずに5節の終了手順へ移る。

2026年10月27日の確認では、無人で負荷（GPUの推論とCPUの探索）をかけたL4のセッションが、借りてから12時間8分の間、Colab側に切断されずに動いた。
この間、作業機からは20分ごとに、すぐに終わる`colab exec`を実行して状態を確かめた。
このため、進捗測定を行うサイクルの終了期限は、借りた時刻から12時間とする。
Colabは上限を公表しておらず、上限は利用状況によって変わり得るので、期限を延ばす前には確認をやり直す。

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

## 6. レンタル機

vast.aiとRunpodのレンタル機の実測は、2026年10月26日の利用者の判断により、[実行環境と実現性の実測](../plans/alphazero-environment.md)のフェーズ2では行っていない。
レンタル機を初めて使う前に、設計書のフェーズ2と同じ実測を行い、その結果に基づいて本書に手順を書き足す。
vast.aiのアカウント、CLI、2要素認証、およびコンテナのCPUの割当に関する運用上の注意は、[SPSAの手引き](spsa.md)の8節にある。
