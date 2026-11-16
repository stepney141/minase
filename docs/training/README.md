# 学習の系列の記録

本ディレクトリには、ネットワーク（以下、ネット）の学習の系列ごとに、系列が進むたびに書き換える記録を置く。
系列とは、1つの初期ネットから続く学習の連なりをいう。
AlphaZero型探索の強化学習では、自己対局の生成と学習を1サイクルとして繰り返すサイクルの連なりが、NNUEの世代反復では、教師の生成と学習を繰り返す世代の連なりが、1つの系列である。
置き場と道具を定めた経緯は、[学習の系列の記録と進捗図の設計書](../plans/archived/training-series-records.md)にある。

## 置き場の規則

系列が進むたびに書き換える記録は`docs/training/`に、1回の測定で凍結する記録は`docs/measurements/`に置く。
この規則により、次の3種類の記録の置き場が決まる。

- 系列の状態（最後に終えたサイクル、最後の定例の測定、次の判断）は、系列が進むたびに書き換えるので、系列のページ`docs/training/<系列名>/README.md`の「状態」の節に書く。
- サイクルごとの値（局面数、損失、対局数など）も系列が進むたびに増えるので、系列のディレクトリのCSVに置く。CSVは道具`series_progress.py`が書き、測定記録にサイクルごとの値の表は書かない。
- 10サイクルごとの定例の測定のような1回の測定は、終わった時点で凍結するので、[設計書の書き方](../plans/README.md)の「測定記録」の節に従い、測定を計画した設計書ごとのディレクトリ`docs/measurements/<設計書名>/`に、目的、コマンドライン、エンジン、環境、結果、および結論の見出しで書く。系列のページの「測定記録」の節から、その記録へリンクする。

ほかの1回で完結する測定、たとえば固定の相手との対局、出力の照合、および持ち時間つきの対局の記録も、同じく`docs/measurements/`に置く。

## 系列のディレクトリ

`docs/training/`の直下に、系列ごとのディレクトリを置く。
ディレクトリの名前（系列名）は系列のページを作るときに決め、強化学習の実行器が使う系列の名前（`minase_zero run-cycles`の`--cycles-dir`の値）は、系列のページに書く。
系列のディレクトリには、次のファイルを置く。

| ファイル | 内容 | 書き手 |
|---|---|---|
| `README.md` | 系列のページ | 人またはエージェント |
| `series.toml` | 系列の定義。固定の相手との対局の集計と、固定の検証用データの診断を、サイクルの番号に対応づける | 人またはエージェント |
| `cycles.csv`、`strength.csv`、`validation.csv` | 図の元になる値 | `series_progress.py` |
| `progress.png` | 進捗図 | `series_progress.py` |
| `validation/` | 固定の検証用データの診断のJSON（サイクルごと） | `minase_zero external-eval` |

`series.toml`は、gitにあるファイルだけを、リポジトリの根からの相対パスで指す。
すでにgitにある診断のJSONを使う系列では、`validation/`を作らずに、そのファイルを指してよい。

## 系列のページ

系列のページは、次の節を順に持つ。

1. 系列の説明（目的、設計書へのリンク、系列の記録とネットの置き場所）
2. 状態（最後に終えたサイクル、最後の定例の測定、次の判断）
3. 進捗図
4. 読み取れること（図と値から言えることを、根拠の値とともに書く）
5. 測定の条件（`series.toml`に書いた測定条件の名前ごとに、MCTSのコミット、1手のシミュレーション回数、対局の規則、および条件を定めた設計書へのリンク）
6. 費用（それまでの累計と、次の10サイクルの見込み）
7. 測定記録（定例の測定の記録へのリンクを、サイクルの順に並べた表）

`cycles.csv`を後述の`series_progress.py`の`collect`ではなく一度だけの変換で作った系列は、最後に「CSVの作り方」の節を置き、実行器が書いた系列の記録のどの欄からCSVのどの列を作ったかを書く。

系列のページは、追記せずに上書きする。
設計書の「先に読む要約」と同じ扱いであり、過去の状態はgitの履歴に任せる。
系列が終わったら、状態の節に終わった日と理由を書いて凍結し、以後は書き換えない。

凍結した系列のネットを、後から新しい測定の条件で測り直すことがある。
たとえば[2回目の強化学習の設計書](../plans/alphazero-rl-2.md)は、MCTSの探索の結果を変える変更がバイナリに入った場合に、1回目の基準のネットを測り直すと定めている。
その場合は、凍結した系列の`series.toml`に対局を足して`strength.csv`を作り直し、系列のページには「測定の条件」の節と「測定記録」の節の表の行だけを足す。

## 道具

CSVと進捗図は、`tools/train/progress/series_progress.py`が作る。
`collect`は、系列の定義が指すファイルを読んで`strength.csv`と`validation.csv`を書き直し、引数`--cycles-record`で強化学習の実行器の系列の記録を渡したときは`cycles.csv`も書き直す。
`plot`は系列のディレクトリのCSVだけを読んで`progress.png`を書き直す。
`plot`がCSVだけを読むので、ネットワーク接続の記憶装置（NAS）のない場所でも図を作り直せる。
どちらもリポジトリの根で実行する。

```console
uv run --frozen --project tools/train/progress python tools/train/progress/series_progress.py collect docs/training/<系列名> [--cycles-record <系列の記録>]
uv run --frozen --project tools/train/progress python tools/train/progress/series_progress.py plot docs/training/<系列名> [--reference docs/training/<参照の系列名>]
```

`series.toml`の欄、CSVの列の名前、型、意味、および空欄にできる条件と、進捗図の描き方の規則は、`series_progress.py`のdocstringが定める。
Pythonの環境がない作業ツリーでは、`uv sync --frozen --project tools/train/progress`で導入する。

## 系列の一覧

| 系列 | 内容 | 状態 |
|---|---|---|
| [alphazero-rl-1](alphazero-rl-1/) | AlphaZero型探索の強化学習の1回目の実行（10ブロック128チャネルのネット、サイクル0〜32） | 2026年11月9日に中断し、凍結 |
