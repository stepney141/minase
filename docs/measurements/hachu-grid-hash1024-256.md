# hachu-grid-hash1024-256

## 目的

HaChu対minaseの条件格子測定（[設計書](../plans/hachu-condition-grid.md)）の置換表比セルとして、両者に同じ持ち時間60秒＋1秒加算を与え、minaseの置換表を1,024 MB、HaChuを256 MBにしたときの強さを固定200ペアのEloで記録する。
採否の判定には使わない。

## コマンドライン

```console
match_runner --run-dir data/matches/hachu-grid-hash1024-256 --seed 20560903 \
  --candidate commit:156816d --candidate-hash 1024 \
  --baseline "cecp:../hachu-debian/hachu" --baseline-hash 256 \
  --rules L1,L3,P0,P5,P6,R2,E1,E2 --each time=60000+1000 --concurrency 12 elo --pairs 200
```

`match_runner`と`match_report`は[time1](hachu-grid-time1.md)と同じ固定worktreeのバイナリ（`match_runner`のSHA-256 `bfc301faf5c19ab7fee0dcd262f5b152f3c67a0aaf06753867f282b1e133c809`）である。
集計は`match_report --run-dir data/matches/hachu-grid-hash1024-256`のJSONを正とする。

## エンジン

候補は補間PST採用後のminaseのコミット`156816d6b3e1624e5e7809436842c1e2315824e8`（バイナリのSHA-256 `6466f222c57b46fcba082492d25b17ef3273e9d406e13f6173fca947b465cc3a`）であり、`Threads`は1である。
基準はHaChu（Debianパッケージ収録のオリジナル版、コミットdf26f4a、`../hachu-debian`で`gcc -O2`によりビルド）であり、規則オプションは既定設定（“Okazaki rule” 無効、“Promote on entry” 有効、“Allow repeats” 無効）である。
規則セットは審判層とminase側の双方に`L1,L3,P0,P5,P6,R2,E1,E2`を与えた。
シードは[time1](hachu-grid-time1.md)と同じ20560903であり、開始局面は同じ200個である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ33,218,965,504バイト）、OSはLinuxである。
候補の`Threads`は1、HaChuはワーカー数を報告しない。
`USI_Hash`は1,024 MB、HaChuの`memory`は256 MBであり、`manifest.json`の`hash_mb`に候補1024、基準256として記録されている。
同時対局数は明示の12、手数上限は4,096手、応答タイムアウトは120秒、時間制御は両者とも60秒＋1秒加算である。
測定中、minase 12プロセスの常駐メモリは合計約12.5 GB、HaChu 12プロセスは合計約3.1 GBで、実メモリの空きは約7〜9 GBを保ち、スワップの入出力は観測されなかった（`match_report`の同時常駐メモリの保守的な合計は16,482,926,592バイト）。
このセルの実行時間帯（2026年9月9日0時02分〜4時38分）のうち1時37分以降は、別worktree `minase-fm` で利用者の因数分解機械の生成と学習が並行して走っていた（ディレクトリの更新時刻による確認であり、そのCPU負荷は測っていない）。両エンジンに同じ時間制御を与えているため競合は対称に働くが、他のセルと比べて探索量が減っている可能性がある。

## 結果

200ペアを実行し、有効ペア200、破棄ペア0であった。
ペンタノミアル度数は[7, 0, 76, 0, 117]、Eloの点推定は+214.8（95%信頼区間+178.1〜+256.5）であった。
不正着手、クラッシュ、応答タイムアウト、および`time_forfeits`は0件、拒否着手は1件である。
累計実行時間は16,572秒、有効ペア毎時は43.4、両エンジンの総CPU時間は191,763秒、1局の実時間の中央値は465秒である。

拒否着手1件（ペア168第2局490手目）は、minaseの着手`7a8b`をHaChuが拒否したものである。
着手列を再現すると、この着手は486手目の局面と駒配置と手番が同じ局面を再現しており、486手目はHaChuの銅将（非獅子）がminaseの麒麟成りの獅子を取った着手であったため、審判層の規則R2は第24条第1項cの先獅子状態が異なる別の局面として着手を合法とし、駒配置だけで同一局面を判定するHaChuは既出局面の再現として拒否した。
これは[time1of4](hachu-grid-time1of4.md)で観測したものと同じ、RULES.md第31条R2の典拠欄に記す既知の判定差である。
当該局は拒否した側であるHaChuの反則負けとして算入した。
この1局をHaChuの勝ちへ振り替えた感度計算ではペンタノミアルは[7, 0, 77, 0, 116]、Elo +212.4、95%信頼区間+175.7〜+253.8であり、結論は変わらない。

## 結論

minaseの置換表を256 MBから1,024 MBへ広げても、対等条件の+207.5 Elo（[time1](hachu-grid-time1.md)）と信頼区間が重なり、この時間制御では置換表の拡大による強さの向上を検出できない。
