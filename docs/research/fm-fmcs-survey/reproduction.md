# fmcs.exe の解析と実行の再現手順

本書は、[fmcs の調査](../fm-fmcs-survey.md)で使った逆コンパイル出力と実行環境を再現する手順を記す。
逆コンパイル出力はリポジトリに含めないので、付録の読解が引く行番号は、ここに書いた版と手順で再生成した出力に対するものである。
Ghidra の版が変わると行番号はずれるので、読解の引用は関数名（`FUN_1400xxxxx`）と文字列アドレスを優先して照合する。

## 対象

| ファイル | 版 | SHA-256 |
|---|---|---|
| fmcs.exe（fmcs.zip、2025年4月25日） | 0.03.003 | `35be94c3bfde3206ebbd2ff2fab987f598d6f552fdd53e8a71834b0d24328029` |
| fmza.exe（fmza.zip、2026年10月4日） | 7.08.011 | `57bea7c39687d9692c4910cf9a99e0774f692028eb4757d71aeaf347e73658de` |

配布元は https://k7ro.sakura.ne.jp/fmz/fmZ.html である。
fmza.exe は逆コンパイルだけを行い、読解はしていない。

## 逆コンパイル

Ghidra 12.1.2（Arch Linux のパッケージ、OpenJDK 27）の headless analyzer で読み込み、[ExportAllC.java](ExportAllC.java) で全関数の C を1つのファイルへ書き出した。

```console
mkdir -p proj scripts out && cp ExportAllC.java scripts/
/opt/ghidra/support/analyzeHeadless proj fmcs -import fmcs.exe -overwrite \
  -scriptPath scripts -postScript ExportAllC.java out/fmcs.c
```

fmcs.exe では861関数、約1.4 MB の C が得られる。
各関数は `// ==== FUNCTION <名前> @ <アドレス> size=<バイト数>` の行で始まる。
日本語の文字列は cp932 で埋め込まれているので、バイナリを走査して文字列と仮想アドレスの表を別に作り、C 中の `DAT_1400xxxxx` と下位桁で突き合わせた。

## wine での実行

fmcs.exe はコンソールアプリであり、wine 11.17 では疑似端末の下で起動する。

```console
WINEPREFIX=$PWD/wineprefix WINEDEBUG=-all script -f -q -e -c \
  "wine ./fmcs.exe /I問題.fmi /O出力.fmo /L3 /LT=3M /XV" 画面.log < /dev/null
```

問題ファイルで規則名（最善詰）の行と盤面の行が分かれていると、駒ごとの処理関数の表（`FUN_1400094f0` が設定する）が未設定のまま盤面走査 `FUN_140001000` に入り、アドレス0を呼び出して落ちる。
推奨する回避は、規則名と盤面を同じ行に書くことである。

```text
"中将棊指南抄 1番"
最善詰 17奔25金a9馬,
11香12反15歩29と3B師3C立43虎45歩51金62鳳66歩71象72玉73麒75銀76歩81金82虎93豹c5横
#21
```

バイナリを書き換える回避もある。
規則名だけの行でも初期化へ進むように、`FUN_14000a1f0` の2つの条件分岐の飛び先を変える。

| 仮想アドレス | ファイルオフセット | 変更前 | 変更後 | 意味 |
|---|---|---|---|---|
| `0x14000a613` | `0x9a13`（変更は `0x9a15`〜`0x9a16`） | `0f 84 80 01 00 00`（`je 0x14000a799`） | `0f 84 08 00 00 00`（`je 0x14000a621`） | 規則行の残りが改行のときも初期化へ |
| `0x14000a61b` | `0x9a1b`（変更は `0x9a1d`〜`0x9a1e`） | `0f 84 78 01 00 00`（`je 0x14000a799`） | `0f 84 00 00 00 00`（`je 0x14000a621`） | 残りが終端のときも初期化へ |

パッチ後の SHA-256 は `42e34fb87d3efde931e7557641e807ddb06a46dc5c969d2157027ff745f5d3f3` である。
同じ行に書いた入力をパッチ版と元の版で解かせると、展開局面数と手順は一致した。

制限時間の書式は、説明書の `/LT<n>M` ではなく `/LT=<n>M` である（`/LT3M` は解数上限 `/L3` として読まれる）。
画面出力は wine が cp932 を CP437 として UTF-8 化するので、行ごとに `encode('cp437').decode('cp932')` で復元し、エスケープ列を正規表現で除く。
出力ファイル `.fmo` は cp932 のままなので `iconv -f CP932 -t UTF-8` で読める。
