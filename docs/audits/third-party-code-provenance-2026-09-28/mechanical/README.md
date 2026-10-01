# M1〜M4の機械的照合

本ディレクトリは `spec.md` の機械的照合を再実行するためのスクリプトと結果を保存する。
対象の基準コミットは `a05478a` であり、作業ツリーではなく Git オブジェクトを読む。
全 ref の履歴では、逆順のトポロジカル順序で各 blob とパスが初めて現れるコミットを記録する。
同じ内容の blob は解析を共用し、すべての参照版とパスを `inventory.jsonl` に保持する。

## 依存と実行

Python 3.14.7、Node.js 25.8.2、Git 2.55.0、Pygments 2.21.0を使用した。
構文解析には、インストール済みの VS Code に同梱された `@vscode/tree-sitter-wasm` 0.3.1を使用した。
言語は `rust`、`python`、`c`、`cpp`、`scala`、`typescript`、`javascript`、`bash`、`c_sharp`、`java` に固定した。
実行環境の版は `tool-versions.json` にも記録した。
各文法の上流バージョン番号は同梱ファイルから確定できないため、実際に使用したバイナリを `tools/tree-sitter/` に保存し、SHA-256を `tools/tree-sitter/sha256.json` に記録した。
TOMLには Pygments の字句解析を用いる。

JPlag 6.2.0の取得と Dolos のパッケージ情報取得を試みたが、ネットワークの名前解決が失敗した。
JPlag／Dolos の実行と、それらによる解析成功率の検証は未完了である。
M4には tree-sitter から生成したトークン列を用いる winnowing の補助結果を保存した。
これは、JPlag の実際の構文解析失敗を確認してから代替するという仕様の条件を満たした実行ではない。
ネットワークの失敗は `logs/jplag-install.log` と `logs/dolos-install.log` に保存した。

各パスは引数で指定する。
`OUT` はこのディレクトリ、`CORPUS` は manifest.tsv の所在、`MINASE` は読むだけのリポジトリを指す。
`bootstrap.py` は初回に同梱の解析器をコピーするためのもので、保存済みの `tools/tree-sitter/` を使う再実行では不要である。
次のコマンドを順に実行する。

```sh
OUT=/absolute/path/to/audit/m1-m4
CORPUS=/absolute/path/to/corpus
MINASE=<minase-repo>
export TMPDIR="$OUT/tmp"
export PYTHONDONTWRITEBYTECODE=1
mkdir -p "$OUT/tmp" "$OUT/logs"
python3 "$OUT/bootstrap.py" --out "$OUT/tools/tree-sitter"
nice -n 19 python3 "$OUT/inventory.py" --out "$OUT" --manifest "$CORPUS/manifest.tsv" --minase "$MINASE" --base a05478a > "$OUT/logs/inventory.log" 2>&1
nice -n 19 node --max-old-space-size=4096 "$OUT/extract.cjs" "$OUT" > "$OUT/logs/extract.log" 2>&1
nice -n 19 python3 "$OUT/test_checks.py" > "$OUT/logs/tests.log" 2>&1
nice -n 19 python3 "$OUT/checks.py" --out "$OUT" > "$OUT/logs/checks.log" 2>&1
nice -n 19 python3 "$OUT/annotate.py" --out "$OUT"
nice -n 19 python3 "$OUT/report.py" --out "$OUT"
```

`checks.py --method M1` などで各手法だけを実行できる。
すべての入力源は読み取り専用として扱い、Git の遅延取得と任意のロックを無効化する。
欠落 blob は `missing-inputs.tsv` に記録して他の入力の検査を続ける。
GitHub API は使用しない。

## 出力と数え方

`coverage.tsv` は入力、参照版、パス、手法ごとの解析状態を示す。
`blob-first-occurrence.tsv` は履歴の導入コミット、`refs.tsv` は要求したタグと解決したコミットを示す。
`excluded-files.tsv` は同梱ライブラリの除外を記録する。
解析エラーの位置は `parse-errors.tsv` に保存する。

`m1_hits.tsv` は注釈と文字列の一致候補であり、除外した一致にも規則名を残す。
`m1_texts.jsonl` には原文、文字列の復号結果、正規化文を保存する。
英語は5語、日本語は12文字の窓を用い、短い全文はコーパスの出現が1〜2回なら別枠に残す。
識別子と式を含む短い句も別枠に残す。
希少度は比較対象のコーパス内での出現数の逆数であり、同一リポジトリの同一 blob を複数タグが参照するだけでは重複して数えない。
異なるリポジトリやスナップショットでの出現は別に数える。

`m2_hits.tsv` は関数または係数名の接頭辞による集合の一致を示す。
共通値を `m2_common_values.tsv` にも1値1行で保存する。
全数値リテラルと単位換算は `m2_literals.tsv`、マクロの既定値と範囲は `m2_params.tsv`、各係数の初出値は `m2_params_introductions.tsv` にある。
明示的な自明値、2の非負整数乗、コーパス内の上位20値を除き、共通値が2個以上の組を候補にする。
単独一致は件数だけを記録する。
単位が明記された近傍のリテラルは比率または秒に換算した集合も比較する。
単位の明記がないリテラルに単位を推定しない。
数値そのものの集合も保持するため、演算中の係数の一致を確認できる。
関数外の定数集合は明示して残し、機能の一致とは判定しない。

`m3_hits.tsv` は8要素以上の配列とレコード配列の数値列を照合する。
数値以外の要素は構文上の表現を保持し、倍率照合だけは数値列に限定する。
Rustの反復配列は要素数を解決できた場合に展開し、未解決の式は `m3_array_limitations.tsv` に記録する。
全配列は `m3_arrays.jsonl` に保存し、一致一覧の値は先頭16要素のプレビューと配列IDで示す。
完全一致、正の倍率と絶対誤差1以内の丸め、連続部分列、多重集合の一致を検出する。
部分列は配列の組ごとに最長の一致を1件記録する。
HaChu の較正では並び順と王駒の値が違うため、倍率と並べ替えと部分集合を組み合わせた検査も加えた。
この追加検査は、両配列が8〜128要素で8種類以上の値を持つ場合に、比の候補を小数第2位まで作り、8要素以上かつ短い表の70%以上の一致を要求する。
HaChu の値だけを特別に一致させる規則は使用しない。

`m4_hits.tsv` は Rust と Python の局所一致について、長さ、両側の行、各ファイルのトークン数に対する局所被覆率を示す。
識別子と文字列をそれぞれ共通トークンに正規化し、数値は基数、型接尾辞、区切りを正規化した値を保持する。
この設定は、異なる数値表が同じトークン列になることを避ける。
winnowing は10トークンの rolling hash と右端最小値を使い、閾値 t に対し窓幅を t−9 とする。
ハッシュ一致後にトークン列の一致を再確認し、左右に延長する。
較正では20、40、80トークンを比較し、人工複製を検出できる閾値のうち独立対照の一致件数が最小のものを採用し、同数なら短い閾値を採用する。
人工複製のソースと改名後のソースは `calibration/`、較正値は `m4_calibration.tsv` に保存する。
独立対照は Reckless と viridithas の静的交換評価、ニューラルネットワーク評価、ビット数計算の各関数であり、その一致を運用上の偽陽性として数えるが、由来の独立性を履歴調査で証明したものではない。
翻訳例による検出力を確認していないため、C++との言語をまたぐ比較は実施しない。

履歴上の各 blob、各出現箇所、各比較先を別の候補として数える。
候補件数は独立した複製の件数ではない。
候補の複製／偶然の判定は本検査の範囲外であり、上位候補と明らかな偽陽性の注記は `report.md` にまとめる。

## 検証の範囲

`extraction_tests.py` は5言語にまたがる8項目の抽出を独立した入力で検証する。
`test_checks.py` の期待値は仕様書の符号、基数、型接尾辞、区切り、文字列連結、5語／12文字、単位、倍率、丸め、20／40／80トークンという要件から作成した。
監査対象の現在の実装を期待値には用いていない。
同じ係数が別の独立ブランチで導入された場合の「最初」は仕様書では一意に定まらないため、記録したトポロジカル順序で最初のものとする。
ミューテーション検証は `mutation-results.json` に記録する。
このコーパス外の実装と未取得の入力について、非検出の結論は出せない。

`annotate.py` はファイル先頭の SPDX 表示を反映し、定型文と反復構造の理由を注記する。
M1の除外とM2の自明値の除外回数は `exclusion-counts.tsv` に保存する。
ファイルごとのライセンス補正は `file-license-overrides.tsv` にある。
