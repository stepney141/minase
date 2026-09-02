# 棋力向上段階1のHaChu固定対局

## 目的

棋力向上段階1の最終候補を外部エンジンHaChuと固定200ペア対局させ、前回測定と同じ開始局面および条件による進捗指標を得る。

## コマンドライン

次のコマンドを実行した。

```console
cargo run --release --bin match_runner -- \
  --run-dir data/matches/strength-stage1-hachu-elo200 \
  --seed 20260829 \
  --candidate commit:98ffcfd \
  --baseline "cecp:../hachu-debian/hachu" \
  --rules L1,L3,P0,P5,P6,R2,E1,E2 \
  --each time=60000+1000 \
  --concurrency 16 \
  elo --pairs 200
```

完走後、次のコマンドで保存記録を再集計した。

```console
cargo run --release --bin match_report -- \
  --run-dir data/matches/strength-stage1-hachu-elo200
```

## エンジン

候補は棋力向上段階1の最終版`98ffcfd6afc6d2137603962ee616fcfce51a4426`であり、バイナリのSHA-256は`16c3984d5939b36324cf4e7248faab3b0a847ed2174c14ea61a08255d2e1e37c`である。
基準はDebianパッケージのコミット`822d512180b7d94bb85a55f871445b30393f7f8a`に収録されたH. G. Muller作のHaChuであり、上流のソース版は`df26f4a`である。
HaChuは既定のMakefileによりGCCで`-O2 -s -Wall -Wno-parentheses`を指定してビルドし、バイナリのSHA-256は`10db1299f95626fe58875b1c24a1cee6acce0eee5fb42ed8b17ce1373a9dcbdf`である。
HaChuの設定は“Okazaki rule”無効、“Promote on entry”有効、“Allow repeats”無効であり、規則セット`L1,L3,P0,P5,P6,R2,E1,E2`を審判層とminase側に与えた。
対局に使った`match_runner`のSHA-256は`45d7ab4487d54a3fbdb945ab65b94afdcca3400929e6172dfb77e85074394a9a`である。

## 環境

測定機はIntel Core Ultra 7 265KFであり、物理コア数と論理CPU数はいずれも20、実メモリは33,218,965,504バイトである。
候補のワーカー数は1であり、Chess Engine Communication Protocolで接続したHaChuのワーカー数は報告されないため欠測として扱った。
`USI_Hash`は候補256 MBであり、HaChuには`memory 256`を送った。
同時対局数は16、手数上限は4,096手、応答タイムアウトは120秒である。
時間制御は両エンジンとも初期時間60,000 ms、1手1,000 ms加算である。

## 結果

2026年9月2日5時53分から9時33分まで200ペアを実行し、200ペアが有効、破棄は0ペアであった。
ペンタノミアル度数は`[27, 0, 101, 0, 72]`、候補の得点率は61.25%であった。
Elo推定値は+79.533754、95%信頼区間は`[+46.347076, +114.215940]`であった。
`illegal_moves=1`、`crashes=3`であり、`timeouts`、`time_forfeits`、`rejected_moves`、および手数上限による打ち切りはいずれも0件であった。
4件の異常はすべてHaChu側であり、ペア145第2局とペア159第2局は開始手順を含む記録着手数2,002、ペア165第1局は2,001でHaChuが終了し、ペア164第1局は記録着手数1,920でHaChuが不正着手を返した。
クラッシュ3件はHaChu側のピーク常駐メモリを取得できなかったため、`match_report`は基準側ピーク常駐メモリの欠測を3件と記録し、依存するメモリ指標を`null`とした。
minase側の異常は0件であり、HaChu側の4件は測定規約どおり当該局の反則負けとして算入した。
保存記録に基づく累計実行時間は13,192.359556秒であった。

## 結論

同条件の前回測定の−111 Eloから約191 Elo上昇し、95%信頼区間も重ならないため段階1後の進展が表れたが、本測定はHaChu側の異常4件を規約どおり算入した固定局数の進捗指標であり、変更の採否には用いない。
