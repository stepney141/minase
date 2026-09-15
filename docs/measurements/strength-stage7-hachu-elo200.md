# 段階7の最終構成とHaChuの固定200ペア測定

## 目的

[棋力向上段階7](../plans/strength-stage7.md)の最終構成をHaChuと固定200ペアで比較し、外部エンジンに対する棋力を進捗指標として記録する。

## コマンドライン

2026年9月14日11:45:24.594596に、日本標準時でリポジトリルートから次の測定を起動した。
保存した起動引数を起動前の期待条件と照合し、起動時のmanifestが一致することを確認した。

```console
data/strength-stage7/match_runner \
  --run-dir data/matches/strength-stage7-hachu-elo200 --seed 20930903 \
  --candidate commit:240fbfd607eb139030d92b5c718d579df62b20f6 \
  --baseline cecp:../hachu-debian/hachu \
  --rules L1,L3,P0,P5,P6,R2,E1,E2 --each time=60000+1000 \
  --candidate-hash 256 --baseline-hash 256 --concurrency 10 \
  --max-ply 4096 --response-timeout 120 \
  elo --pairs 200
```

両者の持ち時間は60秒、1手ごとの加算は1秒で、秒読みは使わない。
同じ開始局面から先後を入れ替えた2局を1ペアとし、200ペアを投入して停止する。
手数上限に達した局を含むペアは破棄し、有効200ペアになるまでの補充は行わない。
エンジン異常による負けは[測定手順](../guides/sprt.md)に従って得点へ算入し、異常が起きたことを理由にペアを除外したり補充したりしない。

## エンジン

候補は`240fbfd607eb139030d92b5c718d579df62b20f6`であり、[世代2の長時間測定](strength-stage7-gen2-ltc.md)でH1となった構成である。
本測定は固定ペア数による進捗記録であり、候補の採否を判定するものではない。

基準はH. G. Muller作のHaChuであり、ソースの`VERSION`は0.21、Debianソースコミットは`822d512180b7d94bb85a55f871445b30393f7f8a`である。
Debian版は`0.21-29-gdf26f4a-4`、上流コミットは`df26f4a7493554971440503ae788686fdcd4cf75`であり、確認時の`hachu.c`と`Makefile`はこの上流版と一致した。

| エンジン | 実行ファイル | SHA-256 |
|---|---|---|
| 候補 | `target/match-cache/240fbfd607eb139030d92b5c718d579df62b20f6/minase` | `47d357cb2021f9d5f758362950fc5b65e9e6381c7fcf2d5016df37a7edd1962b` |
| HaChu | `../hachu-debian/hachu` | `10db1299f95626fe58875b1c24a1cee6acce0eee5fb42ed8b17ce1373a9dcbdf` |

両バイナリの検査和は起動前、起動時、および終了後の照合で一致した。
HaChuのmanifest上の識別は起動コマンドであり、実行ファイルの検査和は別の起動証拠と終了証拠に保存した。
起動前に固定したソース、実行器、監査器、および`match_report`の検査和も、終了後まで不変だった。

[段階1のHaChu測定記録](strength-stage1-hachu-elo200.md)には、同じSHAのHaChuを既定MakefileによりGCCで`-O2 -s -Wall -Wno-parentheses`を指定してビルドしたと記録されている。
現バイナリのELF `.comment`節には`GCC: (GNU) 16.2.1 20260810`が残るが、当時の全引数、環境、および終了コードをまとめたビルド実行証拠は今回の確認範囲には見つからなかった。
現在のMakefileで手順を明示するなら、当該コミットをチェックアウトした別ディレクトリで次のコマンドとなる。
これは今回実行したコマンドではなく、再ビルドの成功や同一SHAの生成を確認したものではない。

```sh
make -B hachu CC=gcc CPPFLAGS= CFLAGS='-O2 -s -Wall -Wno-parentheses' LDFLAGS=
```

規則オプションはHaChuの既定値である“Okazaki rule”無効、“Promote on entry”有効、“Allow repeats”無効とし、対応する`L1,L3,P0,P5,P6,R2,E1,E2`を候補と審判層へ適用した。
固定した`data/strength-stage7/match_runner`は版0.1.0、SHA-256は`900d67a658d6075f34ef44a906e5c930ae661232f3f435be5a4de90498bd62ad`である。

## 環境

manifestのCPUはIntel Core Ultra 7 265KFで、物理20コア、論理20コア、実メモリは33,218,924,544バイトである。
候補の`Threads`は1であり、HaChuはChess Engine Communication Protocol（CECP）経由で探索ワーカー数を報告しないため、`engine_threads.baseline`は`null`として保持した。
候補の`USI_Hash`は256 MBで、HaChuには`memory 256`を送り、同時対局数は10とした。
手数上限は4,096手、応答タイムアウトは120秒である。
起動前の確認では他の重い処理がないことを確認した。

## 結果

測定プロセスは2026年9月14日16:41:48.054504に日本標準時で終了コード0となり、セッション23415の外側の起動処理も実際に終了コード0となった。
`summary.json`の`invocation_active: false`と`interrupted: false`を確認し、独立監査もセッション27677で実際に終了コード0となった。
監査の実行時間は5.519923秒である。

番号1〜200の全200ペアが保存され、全200ペアが有効、破棄は0ペアだった。
ペンタノミアル度数は`[2, 0, 34, 0, 164]`、Eloは`+391.569990`、95%信頼区間は`[+339.796887, +460.032643]`である。
正規化ペア得点の平均は0.905、分散は0.043475、平均の標準誤差は0.0147436427だった。
観測単位は200ペアであり、同じ開始局面を使う2局を独立な標本とは扱っていない。

全400局の保存上の終局分類は、審判による勝敗決着38局、投了360局、反則負け2局だった。
各局の勝者と候補側の先後から得点を再計算し、全149,687回の応答と色別の時計をナノ秒単位で照合した結果、時計の超過は0件だった。
不正着手は1件、クラッシュは1件で、いずれもHaChu側であり、候補側の異常は0件だった。
応答タイムアウト、`time_forfeits`、拒否着手はすべて0件である。
2件の反則負けは規約どおり得点へ算入し、除外や補充は行っていない。

ペア164第1局では、HaChuから着手として解釈された応答が審判の着手検証を通らず、不正着手として保存された。
生の応答行が残っていないため、表記解析の失敗か合法手との不一致かを特定できない。
直前までの1,523手の再生では局面は進行中で合法手が54手あったが、不正着手に至った原因は未特定である。
分類と保存上の限界は[第164ペアの調査](../../data/strength-stage7/hachu-elo200-pair164-illegal-move-evidence.md)、再生結果は[再生の保存記録](../../data/strength-stage7/hachu-pair164-pre-failure-replay.json)に残した。

ペア167第1局のクラッシュは、実測定runnerとの親子関係と時刻が整合するHaChuの`SIGSEGV`に対応した。
OSの故障先から逆算した添字157869は、直前の黒`7i7e`に対応するCECP表記`f4f8`の内部着手値と完全に一致した。
固定バイナリの命令順序と配列配置も、2,000要素の履歴配列の境界外書込みが手数カウンタを上書きし、次の自己着手の保存で故障した経路を強く裏づける。
既存の`force`再生は終了コード0だったが、`go`を送らず、通常の着手選択と次の自己着手保存まで進めなかったため、この経路とは矛盾しない。
内部の`ListMoves`には`Search`呼出しがあるため、`force`再生で探索関数が一度も実行されなかったという意味ではない。
OS記録、命令との対応、およびcore本体や当時の探索を再現していない限界は[第167ペアの追加証拠](../../data/strength-stage7/hachu-pair167-os-binary-evidence.md)に記録した。

基本シードは20930903で、200ペアに対応する派生前入力の範囲は`20930904..20931103`である。
起動前の[シード差分監査](../../data/strength-stage7/hachu-elo200-seed-delta-audit.json)は、それまでの監査を引き継いで172資料のメタデータと最新81件のmanifestを照合し、直前に完走した段階開始版比較との区間重複と派生シード衝突が0件であることを確認した。
この差分監査では、以前確認した155資料のハッシュを一括で再計算していない。
終了後には全200ペアと両エンジン用の保存シードを照合したが、CECP初期化ではHaChuへ乱数シードを送っていない。
保存シードの一致はハーネスの派生値の整合性を示すものであり、HaChu内部の乱数状態や時間制御対局の完全再現を保証しない。

累計実行時間は`summary.json`の`active_wall_time_ns = 17783436891249`から17,783.436891秒である。
標準出力の当該起動の経過時間は17,783.439607秒、終了記録のプロセス全体の計測時間は17,783.459915秒であり、累計時間にはsummaryの値を用いた。

独立監査は`data/strength-stage7/audit_fixed_elo.py`で共有ロックを取得し、全200ペアの得点、番号、派生シード、終局分類、5分類の異常、および全400局の時計を検査して成功した。
監査器のSHA-256は`95c85f49a3d82756f2bb14c3711da6ffe3c842774effcc6608f72a79d947a155`である。
この監査は保存結果の整合性を対象とし、全対局の合法手、開始局面の生成、および審判の終局理由を再実行したものではない。
実終了の証拠と外部バイナリの不変は別途照合し、その結果も成功した。

`match_report`も終了コード0となり、ペンタノミアル度数、Elo、95%信頼区間、および異常件数は独立監査と一致した。
同バイナリのSHA-256は`28c409b18e3902d294517593b7244a1b8e96dc1d243751738934e70cec8ddbbf`であり、実行引数と完全な出力を監査JSONへ保存した。
HaChuのワーカー数は欠測のため、`missing_engine_threads.baseline`を`true`、`maximum_engine_threads`と`leaves_one_physical_core`を`null`として保持した。
また、HaChu側のクラッシュ局でピーク常駐メモリが1件欠測となり、`maximum_game_peak_rss_bytes`、`conservative_concurrent_peak_rss_bytes`、および`conservative_memory_fraction`も`null`となった。
資源統計は`match_report`による集計であり、欠測を0で補完したり、欠測局を除いてメモリ指標を計算したりしていない。

測定条件と各ペアは`data/matches/strength-stage7-hachu-elo200/`、標準出力は`data/strength-stage7/hachu-elo200.stdout`に保存した。
起動条件は`hachu-elo200-invocation.json`、`hachu-elo200-expected-manifest.json`、`hachu-elo200-root-preflight.json`、および`hachu-elo200-startup-checks.json`で照合した。
manifestのSHA-256は`5afb643ec640554f96a027db815ff80a1595b9cb01f5b9e0f6a9da73b57af900`である。
実終了は`hachu-elo200-completion.json`、`hachu-elo200-outer-exit.json`、および`hachu-elo200-auditor-outer-exit.json`に保存した。
最終監査にはmanifest、summary、標準出力、終了証拠、および全200ペアファイルの検査和が含まれる。

| 保存結果 | SHA-256 |
|---|---|
| [独立監査](../../data/strength-stage7/hachu-elo200-audit.json) | `16541cce4b815f970414ce8bde6a394bc99ff367a998ab1ed982d71763c8c8e9` |
| [終了証拠と検査和の照合](../../data/strength-stage7/hachu-elo200-root-completion-checks.json) | `61ca22911ab9593db885d93bf51c16adfab59ce70bf7cb20c0697d7e5079e5bc` |
| [match_reportの出力](../../data/strength-stage7/hachu-elo200-match-report.json) | `768c7cfecd4f636c4ac281b3f3c24f52cdd33fe3d8b4d56d995af01422aeef51` |

## 結論

HaChu側の異常2件を規約どおり負けに算入した固定200ペアでは、段階7の構成はHaChuに対して`+391.57 Elo`、95%信頼区間`[+339.80, +460.03]`となり、変更の採否に用いない進捗指標として記録する。
