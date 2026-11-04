root='crates/minase/src/search/'
pc=open(root+'probe_counters.rs').read()
pc+='''
pub const fn idx(name: &str) -> usize {
    let mut i = 0;
    while i < NAMES.len() {
        let a = NAMES[i].as_bytes(); let b = name.as_bytes();
        if a.len() == b.len() {
            let mut j = 0; let mut eq = true;
            while j < a.len() { if a[j] != b[j] { eq = false; } j += 1; }
            if eq { return i; }
        }
        i += 1;
    }
    panic!("unknown counter")
}
#[macro_export]
macro_rules! pc {
    ($n:literal) => { $crate::pc!($n, 1) };
    ($n:literal, $v:expr) => {{ const I: usize = $crate::search::probe_counters::idx($n); $crate::search::probe_counters::add(I, ($v) as u64); }};
}
'''
open(root+'probe_counters.rs','w').write(pc)

def rep(path, old, new, count=1):
    s=open(path).read()
    assert s.count(old)>=1, (path, old)
    s=s.replace(old,new) if count==0 else s.replace(old,new,count)
    open(path,'w').write(s)

n=root+'alphabeta/negamax.rs'
rep(n,"        let depth = depth.min(MAX_PLY - 1);\n","""        let depth = depth.min(MAX_PLY - 1);
        crate::pc!("nm_nodes");
        match depth { 1 => crate::pc!("nm_d1"), 2 => crate::pc!("nm_d2"), 3 => crate::pc!("nm_d3"), _ => crate::pc!("nm_d4p") }
""")
rep(n,"                        return Some(value.score);\n                    }\n                }\n            }\n        }\n\n        // 置換表の打ち切りは","""                        crate::pc!("nm_tt_cut");
                        return Some(value.score);
                    }
                }
            }
        }

        // 置換表の打ち切りは""")
rep(n,"            return Some(reverse_futility_value(beta, eval));","""            crate::pc!("nm_rfp");
            return Some(reverse_futility_value(beta, eval));""")
rep(n,"            let value = self.quiesce(position, alpha, beta, ply)?;\n            return Some(value);","""            crate::pc!("nm_razor");
            let value = self.quiesce(position, alpha, beta, ply)?;
            return Some(value);""")
rep(n,"                    return Some(null_move_cutoff_score(score, beta));","""                    crate::pc!("nm_nmp_ret");
                    return Some(null_move_cutoff_score(score, beta));""",0)
rep(n,"        self.move_pickers[ply as usize].reset(","""        crate::pc!("nm_loop");
        self.move_pickers[ply as usize].depth_bucket = depth.min(4) as u8;
        self.move_pickers[ply as usize].reset(""")
rep(n,"                self.cutoff_counts[ply as usize] += 1;\n                beta_cutoff = true;","""                match self.move_pickers[ply as usize].last_stage {
                    0 => crate::pc!("nm_cut_at_tt"), 1 => crate::pc!("nm_cut_at_cap"), 2 => crate::pc!("nm_cut_at_killer"), _ => crate::pc!("nm_cut_at_quiet"),
                }
                self.cutoff_counts[ply as usize] += 1;
                beta_cutoff = true;""")
rep(n,"        let bound = if best_score <= original_alpha {\n            Bound::Upper","""        if best_score <= original_alpha { crate::pc!("nm_fail_low"); } else if !beta_cutoff { crate::pc!("nm_exact"); }
        let bound = if best_score <= original_alpha {
            Bound::Upper""")
rep(n,"            let raw = self\n                .nnue\n                .evaluate_accumulator(","""            crate::pc!("eval_calls");
            let raw = self
                .nnue
                .evaluate_accumulator(""")
rep(n,"            .filter(|&mv| {\n                self.generator\n                    .is_legal_move(","""            .filter(|&mv| {
                crate::pc!("legal_move_calls");
                self.generator
                    .is_legal_move(""")

o=root+'alphabeta/ordering.rs'
rep(o,"    quiets_generated: bool,\n}\n\nimpl MovePicker {","    quiets_generated: bool,\n    pub(super) depth_bucket: u8,\n    pub(super) last_stage: u8,\n}\n\nimpl MovePicker {")
rep(o,"            quiets_generated: false,\n        }\n    }\n\n    /// 確保した領域","            quiets_generated: false,\n            depth_bucket: 0,\n            last_stage: 0,\n        }\n    }\n\n    /// 確保した領域")
rep(o,"""                    if let Some(tt_move) = self.tt_move
                        && generator.is_legal_move(""","""                    if self.tt_move.is_some() { crate::pc!("legal_move_calls"); }
                    if let Some(tt_move) = self.tt_move
                        && generator.is_legal_move(""")
rep(o,"                        return Some((tt_move, move_order_key(position, nnue, tt_move).is_some()));","""                        crate::pc!("pk_tt_ret"); self.last_stage = 0;
                        return Some((tt_move, move_order_key(position, nnue, tt_move).is_some()));""")
rep(o,"                        self.captures_generated = true;\n","""                        self.captures_generated = true;
                        crate::pc!("pk_captures_gen"); crate::pc!("pk_captures_total", self.captures.len());
""")
rep(o,"""                        self.capture_index += 1;
                        return Some((mv, true));""","""                        self.capture_index += 1;
                        crate::pc!("pk_cap_ret"); self.last_stage = 1;
                        return Some((mv, true));""")
rep(o,"""                        generator.generate_quiets(position, &mut self.base_moves, &mut self.quiets);
""","""                        crate::pc!("pk_killer_gen");
                        generator.generate_quiets(position, &mut self.base_moves, &mut self.quiets);
""")
rep(o,"""                        self.used_quiets[killer_index] = Some(index);
                        return Some((self.quiets[index], false));""","""                        self.used_quiets[killer_index] = Some(index);
                        crate::pc!("pk_killer_ret"); self.last_stage = 2;
                        return Some((self.quiets[index], false));""")
rep(o,"""                MovePickerStage::Quiets => {
""","""                MovePickerStage::Quiets => {
                    crate::pc!("pk_quiets_stage"); crate::pc!("pk_quiets_total", self.quiets.len());
                    match self.depth_bucket { 1 => crate::pc!("pk_quiets_stage_d1"), 2 => crate::pc!("pk_quiets_stage_d2"), 3 => crate::pc!("pk_quiets_stage_d3"), _ => crate::pc!("pk_quiets_stage_d4p") }
""")
rep(o,"""                        self.quiet_index += 1;
                        return Some((self.quiets[index], false));""","""                        self.quiet_index += 1;
                        crate::pc!("pk_quiets_popped"); self.last_stage = 3;
                        match self.depth_bucket { 1 => crate::pc!("pk_popped_d1"), 2 => crate::pc!("pk_popped_d2"), 3 => crate::pc!("pk_popped_d3"), _ => crate::pc!("pk_popped_d4p") }
                        return Some((self.quiets[index], false));""")
rep(o,"""        controls.sort_unstable_by_key(|&(value, _)| value);
""","""        crate::pc!("lesser_calls");
        controls.sort_unstable_by_key(|&(value, _)| value);
""")
rep(o,"""    let piece = piece_at_for_ordering(position, mv.from);
    let piece_value = nnue.piece_value(piece);
    let moving_kind = piece.kind().expect("quiet ordering mover kind");""","""    crate::pc!("quiet_score_calls");
    let piece = piece_at_for_ordering(position, mv.from);
    let piece_value = nnue.piece_value(piece);
    let moving_kind = piece.kind().expect("quiet ordering mover kind");""")

q=root+'alphabeta/quiesce.rs'
rep(q,"        self.pv[ply as usize].clear();\n\n        #[cfg(feature = \"invariants\")]","        self.pv[ply as usize].clear();\n        crate::pc!(\"qs_calls\");\n\n        #[cfg(feature = \"invariants\")]")
rep(q,"""                    if beta == alpha + 1 {
                        #[cfg(feature = "search-stats")]""","""                    if beta == alpha + 1 {
                        crate::pc!("qs_tt_cut");
                        #[cfg(feature = "search-stats")]""")
rep(q,"""        } else {
            self.nnue
                .evaluate_accumulator(&self.accumulators[ply as usize], side)
        };""","""        } else {
            crate::pc!("eval_calls");
            self.nnue
                .evaluate_accumulator(&self.accumulators[ply as usize], side)
        };""")
rep(q,"            return Some(qsearch_entry_cutoff_score(best, beta));","""            crate::pc!("qs_standpat_cut");
            return Some(qsearch_entry_cutoff_score(best, beta));""")
rep(q,"        let buffers = &mut self.qsearch[ply as usize];\n        buffers.reset(position);","""        crate::pc!("qs_init");
        if tt_move.is_some() { crate::pc!("qs_tt_move"); }
        let buffers = &mut self.qsearch[ply as usize];
        buffers.reset(position);""")
rep(q,"            let candidate_index = candidate_count;\n","""            crate::pc!("qs_cand");
            let candidate_index = candidate_count;
""")
rep(q,"                self.enter_node().then_some(())?;\n","""                crate::pc!("qs_searched");
                self.enter_node().then_some(())?;
""")
rep(q,"        let best = qsearch_post_search_score(best, beta);\n","""        if candidate_count == 0 { crate::pc!("qs_no_candidates"); }
        let best = qsearch_post_search_score(best, beta);
""")
rep(q,"""            self.stats.quiesce_gives_check_calls += 1;
        }
""","""            self.stats.quiesce_gives_check_calls += 1;
        }
        crate::pc!("gc_qs");
""")
s=root+'alphabeta/see.rs'
rep(s,"""    threshold: i32,
) -> bool {
    if mv.mid.is_some() {
        return false;
    }
""","""    threshold: i32,
) -> bool {
    crate::pc!("see_calls");
    if mv.mid.is_some() {
        return false;
    }
""")
se=root+'alphabeta/searcher.rs'
rep(se,"""    pub(super) fn find_repetition(&self, key: u64) -> Option<RepetitionMatch> {
""","""    pub(super) fn find_repetition(&self, key: u64) -> Option<RepetitionMatch> {
        crate::pc!("rep_calls"); crate::pc!("rep_hist_sum", self.history_keys.len());
""")
r=root+'alphabeta/royal.rs'
import re
t=open(r).read()
t=t.replace("pub(super) fn royal_under_attack(position: &Position) -> bool {\n","pub(super) fn royal_under_attack(position: &Position) -> bool {\n    crate::pc!(\"royal_calls\");\n")
open(r,'w').write(t)
# dump
g='crates/minase/src/bin/minase/selfplay_gen/generate.rs'
rep(g,"    let (total, recorded) = total;\n","    let (total, recorded) = total;\n    minase::search::probe_counters::dump();\n")
# gives_check counter in negamax closures: count via minase-core static
gc='crates/minase-core/src/movegen/gives_check.rs'
rep(gc,"    pub fn gives_check(&self, generator: &MoveGenerator, mv: Move) -> bool {\n","    pub fn gives_check(&self, generator: &MoveGenerator, mv: Move) -> bool {\n        GC_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);\n")
rep(gc,"impl Position {","pub static GC_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);\n\nimpl Position {")
