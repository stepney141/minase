# hachu-grid-fixed1-1

## 目的

HaChu対minaseの条件格子測定（[設計書](../plans/hachu-condition-grid.md)）の1手固定時間セルとして、minaseを1手1秒の秒読み、HaChuを1手1秒の固定時間（`st 1`）で対局させ、1手固定時間という方式そのものがminaseに不利かどうかを、時間比と切り離して固定200ペアのEloで見る。
採否の判定には使わない。

## コマンドライン

```console
match_runner --run-dir data/matches/hachu-grid-fixed1-1 --seed 20560903 \
  --candidate commit:156816d --baseline "cecp:../hachu-debian/hachu" \
  --rules L1,L3,P0,P5,P6,R2,E1,E2 --concurrency 12 \
  --each time=0+0,byoyomi=1000 elo --pairs 200
```

`match_runner`と`match_report`は[fixed1-2](hachu-grid-fixed1-2.md)と同じ、コミット67679b2を固定したworktreeのバイナリ（`match_runner`のSHA-256 `014796f572b856cb04b8424145f666b3abc044c43e297c4f2bcd9b99400d9d1e`）である。
集計は`match_report --run-dir data/matches/hachu-grid-fixed1-1`のJSONを正とする。

## エンジン

候補は補間PST採用後のminaseのコミット`156816d6b3e1624e5e7809436842c1e2315824e8`（バイナリのSHA-256 `6466f222c57b46fcba082492d25b17ef3273e9d406e13f6173fca947b465cc3a`）であり、`Threads`は1である。
基準はHaChu（Debianパッケージ収録のオリジナル版、コミットdf26f4a、`../hachu-debian`で`gcc -O2`によりビルド）であり、規則オプションは既定設定（“Okazaki rule” 無効、“Promote on entry” 有効、“Allow repeats” 無効）である。
規則セットは審判層とminase側の双方に`L1,L3,P0,P5,P6,R2,E1,E2`を与えた。
シードは[time1](hachu-grid-time1.md)と同じ20560903であり、開始局面は同じ200個である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ33,218,965,504バイト）、OSはLinuxである。
候補の`Threads`は1、HaChuはワーカー数を報告しない。
`USI_Hash`は256 MB、HaChuの`memory`は256 MB、同時対局数は明示の12、手数上限は4,096手、応答タイムアウトは120秒である。
時間制御は候補が`go btime 0 wtime 0 byoyomi 1000`、基準が起動時`st 1`と毎手`time 100`（センチ秒）である。
HaChuは`st`のとき`time`の0.4倍（0.4秒）を目標とし、0.98倍（0.98秒）で探索を中断する。
測定中に他の対局、生成、学習は走らせていない。

## 結果

200ペアを実行し、有効ペア200、破棄ペア0であった。
ペンタノミアル度数は[4, 0, 46, 0, 150]、Eloの点推定は+322.7（95%信頼区間+277.0〜+379.3）であった。
不正着手、応答タイムアウト、および拒否着手は0件、クラッシュは2件、`time_forfeits`は64件である。
累計実行時間は5,428秒、有効ペア毎時は132.6、両エンジンの総CPU時間は62,735秒、1局の実時間の中央値は150秒である。

実際の1手あたりの思考時間は、minaseが中央値0.52秒、平均0.55秒、最大0.92秒で、1秒を超えた手はない。
HaChuは中央値0.31秒、平均0.37秒、最大1.39秒で、68,249手のうち64手が1秒を超えた。
`time_forfeits`64件はすべてこの64手であり、超過は0〜387 ms、うち5局は初手で起きた。
HaChuの中断上限0.98秒と時間切れ判定1秒の余裕が20 msしかなく、[fixed1-2](hachu-grid-fixed1-2.md)の40 msより狭いため、400局中64局（16%）が時間切れになった。
クラッシュ2件（ペア16第1局とペア94第2局）はHaChuが2,001手目で落ちたもので、hachu.cの棋譜配列上限`MAXMOVES 2000`による既知の挙動である。
66件はいずれもHaChuの負けとして算入した。

66局をすべてHaChuの勝ちへ振り替えた感度計算ではペンタノミアルは[15, 0, 90, 0, 95]、Elo +147.2、95%信頼区間+112.6〜+184.8である。
66局を引き分けとして扱った場合はペンタノミアルは[4, 6, 45, 50, 95]、Elo +222.4、95%信頼区間+189.1〜+259.8である。

## 結論

このセルは400局の16%がHaChuの時間切れで終わっており、名目の+322.7 Eloは強さの測定値として扱えない。
時間切れをすべてHaChuの勝ちに振り替えた下界でも+147.2 Eloであり、1手1秒固定の同条件でminaseがHaChuより強いことは言えるが、対局時計の対等条件（+207.5 Elo）との比較で固定時間がminaseに不利かどうかは、この記録からは判定できない。
HaChuの`st`を判定値と同じ秒数で使う測定は、ハーネスの時間切れ判定との余裕が不足するため、以後は行わない（[教訓](../lessons/fixed-time-forfeit-margin.md)）。
