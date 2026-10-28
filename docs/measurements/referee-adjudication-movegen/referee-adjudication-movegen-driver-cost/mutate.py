"""ソースの複製に誤りを1つずつ入れ、minase-coreの単体試験が失敗することを確かめる。

使い方: python3 -I mutate.py <ソースの複製のルート> <結果のJSON>
"""

import json
import pathlib
import subprocess
import sys

MUTATIONS = [
    {
        "name": "two_royals_judged_by_attacks",
        "description": "can_capture_last_royalで王駒が2枚の場合も、いずれかへの疑似利きで真とする",
        "file": "crates/minase-core/src/game/adjudication/mate.rs",
        "old": "    match royals.popcount() {\n        1 => {",
        "new": "    match royals.popcount() {\n        1 | 2 => {",
    },
    {
        "name": "attacking_move_ignores_royals",
        "description": "move_was_attackingの王駒の条件を外し、動かした駒の捕獲だけで判定する",
        "file": "crates/minase-core/src/game/repetition/r1.rs",
        "old": "        .any(|royal| {\n            !position\n                .attackers_to_by(mover, royal, position.occupied())\n                .is_empty()\n        })\n    {\n        return true;\n    }",
        "new": "        .any(|_royal| false)\n    {\n        return true;\n    }",
    },
    {
        "name": "attacking_move_counts_quiet_moves",
        "description": "move_was_attackingの動かした駒の条件で、捕獲でない手も数える",
        "file": "crates/minase-core/src/game/repetition/r1.rs",
        "old": "generator.generate_captures_from(&probe, played.to, &mut base_moves, &mut moves);",
        "new": "generator.generate_moves_from(&probe, played.to, &mut base_moves, &mut moves);",
    },
    {
        "name": "grace_judged_by_attacks",
        "description": "駒枯れの猶予を、余分な駒と王駒の升への疑似利きで判定する（獅子の捕獲制限を無視する）",
        "file": "crates/minase-core/src/game/adjudication/exhaustion.rs",
        "old": "    if moves.into_iter().any(|candidate| {\n        position\n            .captured_squares(candidate)\n            .into_iter()\n            .flatten()\n            .any(|square| remaining[extra_color.index()].contains(square))\n    }) {",
        "new": "    let _ = &moves;\n    if remaining[extra_color.index()].into_iter().any(|square| {\n        !position\n            .attackers_to_by(position.side_to_move(), square, position.occupied())\n            .is_empty()\n    }) {",
    },
    {
        "name": "checked_move_skips_legality",
        "description": "try_make_move_with_undoが合法性を検査せずに適用する",
        "file": "crates/minase-core/src/movegen/checked.rs",
        "old": "        if generator.is_legal_move(self, mv, &mut base_moves, &mut moves) {",
        "new": "        if generator.is_legal_move(self, mv, &mut base_moves, &mut moves) || true {",
    },
    {
        "name": "mate_skips_immediate_results",
        "description": "詰み判定で候補手の即時の勝ちと引き分けを逃れる手に数えない",
        "file": "crates/minase-core/src/game/adjudication/mate.rs",
        "old": "                            Some(GameResult::Draw { .. }) => true,",
        "new": "                            Some(GameResult::Draw { .. }) => false,",
    },
]


def run_tests(root):
    completed = subprocess.run(
        ["nice", "-n", "19", "cargo", "test", "--offline", "-p", "minase-core", "--lib"],
        cwd=root,
        capture_output=True,
        text=True,
    )
    failed = [
        line.split()[1]
        for line in completed.stdout.splitlines()
        if line.startswith("test ") and line.endswith("FAILED")
    ]
    return completed.returncode, failed


def main():
    root = pathlib.Path(sys.argv[1])
    results = []
    code, failed = run_tests(root)
    if code != 0:
        sys.exit(f"unmutated source fails: {failed}")
    for mutation in MUTATIONS:
        path = root / mutation["file"]
        original = path.read_text()
        if original.count(mutation["old"]) != 1:
            sys.exit(f"{mutation['name']}: pattern must occur once")
        path.write_text(original.replace(mutation["old"], mutation["new"]))
        try:
            code, failed = run_tests(root)
        finally:
            path.write_text(original)
        results.append(
            {
                "name": mutation["name"],
                "description": mutation["description"],
                "file": mutation["file"],
                "detected": code != 0,
                "failed_tests": failed,
            }
        )
        print(mutation["name"], "detected" if code != 0 else "NOT DETECTED", len(failed), flush=True)
    with open(sys.argv[2], "w", encoding="utf-8") as file:
        json.dump(results, file, indent=2, ensure_ascii=False)


if __name__ == "__main__":
    main()
