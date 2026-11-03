# リリースの手引き

本書は、版数の決定からタグのpush、およびcrates.ioへの公開までの標準手順を定める。
コミットメッセージの規約と、リリースノートに載せるコミットの種別は[CONTRIBUTING.md](../../CONTRIBUTING.md)にある。

リリースには git-cliff と cargo-release を使う。Arch Linux では`sudo pacman -S git-cliff cargo-release`で導入できる。設定は cliff.toml と release.toml にある。
ライブラリ`minase-core`とエンジン`minase`は別々の版数を持ち、1つずつリリースする。
タグは、ライブラリが`minase-core-vX.Y.Z`、エンジンが`vX.Y.Z`である。
変更履歴は、ライブラリが crates/minase-core/CHANGELOG.md、エンジンがリポジトリのルートの CHANGELOG.md であり、それぞれのcrateのディレクトリを変更したコミットだけを載せる。両方を変更したコミットは両方に載る。
以下では、ライブラリの版数を 0.2.0、エンジンの版数を 2.1.0 として説明する。

まず master を最新にして、作業ツリーがクリーンであることを確認する。次の版数は、crateごとにコミットの種別から推定した値を参考に決める。`fix`だけならパッチ、`feat`があればマイナー、互換性を壊す変更があればメジャーの版数が上がる。ライブラリは0.x系なので、互換性を壊す変更ではマイナーの版数を上げる。

```console
git switch master
git pull
git status
git cliff --include-path 'crates/minase-core/**' --tag-pattern '^minase-core-v' --bumped-version
git cliff --include-path 'crates/minase/**' --bumped-version
```

2つのコマンドは、それぞれ`minase-core-v0.2.0`と`v2.1.0`の形でタグの名前を表示する。ライブラリのコマンドは、`minase-core-v`のタグが1つもない間は失敗するので、ライブラリの最初のリリースは0.1.0とする。

次に、`--execute`を付けずに cargo-release を実行して予行する。予行では Cargo.toml と変更履歴を変更せず、今回のリリースノートを標準出力に表示するだけである。
予行と本番は「Publishing minase-core」のような行を表示するが、release.toml の`publish = false`により`cargo publish`は呼ばれない。

```console
cargo release -p minase-core 0.2.0
```

内容に問題がなければ、`--execute`を付けて本番を実行する。cargo-release は、そのcrateの Cargo.toml と Cargo.lock の版数を更新し、git-cliff で今回のリリースノートをそのcrateの変更履歴の先頭に追加する。そのうえで、これらを`chore(release): minase-core-v0.2.0`としてコミットし、注釈付きタグ`minase-core-v0.2.0`（タグメッセージは`minase-core v0.2.0`）を作る。pushと crates.io への公開は行わない。

```console
cargo release -p minase-core 0.2.0 --execute
git show --stat HEAD
```

エンジンも同じ手順でリリースする。コミットの件名は`chore(release): v2.1.0`、タグは`v2.1.0`（タグメッセージは`minase v2.1.0`）になる。エンジンが新しい版のライブラリを必要とする場合は、ライブラリを先にリリースする。

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
