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
