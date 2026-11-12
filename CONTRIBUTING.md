# 開発の規約

## 開発ドキュメント

- マイルストーンの設計書は docs/plans/ に置き、書き方は docs/plans/README.md の型に従う。進捗の正は docs/ROADMAP.md のマイルストーン状態表で管理する。新しいマイルストーンに着手するときは設計書を作成して状態表へ行を追加し、完了時に状態表と現在地の節を更新する。設計書は一時的な計画書であり、完了時には、完了後も有効な仕様をコードのdocコメントへ、設計判断を docs/adr/ のADRへ移してから凍結し、docs/plans/archived/ へ移す。
- 設計書の「先に読む要約」「状態」と ROADMAP.md の「現在地」は追記せず上書きする。着手ログ、レビュー記録、コミットハッシュは書かず、過去の経緯は git 履歴に任せる。
- 測定の記録は、その測定を計画した設計書ごとのディレクトリ`docs/measurements/<設計書名>/`に`<測定名>.md`として1測定1ファイルで置き、どの設計書も計画していない測定は`docs/measurements/adhoc/`に置く。設計書からは測定名でリンクする。置き場所の決め方は docs/plans/README.md の「測定記録」節に従う。
- 作業で得た汎用的な教訓は docs/lessons/ に集約する。設計書を書く前、codexへ委任する前、および測定、学習、または最適化を始める前に、docs/lessons/README.md の該当する節を読む。
- テストの書く範囲、検証の層、およびマージ前のローカル検査は [docs/guides/testing-minase-rules.md](docs/guides/testing-minase-rules.md) に従う。
- docs/ 直下には ROADMAP.md と README.md だけを置く。手引き、監査報告、調査メモなどその他の文書は docs/README.md の分類表に従って下位ディレクトリへ置く。

## コミットとリリース

- コミットメッセージは Conventional Commits 1.0.0 に従い、件名を `<type>(<scope>): <summary>` の形の英語の命令形で書く。scopeは変更したモジュール名（`search`、`eval`、`movegen`、`usi`、`match` など）とし、省略してもよい。
- typeは次のとおり使い分ける。`feat`は利用者から見える機能の追加と、探索・評価・時間管理のうち棋力に影響する変更に使う。`fix`は不具合の修正、`perf`は棋力を変えない速度改善、`refactor`は挙動を変えない構造変更、`test`はテストだけの変更、`docs`は docs/、chu-shogi.md および測定記録の変更、`build`は依存関係とビルド設定の変更、`chore`はそれ以外に使う。
- USIオプション、エンジンの起動引数（`--protocol`、`--search`、`--io-log`）、規則コードまたはライブラリの公開APIの互換性を壊す変更は、typeの直後に`!`を付け、本文に`BREAKING CHANGE:`フッターを書く。補助ツールのサブコマンド（`minase match run`など）とその引数は開発者向けであり、互換性の対象としない。
- リリースノートは git-cliff（cliff.toml）がコミット履歴から CHANGELOG.md へ生成し、`feat`、`fix`、`perf`、`refactor`と互換性を壊す変更だけを載せる。マージコミットの件名は既定の`Merge branch '<名前>'`のままでよい。
- リリースは [docs/guides/release.md](docs/guides/release.md) の手順に従う。

## 棋力測定

- 探索・評価・時間管理など棋力に影響し得る変更の採否は、SPRTによる自己対局測定で判定する。
    - SPRT測定はコミット対コミットの対局で行い、ハーネスに機能比較のスイッチを追加してはならない。
    - 測定手順・標準コマンド・統計的契約・記録項目は [docs/guides/sprt.md](docs/guides/sprt.md) を参照。
