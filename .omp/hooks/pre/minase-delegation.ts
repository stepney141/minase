// AGENTS.md の分業のうち、機械的に判定できる二つの場面で親セッションの書き込みを止める。
// 計画用 worktree での実装は implementer に、新しい計画書の起案前調査は lead に任せる。
import * as fs from "node:fs";
import * as path from "node:path";
import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent/extensibility/extensions";

// 計画用 worktree の中で親が編集してよいのは docs/ だけである。
const PLAN_WORKTREE = /\/data\/worktrees\/[^/]+\/(.+)$/;
// docs/plans/ 直下の計画書。README.md と archived/ 配下は対象外。
const PLAN_FILE = /\/docs\/plans\/(?!README\.md$)[^/]+\.md$/;
const URI = /^[a-z]+:\/\//;

function leadSpawned(ctx: ExtensionContext): boolean {
	return ctx.sessionManager.getBranch().some(
		entry =>
			entry.type === "message" &&
			entry.message.role === "assistant" &&
			entry.message.content.some(
				part =>
					part.type === "toolCall" &&
					part.name === "task" &&
					Array.isArray(part.arguments.tasks) &&
					part.arguments.tasks.some((t: { agent?: unknown }) => t.agent === "lead"),
			),
	);
}

export default function (pi: ExtensionAPI): void {
	pi.on("tool_call", async (event, ctx) => {
		if (ctx.agent.kind !== "main") return;
		if (event.toolName !== "edit" && event.toolName !== "write") return;
		const input = event.input as { path?: string; paths?: string[] };
		const targets = input.paths ?? (input.path ? [input.path] : []);
		for (const target of targets) {
			if (URI.test(target)) continue;
			const abs = path.resolve(ctx.cwd, target);
			const worktree = PLAN_WORKTREE.exec(abs);
			if (worktree && !worktree[1].startsWith("docs/")) {
				return {
					block: true,
					reason: `${abs} は計画用 worktree のコードである。計画書に基づく実装は、指示書を書いて implementer に委任する。bash や eval で書き換えて迂回しない。`,
				};
			}
			if (event.toolName === "write" && PLAN_FILE.test(abs) && !fs.existsSync(abs) && !leadSpawned(ctx)) {
				return {
					block: true,
					reason: `新しい計画書 ${abs} を起案する前に、このセッションで lead に調査範囲と判断すべき問いを渡し、analyst との議論を依頼する。`,
				};
			}
		}
	});
}
