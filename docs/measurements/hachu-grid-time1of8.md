# hachu-grid-time1of8

## 目的

HaChu対minaseの条件格子測定（[設計書](../plans/hachu-condition-grid.md)）の時間比セルとして、HaChuを60秒＋1秒加算、minaseをその1/8の7.5秒＋0.125秒加算にしたときの強さを固定200ペアのEloで記録する。
採否の判定には使わない。

## コマンドライン

```console
match_runner --run-dir data/matches/hachu-grid-time1of8 --seed 20560903 \
  --candidate commit:156816d --candidate-limit time=7500+125 \
  --baseline "cecp:../hachu-debian/hachu" --baseline-limit time=60000+1000 \
  --rules L1,L3,P0,P5,P6,R2,E1,E2 --concurrency 12 elo --pairs 200
```

`match_runner`は[time1](hachu-grid-time1.md)と同じ固定worktreeのバイナリ（SHA-256 `bfc301faf5c19ab7fee0dcd262f5b152f3c67a0aaf06753867f282b1e133c809`）である。
候補と基準の時間制御が異なるため`match_report`は使えず、標準出力の要約を記録の値とし、累計実行時間は`summary.json`の`active_wall_time_ns`を使う。

## エンジン

候補は補間PST採用後のminaseのコミット`156816d6b3e1624e5e7809436842c1e2315824e8`（バイナリのSHA-256 `6466f222c57b46fcba082492d25b17ef3273e9d406e13f6173fca947b465cc3a`）であり、`Threads`は1である。
基準はHaChu（Debianパッケージ収録のオリジナル版、コミットdf26f4a、`../hachu-debian`で`gcc -O2`によりビルド）であり、規則オプションは既定設定（“Okazaki rule” 無効、“Promote on entry” 有効、“Allow repeats” 無効）である。
規則セットは審判層とminase側の双方に`L1,L3,P0,P5,P6,R2,E1,E2`を与えた。
シードは[time1](hachu-grid-time1.md)と同じ20560903であり、開始局面は同じ200個である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ33,218,965,504バイト）、OSはLinuxである。
候補の`Threads`は1、HaChuはワーカー数を報告しない。
`USI_Hash`は256 MB、HaChuの`memory`は256 MB、同時対局数は明示の12、手数上限は4,096手、応答タイムアウトは120秒である。
時間制御は候補が7.5秒＋0.125秒加算、基準が60秒＋1秒加算である。
測定中に他の対局、生成、学習は走らせていない。

## 結果

200ペアを実行し、有効ペア200、破棄ペア0であった。
ペンタノミアル度数は[117, 0, 72, 0, 11]、Eloの点推定は−205.0（95%信頼区間−248.0〜−167.1）であった。
不正着手、応答タイムアウト、`time_forfeits`、および拒否着手は0件、クラッシュは1件である。
累計実行時間は8,738秒、1局の実時間の中央値は240秒である。
保存された着手時間から候補の時計を再構成すると、全400局を通じた残り時間の最小値は349 ms（局ごとの最小値の中央値は424 ms）であり、時間切れは出なかったが余裕は小さい。

クラッシュ1件（ペア54第2局）はHaChuが2,002手目で落ちたもので、hachu.cの棋譜配列上限`MAXMOVES 2000`による既知の挙動であり、当該局はHaChuの負けとして算入した。
この1局をHaChuの勝ちへ振り替えた感度計算ではペンタノミアルは[117, 0, 73, 0, 10]、Elo −207.5、95%信頼区間−250.2〜−169.7であり、結論は変わらない。

## 結論

minaseの持ち時間を1/8にすると−205.0 Eloとなり、対等条件の+207.5 Elo（[time1](hachu-grid-time1.md)）とほぼ対称の位置に来る。
時間比の4セルを並べると、対等条件から1/2、1/4、1/8へ縮めるごとにEloは+207.5、+105.6、−26.1、−205.0と下がり、持ち時間を半分にするごとの低下は約100〜180 Eloである。
