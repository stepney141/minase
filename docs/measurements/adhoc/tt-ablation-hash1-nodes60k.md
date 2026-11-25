# tt-ablation-hash1-nodes60k

## 目的

置換表の容量を1 MBにしたminaseと、既定の256 MBのminaseを、1手6万ノードの固定ノード数で対局させ、容量の不足が棋力を下げる量をEloで推定する。

## コマンドライン

`$W`は「エンジン」の節で述べるラッパースクリプトを置いたディレクトリ、`$NET`は測定の時点で標準だった第3世代の評価関数のネット（`/mnt/synology_nas/minase/nnue/nets/cbbccf6f5ee7097fd890086d2cf4bca72ed969da28353cb0c4a2f4d781acc244.bin`）である。

```console
target/release/minase match run --run-dir data/matches/tt-ablation-hash1-nodes60k --seed 20261127 --rules lishogi \
  --candidate "$W/minase-full --protocol usi" --baseline "$W/minase-full --protocol usi" --candidate-hash 1 --baseline-hash 256 \
  --candidate-option EvalFile=$NET --baseline-option EvalFile=$NET \
  --each nodes=60000 --concurrency 5 elo --pairs 150
```

対局ハーネスの版は3.0.0で、ハーネスの実行ファイルのSHA-256は`89f738e320d17583a2f1cbe2c7b7f275266410390da24257c941d93e1511910c`である。

## エンジン

候補と基準は、対局ハーネスとは別にビルドした同じ実行ファイルである。
この実行ファイルは、コミット3997fa96に[診断パッチ](../../audits/transposition-table-contribution-2026-11-25/diagnostic.patch)を当てて`cargo build --release --bin minase`でビルドした。
パッチは環境変数`MINASE_TT_MODE`で置換表の使い方を切り替え、値が`full`のときは現行のまま使う。
候補と基準はどちらも、`MINASE_TT_MODE=full`を設定するラッパースクリプト`minase-full`から起動した。
候補と基準の違いは置換表の容量だけであり、候補の1 MBの表は65,536エントリ、基準の256 MBの表は16,777,216エントリを持つ。
`full`の探索はパッチを当てない版と同じであり、その確認は[置換表の寄与の調査](../../audits/transposition-table-contribution-2026-11-25.md)にある。
実行ファイルは調査の後に削除しており、そのSHA-256は記録なしである。
規則セットは`lishogi`であり、[規則書](../../rules/chu-shogi.md)の規則コードL1、L2、P0、P3、R1、E1、E3に当たる。
両エンジンのネットのSHA-256（`manifest.json`の`network_sha256`）は`cbbccf6f5ee7097fd890086d2cf4bca72ed969da28353cb0c4a2f4d781acc244`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）である。
候補と基準の探索のワーカー数は1、置換表の容量を指定するUSIオプション`USI_Hash`は候補が1 MB、基準が256 MB、同時対局数は5である。
調査を始めた時点で同じ機械ではlishogiのBot（ワーカー数4のminaseが2プロセス）が動いていた。
測定の間は、同じ調査のほかの固定ノード数の対局と、保存済みの対局の着手列に沿って各局面を探索する処理（対局の再生）も動いていた。
思考制限は固定ノード数なので、時間制御と時間切れの件数`time_forfeits`は該当しない。

## 結果

ペアは同じ開始局面から先後を入れ替えて指す2局であり、ペンタノミアル度数は、候補から見たペアの得点が0、0.5、1、1.5、2点だったペアの数を順に並べたものである。
150ペアを実行し、有効ペアは150、破棄ペアは0である。
ペンタノミアル度数は[29, 17, 60, 16, 28]である。
Eloの点推定は−3.5、95%信頼区間は−40.3〜+33.3である。
不正着手、クラッシュ、応答タイムアウト、および拒否着手は0件である。
累計実行時間は2,714秒である。

## 結論

1手6万ノードでは、置換表を256分の1の1 MBへ縮めても棋力の低下を検出できず、低下があっても40 Eloを超えない。
ほかの条件の結果との比較は[置換表の寄与の調査](../../audits/transposition-table-contribution-2026-11-25.md)にある。
