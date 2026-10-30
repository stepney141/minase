# AlphaZero型探索で借りる機械の操作の手引き

本書は、AlphaZero型探索の学習と自己対局に使うGPUの機械を、開発に使う作業機から操作して借り、環境を導入し、ファイルを転送し、計算を実行し、終了するまでの手順を定める。
対象は、Google Colab（以下Colab）の有料プランで借りるGPUつきの仮想機械（以下Colab機）である。
vast.aiとRunpodで時間単位で借りるレンタル機の手順は、まだ実機で確かめていないので、最後の節に、使う前に行う実測と、それまでの規則だけを書く。
方式を選んだ理由は[Colab機の借り方のADR](../adr/rent-colab-gpus-through-the-cli-and-keep-state-on-the-nas.md)と[実行環境の導入のADR](../adr/install-the-zero-toolchain-from-one-lock-on-every-machine.md)に、手順の根拠になった実測の値は[実現性の実測の記録](../measurements/alphazero-environment/alphazero-feasibility.md)にある。

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

Colab機では、書庫を展開し、`tools/train/zero/setup.sh`で環境を導入してから、minaseをビルドする。
次のコマンドは、`colab ssh -s <名前>`で開いたColab機のシェルで実行するか、4節の方法によりバックグラウンドで起動する。
`setup.sh`は、uvのロックファイルからPythonの依存パッケージを、rustupから固定した版のRustのツールチェーンを、Microsoftの配布物からONNX Runtimeを導入し、版を表示して終わる。
uvは、学習器がMNSDを読む拡張モジュール`minase_mnsd_reader`（`tools/train/zero/mnsd_reader/`）も、固定した版のツールチェーンでビルドして導入する。
新しいColab機での所要時間は、導入がL4で81秒、A100で214秒、G4で96〜98秒、minaseのビルドが40〜82秒であった。
2026年10月28日から29日に借りた3台のL4では、依存パッケージの取得だけで2分2秒、10分52秒、14分30秒かかり、取得の時間は日と機械によって大きく変わった（[試走の記録](../measurements/alphazero-rl/alphazero-rl-trial.md)）。

```console
mkdir -p /content/src && tar -xf /content/minase.tar -C /content/src
bash /content/src/tools/train/zero/setup.sh
source /content/src/tools/train/zero/env.sh
cd /content/src && cargo +"$RUST_TOOLCHAIN" build --release --bin minase
```

`env.sh`は、ONNX Runtimeの共有ライブラリを`ORT_DYLIB_PATH`に、TensorRT、CUDA、およびcuDNNの共有ライブラリを`LD_LIBRARY_PATH`に設定する。
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

### 4.1 強化学習のサイクルを回す一連の手順

強化学習のサイクルは、次の手順で1台のColab機に数サイクルずつ回す。
2026年10月の運用で実際に使った起動スクリプトは`docs/measurements/alphazero-rl/alphazero-rl-sessions/`にある（`run-c21.sh`が最新）。

1. 学習器と実行器のコミットの書庫を作る：`git -C <worktree> archive --format=tar -o /mnt/synology_nas/minase/alphazero/src/minase-<コミット>.tar <コミット>`。
2. `colab new -s rl-cNN --gpu L4`で借り、借りた時刻と`colab usage`の残高を`rentals.jsonl`に1行足す。終了期限は借りた時刻の8時間後とする。
3. 書庫と`docs/measurements/alphazero-rl/alphazero-rl-trial/colab-setup.sh`をrsyncで`/content/`へ送り、`colab-setup.sh`を`setsid nohup`で起動する（導入は3〜17分）。
4. 起動スクリプトを、`--first-cycle`、`--last-cycle`、`--deadline`、`C=<コミット>`、およびセッション名を書き換えて`nohup setsid bash`で起動する。スクリプトは導入の完了を待って`minase_zero run-cycles`を走らせ、終了コードによらず`colab stop`し、止めた時刻と残高を`rentals.jsonl`に足す。
5. 進み具合は`/mnt/synology_nas/minase/alphazero/logs/runner-<セッション>-<時刻>.log`の末尾で見る。サイクルの結果は`cycles.jsonl`、CUの区間は`sessions.jsonl`にある。
6. 実行器が異常で止まったら、ログの末尾のエラーと、該当サイクルの`cycles/cycle-NNNN/`の状態ファイル（`*.started.json`、`*.exit`、`*.log`）を見る。不合格の出力を退けて学び直すときは、削除せずに`cycles/cycle-NNNN/rejected-<理由>/`へ移す。

各サイクルの後、作業機で次を行う（GPUが空いているとき）。

- 固定の検証用棋譜の診断：`tools/train/zero`で`uv run --frozen python -m minase_zero external-eval --checkpoint /mnt/synology_nas/minase/alphazero/cycles/cycle-NNNN/train/checkpoint.pt --data <minaseのリポジトリ>/data/nnue-lr-schedule-gen/generated-8200000.bin --games 1000 --seed 20261030 --device cuda --out /mnt/synology_nas/minase/alphazero/external/cycle-NNNN.json`。
- 10サイクルごとの進捗測定：`docs/measurements/alphazero-rl/alphazero-rl-progress-c0020/`の`check.sh`（作業機での3者の照合）と`progress-workstation.sh <新> <旧> 16 <シード> <シード> /mnt/synology_nas/minase/alphazero/progress`。`progress-workstation.sh`の後半の乱択との対局は最初の進捗測定でだけ要るもので、サイクル0の実行ディレクトリが既にあると失敗するので、次からはその部分を除いた版を使う。

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

1手の探索回数、ネットの大きさ、またはGPU種別を変えるときは、強化学習の1サイクルの所要時間とCUを、推論、探索、学習、および転送の処理ごとの実測から次の式で見積もり直す。

```text
1日あたりの学習局面数 ＝ min(GPUの毎秒の評価数, CPUの全スレッドの毎秒のシミュレーション数) × 86,400 ÷ (n ＋ 1)
1サイクルの所要時間 ＝ 100万局面の生成時間 ＋ 学習時間 ＋ 進捗測定の時間 ÷ 10
                     ＋ 機械の起動、環境の導入、ビルド、TensorRTのエンジンの作成、およびデータの転送の時間
```

nは1手の探索回数であり、n ＋ 1は1局面あたりのネットの評価の回数（根の展開の1回と、1手n回の探索）である。
初版の設定（n ＝ 32）では33回になる。
NNキャッシュのヒットと探索木の再利用による評価の回数の減少は考慮しない。
GPUの毎秒の評価数は、推論ストリーム（それぞれが自分のセッションを持って推論を繰り返すスレッド）を1本、2本、および4本にして測った最大の値とする。
推論ストリームが1本では、CPUの遅い機械でホスト側の処理が律速し、GPUの能力を測りきれないからである。
CPUの値は、全vCPUで同時に測った毎秒のシミュレーション数の合計とする。
1スレッドの値にスレッド数を掛けると、ハイパースレッディングの論理コアの分を多く見積もりすぎるからである。
学習時間は、同じGPU種別で1ステップの所要時間を測って、学習窓から抽出する局面の数（約100万）を学習する時間と、学習窓の全体（パラメータ数の10倍の局面）を読み込んで再生する時間とに分けて求める（[強化学習の実行](../plans/alphazero-rl.md)の「学習窓と局面の抽出」の節）。
進捗測定の時間は、1手800シミュレーションの200ペアに要する評価の回数（1局は平均464手として数える）を、生成と同じ律速の値で割って求める。
10で割るのは10サイクルに1回の頻度に合わせた按分であり、セッションの終了期限の判定には割らない値を使う。
1サイクルのCUは、所要時間にそのGPU種別の消費レートを掛けて求める。

処理ごとの実測には、[実現性の実測の記録](../measurements/alphazero-environment/alphazero-feasibility.md)と同じ名前のディレクトリにある`measure.sh`と`measure-trt.sh`を使い、値を`estimate-input.json`の形にまとめて`estimate.py`で計算する。
この見積もりは、並列化の効率、推論との受け渡し、記録の書き込み、および手数の上限で除く対局を含まないので、確定の判定は[強化学習の実行](../plans/alphazero-rl.md)の試走と最初のサイクルで行う。

## 7. レンタル機

vast.aiとRunpodのレンタル機の実測は、2026年10月26日の利用者の判断により、[実行環境と実現性の実測](../plans/archived/alphazero-environment.md)では行っていない。
レンタル機を初めて使う前に、候補のGPU種別の機械を短時間ずつ借り、[実現性の実測の記録](../measurements/alphazero-environment/alphazero-feasibility.md)と同じ`measure.sh`と`measure-trt.sh`による測定を行い、起動から導入とビルドを終えるまでの時間、および約5 GBの試験データの転送時間を測り、時間単価を記録する。
そのうえで6節の方法で1サイクルの料金を見積もり、実際に行った手順を本書に書き足す。

それまでは、レンタル機の扱いを次の規則で定める。
レンタル機には独自のコンテナイメージを使わず、事業者が公開するUbuntuとCUDAのイメージの上で`tools/train/zero/setup.sh`を実行する。
作業機からの操作は、vast.aiでは公式CLI`vastai`、Runpodでは公式CLI`runpodctl`で行い、ファイルの転送はSSH越しのrsyncで行う。
vast.aiでは、vast.aiの中継を通らずにSSHで接続できる出品を選び、出品が借り手に保証するCPUのスレッド数を選定の条件に含める。
Runpodでは、公開IPとTCPポートを使う完全なSSH接続ができる機械を選び、rsyncを作業機とレンタル機の両方に導入する。
借りた直後に`nproc`と、コンテナに割り当てられたCPUの上限（cgroupのCPUの割当）を確かめて記録し、生成のスレッド数をその上限に合わせる。
レンタル機にはColab機のようなセッションの上限時間がないので、承認を得た範囲のサイクルを1台で続けて回してよい。
ただし、vast.aiの出品には貸出期限があり、期限に達した機械は停止するので、サイクルを始める前に残りの貸出期間を確かめ、そのサイクルの処理と転送を終えられる場合に限って始める。
作業を終えるときは、5節と同じく必要なファイルを受け取って検査和を確かめてから、機械を破棄する（vast.aiではdestroy、Runpodではterminate）。
停止しただけの機械には記憶領域の保管料がかかり続けるからである。
vast.aiのアカウント、CLI、2要素認証、およびコンテナのCPUの割当に関する運用上の注意は、[SPSAの手引き](spsa.md)の8節にある。
