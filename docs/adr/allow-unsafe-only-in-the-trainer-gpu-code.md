# `unsafe`は学習器のGPUの処理だけに許し、エンジンとライブラリでは禁じ続ける

## ステータス

承認済み。決定日は2026年10月14日。

## コンテキスト

[学習器のクレートだけに`unsafe`を許す決定](archive/allow-unsafe-only-in-the-trainer-crate.md)は、cudarcによるカーネルの起動とページ固定メモリの確保のためにGPUの処理`nnue::gpu`で、教師データをmemmap2でメモリへ写像するために`data`モジュールの`Dataset::open`で、`unsafe`を許していた。
[教師データを対局単位の連鎖で記録する決定](record-teacher-data-as-chained-games.md)により、学習器は教師データを先頭から順に読んで手順を再生し、復元した局面をメモリ上に保持するようになった。
写像したファイルの内容を読み続ける必要はなくなり、`Dataset::open`はmemmap2を使わなくなった。

## 決定

学習器のクレート`crates/minase-trainer`だけはworkspaceのlintを継承せず、`unsafe`を許す。
学習器の中でも、`unsafe`はGPUの処理を置く`nnue::gpu`に限り、各ブロックに前提条件を書く（詳細は`nnue::gpu`のdocコメントにある）。
`Dataset::open`は安全な関数とし、学習器はmemmap2に依存しない。
`minase`と`minase-core`は`unsafe_code = "forbid"`を保つ。

## 検討した代案

教師データのファイルを引き続きメモリへ写像する案は、版2の局面が手順の再生でしか復元できず、写像しても再生の費用は減らないので採らなかった。
学習器でも`unsafe`を禁じる案と、workspace全体で禁止を外す案を採らない理由は、置き換えた決定と同じである。

## 結果

学習と測定のコマンドの実行中に入力の教師データを書き換えても未定義動作にはならないが、読み込みの後に書き換えた内容は学習に反映されない。
学習器はmemmap2に依存しなくなった。
