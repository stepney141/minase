# 並行処理の回帰テストは連続実行で確認する

## 症状

補助ワーカーが主ワーカーより先に固定深さへ達すると主ワーカーの進捗が空になる競合は、統合検査中に確率的にしか再現しなかった。
1回のテスト成功では修正の効果と偶然の通過を区別できなかった。

## 原因

スレッドのスケジューリングに依存する不具合は、同じテストでも実行ごとに発火の有無が変わる。
単発の`cargo test`の通過は、競合が起きなかった1標本にすぎない。

## 以後の規則

スケジューリングに依存する不具合を修正したら、該当する回帰テストを10回以上連続で実行し、全回通過してから採用する。

## 出典

- [plans/rust-design-audit-remediation.md](../plans/rust-design-audit-remediation.md) の設計判断「固定深さの完了通知」
- `src/search/tests.rs` の `four_worker_fixed_depth_finishes_at_the_limit_with_a_legal_move`
