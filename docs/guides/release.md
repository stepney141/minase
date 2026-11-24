# リリースの手引き

本書は、ブランチ運用、版数の決定と出口条件、クレートのリリース手順、および評価ネットの独立配布の手順を定める。
コミットメッセージの規約と、変更履歴に載せるコミットの種別は [CONTRIBUTING.md](../../CONTRIBUTING.md) にある。

## 版数管理の対象と方針

ワークスペースに含まれる3つのクレートと外部の評価ネットは、それぞれ異なる役割を持つため、版数管理の単位を分ける。

| 対象 | 版数方式 | タグの形 | 変更履歴と配布先 |
|---|---|---|---|
| ライブラリ `minase-core` | セマンティックバージョニング（0.x系） | `minase-core-vX.Y.Z` | `crates/minase-core/CHANGELOG.md`、GitHub リリース、および crates.io |
| エンジン `minase` | セマンティックバージョニング | `vX.Y.Z`（プレリリースは `-alpha.N`、`-rc.N` を添える） | リポジトリルートの `CHANGELOG.md`、GitHub リリース、および crates.io |
| 学習器 `minase-trainer` | 対象外（`publish = false`） | 付けない | 開発専用ツールのため配布しない |
| 評価ネット（NNUE の `.nnue`、PV-MCTS の `.onnx`） | `minase-<方式>-<構造>-<規則>-<YYYYMMDD>-<SHA256先頭8桁>` | コードリポジトリにはタグを付けない | Hugging Face Hub のモデルリポジトリ（`stepney141/minase-nets`） |

`minase-core` と `minase` はそれぞれ独立した版数を持ち、1クレートずつリリースする（[crateごとに版数を付けるADR](../adr/release-each-crate-separately-and-publish-by-hand.md)）。
評価ネットはエンジン本体から分離し、同じ評価形式のまま強いネットが得られた場合はエンジンの版数を上げずに Hugging Face Hub へ公開する（[評価ネットをHugging Face Hubへ置くADR](../adr/host-evaluation-networks-on-hugging-face-hub-and-name-them-by-structure.md)）。

## ブランチ運用

日常の開発はトランクベース開発とし、計画書ごとの作業ブランチを `master` へ統合する。
常設の `develop` ブランチは作らない（[トランクベースと必要時リリースブランチのADR](../adr/release-from-master-with-on-demand-release-branches-and-milestone-criteria.md)）。

### 既公開版のパッチ保守（`release/X.Y`）

公開済みの版に不具合修正を出す必要が生じ、かつ `master` が破壊的変更や次期開発へ進んでいる場合は、該当タグから保守ブランチを切る。
ブランチ名は、エンジンが `release/X.Y`（例: `release/2.6`）、ライブラリが `release/minase-core-X.Y`（例: `release/minase-core-0.3`）とする。
修正はまず `master` へ入れ、そのコミットを保守ブランチへ `git cherry-pick` してからパッチ版をリリースする。

```console
git switch -c release/2.6 v2.6.0
git cherry-pick <修正コミット>
```

### 次期版の安定化とリリース候補

メジャー版やマイナー版の公開前に破壊的変更を凍結し、その間も `master` で後続の研究開発を続けたい場合は、`master` から `release/X.Y`（例: `release/3.0`）を切る。
安定化ブランチ上で `-rc.N` の検証と修正を進め、正式版を出した後に安定化ブランチ上の修正コミットを `master` へ取り込む。
保守の役目を終えた `release/*` ブランチは、タグが残っていることを確認してからローカルとリモートから削除する。

## リリースの出口条件とプレリリース運用

感覚的なタイミングでリリースを切らず、クレートごとに次の基準で判断する。

`minase-core` は需要駆動で小さくリリースする。
不具合修正や互換性のある機能追加が入り、`minase` または外部利用者が必要とした時点で速やかに版数を上げる。
破壊的変更は利用者の追従負担を抑えるため、可能な範囲でまとめて出す。

`minase` の正式版（`vX.Y.Z`）は、次の4条件をすべて満たした時点でリリースする。

1. 予定している公開 API、起動引数、USI（Universal Shogi Interface）オプション、およびファイル形式の互換性変更が確定している。
2. [テストの手引き](testing-minase-rules.md#マージ前の検査) が定める検査と GitHub Actions の継続的インテグレーション（Continuous Integration、以下 CI）がすべて通っている。
3. 次節の手動確認一覧をすべて終えている。
4. 探索、評価、または時間管理に変更を含む場合、推奨する評価ネットと組み合わせた棋力が直前の正式版を下回らないことを確認している。学習の移行途中などで一時的に弱くなっている状態では、正式版を出さない。

プレリリース版は、識別子の役割を次のように使い分ける。

- `-alpha.N`（例: `v3.0.0-alpha.1`）は、開発途中の動作確認や破壊的変更の先行共有に使う。棋力や仕様は未確定であってよい。
- `-rc.N`（例: `v3.0.0-rc.1`）は、破壊的変更を凍結したリリース候補版であり、`minase-gui` と `minase-lishogi-bot` の追従確認に使う。重大な問題が出なければそのまま正式版 `vX.Y.Z` へ昇格させる。

## リリース前の手動確認一覧

自動テストでは外部クライアントや配備環境との実接続を検証できないため、`minase` のリリース候補版または正式版を出す前に次の項目を手元で確認する（[テストの手引き](testing-minase-rules.md#外部との境界)）。

1. 対象のコミットで [マージ前の検査](testing-minase-rules.md#マージ前の検査) の全項目が通っていることを確認する。
2. `minase-gui` から対象バイナリを USI で起動し、`RuleSet` と `EvalFile` を渡して `isready` が `readyok` を返し、対局を1局以上進められることを確認する。
3. 起動引数、USI オプション、または規則セット `lishogi` の裁定に変更がある場合は、`minase-lishogi-bot` のコンテナ内で起動スクリプト経由の握手と `EvalFile` 読み込みが通ることを確認する。
4. 探索や評価の変更を含む正式版では、推奨ネットの測定記録が `docs/measurements/` または `docs/training/` にあり、直前の正式版を下回っていないことを確認する。

## クレートのリリース手順

リリースには `git-cliff` と `cargo-release` を使う。
Arch Linux では `sudo pacman -S git-cliff cargo-release` で導入できる。
設定は [cliff.toml](../../cliff.toml) と [release.toml](../../release.toml) にあり、`allow-branch` は `master` と `release/*` を許可している。
変更履歴は、ライブラリが `crates/minase-core/CHANGELOG.md`、エンジンがリポジトリルートの `CHANGELOG.md` であり、それぞれのクレートのディレクトリを変更したコミットだけを載せる。
両方を変更したコミットは両方に載る。
以下では、ライブラリの版数を 0.3.1、エンジンの版数を 3.0.0 として説明する。

### 1. ブランチの状態確認と版数の推定

対象ブランチ（`master` または `release/*`）を最新にし、作業ツリーがクリーンであることを確認する。
次の版数は、クレートごとのコミット種別から `git-cliff` が推定した値を参考に決める。
`fix` だけならパッチ、`feat` があればマイナー、互換性を壊す変更があればメジャーの版数が上がる。
ライブラリは 0.x 系なので、互換性を壊す変更ではマイナーの版数を上げる。

```console
git switch master
git pull
git status
git cliff --include-path 'crates/minase-core/**' --tag-pattern '^minase-core-v' --bumped-version
git cliff --include-path 'crates/minase/**' --bumped-version
```

### 2. `cargo-release` の予行と実行

まず `--execute` を付けずに予行し、生成される変更履歴と更新対象を確認する。
予行と本番は「Publishing minase-core」のような行を表示するが、`release.toml` の `publish = false` により `cargo publish` は呼ばれない。

```console
cargo release -p minase-core 0.3.1
```

内容に問題がなければ、`--execute` を付けて本番を実行する。
`cargo-release` は、対象クレートの `Cargo.toml` と `Cargo.lock` の版数を更新し、`git-cliff` で今回の節を変更履歴の先頭へ追加する。
そのうえで `chore(release): minase-core-v0.3.1` としてコミットし、注釈付きタグ `minase-core-v0.3.1`（タグメッセージは `minase-core v0.3.1`）を作る。
push と crates.io への公開は行わない。

```console
cargo release -p minase-core 0.3.1 --execute
git show --stat HEAD
```

エンジンも同じ手順でリリースする。
コミットの件名は `chore(release): v3.0.0`、タグは `v3.0.0`（タグメッセージは `minase v3.0.0`）になる。
エンジンが新しい版のライブラリを必要とする場合は、ライブラリを先にリリースする。

```console
cargo release -p minase 3.0.0
cargo release -p minase 3.0.0 --execute
```

### 3. 変更履歴の要約整備（正式版では必須）

自動生成された変更履歴には、同じリリース周期の途中で導入されて後に撤回・置換された変更がそのまま残ることがある。
push の前に変更履歴を開き、正式版では該当バージョンの見出し直下に利用者向けの要約を2〜3文で書き加え、途中で撤回されて最終状態に存在しない項目を整理する。
編集した場合はリリースコミットを amend し、注釈付きタグを付け直す。

```console
git commit -a --amend --no-edit
git tag -f -a v3.0.0 -m "minase v3.0.0"
```

### 4. push と GitHub Actions による下書きリリース確認

対象ブランチとタグを push する。

```console
git push origin master minase-core-v0.3.1 v3.0.0
```

`v[0-9]*` または `minase-core-v[0-9]*` のタグを push すると、[.github/workflows/release.yml](../../.github/workflows/release.yml) が自動で起動する（[GitHub Actionsでの検証と下書きリリースのADR](../adr/verify-commits-and-draft-releases-in-github-actions.md)）。
ワークフローは次の処理を行い、問題がなければ GitHub リリースの下書きを作成する。

1. タグ名と対象クレートの `Cargo.toml` の版数の一致を検査する。
2. `cargo fmt`、`cargo clippy`、`cargo test --workspace`、およびフィーチャ付きテストを実行する。
3. パッケージ内容を検査する。`minase` では `cargo package -p minase --list` の `nets/` 以下に本番の NNUE ネットが混入していないことを確かめる。
4. `scripts/release/release_checks.py` で変更履歴から該当版の節を抽出し、`--draft` 付きで GitHub リリースを作成する。プレリリース版には自動で `--prerelease` を付ける。

ワークフローの成功を確認したら、GitHub 上の下書きリリースを開き、エンジンでは推奨する評価ネットの Hugging Face Hub 上のダウンロード URL と SHA-256（PV-MCTS のネットを含む場合は `NnCpPerLogit` も）を追記してから公開する。
ネットのバイナリ自体は GitHub リリースへ添付しない。

### 5. crates.io への手動公開

crates.io への公開は取り消せないため、タグの検査と push を確認してから手で実行する。
公開の前に手元でも `cargo package -p minase --list` で、`nets/` 以下にリポジトリ追跡済みの固定ネットと PST 比較用ファイルだけがあることを確かめる。
エンジンの依存は crates.io 上のライブラリの版数で解決されるので、ライブラリを先に公開する。

```console
cargo package -p minase --list
cargo publish -p minase-core
cargo publish -p minase
```

## 評価ネットの公開手順（Hugging Face Hub）

エンジンは評価ネットを実行ファイルに埋め込まず、USI のオプション `EvalFile` または `NnModel` で指定した外部ファイルから読む（[EvalFileでのNNUE読み込みのADR](../adr/search-with-an-evalfile-nnue-whose-structure-is-chosen-on-load.md)）。
そのため、同じ評価形式のまま強いネットを学習できたときは、エンジンの版数を上げずに Hugging Face Hub 上のモデルリポジトリへネット単体を公開する（[公開先と命名のADR](../adr/host-evaluation-networks-on-hugging-face-hub-and-name-them-by-structure.md)）。

### リポジトリの構成とファイル名の規則

専用のモデルリポジトリ `stepney141/minase-nets`（`https://huggingface.co/stepney141/minase-nets`）を使い、形式ごとにディレクトリを分けて配置する。
公開ファイル名は `minase-<方式>-<構造>-<規則>-<YYYYMMDD>-<SHA256先頭8桁>.<拡張子>` の規則で付け、NNUE の拡張子は `.nnue`、AlphaZero 型探索の拡張子は `.onnx` とする。
`<規則>` は学習・評価の規則セットを小文字で書く（`jcsa`、`lishogi` など）。

NNUE の `<構造>` は、やねうら王の構造名に倣い、`[sfnn_]<特徴>_<片側の幅>x<第1隠れ層の出力数>x<第2隠れ層の出力数>[_psqt]` の形で書く。
各部分は、重みファイルの先頭にある構造記述（`Features=A(Friend)[13680->256x2],PSQT=1,Network=…` の文字列）から次のように決まる。
構造記述は `head -c 400 <ファイル> | strings | head -1` で表示できる。

- `sfnn_` は、2視点の出力の要素ごとの積を後段へ渡す Stockfish 型のネット（SFNN）に付ける。tatara の LayerStack がこれに当たり、2視点の出力を連結する Simple には付けない。
- `<特徴>` は `Features=` の後の名前を小文字にしたもので、A 型は `a`、HalfKP_vm は `halfkpvm` である。
- `<片側の幅>` は、`Features=` の `[<入力の次元>-><片側の幅>x2]` で `->` の直後にある数であり、`[13680->256x2]` なら `256` である。
- `<第1隠れ層の出力数>` と `<第2隠れ層の出力数>` は、`Network=` の式で入力側から1つ目と2つ目の `AffineTransform`（Simple の1つ目は `AffineTransformSparseInput`）の出力数である。SFNN の第1隠れ層の出力数は skip に回す1つを含む。やねうら王は同じ層を skip を除いた数で書き、たとえば出力数 16 の層を 15 と書くので、値が1だけ違う。
- `_psqt` は、構造記述に `PSQT=1` があるネットに付ける。

モデルリポジトリ内のパスは、次の形になる。

- `nnue/a/minase-nnue-sfnn_a_<片側の幅>x<第1隠れ層の出力数>x<第2隠れ層の出力数>_psqt-<規則>-<YYYYMMDD>-<SHA256先頭8桁>.nnue`（A 型の NNUE、tatara LayerStack。片側の幅が 256 なら構造名は `sfnn_a_256x16x32_psqt`）
- `nnue/halfkp-vm/minase-nnue-halfkpvm_<片側の幅>x<第1隠れ層の出力数>x<第2隠れ層の出力数>-<規則>-<YYYYMMDD>-<SHA256先頭8桁>.nnue`（HalfKP_vm の NNUE、Simple。片側の幅が 256 なら構造名は `halfkpvm_256x32x32`）
- `zero/minase-zero-<構造>-<規則>-<YYYYMMDD>-<SHA256先頭8桁>.onnx`（AlphaZero 型探索の ONNX ネット。例: `minase-zero-b20c256-lishogi-20261116-9a0b1c2d.onnx`）
- `README.md`（公開ネットの一覧と説明を載せるモデルカード）

具体例は `nnue/a/minase-nnue-sfnn_a_256x16x32_psqt-lishogi-20261124-41502087.nnue` である。
単体ファイルをダウンロードしてディレクトリ階層が外れても、ファイル名だけで構造、規則セット、作成日、および SHA-256 の先頭8桁を判別でき、同日に再学習しても末尾のハッシュで衝突しない。
2026年11月25日より前は、NNUE の `<構造>` に入力特徴集合の種類だけを書き、`minase-nnue-p-<規則>-<YYYYMMDD>-<SHA256先頭8桁>.nnue` の名前で公開していた。
この名前で公開した2本は、まずモデルリポジトリのコミット `01ccc6f471e9696a37acfb3a2ac753c0add15a39` で、`<構造>` に構造記述の詳細を入れた `sfnn_p_256x16x32_psqt` の名前へ付け替えた。それより前のリビジョンを固定した URL は、`minase-nnue-p-…` の名前のまま使える。
同じ日に、入力特徴集合の名前を P 型から A 型へ改め、重みファイルの構造記述の名前も `P` から `A` へ変えた（[改名のADR](../adr/rename-the-p-type-input-features-to-a.md)）。
構造記述の名前が `P` のネットは minase 3.0.0 までのエンジンだけが読むので、main の `nnue/p/` から外した。minase 3.0.0 用のネットは、リビジョン `01ccc6f471e9696a37acfb3a2ac753c0add15a39` の `nnue/p/` から取得できる。
同じネットを名前 `A` へ変換したファイルは、モデルリポジトリのコミット `e10de30a70477a8c03d1fb951cdf664cef5c4ec1` から `nnue/a/` に置き、変換したファイルの SHA-256 で名前を付ける。`<YYYYMMDD>` は変換した日ではなく、変換前のネットを作った日とする。
一方で NAS（`/mnt/synology_nas/minase/nnue/nets/` など）の内部保管名は、既存の測定記録や学習スクリプトとの互換性を保つため `<64桁SHA-256>.bin` のままとし、Hugging Face Hub へ公開する段階でこの命名規則へ変換する。
エンジンはファイル名ではなくファイル全体のバイト列の SHA-256 を `info string network_sha256` として報告するため、公開名を変えても照合の仕組みは変わらない（[ファイル全体のSHA-256で識別するADR](../adr/identify-nnue-networks-by-the-sha256-of-the-whole-file.md)）。
ダウンロード URL はコミットハッシュまたはモデルリポジトリ側のタグで固定できる（`https://huggingface.co/stepney141/minase-nets/resolve/<リビジョン>/nnue/a/minase-nnue-sfnn_a_256x16x32_psqt-<規則>-<YYYYMMDD>-<SHA256先頭8桁>.nnue`）。

### モデルカード（`README.md`）の記載項目

新しいネットをアップロードしたら、モデルリポジトリの `README.md` の一覧表へ次の項目を追記する。

- リポジトリ内のパスとファイル全体の SHA-256（64桁の16進数）
- 評価形式と構造（例: tatara LayerStack A（`sfnn_a_256x16x32_psqt`）、Simple HalfKP_vm（`halfkpvm_256x32x32`）、Zero ONNX 20b256c）
- 対応する `minase` の版数範囲（例: `minase >= 3.0.0-alpha.1`）
- 学習と評価に使った規則セット（例: `lishogi`）
- PV-MCTS 用ネットの場合は対応する `NnCpPerLogit` の値
- 強さ確認の測定記録（`docs/measurements/` または `docs/training/`）へのリンク

モデルリポジトリのファイルはすべて CC BY 4.0 で配布する（[ライセンスのADR](../adr/license-evaluation-networks-under-cc-by-4-0.md)）。
モデルカード先頭のメタデータにある `license: cc-by-4.0` と本文の「License」の節は全ネットに共通なので、ネットを追加しても書き換えない。

### アップロード手順

アップロードには Hugging Face の公式 CLI である `hf` を使う（未導入の環境では `uvx --from huggingface_hub hf` でも実行できる）。
まず SHA-256 を再確認し、構造記述を表示して構造名を決め、未ログインなら認証を済ませてからアップロードする。

```console
sha256sum /mnt/synology_nas/minase/nnue/nets/<SHA-256>.bin
head -c 400 /mnt/synology_nas/minase/nnue/nets/<SHA-256>.bin | strings | head -1
hf auth login
hf upload stepney141/minase-nets \
  /mnt/synology_nas/minase/nnue/nets/<SHA-256>.bin \
  nnue/a/minase-nnue-<構造>-<規則>-<YYYYMMDD>-<先頭8桁>.nnue \
  --commit-message "Add NNUE A network (<YYYY-MM-DD>, <先頭8桁>)"
```

アップロード後に返されるコミット URL からリビジョンを確認し、モデルカード `README.md` を更新して同じ方法でアップロードする。
`minase-lishogi-bot` やエンジンのリリースノートから参照するときは、ファイル URL に加えて SHA-256 を必ず併記し、取得側で `sha256sum` による検証を行える状態を保つ。
