# mimallocの環境変数と置換表の大きさの測定

## 目的

[教師生成の高速化](../../plans/teacher-generation-speedup.md)の候補として、アロケータの環境変数と置換表の大きさが、生成機の代理環境の探索速度に与える影響を確かめる。

## コマンドライン

環境変数の比較は[付属資料](teacher-generation-speedup-allocator-and-hash/env_cmp.py)のスクリプトで、各設定を回ごとに順序を入れ替えて3回ずつ実行した。

```console
python3 env_cmp.py --rounds 3 --bin <バイナリ> 'default:' 'nopurge:MIMALLOC_PURGE_DELAY=-1' \
  'large1:MIMALLOC_ALLOW_LARGE_OS_PAGES=1' 'eager:MIMALLOC_ARENA_EAGER_COMMIT=1,MIMALLOC_EAGER_COMMIT_DELAY=0' \
  'reserve:MIMALLOC_RESERVE_OS_MEMORY=4GiB'
```

置換表の比較は`ecore_cmp.py`（[探索結果を変えない高速化の測定](teacher-generation-speedup-engine.md)の付属資料）に`--hash-mb 16`、`4`、`2`を与え、3通りを順に3巡した。

```console
python3 ecore_cmp.py --cwd <固定のworktree> --cpus 8-19 --concurrency 12 --rounds 1 --max-ply 300 --hash-mb <大きさ> h<大きさ>=<バイナリ>
```

## エンジン

環境変数の比較は、基準版`9380aa7f`にインライン閾値の変更と先行の3変更を適用した版で行い、置換表の比較はブランチ`teacher-speedup`のコミット`6988af2b`で行った。
mimallocはv2系（`libmimalloc-sys 0.1.49`、`MI_MALLOC_VERSION 20302`）である。
規則セットは`L0,P0,R1,E0`である。

## 環境

Intel Core Ultra 7 265KFの12個のE-coreで12局を同時に動かし、1局あたり1ワーカーとした。
環境変数の比較では置換表を16 MiBとした。
Transparent Huge Pagesの設定は、測定機が`always`、生成機が`madvise`である。
生成機ではmimallocが`madvise(MADV_HUGEPAGE)`を呼び、常駐メモリの93%にHuge Pagesが適用されている。

## 結果

環境変数による探索速度の差は、`MIMALLOC_RESERVE_OS_MEMORY=4GiB`を除いて0.4%以内だった。
カーネル時間は全CPU時間の1%未満で、ページフォルトは1回の実行で約5,500件にとどまる。

| 設定 | 探索速度 | 基準比 | ユーザー時間 | カーネル時間 | サイクル数/ノード |
|---|---:|---:|---:|---:|---:|
| 既定 | 5,533,678ノード/秒 | 1.000 | 127.4秒 | 1.03秒 | 8,462 |
| `MIMALLOC_PURGE_DELAY=-1` | 5,524,888ノード/秒 | 0.998 | 127.4秒 | 1.02秒 | 8,467 |
| `MIMALLOC_ALLOW_LARGE_OS_PAGES=1` | 5,536,248ノード/秒 | 1.000 | 127.3秒 | 1.04秒 | 8,460 |
| `MIMALLOC_ARENA_EAGER_COMMIT=1`と`MIMALLOC_EAGER_COMMIT_DELAY=0` | 5,513,368ノード/秒 | 0.996 | 127.5秒 | 1.05秒 | 8,471 |
| `MIMALLOC_RESERVE_OS_MEMORY=4GiB` | 5,089,830ノード/秒 | 0.920 | 128.4秒 | 0.96秒 | 8,535 |

置換表を4 MiBへ縮めると探索速度は約3.4%上がったが、ノードあたりのサイクル数はむしろ増えた。
探索速度の差とサイクル数の差が一致しない理由は調べていない。
置換表の大きさは探索木を変えるので、教師ファイルは一致しない。

| 置換表 | 探索速度（3巡の中央値） | サイクル数/ノード | 命令数/ノード | ロードのストール/ノード |
|---:|---:|---:|---:|---:|
| 16 MiB | 7,272,097ノード/秒 | 6,745.6 | 17,669.8 | 1,585.6 |
| 4 MiB | 7,522,056ノード/秒 | 6,765.4 | 17,833.2 | 1,551.9 |
| 2 MiB | 7,440,617ノード/秒 | 6,835.4 | 18,032.7 | 1,533.5 |

## 結論

mimallocの環境変数による改善は見られず、置換表の縮小は探索速度が約3%上がるものの探索木が変わるので、どちらも採用しない。
