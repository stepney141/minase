# 第三者コード混入監査の計画（第2版、codexレビュー反映）

## 目的と結論の書き方

minase（中将棋のRust実装、基準コミット master a05478a）に、第三者実装のコードが複製、逐語的な翻訳、または識別子や分割を変えた翻案として入っていないかを調べる。
主対象はGPL-3.0、AGPL-3.0、GPL-2.0、および無許諾の実装であり、許容型の実装（MIT、Apache-2.0、パブリックドメインなど）は帰属表示と変更表示の要否を記録する。
利用者は監査結果を見てからライセンス（GPL-3.0-or-later または MIT OR Apache-2.0）を決める。
結論は「指定したコミット、コーパス、手法、閾値の範囲では検出されなかった」「検出され、次のとおりである」の形に限定し、未解析、未取得、判定保留の件数を併記する。「保証」とは書かない。
報告は docs/audits/third-party-code-provenance-2026-09-28.md とし、再実行用スクリプトと出力を同名ディレクトリに置く。

## 対象範囲

### 現行版（HEAD）
git ls-files の全追跡ファイル。各ファイルに、適用した手法、解析結果、除外理由を割り当てた被覆表を作る（未解析0件を目標にし、残れば件数を報告する）。
docs/ 配下のPython（docs/measurements/magic-bitboard-prototype/compare.py など）とパッチ（*.patch）もコードとして扱う。
tests/fixtures の棋譜は第三者データとして出所だけを記録する。

### 履歴
全ref（ローカルブランチ50本、805コミット）に現れたコード系blob（.rs、.py、.sh、.toml ほか、1,525個、約42 MB）を機械的手法（M1、M2、M4）の対象にする。
HEADに存在しない削除済みファイル48件のうち、次は対照レビュー（M5）の対象にする。
src/eval/nnue.rs、tools/train/nnue_net.py、tools/train/train_nnue.py、src/movegen/lion.rs ほか旧合法手生成、build.rs、scripts/pgo_*.py、tools/lishogi-bot/*。
削除済みの重み nets/nnue.bin（約7 MB）は M3 の来歴確認の対象にする。

## 比較コーパス

文書中のリンク集計に加え、全追跡テキストから実装名、参照ファイル名、非GitHubのURL（Savannah の XBoard、variant-nnue-tools など）を抽出して補う。
取得単位は「実際に参照したコミット」（文書中のコミット固定リンク、例: Stockfish 5062aee、YaneuraOu 33ccf1f と 0a6dd2cb の親、fishtest b8eecff、scalashogi 9a1c2c3、shogiops e295794）と HEAD、および Stockfish と YaneuraOu の主要リリースタグとする。
ライセンスは参照版のファイル単位で判定する（例: YaneuraOu の source/incbin は UNLICENSE）。
中将棋の規則実装の対照は、実際の参照先である scalashogi（MIT）と shogiops（GPL-3.0）、および HaChu（パブリックドメイン）とする。手元の Fairy-Stockfish は120升までで中将棋を実装しないため、対照から外す（別フォークの有無は確認して記録する）。

## 手法

### M1 注釈と文字列の一致
コメント、docコメント、文字列リテラルを抽出し、原文と正規化文（Unicode NFKC、句読点と空白の統一、エスケープ展開、文字列連結の解消、識別子の snake/camel 統一）を保存する。
英語は5語以上、日本語は12文字以上のn-gramで照合し、短くても希少な句（固有の変数名や式を含む句）を別に候補化する。
一致は、定型文、仕様（USI、規則文）の引用、一般用語を除いたうえで判定する。

### M2 数値定数
符号、基数、型接尾辞、区切り文字、単位（百分率、千分率）を正規化し、宣言と使用箇所を結ぶ。桁数の下限は置かない。
params.rs のマクロ内の既定値と範囲、式の係数は、同じ式または同じ機能の定数集合として扱う。
ある機能（LMR、futility、history bonus/malus、aspiration、reduction表、時間配分、SEE閾値など）の定数集合がコーパスの対応機能の定数集合と2個以上一致する場合を候補とし、単独一致は雑音として件数だけ数える。
params.rs は導入時の初期値（git履歴）も照合する。

### M3 定数表と重み
埋め込み配列を抽出し、完全一致に加えて、倍率、丸め、部分列、並べ替えを考慮して照合する（handcrafted.rs は HaChu の表を2.5倍して丸めたと明記しており、既知の由来をこの手法で再検出できることを較正に使う）。
表を生成するコード（magic数の探索など）も対照する。
nets/pst.bin、nets/pst-init.bin、削除済みの nets/nnue.bin は、ファイルのハッシュから、初期重み、学習入力、実行記録まで遡って来歴を確定する。遡れない場合は「来歴未確定」として報告する。

### M4 同言語のクローン検出
Rust（apery_rust、rshogi、shakmaty ほか）と Python（fishtest、nnue-pytorch、lczero-training、OpenBench、python-shogi ほか）のコーパスに対して、トークン単位のクローン検出を行う。
ツールの版と言語指定を固定し、edition 2024 とマクロを含む minase の全ファイルで解析成功率を記録する（失敗したファイルは被覆表に残す）。
最小一致長を20、40、80トークンで比べ、既知の複製例（人工的に作った改名・並べ替え例）と独立実装例で閾値を較正する。全体の一致率ではなく、局所一致の長さと被覆率を記録する。
C++ のコーパスには、tree-sitter で共通化した演算・分岐列の比較を候補抽出の補助として試し、既知の翻訳例で検出力を確かめたうえで使う。検出力が確認できなければ使わない。

### M5 対応モジュールの対照レビュー
対応するモジュールの組を、組ごとに独立した reviewer が両側の原文を読み比べる。
判定は4段階とする。「複製または翻案を支持する証拠あり」「標準的な方式または仕様で説明可能」「判定保留」「未確認」。
各判定には、両側の行対応、任意の実装選択（同じ結果に至る別の書き方があったのに同じ選択をしている箇所）、反証（異なる選択をしている箇所）、導入コミットを記録する。
数式や仕様上必須の一致（例: stats.rs が fishtest と同じ出力を出すこと）と、実装固有の一致（変数名、分岐順、補助関数の分割、特異な定数、コメント）を分けて扱い、前者だけを根拠に翻訳と判定せず、後者を理由なく除外しない。
「複製または翻案を支持する証拠あり」と「判定保留」は、別の reviewer が独立に再判定する。

必読の組（M1からM4で信号が出たモジュールを追加する）:

| minase | 対照 |
|---|---|
| src/search/alphabeta/（探索本体、枝刈り、延長、LMR、静止探索） | Stockfish search.cpp、YaneuraOu 探索部、Ethereal、Obsidian、Alexandria、Reckless（AGPL） |
| move ordering、history、correction history | Stockfish movepick.cpp、history.h、YaneuraOu 対応部 |
| 置換表、並列探索とワーカー選択 | Stockfish tt.cpp、thread.cpp、YaneuraOu tt.h、yaneuraou-search.cpp |
| 時間管理 | Stockfish timeman.cpp、YaneuraOu timeman.cpp |
| SEE | Stockfish see_ge、YaneuraOu see_ge |
| 利きと bitboard、magic、局面更新（make/unmake）、zobrist、乱数（src/rng.rs） | Stockfish bitboard.cpp、position.cpp、misc.h（PRNG）、YaneuraOu 対応部 |
| 獅子と2段階移動の合法手生成、獅子捕獲規則、旧 src/movegen/lion.rs | scalashogi、shogiops、HaChu |
| src/stats.rs、src/bin/spsa_runner | fishtest の LLRcalc.py、stat_util.py、spsa 処理、OpenBench |
| src/harness、match_runner、protocol/cecp.rs | cutechess、fastchess、XBoard |
| protocol/usi.rs、notation | YaneuraOu usi.cpp、Stockfish uci.cpp |
| tools/train/pst、削除済みの nnue_net.py と train_nnue.py、src/eval/nnue.rs | nnue-pytorch、YaneuraOu learner.cpp と nnue 部、elmo_for_learn、variant-nnue-tools |

### M6 文書とデータ中の引用
docs/ のコードブロック、インライン引用、パッチから、第三者実装のコードを抜き出した箇所を列挙し、出典（リポジトリ、コミット、ファイル、行）、長さ、ライセンスを記録する。
処置は利用者が決める。残置や言い換えを処置完了に数えるのは、著作権法第32条（引用）の要件（必然性、主従関係、明瞭区別、出所明示）またはライセンスの条件を満たすことを確認した場合に限る。

## 報告の構成

基準コミット、対象範囲と被覆表、コーパス（実装、ライセンス、参照版、ファイル単位の例外）、手法ごとのコマンド・閾値・較正結果・検出一覧、M5 の全判定と再判定、文書中の引用の一覧、重みの来歴、限界、未解析・未取得・判定保留の件数。
限界には、学習データに含まれるがコーパスにない実装は検出できないこと、言語をまたぐ翻案の検出は主にレビューに依存すること、を書く。

## 再発防止

AGENTS.md に規則を追加する案を利用者に提示する。案の内容は採用するライセンスに依存するので、監査結果とライセンスの決定の後に確定する。

## 分担

M1からM4のスクリプトは codex に実装と実行を任せる。
M5 は組ごとに独立した Claude の subagent（5体程度）に読ませ、陽性と保留の再判定は別の subagent に任せる。
M6 は1体の subagent に任せる。
Claude が結果を統合し、全検出を分類して報告を書く。
