#!/usr/bin/env python3
"""benchの診断JSON（MINASE_ORDERING_DIAG）から、設計書「診断の定義」項目1〜5の集計表を印字する。"""
import json, sys

def pct(n, d):
    return f"{100.0 * n / d:.2f}%" if d else "n/a"

def mean(n, d):
    return f"{n / d:.1f}" if d else "n/a"

for path in sys.argv[1:]:
    j = json.load(open(path))
    print(f"## {path}")
    depths = ["1", "2", "3", "4+"]
    heads = ["tt", "capture_see_negative", "capture_other", "killer", "quiet"]
    print("\n### 項目1 先行費用W（残り深さ×先頭手の種別）")
    print("| d | 先頭手 | βカット数 | 先頭手カット率 | W合計 | 平均W | Wの全体比 |")
    print("|---|---|---:|---:|---:|---:|---:|")
    total_w = sum(j["w"][d][h]["w_sum"] for d in depths for h in heads)
    for d in depths:
        for h in heads:
            w = j["w"][d][h]
            if w["count"] == 0:
                continue
            print(f"| {d} | {h} | {w['count']:,} | {pct(w['first_move_cutoffs'], w['count'])} | {w['w_sum']:,} | {mean(w['w_sum'], w['count'])} | {pct(w['w_sum'], total_w)} |")
    print("\n### 項目1 深さ別の集計")
    print("| d | ループ入り | βカット | βカットなし | 静かな手生成前カット | 初手カット率 | W合計 | 平均W | カット手 tt/capture/killer/quiet |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|---|")
    for d in depths:
        cuts = sum(j["w"][d][h]["count"] for h in heads)
        first = sum(j["w"][d][h]["first_move_cutoffs"] for h in heads)
        wsum = sum(j["w"][d][h]["w_sum"] for h in heads)
        ct = j["cut_move_type"][d]
        print(f"| {d} | {j['loop_nodes'][d]:,} | {cuts:,} | {j['no_cutoff_nodes'][d]:,} | {j['cutoff_before_quiets'][d]:,} ({pct(j['cutoff_before_quiets'][d], cuts)}) | {pct(first, cuts)} | {wsum:,} ({pct(wsum, total_w)}) | {mean(wsum, cuts)} | {ct['tt']:,}/{ct['capture']:,}/{ct['killer']:,}/{ct['quiet']:,} |")
    print("\n### 項目1 捕獲段の先頭（位置×SEE）")
    print("| d | 位置 | SEE | 件数 | 部分木合計 | 平均部分木 | 先頭自身のカット率 |")
    print("|---|---|---|---:|---:|---:|---:|")
    for d in depths:
        for pos in ["node_head", "after_tt"]:
            for see in ["negative", "other"]:
                c = j["capture_head"][d][pos][see]
                if c["count"] == 0:
                    continue
                print(f"| {d} | {pos} | {see} | {c['count']:,} | {c['subtree_sum']:,} | {mean(c['subtree_sum'], c['count'])} | {pct(c['cut_by_head'], c['count'])} |")
    c1 = j["cand1"]
    r = c1["capture_cut_rank"]
    print("\n### 項目2 候補1")
    print(f"- 発動率（順序が変わる段/2手以上の段）: {c1['segments_changed']:,}/{c1['segments']:,} = {pct(c1['segments_changed'], c1['segments'])}")
    print(f"- 更新の機会率（先に捕獲手を探索した静かなカット/静かなカット）: {c1['quiet_cutoff_nodes_with_searched_captures']:,}/{c1['quiet_cutoff_nodes']:,} = {pct(c1['quiet_cutoff_nodes_with_searched_captures'], c1['quiet_cutoff_nodes'])}")
    print(f"- 捕獲カット手の順位: 件数 {r['count']:,}、前進 {r['forward']:,}（幅計 {r['forward_sum']:,}）、後退 {r['backward']:,}（幅計 {r['backward_sum']:,}）")
    print("\n### 項目3 候補2（零窓ノード、残り深さ×置換表の記録手）")
    print("| d | tt | 零窓ノード | 先頭SEE負 | 同率 | 先頭SEE負が枝刈り | +合法killer | 発動率 | SEE負先頭の部分木合計 | killerあり先頭の部分木合計 | 平均 | killerあり中、静かな手生成前カット | generate_quiets回数 |")
    print("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for d in ["1", "2", "3"]:
        for tt in ["no_tt", "tt"]:
            c = j["cand2"][d][tt]
            z = c["zero_window_nodes"]
            print(f"| {d} | {tt} | {z:,} | {c['head_negative']:,} | {pct(c['head_negative'], z)} | {c['head_negative_pruned']:,} | {c['head_negative_with_killer']:,} | {pct(c['head_negative_with_killer'], z)} | {c['head_negative_subtree_sum']:,} | {c['head_negative_with_killer_subtree_sum']:,} | {mean(c['head_negative_with_killer_subtree_sum'], c['head_negative_with_killer'])} | {c['head_negative_with_killer_cutoff_before_quiets']:,} ({pct(c['head_negative_with_killer_cutoff_before_quiets'], c['head_negative_with_killer'])}) | {c['generate_quiets_calls']:,} |")
    c3 = j["cand3"]
    print("\n### 項目4 候補3")
    print(f"- 静止探索ノード {c3['qnodes']:,}、2候補以上の群 {c3['groups_ge2']:,}、先頭が変わる群 {c3['groups_head_changed']:,} = {pct(c3['groups_head_changed'], c3['groups_ge2'])}")
    print(f"- 読む集合が変わるノード {c3['nodes_set_differs']:,}/{c3['nodes_compared']:,} = {pct(c3['nodes_set_differs'], c3['nodes_compared'])}")
    c5 = j["cand5"]
    s = c5["success_moves"]
    print("\n### 項目5 候補5")
    print(f"- 発動率（先頭が変わるノード/静かな手の段を開いたノード）: {c5['head_changed']:,}/{c5['quiet_stage_nodes']:,} = {pct(c5['head_changed'], c5['quiet_stage_nodes'])}")
    print(f"- 静かな手 {c5['quiets_total']:,}、history 0 の手 {c5['quiets_history_zero']:,} = {pct(c5['quiets_history_zero'], c5['quiets_total'])}、大きさ2以上の同点群に属する手 {c5['quiets_in_tie_groups_ge2']:,} = {pct(c5['quiets_in_tie_groups_ge2'], c5['quiets_total'])}")
    tg = c5["tie_groups"]
    print(f"- 同点群の大きさ別件数: " + ", ".join(f"{k}: {v:,}" for k, v in tg.items()))
    print(f"- 成功手（カットまたはαを上げた静かな手）: 件数 {s['count']:,}、同点群（2以上）から {s['from_tie_group_ge2']:,}、前進 {s['forward']:,}（幅計 {s['forward_sum']:,}）、後退 {s['backward']:,}（幅計 {s['backward_sum']:,}）")
    print()
