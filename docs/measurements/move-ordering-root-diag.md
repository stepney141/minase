# move-ordering-root-diag

## 目的

[手の順序付けの改善](../plans/move-ordering.md)の候補7（根の手の前回反復の情報による整列）と候補8（history malusの再測定）について、候補5までを積み上げた構成で設計書「診断の定義」の末尾2段落の診断を行い、両候補を実装するかどうかを決める。

## コマンドライン

候補5までを積み上げたコミット`a57a09e`へ計測差分[ordering-diag-c.patch](move-ordering-root-diag/ordering-diag-c.patch)を当て、フィーチャ`search-stats`を有効にしたビルドで行った。差分は`search-stats`を無効にした構成では一切コンパイルされず、計数は探索の判断に使わない。

```console
git apply docs/measurements/move-ordering-root-diag/ordering-diag-c.patch
cargo build --release --bin minase
cargo build --release --features search-stats --bin minase --target-dir target/diag

# 候補8（bench。深さ6を判定に、深さ8を補助に使う）
MINASE_ORDERING_DIAG=bench-depth6.json taskset -c 3 target/diag/release/minase dev bench --depth 6
MINASE_ORDERING_DIAG=bench-depth8.json taskset -c 3 target/diag/release/minase dev bench --depth 8

# 候補7（標準の時間制御の予算での探索。環境変数はスクリプトが設定する）
python3 docs/measurements/move-ordering-root-diag/tc_diag.py --binary target/diag/release/minase --tc stc --count 200 --seed 20271005 --out tc-stc-200.json
python3 docs/measurements/move-ordering-root-diag/tc_diag.py --binary target/diag/release/minase --tc ltc --count 200 --seed 20271006 --out tc-ltc-200.json
```

時間制御の予算での探索は、[段階12の`tc_diag.py`](strength-stage12-activation-diag/tc_diag.py)を本診断のキーと記録に合わせて改めた[tc_diag.py](move-ordering-root-diag/tc_diag.py)で行った。局面は[fm-scale-spsa-stc](fm-scale-spsa-stc.md)と[fm-scale-spsa-ltc](fm-scale-spsa-ltc.md)の保存記録から固定シードで200局を選び、各局の10手目から最終手の1手前までの局面を1つ一様に選んだ。残り時間は、同じ記録から時計を再構成した候補側の手数帯ごとの中央値（STCで9,796、8,605、6,973、5,377、3,252、1,487、1,369ミリ秒、LTCで58,086、51,828、44,102、36,032、24,236、12,850、7,128ミリ秒。手数帯は0〜49、50〜99、100〜149、150〜199、200〜299、300〜399、400以上）、加算はSTC 100ミリ秒、LTC 200ミリ秒とした。各局面の結果は[tc-stc-200.json](move-ordering-root-diag/tc-stc-200.json)と[tc-ltc-200.json](move-ordering-root-diag/tc-ltc-200.json)、集計は[tc-stc-200.log](move-ordering-root-diag/tc-stc-200.log)と[tc-ltc-200.log](move-ordering-root-diag/tc-ltc-200.log)にある。

## エンジン

コミット`a57a09e`（候補1から候補5までを積み上げた構成）に計測差分を当てたビルド。規則は`L0,P0,R1,E0`、`Threads`は1、`USI_Hash`は既定の256 MB。計測が受動的であることは、`bench`深さ6と深さ8の15局面で到達深さ、ノード数、最善手、評価値が計測なしの同じコミットのビルドと一致することで確かめた（総ノード数は深さ6で816,626、深さ8で2,567,996）。`cargo test --features search-stats`は588件が通った。

## 環境

2026年10月10日、Intel Core Ultra 7 265KF（物理20コア、論理20コア）。`bench`はCPU 3へ固定した逐次実行、時間制御の予算での探索はSTCとLTCの2プロセスを同時に、各プロセスの中では局面を逐次に実行した。lishogi Botのminaseが2プロセス同居していた。時間制御の探索は、STCで200局面のうち197がsoft limit、3がhard limit、LTCで200すべてがsoft limitで終了した。対局ではないので、ペンタノミアル度数、判定、破棄ペア数、異常件数、`time_forfeits`は該当しない。

## 結果

### 候補7　根の手の整列

| 指標 | STC（200局面） | LTC（200局面） |
|---|---:|---:|
| 完了した反復（深さ1を含む） | 1,761 | 2,102 |
| 直前の反復と最善手が交替した反復 | 610 | 727 |
| 交替率（深さ2〜4、5〜6、7〜8、9以上） | 37.7%、44.8%、40.6%、29.4% | 39.3%、41.0%、40.4%、33.0% |
| 交替した反復のノード数が全反復のノード数に占める割合 | 20,491,591／38,244,067 = **53.6%** | 62,952,933／117,093,950 = **53.8%** |
| 新しい最善手の順位の合計（現行の整列／前回反復の部分木ノード数による整列） | 15,100／11,417（平均24.8／18.7） | 15,900／12,342（平均21.9／17.0） |
| 候補の整列の順位が現行より前／後／同じ | 250／295／65 | 316／346／65 |
| 根の零窓から全窓への読み直し（回数、ノード数、全ノードに占める割合） | 3,338回、8,074,523（21.1%） | 4,020回、18,548,336（15.8%） |

`bench`深さ6では、90反復のうち40が交替し、交替した反復のノード数は全体の82.8%、順位の合計は現行1,589に対して候補1,023、前／後／同じは24／14／2だった。

### 候補8　history malus

影のbutterfly history表は、βカットを起こした静かな手に`+depth²`、同じノードで先に探索した静かな手に`−depth²`を重力つきの更新式（上限`HistoryLimit`）で加え、`go`ごとに初期化した。

| 指標 | bench深さ6 | bench深さ8 | STC | LTC |
|---|---:|---:|---:|---:|
| 静かな手の段を開いたノードのうち、影の表で順序が変わるノード | 15,684／16,990 = **92.3%** | 97.2% | 98.4% | 99.0% |
| `lmr_reduction`を計算した手のうち、影の値で減深量が変わる手 | 1,761／301,469 = 0.58% | 1.96% | 2.17% | 3.64% |
| 同、影の値が`−LmrHistoryThreshold`以下で減深が1増える経路に入る手 | 11,722（3.9%） | 88,815（8.3%） | 1,852,267（9.9%） | 10,765,199（20.0%） |
| 影の表の参照のうち値が負のもの | 53.2% | 63.7% | 58.6% | 64.7% |

## 結論

候補7は、交替した反復のノード比が53.6〜53.8%で基準5%を満たし、新しい最善手の順位の合計は前回反復の部分木ノード数による整列の方が現行より小さい（平均で18.7対24.8、17.0対21.9）ので実装へ進める。ただし反復ごとに数えると候補の順位が後になる反復（295、346）が前になる反復（250、316）よりやや多く、改善は大きく前へ動く少数の反復に偏る。
候補8は、影の表による順序の変更が92〜99%のノードで起こり、減深量の変更（0.6〜3.6%）とは別に順序の差が十分にあるので実装へ進める。負のhistory値で減深が1増える経路は、積み上げ構成でLMRを計算した手の3.9〜20.0%で動くことになる。
