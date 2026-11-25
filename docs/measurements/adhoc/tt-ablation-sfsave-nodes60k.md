# tt-ablation-sfsave-nodes60k

## 目的

同じ局面のエントリを上書きする条件をチェスエンジンStockfishの条件に替えたminaseと、現行の条件のminaseを、1手6万ノードの固定ノード数で対局させ、大きな棋力差があるかを確かめる。

## コマンドライン

`$W`は「エンジン」の節で述べるラッパースクリプトを置いたディレクトリ、`$NET`は測定の時点で標準だった第3世代の評価関数のネット（`/mnt/synology_nas/minase/nnue/nets/cbbccf6f5ee7097fd890086d2cf4bca72ed969da28353cb0c4a2f4d781acc244.bin`）である。

```console
target/release/minase match run --run-dir data/matches/tt-ablation-sfsave-nodes60k --seed 20261129 --rules lishogi \
  --candidate "$W/minase-sfsave --protocol usi" --baseline "$W/minase-sfbase --protocol usi" \
  --candidate-option EvalFile=$NET --baseline-option EvalFile=$NET \
  --each nodes=60000 --concurrency 8 elo --pairs 200
```

対局ハーネスの版は3.0.0で、ハーネスの実行ファイルのSHA-256は`89f738e320d17583a2f1cbe2c7b7f275266410390da24257c941d93e1511910c`である。

## エンジン

候補と基準は、対局ハーネスとは別にビルドした同じ実行ファイルである。
この実行ファイルは、コミット3997fa96に[診断パッチ](../../audits/transposition-table-contribution-2026-11-25/diagnostic.patch)を当てて`cargo build --release --bin minase`でビルドした。
パッチは環境変数`MINASE_TT_MODE`で置換表の使い方を切り替え、値が`full`のときは現行のまま使う。
候補のラッパースクリプト`minase-sfsave`は`MINASE_TT_MODE=sfsave`を、基準の`minase-sfbase`は`MINASE_TT_MODE=full`を設定して実行ファイルを起動する。
現行の条件は、同じ局面のエントリを、新しい結果の深さが保存済みの深さ以上のときだけ上書きする。
`sfsave`は、同じ局面のエントリを、次のいずれかの場合に上書きする（Stockfish `src/tt.cpp:127-128`、コミット3e8fde75）。

- 保存済みの記録が過去の探索の世代のものである。
- 新しい結果の境界の種類が、下界でも上界でもない正確な値である。
- 保存済みの記録が、静的評価だけを持ち探索値を持たない。
- 「新しい深さ＋2×p＋4」が保存済みの深さより大きい。pは、新しい記録に主変化（PV）の木に属するノードの印を付ける場合は1、そうでなければ0である。

`sfsave`は、エントリを上書きしない場合も、新しい記録手があれば記録手だけは書き換える。新しい記録手がないままエントリを上書きする場合は、保存済みの記録手を残す（同`123-124`）。
別の局面のエントリを置き換える条件は現行のままである。
`full`の探索はパッチを当てない版と同じであり、その確認は[置換表の寄与の調査](../../audits/transposition-table-contribution-2026-11-25.md)にある。
実行ファイルは調査の後に削除しており、そのSHA-256は記録なしである。
規則セットは`lishogi`であり、[規則書](../../rules/chu-shogi.md)の規則コードL1、L2、P0、P3、R1、E1、E3に当たる。
両エンジンのネットのSHA-256（`manifest.json`の`network_sha256`）は`cbbccf6f5ee7097fd890086d2cf4bca72ed969da28353cb0c4a2f4d781acc244`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）である。
候補と基準の探索のワーカー数は1、置換表の容量を指定するUSIオプション`USI_Hash`は両方とも256 MB、同時対局数は8である。
測定の初めの数分は、同じ機械で、保存済みの対局の着手列に沿って各局面を探索する同じ調査の処理（対局の再生）が3プロセス動いていた。
思考制限は固定ノード数なので、時間制御と時間切れの件数`time_forfeits`は該当しない。

## 結果

ペアは同じ開始局面から先後を入れ替えて指す2局であり、ペンタノミアル度数は、候補から見たペアの得点が0、0.5、1、1.5、2点だったペアの数を順に並べたものである。
200ペアを実行し、有効ペアは200、破棄ペアは0である。
ペンタノミアル度数は[43, 19, 94, 16, 28]である。
Eloの点推定は−28.7、95%信頼区間は−59.4〜+1.5である。
不正着手、クラッシュ、応答タイムアウト、および拒否着手は0件である。
累計実行時間は1,968秒である。

## 結論

Stockfishの上書き条件に替えた版の点推定は−29 Eloで、信頼区間の上端は+1.5 Eloであり、この条件を採る根拠は得られなかった。
200ペアでは数Eloの差を判定できず、固定ノード数は採否の根拠にならないので、採否を決めるには時間制御の測定が要る。
ほかの条件の結果との比較は[置換表の寄与の調査](../../audits/transposition-table-contribution-2026-11-25.md)にある。
