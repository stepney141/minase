# エラーを文字列へ失わない

## 症状

`RuleCode`の`FromStr`実装と`parse_rule_set`はエラー型に`String`を使い、`RejectReason`は規則エラーと局面エラーを文字列へ変換して保持し、対局未開始という「the game has not started」まで`InvalidPosition`に含めていた。
呼出し側は未知の規則コード、併用できない規則、対局未開始を安全に分類できず、元のエラーを`source`でたどることもできないため、ログ以外の回復処理が文字列比較に依存した。

## 原因

エラーを表示用の文字列として生成し、失敗の分類を型に持たせなかった。

## 以後の規則

失敗の原因ごとに専用のエラー型を設けて分類を型で保持し、表示用文字列への変換はプロトコル出力の直前に限定し、テストや回復処理が文字列比較を要する設計にしない。

## 出典

- [plans/rust-design-audit-remediation.md](../plans/rust-design-audit-remediation.md)
- [rust-design-audit-2026-08-26.md](../rust-design-audit-2026-08-26.md) の「文字列へ失われるエラー情報」
- コミット 8b48929
