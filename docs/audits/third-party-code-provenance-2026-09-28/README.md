# 第三者コード混入監査の補助資料

本ディレクトリは、第三者コード混入監査（報告は [../third-party-code-provenance-2026-09-28.md](../third-party-code-provenance-2026-09-28.md)）の計画、比較コーパスの一覧、機械的照合のスクリプト、対照レビューの記録、および文書中の引用の一覧を保存する。
監査の対象は minase の基準コミット `a05478a`（master 上の `Merge branch 'd-command-display'`）と、その時点の全 ref の履歴である。
本書は、初めて読む人が各ファイルの役割を把握し、機械的照合 M1〜M4 とその絞り込み（以下、トリアージ）を一から再実行して結果を照合できるように書いている。

## 監査の構成

監査は計画書 [plan.md](plan.md) の6手法からなる。
M1 は注釈と文字列、M2 は数値定数の組、M3 は定数表と重み、M4 は同じ言語どうしのトークン列を機械的に照合する。
M5 は、対応するモジュールの組ごとに独立したレビュアーが両側の原文を読み比べる対照レビューであり、M6 は docs/ などの文書に含まれる第三者コードの引用を列挙する。
M1〜M4 のスクリプトは OpenAI の Codex CLI（計画書では codex と表記）が実装して実行し、M5 と M6 は Claude から起動した別の Claude エージェント（計画書では subagent と表記）が担当した。
トリアージは、M1〜M4 の大量の候補を再現可能な規則で数十件へ絞り、上位候補を両側のソースで読んだ工程である。
トリアージとレビューが参照する K1〜K5 は、監査の開始前から由来が分かっていた5件の既知の一致であり、各手法の検出力を確かめる較正に使った（一覧は [mechanical/triage.md](mechanical/triage.md) の「既知5件の検出」の表にある）。

## ファイルの一覧

各ファイルの役割は次の表のとおりである。

| パス | 内容 |
|---|---|
| `plan.md` | 監査の計画書（第2版）。対象範囲、比較コーパスの選び方、6手法の定義、報告の構成を定める。 |
| `corpus/manifest.tsv` | 比較コーパス53行の一覧。リポジトリ名、クローン先、照合したコミット、ライセンスとその根拠、言語、行数、区分を記録する。 |
| `corpus/tags.txt` | Stockfish と YaneuraOu のクローンで照合に使えたリリースタグの一覧。 |
| `corpus/snapshots.tsv` | Git 管理外のファイル単位の複製（manifest の3行分）のうち、解析対象の拡張子を持つ19ファイルの SHA-256。 |
| `mechanical/` | M1〜M4 とトリアージの再実行に必要なスクリプト、仕様、結果の要約。 |
| `mechanical/spec.md` | M1〜M4 の仕様。スクリプトはこの仕様を実装する。 |
| `mechanical/README.md` | M1〜M4 の実行手順、正規化、閾値、出力の数え方の詳細。 |
| `mechanical/report.md` | M1〜M4 の実行結果の報告（被覆、上位候補、未完了部分と限界）。 |
| `mechanical/triage.md` | トリアージの規則、手法ごとの絞り込み結果、既知5件の検出表。 |
| `mechanical/triage_m1.tsv`〜`triage_m4.tsv` | トリアージで読んだ候補と、その分類（FP、KNOWN、NEW）と理由。 |
| `mechanical/m1_top.tsv`〜`m4_top.tsv` | 各手法の上位60行。一致した短い文字列や数値列のプレビューを含む。 |
| `mechanical/attribution.tsv`、`missing-inputs.tsv`、`m2_params.tsv`、`m2_params_introductions.tsv`、`refs.tsv` | 帰属表示の確認対象、欠落した入力、係数マクロの既定値と導入時の値、要求した ref と解決したコミット。 |
| `mechanical/output-counts.json`、`tool-versions.json`、`tree-sitter-sha256.json` | 巨大な出力の行数、実行環境の版、使用した tree-sitter 構文解析器の SHA-256。 |
| `mechanical/validation-fixtures/`、`mutations/` | 抽出の検証に使う自作の小さな入力と、`checks.py` に意図的な誤りを入れたミューテーション検証の複製およびログ。 |
| `mechanical/triage_work/*.py` | トリアージのスクリプト。 |
| `review/rubric.md` | M5 の判定基準（4段階の判定と、必須の一致と実装固有の一致の区別）。 |
| `review/search.md` | 探索部（`src/search/alphabeta/`）の対照レビュー記録。 |
| `review/infrastructure.md` | 置換表、並列探索、時間管理、先読み、USI の対照レビュー記録。 |
| `review/core.md` | 盤面と規則の中核（`src/core/`、`src/rng.rs`、`src/eval/handcrafted.rs` ほか）の対照レビュー記録。 |
| `review/harness-and-statistics.md` | 対局ハーネス、統計、SPSA の対照レビュー記録。 |
| `review/training-and-evaluation.md` | 学習系と評価関数の対照レビュー記録。 |
| `review/second-review.md` | 陽性と判定保留を、上の5本を読まずに独立に判定し直した第2レビューの記録。 |
| `review/scripts/jp_ngram.py`、`jp_ngram10.py` | 学習系のレビューで日本語コメントの一致を調べたスクリプト（窓14文字と10文字）。 |
| `quotes/quotes.tsv`、`make_tsv.py` | M6 の引用一覧と、それを生成するスクリプト（表の値はスクリプトに直接書かれている）。 |
| `quotes/weights-history.tsv` | `nets/pst.bin`、`nets/pst-init.bin`、`nets/nnue.bin` の各版の導入コミットと blob の一覧。 |

ファイル中の `<corpus-dir>`、`<work-dir>`、`<minase-repo>` は、それぞれ比較コーパスのクローンを置くディレクトリ、M1〜M4 の作業ディレクトリ、minase リポジトリのクローンを表すプレースホルダである。
監査時の記録にあった手元の絶対パスは、これらのプレースホルダへ置き換えた。

## 再実行の準備

### 必要なツール

監査では Python 3.14.7、Node.js 25.8.2、Git 2.55.0、Pygments 2.21.0 を使用した。
構文解析器は、インストール済みの VS Code に同梱された `@vscode/tree-sitter-wasm` 0.3.1 と、同じく同梱の GitHub Copilot 拡張に含まれる10言語の文法ファイルである。
`mechanical/bootstrap.py` は、これらを `/usr/share/code/resources/app/` 以下から作業ディレクトリへ複製する。
複製したファイルの SHA-256 が `mechanical/tree-sitter-sha256.json` と一致すれば、監査時と同じ構文解析器であることを確認できる。
一致しない場合は、M1〜M4 の抽出結果が監査時と異なり得ることを記録したうえで進める。
仕様は M4 に JPlag 6.2.0 または Dolos を求めているが、監査時はネットワークの名前解決に失敗して導入できず、tree-sitter によるトークン列の winnowing を補助的に実行した。

### minase のクローン

`<minase-repo>` には、基準コミット `a05478a` を含む minase のクローンを用意する。
履歴側の入力は `git rev-list --all` のすべてのコミットから作るため、監査時（ローカルブランチ50本、805コミット）と ref の集合が異なるクローンでは、履歴側の blob 数と候補数が変わる。
基準版側の318ファイルは、`a05478a` があれば同じになる。

### 比較コーパスのクローン

`corpus/manifest.tsv` の `local_path` 列は `<corpus-dir>/<所有者>_<リポジトリ>` の形であり、`head_commit` 列が照合したコミットである。
`inventory.py` はこの列を実在のパスとして読み、各クローンの `HEAD` を照合するため、次のようにクローンして `head_commit` へ切り替える。
`repo` 列の値は `inventory.py` が Stockfish、YaneuraOu、fishtest、rshogi の追加 ref とライセンスの例外を選ぶキーなので、書き換えない。

```sh
CORPUS=<corpus-dir>
awk -F'\t' 'NR>1 && $1 ~ /^[^ ]+\/[^ ]+$/ && $3 ~ /^[0-9a-f]{40}$/ {print $1, $2, $3}' corpus/manifest.tsv |
while read -r repo dir commit; do
  dest="${dir/<corpus-dir>/$CORPUS}"
  git clone "https://github.com/$repo" "$dest" && git -C "$dest" checkout --detach "$commit"
done
git clone https://github.com/official-stockfish/Stockfish "$CORPUS/official-stockfish_Stockfish-research"
git -C "$CORPUS/official-stockfish_Stockfish-research" checkout --detach 0a215d6c9e48856ef630013b8ab8312941a59057
git clone https://salsa.debian.org/debian/hachu "$CORPUS/debian_hachu"
git -C "$CORPUS/debian_hachu" checkout --detach 822d512180b7d94bb85a55f871445b30393f7f8a
sed "s|<corpus-dir>|$CORPUS|" corpus/manifest.tsv > "$CORPUS/manifest.tsv"
```

クローンは全履歴とタグを含める必要がある。
Stockfish は `sf_10`〜`sf_19`、YaneuraOu は `corpus/tags.txt` のタグとコミット `33ccf1f`、`0a6dd2cb` の親、fishtest はコミット `b8eecff` も照合するためである。
監査時の fishtest の複製には `b8eecff` の blob が29個欠けており、`missing-inputs.tsv` と被覆表に記録した。
完全なクローンではこの29行が消え、その分だけ候補が増え得る。
lczero-training は監査時に浅いクローンだったが、照合したのは `HEAD` だけなので結果には影響しない。
YaneuraOu の `v8.00` と `v9.10` は存在しないタグであり、代わりに `v8.00-fukauraou` と `v9.10-fukauraou` を補足版として解析した。

manifest の末尾3行は、Git 管理外のファイル単位の複製（解析対象は Stockfish の4ファイル、YaneuraOu の5ファイル、SPSA 実装7系統の10ファイル）である。
複製元のコミットは記録されていないため、これらは再クローンでは正確に再現できない。
上流の該当ファイルを `<corpus-dir>/snapshots/` 以下の同じ相対パスに置き、`corpus/snapshots.tsv` の SHA-256 と一致した場合だけ監査時と同じ入力とみなす。
一致するファイルが得られなければ、そのディレクトリを置かずに実行し、`refs.tsv` に `missing_path` と記録されることを確認する。

学習系のレビューは、manifest にない fairy-stockfish/variant-nnue-tools も `git clone --filter=blob:none` で取得して読んだが、M1〜M4 の入力には含めていない。

## M1〜M4 の再実行

まだ存在しないパスを作業ディレクトリ `<work-dir>` とし、そこへ `mechanical/` を複製して、その中で実行する。
スクリプトは出力を作業ディレクトリへ書き、トリアージのスクリプトも作業ディレクトリからの相対パスで入力を読むためである。

```sh
OUT=<work-dir>
MINASE=<minase-repo>
CORPUS=<corpus-dir>
cp -r docs/audits/third-party-code-provenance-2026-09-28/mechanical "$OUT"
export TMPDIR="$OUT/tmp" PYTHONDONTWRITEBYTECODE=1
mkdir -p "$OUT/tmp" "$OUT/logs"
python3 "$OUT/bootstrap.py" --out "$OUT/tools/tree-sitter"
(cd "$OUT/tools/tree-sitter" && python3 -c 'import hashlib,json,pathlib;e=json.load(open("../../tree-sitter-sha256.json"));print(all(hashlib.sha256(pathlib.Path(n).read_bytes()).hexdigest()==d for n,d in e.items()))')
nice -n 19 python3 "$OUT/inventory.py" --out "$OUT" --manifest "$CORPUS/manifest.tsv" --minase "$MINASE" --base a05478a > "$OUT/logs/inventory.log" 2>&1
nice -n 19 python3 "$OUT/verify_sources.py" --out "$OUT" --minase "$MINASE"
nice -n 19 node --max-old-space-size=4096 "$OUT/extract.cjs" "$OUT" > "$OUT/logs/extract.log" 2>&1
nice -n 19 python3 "$OUT/extraction_tests.py" "$OUT"
nice -n 19 python3 "$OUT/test_checks.py" > "$OUT/logs/tests.log" 2>&1
nice -n 19 python3 "$OUT/mutation_check.py" "$OUT"
nice -n 19 python3 "$OUT/checks.py" --out "$OUT" > "$OUT/logs/checks.log" 2>&1
nice -n 19 python3 "$OUT/annotate.py" --out "$OUT"
nice -n 19 python3 "$OUT/verify_m4_samples.py" "$OUT"
nice -n 19 python3 "$OUT/report.py" --out "$OUT"
nice -n 19 python3 "$OUT/validate.py" --out "$OUT"
```

`bootstrap.py` の直後の行は、複製した構文解析器が監査時と同じなら `True` を出力する。
`checks.py --method M1` のように、各手法だけを実行することもできる。
M4 の閾値の較正に使う人工複製は、`checks.py` がコーパスの Reckless と viridithas の関数から `calibration/` へ生成する。
`validate.py` は必要な成果物がすべてそろい、件数が要約と一致することを検査して `verification.json` を書く。
監査時の各段階の正規化、閾値、数え方は `mechanical/README.md` に、結果は `mechanical/report.md` にある。

## トリアージの再実行

トリアージは、M1〜M4 を実行した作業ディレクトリで次を順に実行する。

```sh
cd <work-dir>
python3 triage_work/m1.py && python3 triage_work/m1_pairs.py
python3 triage_work/m2.py && python3 triage_work/m2_show.py 80
python3 triage_work/m3.py && python3 triage_work/m3_loose.py 60
python3 triage_work/m4.py && python3 triage_work/m4_raw.py
python3 triage_work/write_tsv.py
python3 triage_work/calib.py M2
```

`write_tsv.py` は `triage_m1.tsv`〜`triage_m4.tsv` を上書きするので、本ディレクトリの同名ファイルと比べて差分を確かめる。
`calib.py` は既知5件の生の検出を表示し、引数に `M1`〜`M4` を取る。
各規則の意味と、候補数が各段階でどう減ったかは `mechanical/triage.md` にある。

## 再実行で生成される出力

次の出力は、第三者のソースを含むか、または大きすぎるため同梱しなかった。
いずれも上の手順で再生成される。
行数はヘッダ行を除く値であり、M1〜M4 の一致表は `mechanical/output-counts.json` の値である。

| ファイル | 行数 | 内容 |
|---|---:|---|
| `m1_hits.tsv` | 1,006,656 | M1 の一致（候補 820,797行） |
| `m2_hits.tsv` | 492,346 | M2 の一致 |
| `m3_hits.tsv` | 199,405 | M3 の一致 |
| `m4_hits.tsv` | 5,034,236 | M4 の一致 |
| `m1_texts.jsonl` | 686,369 | 注釈と文字列の原文と正規化文 |
| `m2_sets.jsonl` | 69,112 | 関数または係数名ごとの数値の集合 |
| `m2_literals.tsv` | 754,981 | 全数値リテラルと単位換算 |
| `m2_common_values.tsv` | 1,355,339 | M2 の一致の共通値 |
| `m3_arrays.jsonl` | 8,552 | 抽出した全配列 |
| `inventory.jsonl` | 8,027 | 入力 blob の一覧と出現位置 |
| `features.jsonl` | 8,027 | 構文解析による抽出結果 |
| `coverage.tsv` | 43,500 | 入力、参照版、パス、手法ごとの解析状態 |
| `blob-first-occurrence.tsv` | 1,564 | minase の各 blob とパスの初出コミット |
| `minase-rev-list-objects.txt` | 7,454 | minase の全 ref のオブジェクト一覧 |
| `parse-errors.tsv` | 1,200 | 構文エラーの位置 |
| `excluded-files.tsv` | 1,166 | 同梱ライブラリとして除外したコーパスのファイル |

`coverage.tsv` の M1 の行を、出現位置（入力 blob、参照版、パスの組）単位で集計すると次のとおりである。
基準版は `a05478a` のファイル、履歴は全 ref に現れた blob とパスの組の初出、コーパスは各クローンの照合した参照版に現れた blob とパスの組を数える。

| 状態 | 基準版 | 履歴 | コーパス |
|---|---:|---:|---:|
| `ok`（構文解析に成功） | 295 | 1,504 | 7,112 |
| `lexical_ok`（TOML を字句解析で処理） | 6 | 38 | 78 |
| `patch_old_and_new_fragments`（パッチの新旧の断片） | 17 | 22 | 1 |
| `partial_parse`（構文エラーを含む木から抽出） | 0 | 0 | 1,767 |
| `partial_decode`（UTF-8 として読めない入力） | 0 | 0 | 6 |
| `missing_input`（blob の欠落） | 0 | 0 | 29 |
| 計 | 318 | 1,564 | 8,993 |

各出現位置は M1〜M4 の4行を持つので、(318 + 1,564 + 8,993) × 4 = 43,500 が `coverage.tsv` の行数と一致する。
M2 の状態は M1 と同じである。
M3 では、反復配列の長さを解決できなかった `partial_extraction` が基準版21、履歴319、コーパス130あり、配列の対象言語でない `not_applicable` が基準版6、履歴39、コーパス706ある。
M4 は Rust と Python だけを比べるため、`not_applicable` が基準版23、履歴61、コーパス6,760ある。
被覆表とは別に、コーパスの1,166ファイルを同梱ライブラリ（third_party、vendor など）として解析前に除外した。
minase 側には構文解析の失敗も欠落もない。

## 同梱しなかった資料

第三者のソースの複製は、ライセンス上の理由から同梱していない。
監査時の作業ディレクトリにあった `sources/`（全入力 blob の複製）、`calibration/`（Reckless と viridithas の関数から作った人工複製）、M5 のレビュアーが書き出した削除済みファイルと YaneuraOu の `learner.cpp` の複製、および M6 で照合に使った Stockfish と YaneuraOu の `search.cpp` と XBoard と USI の仕様ページの複製がこれに当たる。
M6 の照合に使った原文は、`quotes/quotes.tsv` の各行が示すリポジトリ、コミット、ファイル、行から `git show` で取り出せ、仕様ページは引用元の minase 文書の参考文献欄の URL から取得できる。
構文解析器の複製（`tools/`、約19 MB）は VS Code の同梱物であり、`bootstrap.py` で再作成する。
重みファイルの複製（`pst-init-588363a.bin` など）は minase の履歴にあり、`git -C <minase-repo> show 588363a:nets/pst-init.bin` のように `quotes/weights-history.tsv` のコミットとパスから取り出せる。
監査時の `RULES.md` の複製は `a05478a` の同名ファイルであり、`inventory.py` が作業ディレクトリへ書き出す。
トリアージの中間 JSON（`triage_work/*.json`）と実行ログ（`logs/`）は、再実行で作られ、手元の絶対パスを含むため除いた。
`validation-fixtures/` からは、`extraction_tests.py` が生成する `inventory.jsonl`、`features.jsonl`、および構文解析器へのシンボリックリンクを除き、自作の入力 `case.*` だけを残した。
