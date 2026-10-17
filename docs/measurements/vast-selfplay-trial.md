# vast.aiでの教師生成の試走

## 目的

[vast.aiでの教師生成](../plans/vast-selfplay.md)のフェーズ0として、選定条件に合うvast.aiの機械で、1局面20,000ノードの教師1ファイル（7,500局）の生成速度、1スレッドの探索速度、および起動と取得の所要時間を測る。
これにより、並列度、期限、待ち時間の上限、および費用の見積もりを確定し、作業機での同じ測定と比べて作業機より速いかを判定する。

## コマンドライン

生成機は、2026年10月17日に選定条件で検索した出品のうち最も安い出品46480357を指定し、次のコマンドで作成した。

```console
vastai create instance 46480357 --image rust:1.98.0-bookworm --disk 50 --ssh --direct --cancel-unavail \
  --label minase-selfplay-trial-20261017 --onstart trial-onstart2.sh
```

起動時スクリプト`trial-onstart2.sh`は次のとおりであり、1行目で`/root/.ssh`の所有者と権限を直し、起動から75分後に生成機自身を停止する処理をバックグラウンドで予約する。

```sh
chown root:root /root /root/.ssh /root/.ssh/authorized_keys 2>/dev/null; chmod 700 /root /root/.ssh; chmod 600 /root/.ssh/authorized_keys
mkdir -p /workspace/job/logs
date -u +%s > /workspace/job/logs/onstart-epoch
env | grep -E '^(CONTAINER_ID|VAST_CONTAINERLABEL)=' > /workspace/job/logs/env.txt
nohup setsid bash -c 'sleep 4500; for i in 1 2 3; do curl -sS --max-time 60 -X PUT -H "Authorization: Bearer $CONTAINER_API_KEY" -H "Content-Type: application/json" -d "{\"state\":\"stopped\"}" "https://console.vast.ai/api/v0/instances/$CONTAINER_ID/" >> /workspace/job/logs/selfstop.log 2>&1; echo " try=$i $(date -u +%FT%TZ)" >> /workspace/job/logs/selfstop.log; sleep 60; done' > /dev/null 2>&1 &
```

生成機では、SSHで次を順に実行した。

```console
cd /workspace/job
git clone https://github.com/stepney141/minase.git minase && cd minase && git checkout 9f393ecc
cargo build --release --locked --bin minase
taskset -c 5 target/release/minase dev bench --depth 9 --repetitions 3
target/release/minase data selfplay generate --output /workspace/job/work/c384.mnsd --seed 8000000000 \
  --games 7500 --nodes 20000 --random-moves 0 --concurrency 384 --max-ply 4000 --hash-mb 16
target/release/minase data selfplay generate --output /workspace/job/work/c192.mnsd --seed 8000100000 \
  --games 7500 --nodes 20000 --random-moves 0 --concurrency 192 --max-ply 4000 --hash-mb 16
```

取得、停止、および削除は、作業機で次のとおり実行した。

```console
vastai copy -i ~/.ssh/id_vastai C.56375197:/workspace/job/work/c384.mnsd local:<作業機のディレクトリ>/
vastai stop instance 56375197
vastai copy -i ~/.ssh/id_vastai C.56375197:/workspace/job/work/c192.mnsd local:<作業機のディレクトリ>/
vastai destroy instance 56375197 -y
```

## エンジン

生成器は、生成機でリポジトリをcloneしてコミット`9f393ecce304ba2b8aa83abdf624562224c879b5`を取り出し、`rust:1.98.0-bookworm`のrustc 1.98.0と`target-cpu=native`でビルドした`minase`である。
実行ファイルのSHA-256は`37855edf…`である。
評価関数は埋め込みのNNUE（`network_checksum` `56ef2c93…`）、規則は`L0,P0,R1,E0`、教師探索は1局面20,000ノードである。

## 環境

生成機には、vast.aiの出品46480357をオンデマンドで借りた。
出品の機械はID 146109、カナダのブリティッシュコロンビア州にあり、検証済み、信頼度0.979、送信速度180 Mbpsである。
料金は、ディスク50 GBを含めて1時間1.0225米ドルだった。
CPUはAMD EPYC 9654の2基構成（物理192コア、論理384スレッド、NUMAノード2）、メモリは125 GBであり、搭載するRTX A2000は使っていない。
コンテナの中の`nproc`は384、cpusetは0〜383だった。
cgroupはv1であり、`cpu.cfs_quota_us`の36,864,001と`cpu.cfs_period_us`の100,000から、CPU時間の上限はCPU約368.6個分である。v2の`/sys/fs/cgroup/cpu.max`はなかった。
同時384局の生成中の`/proc/cpuinfo`の平均クロックは約2,667 MHzであり、`lscpu`の最大3,709 MHzより低い。

同じ条件での作業機の測定は、作業機で別の学習が動いていたため、まだ行っていない。

## 結果

### 生成

| 項目 | 同時384局 | 同時192局 |
|---|---:|---:|
| 基本シード | 8000000000 | 8000100000 |
| 所要時間 | 653.4秒 | 720.8秒 |
| 記録局面 | 2,748,180 | 2,738,764 |
| 記録局面/秒 | 4,206 | 3,800 |
| 探索局面/秒 | 5,548 | 5,012 |
| 探索ノードの合計 | 72,496,685,506 | 72,247,111,546 |
| 全体の探索速度 | 約1億1,100万NPS | 約1億20万NPS |
| 手数上限で破棄 | 76局 | 75局 |
| 平均手数 | 495.3 | 493.6 |
| ファイルの大きさ | 25,122,368バイト | 25,063,176バイト |
| SHA-256 | `9c4ce37b…` | `a0acfc8e…` |

同時384局は同時192局より約10%速く、全体の探索速度で見たSMTの上積みは約11%だった。
同時384局の1スレッドあたりの探索速度は約289,000 NPSであり、1スレッドだけを動かしたときの約3割である。
ロードアベレージ（1分平均）は、同時384局では終了の約55秒前に380前後から203へ下がり、同時192局では終了の約160秒前に192前後から下がり始めた。

### 探索速度、起動、および取得

`minase dev bench --depth 9 --repetitions 3`の1スレッドの探索速度は、3回の値が950,331、962,049、および950,372 NPSで、中央値は950,372 NPSだった。
作成から`running`までは、1台目が82秒（イメージの取得を含む）、後述の障害で作り直した2台目が23秒だった。
cloneは3秒、ビルドは44秒だった。

`vastai copy`は、動いている生成機から25.1 MBを12.5秒で、停止した生成機から25.1 MBを6.0秒で取得し、どちらもSHA-256が生成機で計算した値と一致した。
取得の経路は、コンテナのSSHではなく、貸し手の機械のポート27999のrsync（`vastai_kaalia@<IP>::C.<ID>/<パス>`）だった。
作業機で`minase data selfplay inspect`を実行すると、取得した同時384局のファイルを読み終えて、終了コード0で終わった。
`vastai stop instance`の約30秒後の状態は、`actual_status`が`exited`、`intended_status`が`stopped`だった。停止は済んでいたが、`actual_status`は`stopped`ではなく`exited`だった。
試走全体の費用は、作り直した1台目と転送料を含めて約0.54米ドル（残高の9.998米ドルから9.454米ドルへの減少）だった。

### 起動時の障害

1台目（`rust:1.98.0-bookworm`、SSHの起動方式）では、sshdが「Authentication refused: bad ownership or modes for file /root/.ssh/authorized_keys」として公開鍵の認証を拒み、直接接続でも中継経由でもログインできなかった。
1台目を削除し、起動時スクリプトの1行目で`/root/.ssh`の所有者と権限を直した2台目では、認証が通った。
2台目でも、vast.aiがログインごとにtmuxを開くので、端末を割り当てないSSHのコマンドが「open terminal failed: not a terminal」で失敗した。端末を割り当てたセッションで`/root/.no_auto_tmux`を作った後は、非対話のコマンドが通った。

### 過去の記録との比較

作業機の過去の記録（[初代NNUEの教師生成](nnue-replacement-generation.md)、コミット`8d0e382`、PST＋FM、同時12局）は、20ファイルを22.87時間で生成し、記録局面/秒は約705だった。
同時384局の4,206記録局面/秒はその約6.0倍だが、評価関数とコミットが異なり、作業機はlishogiのBotと共用で`nice -n 19`だったので、同じ条件の比較ではない。

## 結論

同時384局が同時192局より約10%速く、この機械では30ファイルを約5.5時間、約5.6米ドルで生成できる見込みである。
作業機より速いかの判定は、作業機での同じ条件の測定を待つ。
