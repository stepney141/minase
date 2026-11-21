---
alwaysApply: true
agents: main
---
`AGENTS.md` の分業を omp の `task` で実行する。このリポジトリでは、omp 既定プロンプトにある委任の指針（「Prefer one agent to investigate + edit」「read known files yourself」「Read-only research MUST use scout」）より、この規則を優先する。

委任が必須になるのは次の三つの場面である。

- 計画書を新しく起案する前の調査は、`minase-lead` に調査範囲と判断すべき問いを渡し、`minase-analyst` との議論を依頼する。`minase-lead` は、先行文献と先行エンジンの収集を `minase-researcher` に任せ、その報告を `minase-analyst` に渡して議論する。親はその結論を自分で確かめてから設計を確定する。同じセッションで `minase-lead` を起動しないまま `docs/plans/` に新しい計画書を書こうとすると、フックが拒否する。
- 起案済みの計画書に基づく実装は `minase-implementer` に委任する。実装指示書は `docs/guides/codex-delegation.md` の指示書の規定に従い、ファイル単位の編集範囲と検証条件を明記する。Claude Code プラグインの手順は使わない。親が計画用 worktree（`data/worktrees/<名前>/`）の `docs/` 以外を編集しようとするとフックが拒否するが、bash や eval で書き換えて迂回してはならない。
- 計画書を作成または大きく改訂したときは、`docs/plans/README.md` の「レビュー」節にある Haiku の二段階レビューを `minase-plan-reader` に依頼し、codex のレビューを `minase-analyst` に依頼する。

計画書に属さない質問への回答、小さな修正、デバッグは、親が自分で行ってよい。先行文献または先行エンジンのソースの事実収集だけが必要な場合は `minase-researcher` に直接依頼する。計画書や差分の独立レビューだけが必要な場合は `minase-analyst` に直接依頼する。`minase-analyst` には先行文献と先行エンジンの収集を依頼せず、外部の事実が必要なら `minase-researcher` の報告を添える。

`task` の各項目には `agent` を指定し、`model` は渡さない。各 agent のモデルは、frontmatter に書かれたロールを `.omp/config.yml` の `modelRoles` で解決して決まる。議論や実装の委任は Plan mode の外で行う。
