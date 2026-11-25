# tt-ablation-off-nodes60k

## 目的

置換表の照会と保存をすべて止めたminaseと、止めていないminaseを、1手6万ノードの固定ノード数で対局させ、置換表の全体が棋力に寄与する量をEloで推定する。

## コマンドライン

`$W`は「エンジン」の節で述べるラッパースクリプトを置いたディレクトリ、`$NET`は測定の時点で標準だった第3世代の評価関数のネット（`/mnt/synology_nas/minase/nnue/nets/cbbccf6f5ee7097fd890086d2cf4bca72ed969da28353cb0c4a2f4d781acc244.bin`）である。

```console
target/release/minase match run --run-dir data/matches/tt-ablation-off-nodes60k --seed 20261125 --rules lishogi \
  --candidate "$W/minase-off --protocol usi" --baseline "$W/minase-full --protocol usi" \
  --candidate-option EvalFile=$NET --baseline-option EvalFile=$NET \
  --each nodes=60000 --concurrency 6 elo --pairs 150
```

対局ハーネスの版は3.0.0で、ハーネスの実行ファイルのSHA-256は`89f738e320d17583a2f1cbe2c7b7f275266410390da24257c941d93e1511910c`である。

## エンジン

候補と基準は、対局ハーネスとは別にビルドした同じ実行ファイルである。
この実行ファイルは、コミット3997fa96に[診断パッチ](../../audits/transposition-table-contribution-2026-11-25/diagnostic.patch)を当てて`cargo build --release --bin minase`でビルドした。
パッチは環境変数`MINASE_TT_MODE`で置換表の使い方を切り替え、値が`full`のときは現行のまま使う。
候補のラッパースクリプト`minase-off`は`MINASE_TT_MODE=off`を、基準の`minase-full`は`MINASE_TT_MODE=full`を設定して実行ファイルを起動する。
`off`では照会が常に外れ、保存は何も書かない。
`full`の探索はパッチを当てない版と同じであり、その確認は[置換表の寄与の調査](../../audits/transposition-table-contribution-2026-11-25.md)にある。
実行ファイルは調査の後に削除しており、そのSHA-256は記録なしである。
規則セットは`lishogi`であり、[規則書](../../rules/chu-shogi.md)の規則コードL1、L2、P0、P3、R1、E1、E3に当たる。
両エンジンのネットのSHA-256（`manifest.json`の`network_sha256`）は`cbbccf6f5ee7097fd890086d2cf4bca72ed969da28353cb0c4a2f4d781acc244`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）である。
候補と基準の探索のワーカー数は1、置換表の容量を指定するUSIオプション`USI_Hash`は両方とも256 MB、同時対局数は6である。
調査を始めた時点で同じ機械ではlishogiのBot（ワーカー数4のminaseが2プロセス）が動いていた。
測定の間は、同じ調査のほかの固定ノード数の対局と、保存済みの対局の着手列に沿って各局面を探索する処理（対局の再生）も動いていた。
思考制限は固定ノード数なので、時間制御と時間切れの件数`time_forfeits`は該当しない。

## 結果

ペアは同じ開始局面から先後を入れ替えて指す2局であり、ペンタノミアル度数は、候補から見たペアの得点が0、0.5、1、1.5、2点だったペアの数を順に並べたものである。
150ペアを予定して始め、52ペアが終わった時点で調査者が止めた。
有効ペアは51、破棄ペアは1である。破棄したペアは、1局が手数上限の4,096手に達したものである。
ペンタノミアル度数は[43, 3, 4, 1, 0]である。
Eloの点推定は−453.1、95%信頼区間は−664.0〜−353.1であり、ハーネスが固定局数の測定で出す値と同じ式で、保存記録から計算した。
保存記録の終局の理由に不正着手、クラッシュ、応答タイムアウト、および拒否着手はなく、いずれも0件である。
累計実行時間は760秒である。

## 結論

置換表を止めた版は51ペアのうち43ペアで2局とも負けており、1手6万ノードでは置換表の全体が約450 Eloに当たる。
途中で止めたのは差が明らかだったためであり、逐次検定による停止ではない。
ほかの条件の結果との比較は[置換表の寄与の調査](../../audits/transposition-table-contribution-2026-11-25.md)にある。
