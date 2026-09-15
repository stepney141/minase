# 開発ドキュメントの構成

docs/ 直下に置くファイルは、本書と [ROADMAP.md](ROADMAP.md) の2つだけとする。
その他の文書は、次の表の分類に従って下位ディレクトリへ置く。
既存の分類に収まらない文書が必要になったときは、ディレクトリを追加してから本書の表へ行を加える。

| ディレクトリ | 置く文書 | 書き方 |
|---|---|---|
| [plans/](plans/) | マイルストーンの設計書。完了後は、そのサブシステムの現行設計の正として保守する。 | [plans/README.md](plans/README.md) の型に従う。 |
| [measurements/](measurements/) | 棋力測定の記録。1測定1ファイルで、ファイル名は `match_runner` の `--run-dir` の名前と一致させる。 | plans/README.md の「測定記録」節に従う。 |
| [lessons/](lessons/) | 作業で詰まった箇所から得た汎用的な教訓。1教訓1ファイルで、[lessons/README.md](lessons/README.md) を索引とする。 | plans/README.md の「教訓」節に従う。 |
| [guides/](guides/) | 人間とエージェントが手順どおりに実行する手引き。棋力測定の手引き [sprt.md](guides/sprt.md) と、PSTの学習手順 [pst-training.md](guides/pst-training.md) を置く。 | 手順を実行順に書き、判断規則と標準コマンドを本文に含める。 |
| [audits/](audits/) | ある時点のコード、テスト、または文書を対象にした監査報告。ファイル名に監査日を含める。 | 冒頭に基準コミットと範囲を明記する。報告は監査後に更新せず、修正は設計書として起案する。 |
| [research/](research/) | 外部資料や保存棋譜を調べた調査メモ。設計書が方式の根拠として参照する。通信プロトコル（USI、CECP）と外部エンジンの調査は [research/protocols/](research/protocols/) にまとめ、[research/protocols/README.md](research/protocols/README.md) を索引とする。 | 結論を先に書き、一次資料へリンクする。 |

分類に迷う文書は、次の順で判定する。
将来の作業に適用できる1文の規則を持つなら教訓であり、ある時点の状態を報告して以後は凍結するなら監査報告であり、設計の根拠として参照され続けるなら調査メモである。
測定の結果は、設計書や調査メモの本文に書かず、測定記録へ置いてリンクする。
