# 学習データの生成（順位の損失の計画）

## 目的

[浅い探索の手の順位を学ぶ損失](../plans/rank-loss-training.md)の値の回帰と順位の教師の根に使う15,000局を、現在のS0の生成器で生成し、記録と履歴の整合性と教師尺度を確かめる。
[静止探索の出力を学ぶPSTの学習](../plans/qsearch-output-training.md)が生成した15,000局は、その後にnull moveをまたぐ反復検出の修正で探索が変わったため、設計書の「依存関係」の節の規定に従って使わない。
棋力測定ではなく、対局の勝敗を使わない。

## コマンドライン

```console
cd data/rank-loss-training/generator   # c8e6fc2 に固定した worktree
P=data/rank-loss-training/data
for SEED in 94000000 94100000; do
  $P/selfplay_gen generate --output $P/generated-$SEED.bin \
    --kinds $P/generated-$SEED.kinds --history $P/generated-$SEED.history.jsonl \
    --games 7500 --seed $SEED --nodes 100000 --random-moves 0 \
    --concurrency 16 --max-ply 4000 --hash-mb 16
  $P/selfplay_gen inspect $P/generated-$SEED.bin \
    --kinds $P/generated-$SEED.kinds --history $P/generated-$SEED.history.jsonl
done
tools/train/.venv/bin/python tools/train/pst/train_qsearch.py estimate-teacher-k \
  --data $P/generated-94000000.bin $P/generated-94100000.bin
```

基本シードは、過去の生成と棋力測定のシードから対局数以上離した。

## エンジン

生成器は、ブランチ`rank-loss-training`のコミット`c8e6fc2`からビルドした`selfplay_gen`（SHA-256 `11e8abb6…`）である。
このコミットは、master `0a0171f`に静止探索の計画の記録の種類と履歴の出力を移したものであり、探索と埋め込み採用PST（検査和 `8125a343…`）はmasterと同一である。
移植後の既定ビルドは、`bench`の深さ6の15局面でノード数、最善手、および評価値がmasterと一致し、記録の種類と履歴を出力しない生成（16局、1手20,000ノード）の出力ファイルもmasterとバイト単位で一致した。
規則は`L0,P0,R1,E0`、各対局は100,000ノード、置換表16 MiB、手数上限4,000であり、静止探索の計画の学習データと同じ設定である。
生成は、生成コミットに固定したworktreeで行い、生成中にそのworktreeでは編集もコミットもしなかった。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）で、2026年9月30日17時22分から10月1日0時57分に、同時16対局で生成した。
生成の最初の約40分は、同じ測定機で判定の差mの再校正（同時4根）と実装の検証が重なった。
着手はノード数で決まるので、負荷は所要時間だけに影響する。

## 結果

| 項目 | シード94,000,000 | シード94,100,000 |
|---|---:|---:|
| 対局数 | 7,500 | 7,500 |
| 手数上限で破棄した対局 | 266 | 270 |
| 記録のある対局 | 7,218 | 7,207 |
| 記録 | 3,548,457 | 3,504,705 |
| 静かな局面 | 3,045,040 | 3,003,363 |
| 捕獲局面 | 459,874 | 458,313 |
| 成り局面 | 43,543 | 43,029 |
| 履歴から復元できなかった記録 | 0 | 0 |
| 経過時間 | 3時間42分 | 3時間52分 |

`inspect`は全記録について、種類の記録と行が対応し、保存した着手列を初期局面から再生すると記録の局面に一致することを確かめた。
静かな局面の訓練記録5,753,831件で推定した教師尺度は$K_g=763.9877$であり、静止探索の計画のデータの767.49とほぼ同じだった。
出力の尺度$K$は採用PSTの1072.6530のままである。

保存物は`data/rank-loss-training/data/`にあり、Gitの管理外である。

## 結論

15,000局の記録と履歴は整合し、フェーズ1の小標本と以後の学習に使う。
