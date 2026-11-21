# GitHub Actionsで定型検査とタグ起点の下書きリリース作成を自動化し、crates.ioへの公開は手動に残す

## ステータス

承認済み。決定日は2026年11月21日。

## コンテキスト

これまでリポジトリには継続的インテグレーション（CI）の設定がなく、書式検査、clippy、ワークスペーステスト、Pythonスクリプトのテスト、パッケージ内容の確認、およびGitHubリリースの作成はすべて手元の手動手順に依存していた（[テストの手引き](../guides/testing-minase-rules.md)、[リリースの手引き](../guides/release.md)）。
手元の確認だけでは実行漏れが起きやすく、リリース時の定型作業も煩雑になっていた。
一方で、crates.ioへの公開は取り消せない操作であり、タグとpushを確認してから手動で実行する方針を採っている（[crateごとに版数を付けて手動公開するADR](release-each-crate-separately-and-publish-by-hand.md)）。

## 決定

GitHub Actionsに2つのワークフローを置く。
1つ目の`.github/workflows/ci.yml`は、`master`と`release/*`へのpushおよびプルリクエストで、Rustの書式・clippy・テストとPythonの各種テストを実行する。
2つ目の`.github/workflows/release.yml`は、`v[0-9]*`と`minase-core-v[0-9]*`のタグのpushを起点に、タグと`Cargo.toml`の版数一致、Rustの全検査、パッケージ内容（`minase`の`nets/`に本番ネットが混入していないこと）を検証し、変更履歴から該当節を抽出してGitHubリリースの下書きを作成する。
GitHubリリースの最終公開とcrates.ioへの`cargo publish`は自動で行わず、エンジンの開発者が手動で行う。

## 検討した代案

`release-plz`を導入してリリース用のプルリクエスト作成からcrates.ioへの公開まで一括自動化する案は、取り消せない公開操作を自動化する点と、既存の`cargo-release`および`git-cliff`の単純な構成へ新たな常駐の仕組みを足す点がプロジェクトの方針に合わないため採らなかった。

CIを置かずに手元の実行だけで済ませる案は、リポジトリ側で検査の通過を客観的に保証できず、タグ作成時のパッケージ検査漏れも防げないため採らなかった。

## 結果

`master`、`release/*`、およびクレートのタグに対して同じ自動検査が常に走り、本番のNNUEネットの誤同梱やタグと`Cargo.toml`の食い違いを機械的に防げる。
下書きリリースは`scripts/release_checks.py`でタグ検証と変更履歴抽出を行い、プレリリース版には自動で`--prerelease`が付くため、開発者は推奨ネットへのリンクを補って公開し、`cargo publish`を手動で実行するだけでよい。
`minase`は未公開の`minase-core`に依存する状態でタグがpushされ得るため、ワークフロー内の`cargo package -p minase`は`--no-verify`でアーカイブ生成と`--list`検査までを行い、レジストリ解決を伴う公開確認は`minase-core`の公開後に行う。
