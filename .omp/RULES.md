`AGENTS.md` の分業を omp の `task` で実行する。親は設計の確定、変更のレビューと検証、`docs/` の編集、コミットを担当する。

- 起案前の調査や複雑な問題は `minase-lead` に調査範囲と判断すべき問いを渡し、`minase-analyst` との議論を依頼する。計画書や差分の独立レビューだけが必要なら `minase-analyst` に直接依頼する。
- 確定済みのフェーズを実装するときは `minase-implementer` に委任する。実装指示書は `docs/guides/codex-delegation.md` の指示書の規定に従い、ファイル単位の編集範囲と検証条件を明記する。Claude Code プラグインの手順は使わない。
- `task` の各項目には `agent` を指定し、`model` は渡さない。各 agent のモデルは、frontmatter に書かれたロールを `.omp/config.yml` の `modelRoles` で解決して決まる。議論や実装の委任は Plan mode の外で行う。
