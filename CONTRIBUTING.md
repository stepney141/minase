# 開発の規約

## 開発ドキュメント

- マイルストーンの設計書は docs/plans/ に置き、書き方は docs/plans/README.md の型に従う。進捗の正は docs/ROADMAP.md のマイルストーン状態表で管理する。新しいマイルストーンに着手するときは設計書を作成して状態表へ行を追加し、完了時に状態表と現在地の節を更新する。
- 設計書の「先に読む要約」「状態」と ROADMAP.md の「現在地」は追記せず上書きする。着手ログ、レビュー記録、コミットハッシュは書かず、過去の経緯は git 履歴に任せる。
- 棋力測定の記録は docs/measurements/ に測定名ごとに置き、設計書からは測定名でリンクする。
- 作業で得た汎用的な教訓は docs/lessons/ に集約する。作業を始める前に docs/lessons/README.md の索引に目を通す。
- docs/ 直下には ROADMAP.md と README.md だけを置く。手引き、監査報告、調査メモなどその他の文書は docs/README.md の分類表に従って下位ディレクトリへ置く。

## コミットとリリース

- コミットメッセージは Conventional Commits 1.0.0 に従い、件名を `<type>(<scope>): <summary>` の形の英語の命令形で書く。scopeは変更したモジュール名（`search`、`eval`、`movegen`、`usi`、`match` など）とし、省略してもよい。
- typeは次のとおり使い分ける。`feat`は利用者から見える機能の追加と、探索・評価・時間管理のうち棋力に影響する変更に使う。`fix`は不具合の修正、`perf`は棋力を変えない速度改善、`refactor`は挙動を変えない構造変更、`test`はテストだけの変更、`docs`は docs/、RULES.md および測定記録の変更、`build`は依存関係とビルド設定の変更、`chore`はそれ以外に使う。
- USIオプション、コマンドライン引数、規則コードまたはライブラリの公開APIの互換性を壊す変更は、typeの直後に`!`を付け、本文に`BREAKING CHANGE:`フッターを書く。
- リリースノートは git-cliff（cliff.toml）がコミット履歴から CHANGELOG.md へ生成し、`feat`、`fix`、`perf`、`refactor`と互換性を壊す変更だけを載せる。マージコミットの件名は既定の`Merge branch '<名前>'`のままでよい。

### リリースの手順

リリースには git-cliff と cargo-release を使う。Arch Linux では`sudo pacman -S git-cliff cargo-release`で導入できる。設定は cliff.toml と release.toml にある。
ライブラリ`minase-core`とエンジン`minase`は別々の版数を持ち、1つずつリリースする。
タグは、ライブラリが`minase-core-vX.Y.Z`、エンジンが`vX.Y.Z`である。
変更履歴は、ライブラリが crates/minase-core/CHANGELOG.md、エンジンがリポジトリのルートの CHANGELOG.md であり、それぞれのcrateのディレクトリに触れたコミットだけを載せる。両方に触れたコミットは両方に載る。
以下では、ライブラリの版数を 0.2.0、エンジンの版数を 2.1.0 として説明する。

まず master を最新にして、作業ツリーがクリーンであることを確認する。次の版数は、crateごとにコミットの種別から推定した値を参考に決める。`fix`だけならパッチ、`feat`があればマイナー、互換性を壊す変更があればメジャーの版数が上がる。ライブラリは0.x系なので、互換性を壊す変更ではマイナーの版数を上げる。

```console
git switch master
git pull
git status
git cliff --include-path 'crates/minase-core/**' --tag-pattern '^minase-core-v' --bumped-version
git cliff --include-path 'crates/minase/**' --bumped-version
```

次に、`--execute`を付けずに cargo-release を実行して予行する。予行では Cargo.toml と変更履歴を変更せず、今回のリリースノートを標準出力に表示するだけである。
予行と本番は「Publishing minase-core」のような行を表示するが、release.toml の`publish = false`により`cargo publish`は呼ばれない。

```console
cargo release -p minase-core 0.2.0
```

内容に問題がなければ、`--execute`を付けて本番を実行する。cargo-release は、そのcrateの Cargo.toml と Cargo.lock の版数を更新し、git-cliff で今回のリリースノートをそのcrateの変更履歴の先頭に追加する。そのうえで、これらを`chore(release): minase-core-v0.2.0`としてコミットし、注釈付きタグ`minase-core-v0.2.0`（本文は`minase-core v0.2.0`）を作る。pushと crates.io への公開は行わない。

```console
cargo release -p minase-core 0.2.0 --execute
git show --stat HEAD
```

エンジンも同じ手順でリリースする。コミットの件名は`chore(release): v2.1.0`、タグは`v2.1.0`（本文は`minase v2.1.0`）になる。エンジンが新しい版のライブラリを必要とする場合は、ライブラリを先にリリースする。

```console
cargo release -p minase 2.1.0
cargo release -p minase 2.1.0 --execute
```

変更履歴を手で補う場合は、pushの前に編集してリリースコミットを amend し、タグを付け直す。

```console
git commit -a --amend --no-edit
git tag -f -a v2.1.0 -m "minase v2.1.0"
```

次に、master とタグをpushする。

```console
git push origin master minase-core-v0.2.0 v2.1.0
```

最後に、crates.io へ公開する。公開は取り消せないので、タグとpushを確認してから手で実行する。エンジンの依存はcrates.io上のライブラリの版数で解決されるので、ライブラリを先に公開する。

```console
cargo publish -p minase-core
cargo publish -p minase
```

## 棋力測定

- 探索・評価・時間管理など棋力に影響し得る変更の採否は、SPRTによる自己対局測定で判定する。
    - SPRT測定はコミット対コミットの対局で行い、ハーネスに機能比較のスイッチを追加してはならない。
    - 測定手順・標準コマンド・統計的契約・記録項目は [docs/guides/sprt.md](docs/guides/sprt.md) を参照。
