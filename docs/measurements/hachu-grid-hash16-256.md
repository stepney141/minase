# hachu-grid-hash16-256

## 目的

HaChu対minaseの条件格子測定（[設計書](../plans/hachu-condition-grid.md)）の置換表比セルとして、両者に同じ持ち時間60秒＋1秒加算を与え、minaseの置換表を16 MB、HaChuを256 MBにしたときの強さを固定200ペアのEloで記録する。
採否の判定には使わない。

## コマンドライン

```console
match_runner --run-dir data/matches/hachu-grid-hash16-256 --seed 20560903 \
  --candidate commit:156816d --candidate-hash 16 \
  --baseline "cecp:../hachu-debian/hachu" --baseline-hash 256 \
  --rules L1,L3,P0,P5,P6,R2,E1,E2 --each time=60000+1000 --concurrency 12 elo --pairs 200
```

`match_runner`と`match_report`は[time1](hachu-grid-time1.md)と同じ固定worktreeのバイナリ（`match_runner`のSHA-256 `bfc301faf5c19ab7fee0dcd262f5b152f3c67a0aaf06753867f282b1e133c809`）である。
集計は`match_report --run-dir data/matches/hachu-grid-hash16-256`のJSONを正とする。

## エンジン

候補は補間PST採用後のminaseのコミット`156816d6b3e1624e5e7809436842c1e2315824e8`（バイナリのSHA-256 `6466f222c57b46fcba082492d25b17ef3273e9d406e13f6173fca947b465cc3a`）であり、`Threads`は1である。
基準はHaChu（Debianパッケージ収録のオリジナル版、コミットdf26f4a、`../hachu-debian`で`gcc -O2`によりビルド）であり、規則オプションは既定設定（“Okazaki rule” 無効、“Promote on entry” 有効、“Allow repeats” 無効）である。
規則セットは審判層とminase側の双方に`L1,L3,P0,P5,P6,R2,E1,E2`を与えた。
シードは[time1](hachu-grid-time1.md)と同じ20560903であり、開始局面は同じ200個である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ33,218,965,504バイト）、OSはLinuxである。
候補の`Threads`は1、HaChuはワーカー数を報告しない。
`USI_Hash`は16 MB、HaChuの`memory`は256 MBであり、`manifest.json`の`hash_mb`に候補16、基準256として記録されている。
同時対局数は明示の12、手数上限は4,096手、応答タイムアウトは120秒、時間制御は両者とも60秒＋1秒加算である。
測定中に他の対局、生成、学習は走らせていない。

## 結果

200ペアを実行し、有効ペア200、破棄ペア0であった。
ペンタノミアル度数は[7, 0, 70, 0, 123]、Eloの点推定は+230.2（95%信頼区間+192.0〜+274.0）であった。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件である。
累計実行時間は16,439秒、有効ペア毎時は43.8、両エンジンの総CPU時間は188,614秒、1局の実時間の中央値は459秒である。

## 結論

minaseの置換表を256 MBから16 MBへ縮めても、対等条件の+207.5 Elo（[time1](hachu-grid-time1.md)）と信頼区間が重なり、この時間制御では置換表の縮小による強さの低下を検出できない。
