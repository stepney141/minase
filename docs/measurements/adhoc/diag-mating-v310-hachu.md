# diag-mating-v310-hachu

## 目的

minaseのv3.1.0と外部の中将棋エンジンHaChuを対局させ、minaseが駒得の優勢を得てから勝ち切るまでの手数を記録する。
minaseの3つの版を比べた結果と集計の方法は[HaChu戦で優勢から勝ち切るまでの速さの調査](../../audits/mating-speed-against-hachu-2026-11-25.md)にあり、本書は採否の判定を行わず、対局の条件と結果だけを記す。

## コマンドライン

```console
target/release/minase match run --run-dir data/matches/diag-mating-v310-hachu --seed 20261125 \
  --baseline "cecp:../hachu-debian/hachu" --rules L1,L3,P0,P5,P6,R2,E1,E2 --max-ply 1900 \
  --each time=10000+1000 --concurrency 5 \
  --candidate commit:v3.1.0 \
  --candidate-option EvalFile=/mnt/synology_nas/minase/nnue/nets/415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9.bin \
  elo --pairs 20
```

コマンドを実行した対局ハーネス（`target/release/minase`）の版は3.1.0で、その実行ファイルのSHA-256は`8af9c7888d691db8d05de2aadcf8bc456a0405ed8f5b21e5603ca4f4ec0009d7`である。

## エンジン

候補はminaseのv3.1.0であり、評価関数のネットは、USIオプション`EvalFile`で与えた世代反復の第3世代のネットである。
対局ハーネスが実行ディレクトリに書く`manifest.json`の`network_sha256`は`415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9`である。
候補の実行ファイルは、対局ハーネスがコミット`532b6e34c99eed1789ddca4d06eb1f09ecce2b55`からビルドしたもので、SHA-256は`5834ace483c8e7575152e9813102b496cc2f2df4fcaa8e48120b0692bec13975`である。
基準はHaChu（`../hachu-debian/hachu`、SHA-256は`10db1299f95626fe58875b1c24a1cee6acce0eee5fb42ed8b17ce1373a9dcbdf`、ソースのコミット`822d5121`）であり、チェスエンジンの通信規約CECPでつないだ。
規則は、HaChuの既定設定に合わせた規則コードL1、L3、P0、P5、P6、R2、E1、E2（[規則書](../../rules/chu-shogi.md)）である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）である。
minaseの探索のワーカー数は1、置換表の容量は既定の256 MBであり、同時対局数は5である。
この機械では、本調査で比べるほかの2版の対局も同時対局数5ずつで並行して動いており、機械全体では15局が同時に進んでいた。
持ち時間は10秒、1手ごとの加算は1秒である。対局ハーネスはCECPのエンジンに秒単位の持ち時間しか渡せないので、加算を秒単位にした。

## 結果

ペアは同じ開始局面から先後を入れ替えて指す2局であり、ペンタノミアル度数は、候補から見たペアの得点が0、0.5、1、1.5、2点だったペアの数を順に並べたものである。
20ペアがすべて有効で、破棄ペアはない。
ペンタノミアル度数は[0, 0, 0, 0, 20]であり、40局すべてがHaChuの投了によるminaseの勝ちである。
全勝なのでEloは推定できない。
不正着手、クラッシュ、応答タイムアウト、時間切れ、および拒否着手は、いずれも0件である。
累計実行時間は1,456秒である。
