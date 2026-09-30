# 根の除外規則の検証

2026年9月30日、Python診断器が元の最終診断と同じ条件で根を除外するよう修正した。
元の選定256根のうち、保存済み教師値が通常評価である248根だけを診断へ渡し、残る8根を理由とともに記録する。
この変更により、除外対象の根`202610634-285-r`で提案手の欠落によって停止する問題を解消した。

除外条件の根拠は、参照worktreeの[`pipeline.rs`の842–855行](/home/stepney141/board-games/minase/data/worktrees/search-aware-evaluation/src/bin/search_aware_data/pipeline.rs:842)にある。
参照worktreeに未コミットの変更がないことを確認して読んだ。
診断集合では組全体の受理条件を適用せず、局面ごとに教師値を判定する。

```rust
if !accepted && group.split != Split::Diagnostic {
    continue;
}
```

局面を出力する直前の853–855行は、通常評価以外を除外している。

```rust
let Score::Cp { value } = response.record.data.search.score else {
    continue;
};
```

最終診断器も、[`diagnostic.rs`の469–488行](/home/stepney141/board-games/minase/data/worktrees/search-aware-evaluation/src/bin/search_aware_diagnostic/diagnostic.rs:469)で出力された根だけを対象とし、`matches!(r.record.data.search.score, Score::Cp { .. })`で出力集合を検証する。
したがって、`mate`、`terminal`、`incomplete`の根は探索前に除外する。
一方、同ファイルの[71–82行](/home/stepney141/board-games/minase/data/worktrees/search-aware-evaluation/src/bin/search_aware_diagnostic/diagnostic.rs:71)は、対象となった根で提案手が欠ける場合に`Fault::Integrity("missing proposal".into())`を返す。
Python側でも、この場合はエラーを維持する。

実際の保存記録では、未出力の8根はすべて`mate`だった。
`load_roots`で読み込んだ教師値から求めた除外集合と、選定256根から`phase4/final/roots.jsonl`の248根を引いた集合が完全一致した。
この照合では探索を実行していない。

| 根の識別子 | 保存済み教師値 |
|---|---:|
| `202610479-370-r` | 29993 |
| `202610531-391-r` | 29997 |
| `202610542-394-r` | 29993 |
| `202610602-368-r` | 29991 |
| `202610634-285-r` | -29994 |
| `202610938-321-r` | -29990 |
| `202613360-251-r` | 29997 |
| `202615418-483-r` | 29995 |

診断器は教師値の入力ファイルを再開条件のハッシュへ加え、除外した根を`roots.jsonl`と`result.json`の`excluded_roots`へ保存する。
`--compare-final`では、除外集合の一致に加えて、選定数、除外数、診断対象集合も照合する。
除外対象を取り違えた場合は差分を記録し、異常終了する。

単体テスト21件は、`nice -n 19`で直列実行して通過した。
追加した検証は、3種類の通常評価以外の根の除外、組になる標本からの独立性、教師応答の欠落と重複、対象根での提案手欠落、除外集合の取り違え、および根の欠落と重複を扱う。
修正前には、追加した除外テストが実際に`missing proposal; cannot form the prescribed candidate set`で失敗することを確認した。
除外処理を無効化する変異と、除外集合の照合を削る変異も、いずれも追加テストが検出した。

問題の1根については、次のコマンドを実行した。
出力先を変更すれば同じ検証を再現できる。

```bash
diagnostic_dir=docs/measurements/qsearch-output-diagnostics
diagnostic_python=/home/stepney141/board-games/minase/tools/train/.venv/bin/python
engine_dir=/home/stepney141/board-games/minase/data/qsearch-output-training/engines
archive_dir=/home/stepney141/board-games/minase/data/search-aware-evaluation
nice -n 19 "$diagnostic_python" -B "$diagnostic_dir/diagnose.py" run \
  --engine S0="$engine_dir/S0/minase" --engine A="$engine_dir/A/minase" \
  --engine B="$engine_dir/B/minase" --weights S0=nets/pst.bin \
  --weights A="$archive_dir/phase4/training/A/pst.bin" \
  --weights B="$archive_dir/phase4/training/B/pst.bin" \
  --include-phase0-roots --root-id 202610634-285-r \
  --proposal-nodes 100000 --teacher-low-nodes 1000000 --teacher-high-nodes 10000000 \
  --jobs 1 --out "$diagnostic_dir/fix1-single-root"
```

本番予算を指定した1根の実行は完了し、[result.json](fix1-single-root/result.json)に`planned_roots: 1`、`roots: 0`、除外理由`source_teacher_non_cp`、教師値`mate: -29994`を記録した。
元の規則に従って探索前に除外するため、この根の探索は発生していない。
同じコマンドで再実行すると`reusing 1/1 completed roots`を出力し、終了コード0で除外記録を再利用した。
`--root-id`を用いた実行は`smoke: true`となり、採否判定には使用しない。
探索を伴う全根の再現照合は実施していない。
