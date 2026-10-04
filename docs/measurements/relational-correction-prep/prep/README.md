# 関係補正項の測定準備

2026年10月1日に、採用PSTの分割と標本抽出を検証し、近接2駒で分類が一致した15根の代理診断を実行した。
Gの校正と指定先への保存は、サンドボックスの書き込み制限により未完了である。
G*は未確定なので、倍率の判定にはまだ進めない。

指標と測定条件は、[関係補正項の計画](../../../plans/relational-correction.md)の「利用者の判断による測定」と「誤りを分類する」、および[相対2駒評価の計画](../../../plans/relative-pair-eval.md)の「自己選択の大きさを事前の基準で診断し、倍率を1つに決める」に従う。

## 分割と標本の検証

`lookahead-pst.toml`の11ファイルを`Dataset`で読み、学習器と同じ先読み指定、再評価なし、混合比の上書きなしで分割した。
訓練は23,879,611局面、検証は1,241,072局面で、指定された見込みと一致した。
ファイル別の件数と入力設定のSHA-256は[samples-check.json](samples-check.json)に保存した。

検証分割から100,000局面と10,000局面をそれぞれ非復元抽出し、標本内の重複がなく、全件が検証分割に属することを確認した。
各抽出で`numpy.random.default_rng(1).choice(validation_indices, size, replace=False)`を独立に実行した。
NumPyは2.5.3で、2標本間には778局面の重複があった。
標本間の重複を除く指定はないため、そのまま保持する。

抽出した大域行番号をリトルエンディアンの64ビット符号付き整数として並べたSHA-256は、100,000局面が`70e0e8100eb7b01c648a7ffa61edd442245d7f17b467eea739a0c7b3aa78a9f5`、10,000局面が`d20519b5522252e6d199f21666cea0efd0858010cc406db8e36c4f08348c6970`だった。
今回の検証実行では行番号のJSONは保存していない。
書き込み可能な環境で`samples.py`を通常実行すると、指定先の`data/relational-correction/prep/samples/teacher.json`と`quantization.json`へ、0起算のファイル番号とファイル内レコード番号を保存する。

## 代理診断

差が117.7 cp以上でa*を支持した根は15根中6根、割合は40.0%だった。
両子局面とも全15根でセンチポーン値を得たので、比較不能による除外はなかった。
各根の着手列、探索結果、根の手番側に符号をそろえた評価値と差は[proxy-check.json](proxy-check.json)に保存した。

探索には`data/relational-correction/bin/minase`を使い、そのSHA-256が各根の教師記録のエンジンと一致することを確認した。
`phase0.py`の`run_search`を呼び、子局面ごとに新しいプロセスを起動して、`engine-default`、`USI_Hash=64`、`Threads=1`、`ResignValue=99999`、`go nodes 100000`で実行した。
差は`-child_score(a_star) + child_score(s0)`として計算した。
これは訓練教師の探索値部分に対する代理診断であり、実際の訓練ラベルによる識別や採否を示す結果ではない。

## Gの校正を妨げた制限

指定先`/home/stepney141/board-games/minase/data/relational-correction/prep/`への`mkdir -p`は、`Read-only file system`で失敗した。
`fm-eval-tools`にはビルド済みの`target/release`もなく、同worktreeで次のコマンドを実行すると、target用のパスを作る段階で同じエラーになった。

```sh
cargo build --offline --release --bin pst_probe --bin match_positions
```

したがって、3143b08の重みを`pst_probe`で読めるかの確認、局面の合法性検査を伴う抽出、静的評価、共通層の被覆率、およびGとG*の算出は未実施である。
保存記録を読み取って数えた候補のセンチポーン評価は7,398件、詰み評価は68件、評価欠損は0件だった。
教師ファイルのヘッダは91,841局面を示した。
この件数確認は`match_positions`による棋譜再生の検証を代替しない。

`g_star.py`は固定した`fm-eval-tools`のコミット`01aedf589696fe228b72a6bfa98576ee06cd8ef6`を検査し、候補コミット`3143b082ac15efceb889ba35ce79ed37abe8594d`から重みを取得する。
旧形式の重みを読めない場合に限り、指示書で認められた候補コミットのdetached worktreeを作り、再ビルドする。
比較は`fm_distribution_comparison.compare(..., center_only=False)`で全域を対象とし、共通層内の対局頻度で重み付けする。
G*は補正1/4の重みで測ったGと0の大きい方であり、算出したGをさらに4で割ることはない。
実測後は指定先の`g-star/g_star.json`に値、被覆率、件数、SHA-256、コマンド列を保存するが、この実行経路の全体は未検証である。

## 実行コマンド

書き込み可能な環境では、主worktreeから次の3コマンドを実行する。
各スクリプトは専用の出力ディレクトリを新規作成し、既存の測定結果は上書きしない。

```sh
python -B docs/measurements/relational-correction-prep/prep/g_star.py
python -B docs/measurements/relational-correction-prep/prep/samples.py
python -B docs/measurements/relational-correction-prep/prep/proxy.py
```

今回実行して結果を確認したコマンドは次のとおりである。
出力先の制限により、検証結果は本ディレクトリの新規ファイルに保存した。

```sh
python -B docs/measurements/relational-correction-prep/prep/samples.py --check-only > docs/measurements/relational-correction-prep/prep/samples-check.json
python -B docs/measurements/relational-correction-prep/prep/proxy.py --stdout-only > docs/measurements/relational-correction-prep/prep/proxy-check.json
python -B -m unittest discover -s docs/measurements/relational-correction-prep/prep -p 'test_*.py' -v
```

3テストが通過し、Pythonファイルの構文も検査した。
単体テスト`test_prep.py`と`test_phase3.py`は、参照先のモジュールとパスが失われて実行できなくなったため、2026年10月4日のテスト監査で削除した。削除前の最終版はコミットa81e859に残る。
テストは根視点の符号と117.7 cpの境界、検証分割内の非復元抽出と標本ごとのシード再初期化、全域の共通層と対局頻度の重みを対象とする。
符号反転、閾値の厳密不等号への変更、中央範囲だけへの限定をメモリ上で個別に加え、対応するテストが各変更を検出することも確認した。
