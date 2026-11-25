# 第3世代のネットに合わせた探索係数の再調整の候補の事前対局

## 目的

[SPSAの手引き](../../guides/spsa.md)の第7節に従い、[SPSAの最終値](nnue-gen3-search-retune.md)を反映した候補がSTCの条件で時間切れやエンジン異常を起こさないことを確かめ、時間の使い方の変化を記録するために、候補と起点を32ペア（64局）対局させる。
棋力の判定には使わない。

## コマンドライン

```console
target/release/minase match run \
  --run-dir data/matches/nnue-gen3-search-retune-smoke --seed 7400000000 \
  --candidate commit:2b4bbbcb --baseline commit:b74f11c4 \
  --candidate-option EvalFile=/mnt/synology_nas/minase/nnue/nets/415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9.bin \
  --baseline-option EvalFile=/mnt/synology_nas/minase/nnue/nets/415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9.bin \
  --rules lishogi --each time=10000+100 --concurrency 16 elo --pairs 32
```

手数帯ごとの集計は次のコマンドで出した。
予算に対する比の欄は使わず、手数帯ごとの実測値だけを用いた。

```console
scripts/match/clock_profile.py data/matches/nnue-gen3-search-retune-smoke \
  --budget quadruple-soft --role candidate
scripts/match/clock_profile.py data/matches/nnue-gen3-search-retune-smoke \
  --budget quadruple-soft --role baseline
```

## エンジン

候補は、SPSAの最終値を係数の表へ反映したコミット`2b4bbbcbfe3fc8522df70c4dd8cc2829ed825923`である。
起点は、探索スレッドのスタック容量の修正を含むmasterのコミット`b74f11c45e7ba6cb43062014c4c34508398bf6a7`であり、候補の親である。
ネットは両側とも標準の第3世代のA型のネット（SHA-256は`415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9`）である。
規則セットは`lishogi`（`L1,L2,P0,P3,R1,E1,E3`）である。
runnerは、masterのコミット`b74f11c4`をビルドした通常のminaseである。

## 環境

手元の測定機（Intel Core Ultra 7 265KF、物理20コア、実メモリ約33 GB）で、同時対局数16で実行した。
lishogiのBotは止めずに動かした。
経過時間は665秒である。

## 結果

有効ペア31、手数上限による破棄ペア1であった。
不正着手、クラッシュ、応答タイムアウト、時間切れ（`time_forfeits`）、および拒否着手はすべて0件である。
64局のどの側も持ち時間を使い切らなかった。

有効ペアと破棄ペアを合わせた64局の総思考時間は、候補が2,345.8秒、起点が2,323.9秒であり、起点に対する比は1.009である。
停止理由は、候補が`soft`18,285手と`hard`39手、起点が`soft`18,257手、`hard`61手、および深さ到達3手であった。

手数帯ごとの平均完了深さ、思考時間の中央値、および着手前の残り時間の中央値は次のとおりである。

| 手数帯 | 候補の深さ | 起点の深さ | 候補の思考時間（ミリ秒） | 起点の思考時間（ミリ秒） | 候補の残り時間（ミリ秒） | 起点の残り時間（ミリ秒） |
|---|---:|---:|---:|---:|---:|---:|
| 0〜50 | 15.82 | 16.55 | 111 | 91 | 9,572 | 9,794 |
| 50〜100 | 15.66 | 16.36 | 111 | 81 | 8,604 | 9,458 |
| 100〜150 | 15.39 | 16.20 | 121 | 91 | 7,309 | 8,865 |
| 150〜200 | 15.76 | 16.84 | 121 | 91 | 5,999 | 8,278 |
| 200〜300 | 15.44 | 17.21 | 111 | 101 | 4,206 | 7,528 |
| 300〜400 | 15.46 | 18.07 | 101 | 112 | 2,881 | 5,257 |
| 400〜 | 17.48 | 20.91 | 81 | 91 | 2,013 | 2,655 |

候補は200手までの序中盤で1手あたりの思考時間を長くとり、その分だけ中盤以降の残り時間が起点より少ない。
総思考時間はほぼ変わらない。
完了深さは全手数帯で候補のほうが浅い。

## 結論

時間切れとエンジン異常は0件だったので、候補を[SPRTによる棋力測定の手引き](../../guides/sprt.md)の段階ゲートへ進める。
時間の使い方と完了深さの違いは合否の条件にせず、採否の結果を解釈するための記録とする。
