# 学習ツールの再編の設計書

## 先に読む要約

`tools/train/` は、PST（駒の種類と位置に応じた評価表）を自己対局の教師データから学習するPythonのツール群である。
現在は26個のファイルが `tools/train/pst/` に平らに並び、学習器 `train_pst.py`（1,047行）が重みファイルの読み書き、教師値の尺度Kの推定、モデル、損失、学習ループ、参照評価、およびコマンドラインをすべて抱えている。
テストは計画のフェーズ名（`test_phase4.py` など）で分かれ、他のテストファイルから補助関数を import し合っている。
本書は、コードを機能ごとにファイルとディレクトリへ分け、テストを専用のディレクトリ `tools/train/tests/` へ移し、依存関係を uv（Pythonのパッケージと仮想環境を管理するツール）で管理する。
あわせて、段階9で不採用に終わった3つの診断スクリプトを削除し、各ソースの目的を説明する `tools/train/README.md` を新設する。
再編は処理の中身を変えない移動であり、次の3つを完了条件とする。
第1に、残るテストがすべて通る。
第2に、移動前後のソースの行が、import 文など本書が列挙した差を除いて一致する。
第3に、合成データを入力にCPUで同じ乱数の種を与えて学習した重みファイルが、再編の前後でバイト単位で一致する。

## 状態

起案。
2026年10月3日に起案した。
段階9の診断3本の削除、測定記録のスクリプトの書き換え、実行方法をコンソールスクリプトにすること、およびテストを専用ディレクトリへ移すことは、同日に利用者が決定した。
次の一手は、フェーズ1の codex への委任である。

## 目的

学習ツールを変更するときに、変更すべきファイルを機能から一意に決められる状態にする。
たとえば重みファイルの形式を変えるなら `data/mnpt.py` だけを、駒除去の損失を変えるなら `pst/removal.py` だけを開けばよい状態にする。
また、学習環境を `uv sync` の1コマンドで再現でき、PyTorch と NumPy の版が `uv.lock` に固定されている状態にする。

本書で扱うファイル形式と学習の用語は次のとおりである。
いずれも本体の Rust 側が書き出すか読み込む独自のバイナリ形式であり、名前は先頭4バイトの識別子に由来する。

| 名前 | 内容 |
|---|---|
| MNSD | 自己対局の局面、探索値、および対局結果を並べた教師データ。`selfplay_gen` が書き出す |
| MNRS | MNSD の各局面に別の探索で付け直した教師値 |
| MNKF | MNSD の各局面に対応する追加の評価特徴（王の安全度など）の列 |
| MNPT | 学習したPSTの重みと探索用の駒価値。`nets/pst.bin` がこの形式であり、エンジンが読み込む |
| 駒除去の損失 | 訓練局面から王駒以外の駒を1枚ずつ取り除いたときの評価差が、初期MNPTで計算した評価差と同じ符号になることを促す追加の損失（詳細は `docs/guides/pst-training.md`） |

## 適用範囲

対象は `tools/train/` の全体、`tools/train/pst/` のパスを参照する `docs/guides/pst-training.md` と `src/bin/pst_probe.rs` の文書コメント、および `docs/measurements/` のうち作業ツリーの `tools/train/pst` を `sys.path` に差し込む測定スクリプトである。

関数、クラス、定数、およびテストメソッドの名前は変えない。
処理の内容と引数も変えない。
例外は、「重複の統合」の節が定める重複定義の統合、「段階9の診断の削除」の節が定める未使用コードの削除、および「学習工程の変更点」の節が定める3点だけである。

`docs/plans/`、`docs/audits/`、および `docs/measurements/` の Markdown は、ある時点の作業を記録する文書なので、パスの参照を書き換えない。
`docs/measurements/` の Python スクリプトは、利用者の決定により例外として新しいパスへ書き換える（「測定スクリプトの書き換え」の節）。
固定コミットのワークツリー（`fm-eval-tools` など）を `sys.path` に差し込む行は、そのコミットのコードを指すので書き換えない。

## 依存関係

`tools/train/` を変更する未統合のブランチのうち、`worktree-search-revival-lmp` は SPSA の調整中であり、学習ツールには触れていない。
他の未統合ブランチ（`data/worktrees/` 以下）は完了済みまたは不採用の作業であり、本書の統合後に学習ツールを変える場合は、新しい配置へ載せ直す。

`pst_workflow.py` は、`prepare` の時点で学習ツールのソースの検査和を記録し、以後の工程で一致しなければ停止する。
再編によって検査和が変わるので、再編前に `prepare` した実行ディレクトリは、再編後のツールでは `generate`、`train`、`diagnose` を続けられない。
2026年10月3日の時点で未完了の実行ディレクトリは、`data/gen3`（世代3の教師データの生成だけを目的とし、生成は完了済み）、`data/strength-stage7/gen2`（2026年9月13日に作成し、世代2の学習は別の実行ディレクトリで完了済み）、および作業ツリーが異なる3件であり、いずれも続きを実行する予定はない。

## 設計判断

| 論点 | 採用 | 棄却した代案 |
|---|---|---|
| パッケージの形 | `tools/train/src/minase_train/` に src 配置のパッケージを置く | `tools/train/pst/` の平らな配置に `__init__.py` だけを足す |
| テストの置き場所 | パッケージの外の `tools/train/tests/` に置き、共有の補助関数は `tests/helpers.py` へ集める | パッケージ内の各ディレクトリへ散らす |
| テストファイルの分け方 | 検査する主な関数の定義元モジュールごとに1ファイル | 現在の計画フェーズ名のまま移す |
| 学習器の分割 | `train_pst.py` を形式、モデル、教師値、駒除去、参照評価、学習ループの6モジュールへ分ける | 1ファイルのまま移す |
| 段階9の診断A/B/C | 削除し、git 履歴に任せる | `research/` へ移して残す |
| 実行方法 | `[project.scripts]` のコンソールスクリプトを `uv run --project tools/train <名前>` で呼ぶ | `python -m minase_train...` |
| 依存関係の版 | `pyproject.toml` は下限だけを書き、`uv.lock` で現行の版（torch 2.13.0、numpy 2.5.2）に固定する | `pyproject.toml` に `==` で固定する、または最新版へ上げる |
| テストの実行器 | 標準ライブラリの unittest を続けて使う | pytest を依存に加える |

src 配置を採るのは、テストやリポジトリのルートからの実行が、インストールされたパッケージではなく作業ディレクトリのファイルを偶然 import する事故を防げるからである。
平らな配置のまま `__init__.py` を足す案は、現在の `from mnsd import ...` という同じディレクトリ前提の import を残し、テストを別ディレクトリへ移すと解決できなくなる。

テストをパッケージの外へ置くのは、利用者の要望により、件数の多いテストを専用のディレクトリへ分けるためである。
現在のテストは `test_train_pst.py` の `write_provenance` や `test_phase4.py` の `write_rescore` を他のテストが import しており、あるテストファイルを消すと無関係なテストが壊れる。
共有の補助関数を `tests/helpers.py` へ集めると、テストファイル同士の依存がなくなる。

テストファイルを定義元モジュールごとに分けるのは、本書の目的である「変更すべきファイルを機能から決める」をテストにも適用するためである。
たとえば `test_lambda_override.py`、`test_train_half.py`、`test_train_pst_removal.py` は、それぞれ混合比、半分割、駒除去の計画で追加された学習器のテストであり、計画名ではなく検査する機能で探せるようにする。
1つのテストクラスが複数のモジュールの関数を検査している場合は、テストメソッドごとに主な検査対象のモジュールのファイルへ振り分けるので、クラスが分割されることがある。

段階9の診断A/B/C（`depth_sensitivity_diag.py`、`king_features_diag.py`、`human_signal_diag.py`）を削除するのは、段階9が完了し、いずれの診断も教師の候補を採用しなかったからである。
これは、未使用コードを温存しないという本プロジェクトのコーディング方針による。
診断の結果と判断は [段階9の設計書](strength-stage9.md) と `docs/measurements/` の記録に残っており、スクリプトが必要になれば git 履歴から取り出せる。
先読み教師の事前診断 `lookahead_diag.py` は、先読み教師を採用した後も `docs/guides/pst-training.md` の手順で使うので残す。

コンソールスクリプトを採るのは、リポジトリのルートから `uv run --project tools/train pst-workflow prepare --config ...` と呼べば、作業ディレクトリをルートに保ったまま仮想環境を自動で同期できるからである。
`uv run --project` は作業ディレクトリを変えないので、設定ファイルやデータの相対パスは従来どおりリポジトリのルートから解決される。

依存の版を `uv.lock` で現行の版に固定するのは、学習結果の来歴（`environment.json`）が PyTorch の版を記録しており、再編のついでに版が上がると、採用済みの世代の学習条件と比べられなくなるからである。
2026年10月3日の時点で uv が解決する最新版は torch 2.14.1 と numpy 2.5.3 であり、版の更新は別の作業として扱う。

## 新しい配置

再編後の `tools/train/` は次のとおりである。

```text
tools/train/
├── README.md
├── pyproject.toml
├── uv.lock
├── pst.example.toml
├── src/minase_train/
│   ├── __init__.py
│   ├── checksum.py
│   ├── workflow.py
│   ├── data/
│   │   ├── __init__.py
│   │   ├── mnsd.py
│   │   ├── mnpt.py
│   │   ├── features.py
│   │   ├── taper.py
│   │   └── lookahead.py
│   ├── pst/
│   │   ├── __init__.py
│   │   ├── model.py
│   │   ├── teacher.py
│   │   ├── removal.py
│   │   ├── evaluate.py
│   │   └── train.py
│   └── diagnostics/
│       ├── __init__.py
│       ├── comparison.py
│       ├── taper_report.py
│       └── lookahead_teacher.py
└── tests/
    ├── helpers.py
    └── test_<モジュール名>.py
```

`data/` は学習データと重みファイルの読み書き、および学習器と診断が共有する前処理を持つ。
`pst/` はPSTの学習そのものを持ち、`diagnostics/` は学習済みの重みや教師データを調べる報告を持つ。
`workflow.py` は、準備、生成、学習、診断の4工程を記録付きで実行する最上位の手順である。

依存の向きは、`workflow` と `diagnostics` が `pst` を、`pst` が `data` を、すべてが `checksum` を使う一方向に限る。
`data/` は形式と前処理だけを持ち、学習の手法を知らないので、学習の手法を変えても `data/` を開く必要がない。
逆向きの import を許すと、たとえば `data/mnpt.py` が `pst/` を import し、`pst/` が `data/mnpt.py` を import する循環が起こり得る。

各ファイルの移動元と内容は次のとおりである。

| 移動先 | 移動元 | 内容 |
|---|---|---|
| `data/mnsd.py` | `mnsd.py` | MNSD、MNRS、MNKF、および来歴JSONの検証と読み込み。`Dataset` と `KingFeatures` |
| `data/mnpt.py` | `train_pst.py` の一部 | MNPT の定数、駒価値の検証、`write_mnpt`、`read_mnpt`、初期重み、実数重みのパス、量子化 |
| `data/features.py` | `features.py` | 駒状態、視点変換、左右鏡映、特徴番号 |
| `data/taper.py` | `taper.py` | 盤上の駒数による補間係数、局面帯、端点の識別性 |
| `data/lookahead.py` | `lookahead.py` | 先読み教師（将来の探索値の幾何加重平均） |
| `pst/model.py` | `train_pst.py` の一部 | 線形PSTのモデルとロジット |
| `pst/teacher.py` | `train_pst.py` の一部 | 教師値の構成と尺度Kの推定 |
| `pst/removal.py` | `train_pst.py` の一部 | 駒除去の参照と損失 |
| `pst/evaluate.py` | `train_pst.py` の一部 | 整数と実数の参照評価、初期局面の評価 |
| `pst/train.py` | `train_pst.py` の残り | 検証損失、学習ループ、コマンドライン（`init`、`estimate-k`、`train`） |
| `workflow.py` | `pst_workflow.py` | 学習工程の4コマンド |
| `diagnostics/comparison.py` | `pst_diagnostics.py` | 基準PSTと候補PSTの帯別、駒除去、成りの比較 |
| `diagnostics/taper_report.py` | `taper_report.py` | 駒数分布と端点の識別性の報告 |
| `diagnostics/lookahead_teacher.py` | `lookahead_diag.py` | 先読み教師の事前診断 |
| `checksum.py` | 3か所の重複（「重複の統合」の節） | ファイルのSHA-256 |
| `pst.example.toml` | `pst/pst.example.toml` | 学習工程の設定例 |

`train_pst.py` の関数と定数の置き場所は次のとおりである。
置き場所は、その関数を使う側ではなく、関数が表す概念で決める。
たとえば `_selected_indices` は検証損失と学習ループからも呼ばれるが、訓練分割から尺度Kを推定するための抽出なので `pst/teacher.py` に置く。

| 移動先 | 関数と定数 |
|---|---|
| `data/features.py` | `PAWN_STATE`、`ROYAL_STATES`、`REACHABLE_NON_ROYAL_STATES`（駒状態の分類） |
| `data/mnpt.py` | `HEADER_LENGTH`、`FORMAT_VERSION`、`RULE_SET`、`BODY_LENGTH`、`FILE_LENGTH`、`EVALUATION_LIMIT`、`PIECE_VALUES`、`PROMOTABLE_KINDS`、`STATE_KIND`、`validate_piece_values`、`_rule_field`、`write_mnpt`、`read_mnpt`、`initial_weights`、`initial_piece_values`、`quantize`、`float_weights_path` |
| `pst/model.py` | `MODEL_KINDS`、`MirroredEmbedding`、`make_model`、`expanded_model_weights`、`phase_weights`、`model_logits` |
| `pst/teacher.py` | `binary_cross_entropy_sum`、`estimate_k`、`_selected_indices`、`_selected_scores_results`、`estimate_generation_ks`、`estimate_mixed_k`、`build_targets` |
| `pst/removal.py` | `REMOVAL_MARGIN_CP`、`make_removal_reference`、`removal_loss` |
| `pst/evaluate.py` | `QUANTIZATION_ERROR_LIMIT`、`integer_evaluate`、`float_evaluate`、`initial_position_score` |
| `pst/train.py` | `validation_loss`、`TrainEpochResult`、`train_epoch`、`command_init`、`command_estimate_k`、`_format_validation_loss`、`_print_feature_observations`、`should_replace_best_epoch`、`command_train`、`build_parser`、`main` |

この表にない定義が見つかった場合は、同じ規則で置き場所を決め、実装の報告に挙げる。

コンソールスクリプトは次の5つである。
名前は旧ファイル名に合わせ、利用者が旧手順から対応を推測できるようにする。

| コマンド | 呼び出す関数 |
|---|---|
| `pst-workflow` | `minase_train.workflow:main` |
| `train-pst` | `minase_train.pst.train:main` |
| `pst-diagnostics` | `minase_train.diagnostics.comparison:main` |
| `taper-report` | `minase_train.diagnostics.taper_report:main` |
| `lookahead-diag` | `minase_train.diagnostics.lookahead_teacher:main` |

`pyproject.toml` には、`[build-system]` に uv_build を、`[project]` に静的な `version` を加える。
プロジェクト名 `minase-train` は uv_build がモジュール名 `minase_train` へ正規化するので、モジュール名の設定は不要である。

## 重複の統合

次の重複は、値と処理が同一であることを確かめたうえで1か所の定義へ統合する。

| 重複 | 統合先 |
|---|---|
| `pst_workflow.digest` と `taper_report.digest`（16進文字列）、および `mnsd.sha256_file`（バイト列） | `checksum.sha256_file`。16進文字列が必要な呼び出し側は `sha256_file(path).hex()` と書く |
| `train_pst.RULE_SET`（`b"L0,P0,R1,E0"`）と `pst_workflow.RULES`（`"L0,P0,R1,E0"`） | `data/mnpt.py` の `RULE_SET`。`workflow` は復号した文字列を使う |
| `train_pst.PIECE_STATE_COUNT` と `features.PIECE_STATE_COUNT`（ともに47） | `data/features.py` |
| `features.NO_LION_SQUARE` と `mnsd.NO_LION_SQUARE`（ともに255） | `data/features.py` |
| `pst_workflow.PIECE_VALUE_BYTES = 47 * 4` | MNPT の末尾にある探索用駒価値（駒状態47個ぶんの int32）のバイト数なので、`data/mnpt.py` で `PIECE_STATE_COUNT * 4` として定義する |
| 2つのテストの `PIECE_VALUES = initial_piece_values()` | `tests/helpers.py` |

次の同名の定義は、値または型が異なるので統合しない。

| 定義 | 統合しない理由 |
|---|---|
| `PROMOTABLE_KINDS` | 値は同じだが、`features` は int16、`train_pst` は int32 である |
| `write_mnsd`（本体とテスト） | 本体はレコード配列を検証して書き、テストは人工のレコードと来歴JSONを作る |
| `HEADER_LENGTH` | MNSD は136バイト、MNPT は80バイトで、別の形式の定数である |

実装時には、この2つの表以外の重複定義（同名で同じ本体を持つ関数や定数）を列挙して報告する。
報告された重複のうち処理が同一のものは同じ規則で統合し、少しでも処理が異なるものは統合せずに報告だけを残す。

## 段階9の診断の削除

3本の診断スクリプトと、それらだけを検査するテストを削除する。
削除によってどこからも使われなくなるコードも、未使用コードを温存しない方針により削除する。
`docs/measurements/` の Python スクリプトは、`KingFeatures` の列選択、`COLUMN_COUNT`、および `DEFINITION_ID` を使っていないことを確認済みである。

| 対象 | 扱い |
|---|---|
| `KingFeatures` の列選択の引数 `extra_columns` | 削除する。残る利用者 `lookahead_diag.py` は全列を読む |
| `mnsd.DEFINITION_ID` と `mnsd.COLUMN_COUNT` | 削除する。テストの人工データは幅68を直接書く |
| `KingFeatures` のインスタンス属性 `definition_id` と `column_count` | 残す。`lookahead_diag.py` が入力の検証に使う |

`test_king_features_diag.py` と `test_king_feature_selection.py` のうち、次のテストは `KingFeatures` 自体を検査するので、全列を読む形に書き換えて残す。

- 全列の読み込み
- 入力のSHA-256の不一致の拒否
- ヘッダと本体の長さの不一致の拒否
- ファイルをまたいだ大域番号の順序
- 定義と列幅が混在する入力の拒否

列選択だけを検査する2つのテスト、`parse_ranges` のテスト、および診断の統計を検査するテストは削除する。
`test_phase4.py` の、教師値を付け直した後も元の特徴行へ対応することを検査するテストは、第3引数 `[0]` を省いて残す。

## 学習工程の変更点

`workflow.py` は、配置の変更に合わせて次の3点だけを変える。

1. リポジトリのルートは `ROOT = Path(__file__).resolve().parents[4]` とする。`workflow.py` の位置 `tools/train/src/minase_train/workflow.py` から数えて4階層上がリポジトリのルートである。import 時に `ROOT / "Cargo.toml"` が存在しなければ例外を出す。`prepare` はこのディレクトリで `git worktree add` を実行するので、将来の移動で階層がずれた場合に誤ったディレクトリで実行させない。
2. ソースの検査和 `source_hashes` は、パッケージのディレクトリ `tools/train/src/minase_train/` 以下のすべての `.py` ファイルを再帰的に対象とし、このディレクトリからの相対パス（たとえば `data/mnsd.py`）をキーにする。テストはパッケージの外へ出るので、ファイル名の `test_` による除外は不要になる。
3. 学習の子プロセスは、`train_pst.py` のパスではなく `[sys.executable, "-m", "minase_train.pst.train", "train", ...]` で起動する。以降の引数は変えない。

この3点は既存のテストでは検出できないので、次のテストを新たに加える。

- 下位ディレクトリの `.py` と `__init__.py` が相対パスのキーで記録され、別のディレクトリにある同名のファイルが別のキーになること。
- パッケージ内のソースの変更、追加、削除で後続の工程が停止し、`tests/` の変更では停止しないこと。
- `ROOT` が実際の作業ツリーのルートと一致すること。
- 学習の子プロセスの先頭の引数が `[sys.executable, "-m", "minase_train.pst.train", "train"]` であること。

## 測定スクリプトの書き換え

`docs/measurements/` の次のスクリプトは、作業ツリーの `tools/train/pst` を `sys.path` に差し込み、平らなモジュール名で import している。

- `qsearch-output-leaf-sample/` の `leaf_sample.py` と `test_leaf_sample.py`
- `qsearch-output-diagnostics/` の `diagnose.py` と `test_diagnose.py`
- `relational-correction-prep/` の `phase0.py`、および `prep/` の `samples.py`、`removal.py`、`phase3_common.py`、`g_candidate.py`、`test_phase3.py`

これらは、差し込むパスを `tools/train/src` に、import をパッケージ名つき（`from minase_train.data.mnsd import ...` など）に書き換える。
検査和を記録するためにソースのパスを列挙している箇所（`samples.py` の `identity(WT / 'tools/train/pst/mnsd.py')` など）も新しいパスへ書き換える。
固定コミットのワークツリー `fm-eval-tools` を指す行（`g_star.py`、`g_candidate.py` の一部、`test_prep.py`）は書き換えない。

`relational-correction-prep/prep/` の `phase3_common.py`、`g_candidate.py`、および `test_phase3.py` は、master に存在しないモジュール `mnpt_v3` を import する。
このモジュールは未統合の関係補正項のブランチにだけあり、これらのスクリプトは再編前から master では実行できない。
本書はこの欠落を補わず、`mnpt_v3` の import はそのまま残す。

書き換えの前に、各スクリプトを import できるかと、各ディレクトリのテストスクリプトのうちデータを必要としないテストが通るかを記録する。
書き換えの後に同じ確認を行い、結果が書き換え前と一致することを確かめる。

## 文書

`tools/train/README.md` は、次の内容を持つ。

1. 学習ツールの目的と、本体の Rust 側（`selfplay_gen`、`pst_probe`、`nets/pst.bin`）との関係。
2. `uv sync` による環境の構築、テストの実行、およびコンソールスクリプトの呼び方。
3. 「新しい配置」の節の表と同じ粒度で、各ソースファイルが何のためにあるかの説明。
4. 交換するファイル形式（MNSD、MNRS、MNKF、MNPT）の一覧と、それぞれを読み書きするモジュール。
5. 学習手順の詳細は `docs/guides/pst-training.md` にあるという案内。

`docs/guides/pst-training.md` は、`tools/train/.venv/bin/python tools/train/pst/<名前>.py` の形の呼び出しをすべてコンソールスクリプトへ、テストの実行コマンドを新しいディレクトリへ、ソースと設定例へのリンクを新しいパスへ書き換える。
環境構築の段落は、`.venv` の手作業による用意から `uv sync --project tools/train` へ書き換える。
`src/bin/pst_probe.rs` の文書コメントは、呼び出し元のパスを `tools/train/src/minase_train/diagnostics/comparison.py` へ書き換える。

## 実装フェーズ

実装は codex へ委任し、1回の委任で1フェーズを実行させる。
codex のサンドボックスはネットワークに接続できず、ワークツリーでコミットもできないので、`uv lock`、`uv sync`、およびコミットはメインセッションが行う。
フェーズ2までは、既存の仮想環境 `tools/train/.venv` の Python を直接使い、`PYTHONPATH=tools/train/src` を与えてテストを実行する。
`uv run` は実行前に依存の解決と同期を行うので、版を固定するフェーズ3より前には使わない。

1. 段階9の診断の削除。「段階9の診断の削除」の節を適用する。完了条件は、残るテストがすべて通り、削除したテストがいずれも削除したスクリプトまたは削除した引数に依存するものであることである。フェーズ1のコミット後、メインセッションは「検証」の節の5で比較する基準の出力を、この時点のコードと既存の仮想環境で作って保存する。
2. パッケージへの移動と学習器の分割。「新しい配置」の節のとおりにファイルを `git mv` で移し、`train_pst.py` を分割し、import をパッケージ名つきに書き換え、「重複の統合」と「学習工程の変更点」を適用し、テストを `tools/train/tests/` へ定義元モジュールごとに再配置する。完了条件は「検証」の節の1と2、および既存の仮想環境で全テストが通ることである。
3. uv による依存関係の固定（メインセッション）。`uv lock` で `uv.lock` を作り、`--upgrade-package` で torch 2.13.0 と numpy 2.5.2 に固定し、`uv sync` の後に「検証」の節の3から5を確かめる。
4. 測定スクリプトの書き換え。「測定スクリプトの書き換え」の節を適用する。
5. 文書（メインセッション）。「文書」の節を適用する。

コミットは、フェーズ1を `chore(train)`、フェーズ2を `refactor(train)`、フェーズ3を `build(train)`、フェーズ4とフェーズ5を `docs` とする。
いずれも minase の USI オプション、コマンドライン引数、規則コード、および Rust の公開APIを変えないので、互換性を壊す変更の印 `!` は付けない。

## 検証

1. フェーズ1完了時点のテストメソッドの名前の多重集合が、フェーズ2完了時点の多重集合に含まれ、差が「学習工程の変更点」の節で加えたテストだけである。テストクラスは再配置で分割され得るので、クラス名は比較に含めない。
2. 移動前後のソース行の多重集合の差が、次のものだけである。比較は、フェーズ1完了時点の `tools/train/pst/` のテスト以外の `.py` と、フェーズ2完了時点の `tools/train/src/` の `.py` の間で行い、テストも旧テストと新テストの間で同じ方法で行う。
   - import 文、モジュールの文書文字列、および `__init__.py`。
   - 「重複の統合」の節で削除した定義と、それに伴う呼び出し式の書き換え（`digest(path)` から `sha256_file(path).hex()` など）。
   - 「学習工程の変更点」の節の3点と、そこで加えたテスト。
   - テストにおける、移動したファイルへの参照パス（`pst.example.toml` の位置など）、`unittest.mock.patch` の対象名（`"train_pst.estimate_k"` から呼び出し側のモジュールの名前へ）、補助関数の移動、およびクラスの分割に伴うクラス定義と `setUp` の複製。
3. リポジトリのルートで `uv run --locked --project tools/train python -m unittest discover -s tools/train/tests` を実行し、すべて通る。`--locked` は `uv.lock` が `pyproject.toml` と食い違えば失敗するので、ロックの更新漏れを検出できる。
4. リポジトリのルートを作業ディレクトリとして、5つのコンソールスクリプトがそれぞれ `uv run --locked --project tools/train <名前> --help` に応答する。
5. テストの補助関数で作る小さな合成データを入力に、フェーズ1完了時点のコードと再編後のコードで同じ処理を実行し、出力を比べる。比較はCPU（`--device cpu`）と同じ乱数の種で行い、PyTorch の版も torch 2.13.0 で揃えるので、浮動小数点の計算順序は変わらない。
   - `train-pst train` が出力するMNPTと実数重みのファイルが、旧 `train_pst.py train` の出力とバイト単位で一致する。
   - `taper-report` の出力JSONが、旧 `taper_report.py` の出力と、時刻とパスを記録する欄を除いて一致する。
   - `lookahead-diag` の出力JSONが、旧 `lookahead_diag.py` の出力と同じ条件で一致する。入力には、MNSD に加えて定義2の118列のMNKF、基準のMNPT、および露出標本のJSONが必要であり、`test_lookahead.py` の `diagnostic_files` と同じ方法で作る。

## 完了条件

「検証」の節の1から5をすべて満たし、`docs/guides/pst-training.md` の手順に現れるすべてのコマンドが新しい配置で実行できる状態で master へ統合したとき、本書を完了（採用）とする。

## 参考資料

- [PSTの学習手順](../guides/pst-training.md)
- [中核モジュールの再編の設計書](core-layout.md)（移動前後の行の多重集合による検証の先例）
- [uv のプロジェクト管理](https://docs.astral.sh/uv/concepts/projects/)
- [uv のビルドバックエンド](https://docs.astral.sh/uv/concepts/build-backend/)
