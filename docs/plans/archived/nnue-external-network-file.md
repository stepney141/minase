# NNUEのネットをバイナリに埋め込まず、USIのオプションEvalFileで外部のファイルから読む設計書

## 先に読む要約

本書は、中将棋エンジンminaseのαβ探索が使う評価関数NNUE（着手ごとに差分で更新できる小さなニューラルネットワーク）の重みを、実行ファイルに埋め込むのをやめ、外部のファイルから読む形へ改める手順を定める。
いまの重みのファイル`crates/minase/nets/nnue.bin`（約7.1 MB）はgitで管理され、ビルドのときに実行ファイルへ埋め込まれている。
次に試す入力特徴集合HalfKP_vm（自玉の位置ごとに別の重みを引く入力）のネットは約500 MBになり、GitHubが1ファイルに許す100 MiBを超えるので、gitに入れることも埋め込むこともできない。
そこで、将棋のエンジンとGUIの通信規約USI（Universal Shogi Interface）のオプション`EvalFile`でネットのファイルのパスを受け取り、対局の前に送られる準備の命令`isready`のときに読む。
既定のネットは持たず、`EvalFile`が未設定か、ファイルを読めないか、形式の検証に通らなければ、エラーの行を出して止まる。
読み込みに成功すると、エンジンは、読み込んだファイル全体のチェックサム（SHA-256）を、ネットを特定する値として報告する。エンジンを子プロセスとして動かして自己対局で強さを測る道具（以下、対局の実行器。サブコマンド`match run`）は、その値を記録して局ごとに照合する。
これらは、minaseのもう1つの探索方式であるPV-MCTS（ニューラルネットで木を広げるモンテカルロ木探索）が、ONNX形式のネットのファイルをオプション`NnModel`で受け取る方式と同じである。
ファイル全体を覆うので、重みの本体だけでなく、ヘッダにある勝率尺度（評価値と勝率を結ぶ実数であり、評価値の換算に使う）が違うネットも区別できる。
測定では、コードをgitのコミットで、ネットをこのチェックサムで特定し、ネットだけを比べるときは候補と基準に同じコミットと別々のネットを渡す。
重みのファイルの形式は本書では変えず、将棋のNNUE学習器tataraの形式へ揃える変更は、HalfKP_vmの設計書で扱う。
完了の条件は、同じネットを与えたときに固定の局面集を探索する診断の命令`bench`の結果がいまと完全に一致し、ネットを使うすべての入口がネットのファイルを明示して受け取り、仕様と設計判断をソースのdocコメント、手引き、および決定記録（ADR）へ移したことである。

## 状態

> 本書は完了時点で凍結した記録であり、以後の変更を反映しない。現行の仕様はコードのdocコメントに、設計判断は[docs/adr/](../../adr/README.md)にある。

状態は完了（採用）である。2026年11月15日に起案し、2026年11月17日に着手して、同じ日に完了した。
αβ探索のNNUEのネットは実行ファイルに埋め込まず、USIのオプション`EvalFile`で指定した外部のファイルから`isready`で読むようになり、ネットを使うすべての入口がネットのファイルを明示して受け取る。
同じネットを渡した深さ5の`bench`は、ネットを埋め込んだmasterと15局面すべてで一致した（[benchの一致](../../measurements/nnue-external-network-file/nnue-external-network-file-bench-equivalence.md)）。
性能コアに固定した5回の交互の測定では、ブランチの平均がmasterより0.62%速く、差はmasterの5回の幅（0.82%）より小さく、探索の区間の命令数も一致した（[benchの速さ](../../measurements/nnue-external-network-file/nnue-external-network-file-bench-speed.md)）。
[対局の実行器での記録と照合](../../measurements/nnue-external-network-file/nnue-external-network-file-manifest.md)、[教師生成](../../measurements/nnue-external-network-file/nnue-external-network-file-teacher-generation.md)、および[CECP](../../measurements/nnue-external-network-file/nnue-external-network-file-cecp.md)の確認の結果は、それぞれの測定記録にある。
masterにあった`crates/minase/nets/nnue.bin`と同じバイト列のファイルを、NASの`/mnt/synology_nas/minase/nnue/nets/987f060d545c09f0a42b4c668ea8b24cbf83c1770d11d663a94e4b82601274cd.bin`に置いた。

## 目的

本書の目的は、ネットの大きさに関係なくエンジンがネットを使えるようにし、コードとネットを別々に管理できるようにすることである。
具体的には、次の3つを目的とする。

第1は、gitと埋め込みの大きさの制約を外すことである。
NNUEの入力は、局面から「どの駒がどの升にあるか」を表す基底特徴を取り出したものであり、第1層は特徴ごとに幅256の重みの行を持つ。
基底特徴は、駒の状態47種×2陣営×144升の13,536個に、先獅子（獅子の取り合いを制限する中将棋の規則）の対象の升を表す144個を加えた13,680個である。
駒の状態の47種は、29の駒種を、これから成れる駒と既に成った駒に分けた数である。
いまのP型の入力特徴集合は基底特徴をそのまま使う。
HalfKP_vmは、盤（12×12の144升）を左右に鏡映して自玉を右半分の72升に寄せたうえで、自玉の升ごとに別の行を引くので、行の数は基底特徴の72倍になる。
重みは16ビットなので、第1層だけで、基底特徴をそのまま使えば72×13,680×256×2バイト＝約504 MB、やねうら王に倣って王将を入力の駒から除けば72×13,392×256×2バイト＝約494 MBになる。どちらを採るかはHalfKP_vmの設計書で決める。
また、いまは7.1 MBのネットがcrates.ioへ公開するパッケージにも含まれているが（`crates/minase/Cargo.toml`の`include`）、ネットを外部のファイルに分ければ、パッケージからも本番のネットを外せる。

第2は、ネットをgitに入れずに世代反復を回せるようにすることである。
世代反復は、あるネットの自己対局で教師（自己対局の局面に探索値と対局結果を付けた学習データ）を作り、その教師から次のネットを学ぶことの繰り返しである。
2026年11月15日に利用者は、NNUEに関わる実装をすべて直してからP型の世代反復を回し、その後にHalfKP_vmの世代反復を行うと決めた。
いまの連続生成スクリプトは、世代ごとのネットをgitにコミットし、時間単位で借りる計算機（以下、生成機）でそのコミットからエンジンをビルドして教師を作るので、HalfKP_vmのような大きなネットの世代反復を回せない。

第3は、[tatara化とHalfKP_vmの設計書](nnue-tatara-framework-and-halfkp-vm.md)が使う部品を先に揃えることである。
その部品は、大きさによらずネットを外部のファイルで渡す経路、ネットを1つの値で特定するチェックサム、およびネットをgitに入れずに世代を回す連続生成スクリプトである。
HalfKP_vmの入力特徴集合と出力部（第1層より後ろの層）の実装、P型とHalfKP_vmをtataraに倣った同じ枠組みに載せる変更、および重みのファイルの形式の変更は、本書の後に別の設計書で扱う。

## 適用範囲

本書では次の項目を扱う。

- αβ探索のUSIオプション`EvalFile`、ネットを読む時期、失敗時の動作、およびチェックサムの報告を定める。
- ネットの識別子を、読み込んだファイル全体のSHA-256に揃え、教師ファイルにもその値を書く。
- CECP（チェス用の通信規約）で`EvalFile`を受け取り、対局の実行器がCECPのエンジンへ追加のオプションを送るようにする。minaseは、中将棋エンジンHaChuとの比較と、そのための結合テストでCECPを使う。
- PV-MCTSが探索値のセンチポーンへの換算などに使う勝率尺度を、埋め込みのNNUEのヘッダから読むのをやめ、USIのオプションで受け取る形へ改める。
- ネットを使うサブコマンド（教師生成、棋譜の取り込み、係数調整SPSA）の引数、対局の実行器の起動時の検査、およびSPSAの記録を改める。SPSA（simultaneous perturbation stochastic approximation）は、対局の結果から探索の係数を調整する手法である。
- 教師生成と連続生成のPythonスクリプト、速度比較と調査のスクリプト、およびPV-MCTSのネットの検査のスクリプトを改める。
- ネットの保管場所、crates.ioのパッケージ、リリースの手順、テストの固定ネット、手引き、およびADRを改める。

次の項目は扱わない。

- HalfKP_vmの入力特徴集合と出力部の実装、重みのファイルの形式の変更、および大きなネットの読み込み時間と常駐メモリの測定は、HalfKP_vmの設計書で扱う。
- ネットの構造を実行時に選ぶ仕組みも、HalfKP_vmの設計書で扱う。本書の時点では、探索が使うネットの型は、いまと同じくコンパイル時の型の別名`ActiveFeatureSet`と`ActiveHead`で決まり、型の合わないファイルは復号の検証で拒否する。
- 開発用GUI（別リポジトリ`minase-gui`）とlishogi Botの配備一式（別リポジトリ`minase-lishogi-bot`）の追従は、それぞれのリポジトリで別に計画する。

## 依存関係

本書は、ほかの設計書の完了を前提としない。

NNUEの次の2つの設計書は、本書の完了を前提とする。

- [tatara化とHalfKP_vmの設計書](nnue-tatara-framework-and-halfkp-vm.md)は、本書の完了を前提とし、P型とHalfKP_vmをtataraに倣った同じ枠組みに載せ、重みのファイルの形式をtataraの形式に揃える（2026年11月15日の利用者の決定）。
- P型の[世代反復の設計書](../nnue-generation-iteration.md)は、本書とtatara化の完了を前提とし、2026年11月15日に外部のネットを使う形へ書き直してある。同書のフェーズ1は、連続生成スクリプトに学習だけのために機械を借りる処理を足すので本書と同じファイルを変えるが、本書の完了後に行う。

本書の変更は、次の設計書の手順にも及ぶ。
どれも本書の完了を待つ必要はないが、本書の後に着手するなら、次のとおりに手順を合わせる。

- [PUCTの係数のSPSA](../mcts-puct-spsa.md)は、本書と同じく`minase spsa`を変える。後からmasterへ入れる側が、先に入った変更に合わせる。本書の後は、PV-MCTSのネットと勝率尺度を`minase spsa`の`--option NnModel=<パス>`と`--option NnCpPerLogit=<値>`で渡す。
- AlphaZero型探索の[2回目の実行](../alphazero-rl-2.md)と[学習レシピを先行実装に揃える設計書](zero-recipe-precedents.md)では、強さを測る相手のαβ探索がネットを埋め込んだコミット`45d5db54`に固定されているので、その対局は本書の影響を受けない。PV-MCTSのネットの検査（`tools/train/zero/minase_zero/runner.py`）は、エンジンの版によらず`NnCpPerLogit`を送る。このオプションを宣言しない本書より前の版のminaseは、宣言していない名前の`setoption`を無視する。対局の実行器はエンジンが宣言しないオプションを送る前に拒否するので、定例の測定に固定したコミットが本書より前なら、その対局にはこのオプションを渡さない。マージの前に行うαβ探索の`bench`の一致の確認では、`RuleSet`、`EvalFile`、および`isready`の後に`bench`を送る。
- [AlphaZero型探索の統括計画](../alphazero.md)の到達の判定は、その時点のmasterのαβ探索を相手にするので、本書の後は、比べるNNUEのネットのファイルを決めて`EvalFile`で渡し、そのSHA-256を判定の記録に書く。
- [PV-MCTSのデバッグ道具](mcts-debugging-tools.md)は本書より先にmasterへ入り、PV-MCTSで起動したエンジンの`bench`を、ネットを使わない試験用の評価器による固定局面のPV-MCTSの探索にした。本書はこの`bench`を変えない。
- [合法手生成の高速化（第3期）](../movegen-speedup-3.md)、[詰み探索](../mate-search.md)、および[探索の強化の段階](../strength-stages.md)のように、αβ探索の`bench`や対局で結果と速さを確かめる設計書では、本書の完了後は、`bench`の前にオプション`RuleSet`と`EvalFile`を設定して`isready`を送り、対局の実行器には`--candidate-option EvalFile=<パス>`と`--baseline-option EvalFile=<パス>`を渡す。

開発用GUIは、いまも規則セットの指定の変更（[起動引数と診断ツールをUSIへ移す計画](cli-to-usi.md)）に追従しておらず、本書の変更の後も`EvalFile`を送るまでエンジンを起動できない。
lishogi Botはcrates.ioのminase 2.6.0を固定しているので、本書の変更を含む版をリリースするまで影響を受けない。

## 設計判断

| 項目 | 採る方式 | 採らない代案と理由 |
|---|---|---|
| ネットの供給 | 実行ファイルに埋め込まず、外部のファイルから読む（2026年11月15日の利用者の決定）。 | 埋め込みを続ける案は、約500 MBのネットをgitにも埋め込みにも置けない。Git LFSに置く案は、生成機がコミットからビルドする流れに、LFSの取得と容量の管理を足す。Stockfishのように既定のネットを埋め込み、`EvalFile`で上書きできる案は、既定値を持たないという利用者の決定に反し、大きなネットではやはり埋め込めない。なお、Stockfishは明示した外部のネットを読めなければ既定のネットへ戻らずに止まるので、この案もフォールバック（失敗を隠して別の経路で続けること）には当たらない。 |
| オプションの名前と型 | `option name EvalFile type string default <empty>`。値はファイルのパスである。 | やねうら王の`EvalDir`は、ディレクトリを受け取り、決まった名前のファイル`nn.bin`を読む。パスを直接受け取るほうが、測定で候補と基準に別々のファイルを渡す用途に合う。PV-MCTSの`NnModel`に倣って`NnueModel`とする案は、GUIの利用者が知る名前ではない。`EvalFile`は、Stockfishでも同じくネットのファイルのパスを受け取る。 |
| 既定値 | 持たない。空のまま`isready`を受けたらエラーにする。 | やねうら王は既定のディレクトリ`eval`を持つ。minaseでは、設定の既定値で黙って続けることをCLAUDE.mdが禁じており、PV-MCTSの`NnModel`も既定値を持たない。 |
| 読み込む時期 | `isready`で読む。`setoption name EvalFile`はパスを保存し、読み込み済みのネット、置換表、およびhistory表を捨てるだけにする。同じパスを設定し直した場合も捨て、次の`isready`で読み直す。 | Stockfishは`setoption`の処理の中で読む。minaseでは、重い準備を`isready`で行うというUSIの約束と、`NnModel`と同じ扱いに揃える。パスが変わったときだけ読み直す案は、同じパスのファイルを差し替えた場合に古いネットのまま進む。 |
| 失敗時の動作 | 未設定、読み取りの失敗、復号の検証の失敗では、原因とパスを含むエラーの行を出し、`readyok`を返さずに終了コード1で止まる。成功したら`readyok`の直前に`info string network_sha256 <64桁の16進数>`を出す。 | 読み込みに失敗しても埋め込みのネットや前のネットで続ける案は、CLAUDE.mdのフォールバックの禁止に反する。 |
| ネットの識別子 | `isready`で読み込んだファイル全体のバイト列のSHA-256を、ネットの唯一の識別子とする。この値を、エンジンの報告、対局の実行器の記録、および教師ファイルの`network_checksum`に共通に使う。PV-MCTSの`NnModel`がONNXのファイルに使う定義と同じである。理由は各論の「ネットの識別子」の節にある。 | いまの重みのファイルの形式MNUE（minaseのNNUEの重みの形式、版5）のヘッダにあるチェックサムは本体（192バイト目以降）だけを覆うので、評価値の換算に使うヘッダの勝率尺度や規則セットの名前が違っても、本体が同じなら区別できない。MNUEのチェックサムの範囲をヘッダまで広げて版を上げる案は、HalfKP_vmの設計書で重みの形式をtataraの形式（チェックサムを持たない）へ揃えるので、すぐに捨てる変更になる。ファイル全体のSHA-256とMNUEのチェックサムを両方とも識別子として記録する案は、同じネットに2つの識別子を持たせ、照合を重ねる。2026年10月13日に利用者は、同じ対象にチェックサムの照合を重ねないと決めている。 |
| 測定でのネットの固定 | コードはコミットで、ネットは実行記録（対局の実行器の`manifest.json`）のチェックサムで固定する。ネットだけを比べる測定では、候補と基準に同じコミットを指定し、`--candidate-option EvalFile=<パス>`と`--baseline-option EvalFile=<パス>`で別々のファイルを渡す。 | ネットのチェックサムを書いた小さなファイルをgitに置き、ネットを変えるたびにコミットする案は、対局の実行器の記録とgitの2か所に同じ事実を持たせる。対局の実行器にネット専用の引数を足す案は、ハーネス（規約の語で、対局の実行器を指す）に機能比較のスイッチを足さないという規約（`CONTRIBUTING.md`）に触れ、既存の追加オプションの仕組みで足りる。 |
| CECP | `feature option="EvalFile -file "`を宣言し、`option EvalFile=<パス>`で受け取る。`new`の処理と探索の開始の共通の入口で、未準備ならネットを読み、失敗ならエラーを出して終了コード1で止まる。対局の実行器は、CECPのエンジンにも`--candidate-option`と`--baseline-option`を`option <名前>=<値>`として`new`の前に送り、初期化の最後に`ping`を送って`pong`を待つ。 | CECPだけ起動引数`--eval-file`で受け取る案は、起動引数をプロトコルの開始前に要る情報に限るという[ADR](../../adr/keep-protocol-search-method-and-io-log-as-startup-arguments.md)に反する。CECPには`isready`がないので、`new`だけで準備する案は、`setboard`から`go`へ進む経路で未準備のまま探索に入る。初期化で`pong`を待たない案は、ネットを読む時間が最初の着手の消費時間に入る。 |
| PV-MCTSの勝率尺度 | PV-MCTSは、探索値（[MultiPV](multipv.md)の2本目以降の読み筋の探索値を含む）をセンチポーンで出すときと、引き分けの値をセンチポーンから勝率へ換算するときに勝率尺度を使い、いまは埋め込みのNNUEのヘッダから読んでいる。これをUSIのオプション`NnCpPerLogit`（型string、既定値なし）で受け取り、正の有限の実数であることを`isready`で検査する。 | PV-MCTSにも`EvalFile`を要求する案は、1つの実数のために約500 MBのネットを読ませ、PV-MCTSの実行条件を2つのネットに依存させる。定数にする案は、学習で決まる値を手で固定する。 |
| サブコマンドでのネットの指定 | ネットで評価するサブコマンド（`data selfplay generate`、`data lishogi games`、`data lishogi openings`）は、必須の引数`--eval-file <パス>`を取る。対局の実行器`match run`は、起動時に自分のプロセスに埋め込んだネットを検査する処理をやめる。`minase spsa`の調整の実行は、調整する係数のオプション`Tune_<名前>`と別に、繰り返し指定できる固定のUSIオプション`--option <名前>=<値>`を取る。ネットを使わないサブコマンド（`data selfplay inspect`、`match report`、`spsa params`、`spsa apply`など）はネットを要求しない。 | サブコマンドはプロトコルではないので、起動引数のADRの対象ではない。SPSAの固定のオプションを`--eval-file`に限る案は、PV-MCTSの係数を調整するときに`NnModel`と`NnCpPerLogit`も渡す必要があり、同じ仕組みを2度作ることになる。 |
| `bench`と評価の内訳の表示 | どちらもUSIの独自の命令であり、αβ探索では`isready`で準備したネットを使う。`isready`の前に受けたらエラーを返す。PV-MCTSで起動したエンジンの`bench`は、ネットを使わない試験用の評価器で探索するので、ネットの準備を要求しない。 | `bench`だけ別にネットを指定する案は、同じセッションの中にネットの指定の経路を2つ作る。 |
| 保管と配布 | ネットはgitに入れない。開発中のネットは、PV-MCTSのONNXのネットと同じNAS（ネットワークに接続した記憶装置）に、チェックサムの16進数をファイル名にして置く。リリースでは、採用したネットをGitHubのリリースに添付し、チェックサムをリリースノートに書く。いまの`nnue.bin`はgitの管理から外し、履歴は書き換えない。 | ネットのチェックサムを書いた小さなファイルをgitに置く案は、上の「測定でのネットの固定」の理由で採らない。GitHubのリリースの添付は1ファイル2 GiB未満を受け付ける。 |
| テストの固定ネット | `nnue-epoch0.bin`などの小さな固定ネットは、テストの資材としてgitに残し、テストは`include_bytes!`（ビルドのときにファイルを埋め込むRustのマクロ）で読むか、一時ファイルに書いて`EvalFile`で渡す。 | テストで本番のネットを読む案は、学習で変わる値にテストを依存させる（[固定ネットの教訓](../../lessons/search-tests-use-a-fixed-net.md)）。テスト用の埋め込みは本番の読み込みの代わりに使わないので、フォールバックには当たらない。 |
| 教師ファイルの形式 | MNSD（minaseの教師ファイルの形式）の配置と版3は変えず、ヘッダの`network_checksum`に入れる値だけを、ネットのファイル全体のSHA-256にする。 | MNSDの版を上げる案は、欄の配置が変わらないのに既存の教師ファイルを読めなくする。既存の教師ファイルの値はMNUEの本体のチェックサムであり、同じネットでもファイル全体のSHA-256とは一致しないので、再開の照合で取り違えることはない。 |

## 各論

### USIのαβ探索の契約

αβ探索の探索器は、`EvalFile`で読んだネットを`Arc<Nnue>`として自分で持ち、探索と診断へ渡す。
いまのプロセス全体で1つのネットを返す関数`eval::weights()`と、その中の`OnceLock`、および埋め込みの`EMBEDDED`は削除する。
エンジンの起動時に埋め込みのネットを検査する処理（`crates/minase/src/bin/minase/engine.rs`の`main`）も削除する。
`isready`での準備は、置換表の確保と同じ`Searcher::prepare`の中で行い、PV-MCTSの`NnModel`と同じく、成功したら`Searcher::network_sha256`がチェックサムを返す。
ネットの準備がないまま`go`、`bench`、または評価の内訳の表示を受けたら、エラーの行を返して命令を実行しない。

`bench`は局面と規則を自分専用のもの（日本中将棋連盟の規則に基づく規則セット`JCSA`）に固定して探索するので、`RuleSet`の値は`bench`の結果に影響しない。
しかし、`bench`は`isready`で準備したネットを使い、`isready`はいまも規則セットの指定`RuleSet`を必須としているので、`bench`の前にも`RuleSet`、`EvalFile`、および`isready`を送る。

相対パスは、エンジンの作業ディレクトリを起点に解決する。
GUI、対局の実行器、および生成機では、絶対パスを渡す。

### ネットの識別子

NNUEの評価値は、駒と升ごとの点数を足す線形の経路と、第1層から後段を通る非線形の経路の和である。
ヘッダの勝率尺度は、復号のときに非線形の経路の出力をセンチポーンへ換算する整数の係数scale＝round(勝率尺度×65,536÷(127×4,096))になり（`format.rs`）、いまのネットの勝率尺度1072.65ではscaleは135である。
したがって勝率尺度が違えば、本体が同じでも評価値が変わる。
また学習器は、学習の出発点として読み込む前の世代のネット（基点の重み、引数`--base`）から、勝率尺度と規則セットの名前を引き継ぐ。
いまのMNUEのチェックサムは本体だけを覆うので、ヘッダだけが違う2つのネットを区別できない。

そこで、`isready`で読み込んだファイル全体のバイト列のSHA-256を、ネットの識別子とする。
読み込んだバイト列を復号と同じ一度の読み取りから求めるので、識別子と実際に使うネットが食い違うことはない。
MNUEのヘッダのチェックサムは、本書の変更の後も復号のときに本体の破損を見つける検査として残すが、識別子としては使わない。
重みの形式は、HalfKP_vmの設計書でtataraの形式へ揃えるときに替わり、そのときこのチェックサムもなくなる。

いまのmasterのネット（P型、幅256、勝率尺度1072.6529541015625）は、本書の変更の後も、旧基準として埋め込んだコミットのまま比較に使える。
対局の実行器は、エンジンが宣言しないオプションを送る前に拒否するので、旧基準のコミットを測るときは、そのコミットの側の追加のオプションに`EvalFile`を指定しない。
本書の検証で同じネットを新しいエンジンに渡すため、いまの`nnue.bin`をgitの管理から外す前に、同じバイト列のままNASへ写す。

### CECPの契約

CECPの`option EvalFile=<パス>`はパスを保存し、読み込み済みのネット、置換表（探索した局面の結果を貯める表）、およびhistory表（着手ごとの良さの統計を貯めて手の並べ替えに使う表）を捨てるだけで、その場では読まない。
`option RuleSet=<値>`の扱いは変えず、`EvalFile`との順序に制約を付けない。
`new`は、対局を始める前にネットを準備する。
CECPでは、相手の着手を受けたエンジンは、自分の手番になると命令を待たずに探索して指す。
探索の開始は、`go`と、相手の着手を受けて探索する経路の両方から共通の関数`start_search`へ集まるので、この関数でも、未準備なら準備する。
これにより、`new`を経ずに`setboard`から`go`へ進む経路と、対局の途中で`EvalFile`を変えた後に相手の着手を受けて探索する経路も、未準備のまま探索に入らない。
準備に失敗したら、エラーを出力して終了コード1で止まる。

対局の実行器は、いまCECPのエンジンへの追加のオプションを拒否し（`harness/player.rs`）、初期化の命令を送るだけで応答を待たない（`harness/engine/process.rs`）。
本書の変更の後は、CECPのエンジンにも追加のオプションを`option <名前>=<値>`として`new`の前に送り、初期化の最後に`ping`を送って対応する`pong`を受けてから対局へ進む。
オプションの名前は、エンジンが`feature option`で宣言した名前と照合し、宣言にない名前は送る前に、測定全体を止めるエラーとして拒否する。
対局の実行器は、CECPのエンジンも、対局を始める前に同じ初期化の列で`pong`まで1回起動し（事前の起動）、宣言にない名前や`EvalFile`の誤りを最初の対局より前に見つける。
CECPでminaseを動かす`scripts/hachu_replay.py`は、ネットのパスを自分の引数で受け取り、`option RuleSet`の後、`new`の前に`option EvalFile=<パス>`を送る。

### 対局の実行器とSPSA

対局の実行器は、いまも候補と基準に別々のUSIオプションを`isready`の前に送り、エンジンが報告した`network_sha256`を`manifest.json`に記録して、局ごとの開始と再開のときに照合している（[ADR](../../adr/archive/record-and-verify-the-network-checksum-reported-by-match-engines.md)）。
本書の変更の後は、αβ探索のエンジンもチェックサムを報告するので、この記録と照合がαβ探索にもそのまま効く。
USIの経路で対局の実行器のコードを変えるのは、起動時に自分の埋め込みのネットを検査する処理（`crates/minase/src/bin/minase/match_runner/mod.rs`の`main`）を削ることだけである。

`minase spsa`は、いま対局ごとに追加のオプションを`Tune_*`の列で置き換えている（`spsa_runner/mod.rs`の`configure`）。
本書の変更の後は、`--option`で受け取った固定のオプションに`Tune_*`の列を足して送る。
固定のオプションと、エンジンが報告したチェックサムをセッションの記録に残し、再開のときと対局ごとに照合する。
照合の扱いは対局の実行器と同じにする。

### 教師生成と連続生成

`data selfplay generate`は、`--eval-file`で受け取ったネットを1回だけ読み、各スレッドで共有する。
生成条件`config.json`の`network_checksum`と、MNSDのヘッダの`network_checksum`には、ネットのファイル全体のSHA-256を書く。
再開の照合は、いまと同じく生成コミットとこのチェックサムで行う。

生成機での教師生成（`scripts/vast/teacher_generation.py`と`generation_worker.py`）は、コードのgitのバンドルとは別に、ネットのファイルを転送する。
ジョブの記録`job.json`には、ネットのパスとチェックサムを必須の欄として書き、生成機では`--eval-file`を付けて教師を生成する。
期待するチェックサムは、いまの`git show <コミット>:crates/minase/nets/nnue.bin`の代わりに、手元のネットのファイル全体のSHA-256として求める。
転送で壊れたネットや取り違えたネットは、`data selfplay generate`が教師ファイルに書いたチェックサムを回収のときに期待値と比べることで見つかる。この照合は、いまの`common.inspect_teacher`が行っている。

連続生成スクリプト`scripts/vast/generation_chain.py`は、世代ごとのネットをgitにコミットするのをやめる。
世代ごとの実行記録に、生成に使ったコードのコミット、ネットのパスとチェックサム、および世代の完了の状態を書き、再開はこの記録から判断する。
いまは世代の完了をgitの参照の有無で判定しているので、この判定も記録へ移す。
学習した世代のネットは、手元の機械へ回収した後、チェックサムを求めてNASに置く。
世代ごとに行う深さ5の`bench`の完走の確認では、`RuleSet`、`EvalFile`、`isready`の後に`bench`を送る。

### そのほかのスクリプト

速度比較の`scripts/bench_compare.py`は、ネットのパスを引数で受け取り、`RuleSet`、`EvalFile`、および`isready`の後に`bench`を送る。
宣言していない名前の`setoption`をminaseは黙って無視するので（[USIの拡張の手引き](../../guides/usi-extensions.md)）、この命令列は、ネットを埋め込んだ旧版のminaseにもそのまま使える。
結果のキャッシュの名前には、深さとスレッド数に加えてネットのチェックサムを入れる。
生成機でのSPSA（`scripts/vast-spsa/launch.py`、`remote.py`、`bench.py`）は、教師生成と同じくネットを転送し、調整の前の速さの測定でも同じ命令列で`bench`を送り、`minase spsa`に`--option EvalFile=<パス>`を渡す。
USIでminaseを起動する調査のスクリプト（`byoyomi_game_profile.py`、`lishogi_byoyomi_usage/`の`paired_depth_probe.py`と`signal_probe.py`、`mate_report_check.py`、`movetime_deadline/deadline_probe.py`）は、ネットのパスを引数で受け取り、`EvalFile`を送る。
PV-MCTSのネットの検査（`tools/train/zero/minase_zero/runner.py`）は、`NnModel`に加えて`NnCpPerLogit`を送る。
送る値は、いまのmasterのネットのヘッダの勝率尺度1072.6529541015625とし、学習の設定に明示して記録する。

### 保管、パッケージ、およびリリース

`crates/minase/nets/nnue.bin`はgitの管理から外す。
`crates/minase/Cargo.toml`の`include`は`nets/`を含んだままにし、テストの固定ネットだけがパッケージに入る。
[リリースの手引き](../../guides/release.md)には、採用したネットをGitHubのリリースに添付し、チェックサムをリリースノートに書く段を加える。
`crates/minase/README.md`の導入の手順には、ネットを取得して`EvalFile`で指定する段を加える。
`EvalFile`を必須にする変更はUSIオプションの互換を壊すので、コミットのtypeに`!`を付け、本文に`BREAKING CHANGE:`のフッターを書く。
リリースの版の番号は、リリースのときに利用者が決める。

## 実装フェーズ

実装はcodexへ委任し、Claudeはフェーズごとに検証してコミットする。
フェーズの境界ごとに`cargo fmt`、`cargo clippy`、`cargo test`、`--features search-stats,tuning`を付けた検査（[テストの手引き](../../guides/testing-minase-rules.md)）、および変更したPythonのテストを通す。

1. エンジンのプロトコルと、エンジンを子プロセスとして動かす道具を変える。エンジンが`EvalFile`を必須にした時点で、子プロセスへ`EvalFile`を送れない道具は新しいエンジンを動かせなくなるので、両方を同じフェーズで変える。`eval::weights()`と埋め込みは、教師生成と棋譜の取り込みのためにこのフェーズでは残す。
   - エンジンには、αβ探索の`EvalFile`、`isready`での準備とチェックサムの報告、αβ探索の`bench`と評価の内訳の表示での準備の要求、CECPの`option EvalFile`、およびPV-MCTSの`NnCpPerLogit`を入れ、起動時の検査を削除する。
   - 道具には、対局の実行器のCECPへの追加のオプションと`ping`による同期、および`minase spsa`の`--option`とチェックサムの記録と照合を入れ、`match run`と`minase spsa`の起動時のネットの検査を削除する。
   - 完了条件は、次の4点をテストで確かめることである。第1に、未設定、存在しないパス、および壊れたファイルのそれぞれで、USIとCECPが文書どおりのエラーで止まり、正しいネットでは準備に成功すること。第2に、CECPで`setboard`から`go`へ進む経路と、`EvalFile`を変えた後に相手の着手を受けて探索する経路が、どちらも準備を経ること。第3に、`EvalFile`を要求してチェックサムを報告する模擬のエンジンを使ったSPSAのテストで、固定のオプションが`Tune_*`と一緒に送られ、別のネットでの再開が拒否されること。第4に、CECPとUSIの対局の結合テストが通ること。
2. 教師生成と棋譜の取り込みのサブコマンドに`--eval-file`を入れ、`eval::weights()`、`EMBEDDED`、および`nnue.bin`を削除する。`nnue.bin`は削除の前にNASへ写す。本番のネットを読んでいたテストは固定ネットへ移す。完了条件は、教師生成の再開が別のネットで拒否されることをテストで確かめることである。
3. 教師生成、連続生成、速度比較、生成機でのSPSA、調査、およびPV-MCTSのネットの検査のスクリプトを改める。完了条件は、Pythonのテストが通り、連続生成スクリプトがgitにネットをコミットしないことである。
4. 手引き、README、パッケージの設定、およびリリースの手引きを改める。完了条件は、`cargo package -p minase --list`に`nnue.bin`が含まれないことである。

## 検証

完了の前に次を確かめる。

- 探索が変わらないことを確かめる。masterのコミット（ネットを埋め込んだもの）と本書のブランチの先頭のコミット（NASへ写した同じ`nnue.bin`を`EvalFile`で渡したもの）に、`setoption name RuleSet value JCSA`、`setoption name EvalFile value <NASのnnue.bin>`、`isready`、`bench depth 5`を送り、各局面の最善手、評価値、およびノード数が完全に一致することを確かめる。深さ5は、USIの拡張の手引きの例と、世代反復の`bench`の確認で使っている深さである。評価と探索の手順を変えないので、自己対局の勝敗から逐次検定で採否を決める棋力測定（SPRT）は行わない。
- 速さが変わらないことを確かめる。同じ2つのコミットを、`scripts/bench_compare.py`で性能コアへ固定し、`--rounds 5`で交互に5回ずつ測る。回数と交互の測り方は[速度の比べ方の教訓](../../lessons/alternate-speed-rounds-against-parent.md)に従う。本書のブランチの5回の平均とmasterの5回の平均の差が、masterの5回の値の幅（最大値と最小値の差）より小さいことを確かめる。教訓と同じく、比べる元のコミットであるmasterの値の幅を測定のばらつきの目安とし、幅を超えて遅ければ原因を調べる。
- 対局の実行器がαβ探索のネットを記録し照合することを確かめる。同じコミットを候補と基準に指定し、同じネットを渡した2ペア（同じ開始局面で先後を入れ替えて指す2局の組を1ペアとする）を、決まったペア数を指す実行の型`elo --pairs 2`で実行して、`manifest.json`の両側に`network_sha256`が記録されることを確かめる。続けて、基準のネットのファイルを別のネットに差し替えて再開し、照合の不一致で止まることを確かめる。
- 教師生成が動くことを確かめる。`data selfplay generate`を2局で実行し、出力の`config.json`とMNSDのヘッダのチェックサムが渡したネットのチェックサムと一致すること、および別のネットで再開すると止まることを確かめる。
- CECPが動くことを確かめる。`scripts/hachu_replay.py`で1局を再生できることと、`option EvalFile`を送らない場合は`new`でエラーになることを確かめる。CECPの探索の開始の準備は、フェーズ1のテストで確かめる。

## 完了条件

本書は、次をすべて満たしたときに完了（採用）とする。

- 「検証」の節の確認をすべて満たしている。
- ネットを使うすべての入口（USI、CECP、および「設計判断」の表に挙げたサブコマンド）が、ネットのファイルを明示して受け取り、未指定と失敗で止まる。
- `crates/minase/nets/nnue.bin`がgitの管理から外れ、crates.ioのパッケージに含まれない。
- 仕様を、`EvalFile`と`NnCpPerLogit`の宣言、ネットの識別子、およびサブコマンドの引数のdocコメントと、次の手引きへ移している：[USIの拡張](../../guides/usi-extensions.md)、[測定](../../guides/sprt.md)、[SPSA](../../guides/spsa.md)、[NNUEの学習](../../guides/nnue-training.md)、[PSTの学習](../../guides/pst-training.md)、[NNUEの評価関数](../../guides/evaluation-nnue.md)、[ファイル形式](../../guides/file-formats.md)、[デバッグ](../../guides/debugging.md)、[テスト](../../guides/testing-minase-rules.md)、[PV-MCTSのネットの検査](../../guides/alphazero-network-check.md)、[AlphaZero型探索の学習](../../guides/alphazero-training.md)、および[リリース](../../guides/release.md)。
- 設計判断を新しいADRへ移している。次のADRの記述は、置き換えるか補うADRを書き、置き換えた旧版は`docs/adr/archive/`へ移す。
  - [探索が使う評価関数をNNUEの1つにするADR](../../adr/archive/use-nnue-as-the-only-search-evaluation.md)の、ネットを埋め込む決定。
  - [教師ファイルを決まった局数の塊で確定するADR](../../adr/archive/commit-nnue-teacher-files-in-fixed-game-chunks-and-reassemble-them-into-one-file.md)の、ネットが埋め込まれているので生成コミットがネットを決めるという前提。
  - [ネットのチェックサムを記録して照合するADR](../../adr/archive/record-and-verify-the-network-checksum-reported-by-match-engines.md)の、SPSAとαβ探索を記録の対象から外す扱い。
  - [コミットを別々のエンジンのプロセスとして比べるADR](../../adr/archive/compare-commits-as-separate-engine-processes.md)の、測定の対象をコミットで固定する記述。
  - [診断をUSIの命令にするADR](../../adr/archive/run-engine-diagnostics-as-usi-commands-and-keep-experiment-tools-as-subcommands.md)の、`bench`には`isready`が要らないという決定。
  - [NNUEを入力特徴集合と出力部の型で表すADR](../../adr/archive/parameterize-the-nnue-by-feature-set-and-head-types.md)の、埋め込んだネットを起動時に検証するという記述。
  - [学習の後に線形の経路を補正しないADR](../../adr/archive/do-not-correct-the-nnue-linear-path-after-training.md)の、世代のネットをコミットして次の世代の初期値にするという記述。
- [世代反復の設計書](../nnue-generation-iteration.md)の手順を、外部のネットを使う形に書き直している。

## 参考資料

- やねうら王（`../YaneuraOu`、コミット`3b8d53fa`、2026年11月11日）：`source/eval/nnue/evaluate_nnue.cpp`の64〜90行（`EvalDir`の宣言と既定値）と891〜952行（`load_eval`での読み込みと失敗時の終了）、`source/eval/nnue/evaluate_nnue.h`の89行（決まったファイル名`nn.bin`）、`source/engine/yaneuraou-engine/yaneuraou-search.cpp`の238行（`isready`の処理）、`source/config.h`の378〜385行（埋め込みは`EVAL_EMBEDDING=ON`のときだけ）、`source/Makefile`の171行（既定は`EVAL_EMBEDDING = OFF`）。
- Stockfish（`../stockfish`、コミット`5c4cef0b`、2026年10月13日）：`src/engine.cpp`の136行（`EvalFile`の宣言）、`src/nnue/network.cpp`の50行（既定のネットの埋め込み）、91〜110行（読み込みの順序）、164〜191行（読み込めなかったときのエラーと終了）、238〜262行（外部と内部の読み込み）。
- minase：`crates/minase/src/search/mcts/engine.rs`（`NnModel`の宣言と準備）、`crates/minase/src/protocol/usi.rs`（`isready`での`RuleSet`の要求と`network_sha256`の報告）、`crates/minase/src/harness/engine/process.rs`（報告値の照合とCECPの初期化）、`crates/minase/src/eval/nnue/format.rs`（MNUE版5の検証とチェックサムの範囲、勝率尺度から求める係数`scale`）、`crates/minase/src/eval/nnue/accumulator.rs`（`nonlinear_cp`での`scale`の使用）、`crates/minase/tests/match_runner.rs`（CECPとUSIの対局の結合テスト）、`docs/guides/usi-extensions.md`（宣言していないオプションの`setoption`を無視する規則）。
- GitHubのファイルの大きさの上限：[About large files on GitHub](https://docs.github.com/en/repositories/working-with-files/managing-large-files/about-large-files-on-github)（gitの1ファイル100 MiB）、[About releases](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases)（リリースの添付は1ファイル2 GiB未満）。
