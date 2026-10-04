# 採否前の診断器

[学習の設計書](../../plans/qsearch-output-training.md)のフェーズ3で、候補集合内の選択損失と通常の根の静的評価誤差を測る。
`diagnose.py`は、重みを埋め込んだ各エンジンへUniversal Shogi Interface（USI）のコマンドを送り、探索ごとに新しいプロセスを起動する。
すべての起動で`ResignValue`の宣言を確認し、最初の`isready`より前に`setoption name ResignValue value 99999`を送って投了を無効にする。
このオプションがないエンジンは診断対象にできないため、探索前に停止する。
`build_engine.sh`は現在のHEADから一時worktreeを作り、指定したMNPT形式の重みを`nets/pst.bin`へ置いてエンジンをビルドする。
いずれもRustの診断器の移植を必要としない。

## 元の定義との対応

参照実装は別worktreeの`search-aware-evaluation/src/bin/`にある。
対応する関数は次のとおりである。

| Pythonの関数 | 元の関数 | 保存する意味 |
|---|---|---|
| `load_roots` | `search_aware_data::pipeline::restore`と`export` | 開始手順を含む着手列と、局面ごとの保存済み教師値を読み込む。 |
| `run_engine` | `teacher_common::teacher::search_game` | 探索ごとに1スレッドと64 MiBの新しい置換表を使い、終局と未完了を区別する。 |
| `parse_search` | `teacher_common::teacher::searched_score` | 子局面の値を根手番側へ反転し、詰みと通常評価を分ける。 |
| `evaluate_root` | `search_aware_diagnostic::diagnostic::evaluate_root` | 各エンジンと教師の提案手の和集合を作り、全候補の子を2予算で独立に探索する。 |
| `classify` | `teacher_common::classify::category` | 差mと予算間の変化の境界を含め、支持の方向、小差、不安定、比較不能を判定する。 |
| `decision` | `search_aware_diagnostic::diagnostic::decision` | 全組の分類と集合内の選択損失を計算する。 |
| `aggregate`と`root_error` | `search_aware_diagnostic::diagnostic::static_summary` | 通常評価の根だけについて、手番側の静的評価から教師値を引く。 |

教師の高予算の提案手を得る探索は、根自体をS0で独立に探索した結果でもあるため、その値を静的誤差の基準にも使う。
USIの`eval`はすでに手番側の値を返すので、後手の根でも符号を変えない。
詰みの値を通常の評価誤差や選択損失には混ぜない。
対局がすでに終わっている場合は、`d`の`status`から比較不能と判定する。

詰み表示の変換は、[`src/protocol/usi.rs`](../../../src/protocol/usi.rs)の`score_text`（1475〜1485行）と、[`src/search/mod.rs`](../../../src/search/mod.rs)の`MATE = 30000`（32行）に従う。
内部値をsとすると、表示する距離は`max(30000 − |s| − 2, 0)`である。
したがって、非ゼロの`mate n`は`sign(n) × (30000 − |n| − 2)`へ戻し、子局面ではその後に根手番側へ符号を反転する。
`mate +0`と`mate -0`は、内部距離0〜2を同じ表示へ丸めるため一意に復元できない。
診断器では符号を保持する代表値としてそれぞれ30,000と−30,000を記録するが、これは内部値の厳密な復元ではない。
照合は評価値の完全一致を要求するため、元の内部値がこの代表値と異なる場合は差として報告する。

診断の母集団は`phase3/selection.json`の256根である。
元の`phase4/final/roots.jsonl`には、通常の教師値を得た248根だけが保存されているため、このファイルだけから新しい診断の母集団を作ることはできない。
通常実行では選定された256根からフェーズ0の6根を除き、`--include-phase0-roots`を付けた再現実行では256根すべてを使う。
いずれの場合も、`phase3/teacher.jsonl`の教師値が`cp`でない根を探索前に除外する。
保存記録では詰みの8根がこの条件に該当するため、探索対象は通常実行で242根、再現実行で248根となる。
根と組になる診断標本が通常評価でない場合でも、根自身が通常評価なら根は残す。
除外条件の根拠と8根の照合結果は[根の除外規則の検証](root-exclusion-validation.md)に記す。

成り手は、元の`search_aware_diagnostics.py::check_promotions`が使った262局面の368手に固定する。
保存済みの着手前後の盤面を`train_pst.py::integer_evaluate`で再評価し、`−着手後の手番側評価−着手前の手番側評価 < 0`を数える。
この標本には初期配置、モデル選択集合の固定標本、局面帯の代表が含まれ、診断の250根とは別の対象である。
診断開始時に、S0の着手前後の評価と差分を保存値へ照合する。
各候補のMNPT全体が指定実行ファイルに埋め込まれていることも確認し、異なる重みの取り違えを拒否する。

QとQcのkは、各根をMNSD形式へ変換して`qsearch_leaf`へ与え、候補自身の重みで抽出する。
MNSDは盤面と先獅子状態を保持し、対局の着手履歴は保持しない。
この取り出しのkと末端の種類は記録にだけ使い、履歴を含むUSI探索による採否条件には加えない。

## 対局単位の標準誤差

根ごとの差をd、対象の根数をN、対局数をGとする。
各対局gの対象根数をn_g、差の合計をS_gとし、根ごとの平均差をμ=ΣS_g/Nで求める。
標準誤差には次の式を使う。

```text
SE_game = sqrt(G / (G - 1) * Σ_g (S_g - n_g * μ)^2) / N
lower = μ - 1.96 * SE_game
```

この式は各根を同じ重みで扱い、同じ対局内の差を合計してから分散を求める。
独立な根を仮定した標準誤差も、標本標準偏差を√Nで割って併記する。
Nが150未満、またはGが2未満ならその指標を判定に使わず、判定可能な指標の下限が0を超えた候補をS0との採否測定から除外する。
実際の対局は`game_seed`で識別し、根の識別子の先頭部分とも照合する。

## 保存記録への規則適用

2026年9月29日に、探索を実行せず、元の`report.json`と`roots.jsonl`だけから[ab-rule.json](ab-rule.json)を作成した。
保存された248根すべてについて、子局面の値から再計算した分類、全組の差、選択損失が元の判定と一致した。
元の報告にある対局群の識別子と件数は、各根の`game_seed`による件数と一致するため、過去の静的誤差にも同じ対局単位を使える。

| 候補 | 指標 | 平均差 | 対局単位の標準誤差 | 根独立の標準誤差 | 下限 | 根数 | 対局数 |
|---|---|---:|---:|---:|---:|---:|---:|
| A | 選択損失 | +1.714 | 5.132 | 5.120 | −8.345 | 203 | 172 |
| B | 選択損失 | +24.099 | 8.592 | 8.573 | +7.258 | 203 | 172 |
| A | 静的評価の絶対誤差 | −5.423 | 5.838 | 算出不可 | −16.865 | 248 | 204 |
| B | 静的評価の絶対誤差 | +166.028 | 16.631 | 算出不可 | +133.431 | 248 | 204 |

値の単位はセンチポーンである。
静的誤差の対局単位の標準誤差は、報告に保存された対局別の件数と平均絶対誤差から求めた。
指定の2ファイルには、根ごとの静的評価の対応や対局内の二次モーメントがないため、静的誤差差の根独立の標準誤差は復元できず、JSONに欠測理由を記録した。
新しい診断では根ごとの誤差を保存するため、両方の標準誤差を出力する。

対局単位でもAは2条件のいずれにも該当せず、Bは両条件に該当する。
これは今回の選別規則を過去記録へ当てた結果であり、Aの棋力改善を意味しない。
この計算ではフェーズ0の6根も含め、元の選定256根のうち保存された対象根をすべて使用した。

## 実行手順

以下のコマンドは本worktreeのルートから実行する。
Pythonには学習用仮想環境を使い、本番の探索は他の重い生成や測定と重ならない時間に実施する。

```bash
diagnostic_dir=docs/measurements/qsearch-output-diagnostics
diagnostic_python=/home/stepney141/board-games/minase/tools/train/.venv/bin/python
archive_dir=/home/stepney141/board-games/minase/data/search-aware-evaluation
engine_dir=/tmp/qsearch-diagnostic-engines
mkdir -p "$engine_dir"
```

候補の重みごとに実行ファイルを作る。
`build_engine.sh`は`CARGO_BUILD_JOBS`を上書きせず、成功時と異常終了時に一時worktreeを削除する。
出力先がすでに存在する場合は停止し、成功した場合は複製した実行ファイルとMNPTのSHA-256を表示する。
現在のHEADの追跡済みファイルを使うため、未コミットの実装変更はビルドへ入らない。

```bash
CARGO_BUILD_JOBS=1 nice -n 19 bash "$diagnostic_dir/build_engine.sh" \
  nets/pst.bin "$engine_dir/S0"
for candidate in A B; do
  CARGO_BUILD_JOBS=1 nice -n 19 bash "$diagnostic_dir/build_engine.sh" \
    "$archive_dir/phase4/training/$candidate/pst.bin" "$engine_dir/$candidate"
done
```

保存記録だけへの規則適用は次のコマンドで再実行できる。
探索を使わず、既定ではこのディレクトリの`ab-rule.json`を更新する。

```bash
"$diagnostic_python" -B "$diagnostic_dir/diagnose.py" archive
```

探索も含む元記録の再現には、本番の予算と256根を使う。
`--compare-final`は保存された248根について提案手、深さ、値、要求ノード数、候補集合、子探索、および通常根の静的誤差の集計を照合する。
`nodes`は照合から除く。
元の記録は中断した反復を含む探索全体の消費ノード数を持つが、診断器は最後に完了した反復の`info`行から読み、USIは最終的な総数を常に出力するわけではない。
定義が異なる数値を比較しないため、除外理由を`result.json`の`archive_comparison.excluded_search_fields.nodes`に保存し、照合する探索項目を`search_fields`に列挙する。
各探索の`nodes`自体は`roots.jsonl`に記録し、予算を表す`requested_nodes`は照合を続ける。
選定された256根から保存済み248根を引いた集合が、今回の除外8根と一致することも検査する。
差があれば`result.json`の`archive_comparison`に保存して異常終了する。
この全根の実行は実装検証時には行っていない。

```bash
nice -n 19 "$diagnostic_python" -B "$diagnostic_dir/diagnose.py" run \
  --engine S0="$engine_dir/S0" --engine A="$engine_dir/A" --engine B="$engine_dir/B" \
  --weights S0=nets/pst.bin \
  --weights A="$archive_dir/phase4/training/A/pst.bin" \
  --weights B="$archive_dir/phase4/training/B/pst.bin" \
  --judge A,B --include-phase0-roots \
  --compare-final "$archive_dir/phase4/final" \
  --out /tmp/qsearch-diagnostic-reproduction --jobs 1
```

R、Q、Qcの学習完了後は、`training_dir`を実際のMNPT保存先に設定し、次の手順で診断する。
`qsearch_leaf`は現在のworktreeにあるビルド済みの`target/qsearch-leaf/release/qsearch_leaf`を使う。
別の場所にある場合は`--qsearch-leaf PATH`を指定する。

```bash
training_dir=/path/to/training
for candidate in R Q Qc; do
  CARGO_BUILD_JOBS=1 nice -n 19 bash "$diagnostic_dir/build_engine.sh" \
    "$training_dir/$candidate/pst.bin" "$engine_dir/$candidate"
done
nice -n 19 "$diagnostic_python" -B "$diagnostic_dir/diagnose.py" run \
  --engine S0="$engine_dir/S0" --engine R="$engine_dir/R" \
  --engine Q="$engine_dir/Q" --engine Qc="$engine_dir/Qc" \
  --weights S0=nets/pst.bin --weights R="$training_dir/R/pst.bin" \
  --weights Q="$training_dir/Q/pst.bin" --weights Qc="$training_dir/Qc/pst.bin" \
  --judge Q,Qc --out /tmp/qsearch-diagnostic-candidates --jobs 1
```

Qcを実施しない場合はQcのエンジンと重みの引数を除き、`--judge Q`にする。
`--judge`を省略した場合は、指定されたエンジンのうちQとQcを判定する。
提案手の既定予算は100,000ノード、教師の予算は1,000,000ノードと10,000,000ノードである。
`--jobs`は独立な根の同時処理数であり、各根で行う探索は順次実行するため、同時探索数もこの値以下となる。

結果は`result.json`と`summary.md`へ出力し、完了した根を`roots.jsonl`へ追記する。
`result.json`の`planned_roots`は選定根数、`roots`は除外後の診断根数であり、`excluded_roots`には除外した根の識別子、理由、保存済み教師値を記録する。
除外した根も`roots.jsonl`に記録し、再開時に再利用する。
同じ引数と出力先で再実行すると完了済みの根を再利用する。
入力、履歴、実行ファイル、重み、スクリプト、予算、並列数などの条件が変わると停止し、壊れた末尾行も黙って捨てない。
縮小実行でノード数や根数を変えた場合は`smoke: true`を記録し、候補の採否は判定しない。
`--root-id ID`では指定した1根だけを処理し、本番予算でも縮小検証として扱う。
この引数は`--limit`と併用できず、`--compare-final`にも使用できない。

## 検証

単体テストは分類の境界、同じ手の損失、教師最良手の損失、対局単位の標準誤差、150根の境界、評価の符号、詰み距離、終局の除外、履歴を含むUSI入力、投了の無効化、再開条件、およびkの入力を検証する。
投了オプションの欠如による停止、詰み表示の±1と±254の境界、符号付き0、照合項目だけを変えた場合の不一致も検査する。
期待値は設計書の定義と手計算から定めた。
検証コマンドは次のとおりである。

```bash
nice -n 19 "$diagnostic_python" -B -m unittest discover -s "$diagnostic_dir" -v
bash -n "$diagnostic_dir/build_engine.sh"
nice -n 19 "$diagnostic_python" -B "$diagnostic_dir/diagnose.py" run \
  --engine S0=target/release/minase \
  --limit 2 --proposal-nodes 2000 --teacher-low-nodes 5000 --teacher-high-nodes 20000 \
  --jobs 1 --out /tmp/qsearch-output-diagnostic-final-smoke
```

2026年9月30日に25件の単体テストが成功した。
単体テスト`test_diagnose.py`は、計画の完了後に実行経路がなくなったため2026年10月4日のテスト監査（[docs/audits/test-audit-2026-10-04.md](../../audits/test-audit-2026-10-04.md)）で削除した。削除前の最終版はコミットa81e859に残る。
追加した回帰テストは、修正前に投了設定の欠如、詰み値のずれ、およびノード数の定義差を検出した。
保存済みの`data/qsearch-output-training/phase3/diagnostic-reproduction/roots.jsonl`と`data/search-aware-evaluation/phase4/final/roots.jsonl`を照合し、詰み値の差5件が換算修正で一致することも確認した。
下表の値は根手番側であり、USIの値は保存値と修正前の換算式から逆算したもので、取得済みの生ログではない。

| 根の識別子 | 候補手 | 予算（ノード） | 保存された実測値 | 復元したUSI表示 | 修正後の値 | 元記録の期待値 |
|---|---|---:|---:|---|---:|---:|
| 202610069-395-r | 2k1k2k | 10,000,000 | 29,992 | `mate -8` | 29,990 | 29,990 |
| 202613302-422-r | 3k2j | 10,000,000 | −29,993 | `mate 7` | −29,991 | −29,991 |
| 202615380-917-r | 11f10f | 1,000,000 | −29,997 | `mate 3` | −29,995 | −29,995 |
| 202615380-917-r | 11f10f | 10,000,000 | −29,997 | `mate 3` | −29,995 | −29,995 |
| 202615380-917-r | 11f12f | 10,000,000 | −29,995 | `mate 5` | −29,993 | −29,993 |

この確認では保存ファイルを書き換えず、探索も実行していない。
投了によるものと推定される最善手の欠落1件は、保存記録からは復元できず、エンジンを使った再実行で確認する必要がある。

2026年9月29日の縮小検証では2根を完了し、安定した根は1根、通常の根は2根だった。
S0の静的評価の平均誤差は−181センチポーン、平均絶対誤差は196センチポーンだった。
同じ条件で再実行すると完了済み2根を再利用し、探索を追加しなかった。
保存標本の成り手を再評価した件数も、S0が1件、Aが44件、Bが294件となり、元の368手の診断と一致した。
ビルド用スクリプトは構文を検査し、実際のRustビルドと全根の探索は実行していない。

単体テストは17件を通過した。
支持の境界、小差の境界、対局ごとの中心化、150根の下限、子局面の符号、および再開条件の照合をそれぞれ変更した6種類の変異は、いずれもテストで検出した。
根がまだ保存されていない段階でも、条件を変えて再開すると停止することを確認した。
根独立の静的誤差標準誤差を過去の2ファイルだけから復元できない点は、前述のとおり欠測として扱っている。
