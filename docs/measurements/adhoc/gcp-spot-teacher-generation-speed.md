# GCPのSpot VMでの教師生成の速さと、CPUの異なる機械の間での塊の一致

## 目的

GCPのSpot VM 2機種、c3d-highcpu-16とt2d-standard-16で、作業機での[中断と再開の照合](../resumable-nnue-teacher-generation/resumable-nnue-teacher-generation-equivalence.md)と同じ1,500局の教師生成を走らせ、同じ局数に達するまでの時間を作業機と比べる。
あわせて、CPUの異なる機械が同じ生成条件で同じ塊を作るかを、最初の塊のバイト列で確かめる。
塊は、生成器が教師ファイルの対局を250局ずつに分けて確定させる単位である。
GCPのvCPUの枠の上限により16 vCPUの機械しか借りられず、利用者が遅すぎると判断したため、生成は1,500局を終える前に止めた。

## コマンドライン

各機械では、[run.sh](gcp-spot-teacher-generation-speed/run.sh)がminaseをビルドしてから次のコマンドを実行した。
引数は、出力先を除いて、作業機の照合の生成2（[記録](../resumable-nnue-teacher-generation/resumable-nnue-teacher-generation-equivalence.md)）と同じである。

```sh
minase data selfplay generate --output-dir ~/gen --chunk-games 250 \
  --games 1500 --seed 9010000000 --nodes 20000 --random-moves 0 --max-ply 4000 --hash-mb 16 \
  --concurrency 16
```

機械は次のコマンドで作った（c3dの例）。
t2dは`--zone europe-southwest1-a --machine-type t2d-standard-16`とした。

```sh
gcloud compute instances create minase-gen-c3d --project <プロジェクトID> \
  --zone australia-southeast1-c --machine-type c3d-highcpu-16 --provisioning-model=SPOT \
  --instance-termination-action=DELETE --max-run-duration=3h \
  --image-family=ubuntu-2404-lts-amd64 --image-project=ubuntu-os-cloud \
  --boot-disk-size=30GB --boot-disk-type=pd-balanced
```

## エンジン

GCPの2台へは、コミット`95d67282a9c490cf0f8bbf96ce001af1f1f173bc`をgitのbundleで送り、各機械で`cargo build --release --locked --bin minase`によりビルドした（rustc 1.100.0、`target-cpu=native`）。
ビルドは、c3dで89秒、t2dで107秒かかった。
`target-cpu=native`で機械のCPUに合わせてビルドしたので、実行ファイルのSHA-256は、c3dが`84ca3300…414c`、t2dが`deb65f43…34bf`とCPUごとに異なる。
比較対象の作業機の生成2は、コミット`7ccb2b10`のビルドである。
規則セットは生成器が固定する`JCSA`（`L0,P0,R1,E0`）、教師の評価関数は埋め込みのNNUEである。

## 環境

| 機械 | CPU | 物理コア、論理コア | L3 | 地域 | Spot料金 |
|---|---|---|---|---|---|
| c3d-highcpu-16 | AMD EPYC 9B14 | 8、16 | 32 MiB | australia-southeast1-c | 0.0782米ドル/時 |
| t2d-standard-16 | AMD EPYC 7B13 | 16、16 | 64 MiB | europe-southwest1-a | 0.1168米ドル/時 |
| 作業機 | Intel Core Ultra 7 265KF | 20、20 | | | |

表のSpot料金は、Cloud Billing Catalog APIが2026年11月13日に有効とした、vCPUとメモリの単価の和である。
機械の`lscpu`の出力は[c3d](gcp-spot-teacher-generation-speed/c3d-lscpu.txt)と[t2d](gcp-spot-teacher-generation-speed/t2d-lscpu.txt)にある。

生成の条件は3台でそろえ、同時対局数は16、生成器のスレッドごとの置換表は16 MBとした。
GCPの2台は2026年11月14日に並行して動かし、生成のほかに重い処理は走らせず、動かした約30分の間にSpotの中断も受けなかった。

## 結果

生成器が約10秒おきに標準エラー出力へ出した進捗の行（[c3d](gcp-spot-teacher-generation-speed/c3d-generate.stderr)、[t2d](gcp-spot-teacher-generation-speed/t2d-generate.stderr)）から、終えた対局数が各値に初めて達した時刻を比べた。

| 終えた対局数 | 作業機（秒） | c3d（秒） | c3dの倍率 | t2d（秒） | t2dの倍率 |
|---:|---:|---:|---:|---:|---:|
| 250 | 180.7 | 399.9 | 2.21 | 335.3 | 1.86 |
| 500 | 348.5 | 778.7 | 2.23 | 649.8 | 1.86 |
| 750 | 524.0 | 1,159.6 | 2.21 | 969.5 | 1.85 |
| 1,000 | 682.9 | 1,539.9 | 2.25 | 1,280.3 | 1.87 |

止めた時点の最後の進捗の行では、c3dが1,596.8秒で1,046局を終えて3塊を確定させ、t2dが1,417.1秒で1,100局を終えて4塊を確定させていた。
この時点までの時間を、同じ局数に達した時点の作業機の時間で割った比は、c3dが2.23倍、t2dが1.87倍である。
作業機の生成2の1,500局は1,033.8秒であり、この比を当てはめると、7,500局の教師ファイル1つはc3dで約3.2時間、約0.25米ドル、t2dで約2.7時間、約0.31米ドルとなる。
この換算は、1,500局の前半の比が教師ファイル全体でも変わらないと仮定している。

最初の塊`chunk-00000.mnsd`（878,484バイト）を比べると、c3dとt2dの塊は`cmp`で一致し、SHA-256はどちらも`19c23e10…0d43`であった。
作業機の生成2の塊とは、`cmp -l`で異なるのが先頭から41〜80バイト目の範囲の36バイトだけであった。
この範囲はMNSDヘッダの生成コミットの欄であり、作業機のビルドとGCPのビルドのコミットの違いによる。

## 結論

16 vCPUのSpot VMでの教師生成は、作業機の同時16局に比べてc3d-highcpu-16で2.23倍、t2d-standard-16で1.87倍の時間がかかり、1教師ファイルあたりの料金はc3dが約0.25米ドル、t2dが約0.31米ドルの見込みである。
3種類のCPU（Core Ultra 7 265KF、EPYC 9B14、EPYC 7B13）は、それぞれの機械でビルドしたバイナリで同じ塊を作ったので、CPUの異なる機械へ塊を移して生成を続けても、同じ教師データになると見込める。
