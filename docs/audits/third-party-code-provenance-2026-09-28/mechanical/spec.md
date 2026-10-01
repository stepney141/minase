# M1〜M4 機械的照合の仕様（codex 担当）

監査計画の全体は ../../gpl-audit-plan.md にある。本書はその M1〜M4 の実装と実行を依頼する。
報告は日本語で書く。

## 制約
- minase（<minase-repo>）は読むだけで変更しない。作業ツリーには未コミットの変更があり得るので、ファイルは `git show a05478a:<path>` か `git archive a05478a` で取り出した一時コピーから読む。
- コーパスは scratchpad/corpus/manifest.tsv の local_path にある。クローンも変更しない（別の版が必要なら `git -C <clone> show <rev>:<path>` か `git -C <clone> archive <rev>` で一時ディレクトリへ）。
- GitHub API は使わない。ネットワークからのツール導入（npx、pip、Maven からのダウンロード）は可。導入した版を記録する。
- スクリプトと出力はすべて scratchpad/audit/m1-m4/ に置く。スクリプトは後で docs/audits/ へ移して再実行できるよう、パスを引数か先頭の定数で受け取り、依存と実行手順を README.md に書く。
- 実行時間が長くなる処理は nice 19 で動かす（同じ機械で棋力測定が動いている場合に測定を乱さないため）。

## 対象の minase 側ファイル
1. 基準コミット a05478a の追跡ファイルのうちコード系（.rs、.py、.sh、.toml、docs 配下の .py と .patch を含む）。
2. 全ref（`git rev-list --objects --all`）に現れたコード系 blob（約1,525個）。blob ごとに、それが現れた最初のコミットとパスを記録する。
各ファイル・blob に、適用した手法、解析の成否、除外理由を付けた被覆表 coverage.tsv を作る。

## 対象のコーパス
manifest.tsv の category が copyleft または none の実装すべてを主対象にし、permissive と public-domain も同じ処理にかけて帰属表示の要否を記録する。
Stockfish は HEAD と sf_10〜sf_19 の全タグ、YaneuraOu は HEAD と主要タグ（V5.00、v6.00、v7.00、v7.10、V7.61、v8.00、V8.30、V9.00、v9.10、v9.40 と、minase の文書が固定リンクで参照する 33ccf1f と 0a6dd2cb^）を対象にする。fishtest は b8eecff220b562a0dc2c4e68d1fa02521e06d72c も対象にする。
同梱ライブラリのディレクトリ（3rdparty、third_party、external、deps、vendor、incbin など）は除いてよいが、除いたことを記録する。

## M1 注釈と文字列
- 抽出: Rust/C/C++/Python/Scala/TypeScript のコメント（docコメント含む）と文字列リテラル。tree-sitter か言語ごとの字句解析で行い、正規表現だけに頼らない。
- 正規化: Unicode NFKC、句読点と空白の統一、エスケープ展開、隣接文字列の連結、識別子の snake/camel 統一。原文と正規化文の両方を保存する。
- 照合: 英語は5語以上、日本語は12文字以上の n-gram の一致。加えて、長さは短くても希少な句（識別子や式を含む句、コーパス全体で出現が1〜2回の句）を別枠で候補化する。
- 除外: 定型文（ライセンス文、"TODO" など）、USI/CECP/SFEN の仕様語、RULES.md の条文引用。除外件数を規則ごとに数える。
- 出力: m1_hits.tsv（minase 側位置、コーパス側位置、一致文、一致長、希少度、除外理由）。

## M2 数値定数
- 抽出: 全数値リテラル（桁数の下限なし）。符号、基数、型接尾辞、区切り（_ や '）、単位（百分率、千分率、ミリ秒）を正規化する。minase の params.rs はマクロで既定値と範囲を宣言しているので、マクロ呼び出しを解析して名前・既定値・範囲を取り出す。
- 機能単位の束ね: 定数を、それが現れる関数（または params.rs の名前の接頭辞）ごとの集合にまとめる。コーパス側も関数ごとの集合にする。
- 照合: minase の各集合とコーパスの各集合で、自明な値（0、1、2、-1、10、100、1000、2の冪など、コーパス全体の出現頻度で上位の値）を除いた共通要素が2個以上ある組を候補にする。単独一致は件数だけ数える。
- params.rs は git 履歴上の各係数の導入時の値でも同じ照合を行う（`git log -L` かマクロ行の履歴）。
- 出力: m2_hits.tsv と、候補の組ごとの共通値の一覧。

## M3 定数表
- 抽出: 要素数8以上の配列リテラル（Rust の const/static 配列、C/C++ の配列初期化子、Python のリスト）。
- 照合: 完全一致、正の定数倍（比が一定、丸め誤差±1を許す）、部分列（長さ8以上）、並べ替え（多重集合の一致）。
- 較正: src/eval/handcrafted.rs の PIECE_VALUES は HaChu（debian_hachu の hachu.c または ddugovic 版の variant.c の chuPieces[]）の値を2.5倍して丸めたと明記されている。これを M3 が検出できることを確認し、できなければ手法を直す。
- 出力: m3_hits.tsv。

## M4 同言語のクローン検出
- Rust 対 Rust（apery_rust、rshogi、shakmaty、Reckless、viridithas、akimbo、bullet、hobbes ほか manifest の Rust 実装）と Python 対 Python（fishtest、nnue-pytorch、lczero-training、OpenBench、python-shogi、DeepLearningShogi の Python 部、nettest、fishutils ほか）。
- ツールは JPlag（java あり）か Dolos（node あり）。版と言語指定を固定する。edition 2024 とマクロを含む minase の全 .rs で解析成功率を測り、失敗したファイルは coverage.tsv に残す。JPlag が解析できない場合は、tree-sitter でトークン列を作って自前で winnowing（MOSS 方式）を行ってもよい。
- 閾値の較正: 最小一致長を 20、40、80 トークンで比べる。既知の複製例（コーパスの関数を1つ minase 風に改名・並べ替えした人工例）と独立実装例（同じ機能の別エンジン同士、例えば Reckless と viridithas の同名機能）で、検出率と誤検出を測って閾値を決める。
- 全体の一致率ではなく、局所一致ごとの長さと被覆率を記録する。
- C++ コーパスに対する言語をまたぐ比較（tree-sitter で演算・分岐列を共通化した比較）は、既知の翻訳例で検出力を確かめられた場合だけ補助として使い、確かめられなければ実施しない旨を記録する。
- 出力: m4_hits.tsv。

## 報告
- 各手法の実行コマンド、ツールの版、閾値と較正結果、候補件数、除外件数、解析失敗件数。
- 候補（除外後）の上位を、minase 側位置、コーパス側位置、ライセンス、一致の内容とともに一覧にする。候補の判定（複製か偶然か）は行わなくてよいが、明らかな偽陽性には理由を付けて印を付ける。
- 返信は日本語で1,000語以内。詳細はファイルに置く。
