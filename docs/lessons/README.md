# 教訓の索引

作業で詰まった箇所から得た汎用的な教訓を、1教訓1ファイルで集約する。
各ファイルは「症状」「原因」「以後の規則」「出典」の4見出しを持つ（書式は plans/README.md）。
本書は1行1教訓の索引であり、作業を始める前に目を通す。

- [外部入力でパニックしない](no-panic-on-external-input.md) — 外部から受ける値は専用エラーへ変換し、バイト添字、桁あふれし得る換算、`unreachable!`、黙示的補完を外部入力の経路に置かない。
- [不正状態を作れる公開型を避ける](checked-constructors-for-public-types.md) — 不変条件を持つ公開型はフィールドを非公開にし、検査つきコンストラクタだけで構築し、0を認めない数値は`NonZero`型で受ける。
- [スレッドを所有するハンドルはDropで回収する](join-threads-on-drop.md) — スレッドを生成する型が唯一の回収責任者となり、`Drop`でも停止とjoinを行い、パニック時は残るワーカーへ停止を通知する。
- [エラーを文字列へ失わない](typed-errors-over-strings.md) — 失敗の原因ごとに専用エラー型で分類を保持し、文字列化はプロトコル出力の直前に限る。
- [最小対応版は実際のツールチェーンで検査する](verify-msrv-with-toolchain.md) — `rust-version`の宣言や引き上げはその版のツールチェーンで全ターゲットを検査してから確定する。
- [並行処理の回帰テストは連続実行で確認する](repeat-concurrency-regression-tests.md) — スケジューリング依存の不具合を修正したら回帰テストを10回以上連続で通してから採用する。
- [回帰テストは修正を外して失敗を確認する](confirm-regression-test-fails-without-fix.md) — 回帰テストを追加したら修正を一時的に外して失敗することを確認してから採用する。
- [xboardの-autoflagは値を取らない](xboard-autoflag-argtrue.md) — xboardで時間切れ検出を有効にするときは`-autoCallFlag true`を書き、`-autoflag`に値を続けない。
- [外部エンジンは接続前に棋譜照合と着手転送で検証する](verify-external-engine-before-cecp-match.md) — `cecp:`で外部エンジンを接続する前に、自己対局棋譜の審判層照合と`usermove`転送の盤面一致を確認する。
- [置換表なしの静止探索は捕獲木の重複を全列挙する](qsearch-dag-without-tt.md) — 速度や並列化を論じる前にbenchで置換表を照会するノードの割合を数え、大半が置換表を使わない経路なら先にその経路へ置換表を適用する。
- [検証損失の近さは駒価値の正しさを保証しない](validation-loss-hides-material-distortion.md) — 学習した評価関数をGSPRTへ出す前に、教師探索値との平均絶対誤差と駒種を1つ除いた局面の評価で駒価値の歪みを点検する。
- [ランダム初期化の評価関数では探索が止まらない](random-init-net-stalls-search.md) — 推論コードは学習済み重みと同じコミットで入れ、ランダム初期化の重みは一致テストと参照実装の照合にだけ使う。
- [GPUを要する学習はcodexへ委任しない](run-gpu-training-outside-codex-sandbox.md) — codexのサンドボックスからGPUは見えないので、学習器のコードだけを委任し、学習の実行は本環境で直接行う。
- [学習前に1エポックのステップ数を確認する](check-steps-per-epoch-before-training.md) — 局面数÷バッチサイズで1エポックのステップ数を計算し、総ステップ数が数千回以上になる設定にしてから学習曲線を評価する。
- [速度指標の計測区間に大容量メモリの確保を含めない](bench-allocation-outside-timing.md) — benchでは置換表を計測区間外で1個を使い回し、確保サイズを変えても指標が動かないことを確認する。
- [採否測定の条件で改良が発動することを実装前に確認する](measure-feature-activation-before-sprt.md) — 探索改良は実装前に採否測定の思考制限で発動するかをbenchで確かめ、発動しなければ条件変更か先送りにする。
