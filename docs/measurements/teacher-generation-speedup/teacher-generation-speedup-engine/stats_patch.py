import re
root='crates/minase/src/search/'
# 1. counters module
open(root+'probe_counters.rs','w').write('''
use std::sync::atomic::{AtomicU64, Ordering};
pub const NAMES: &[&str] = &[
 "nm_nodes","nm_d1","nm_d2","nm_d3","nm_d4p","nm_tt_cut","nm_rfp","nm_razor","nm_nmp_ret","nm_loop",
 "pk_captures_gen","pk_captures_total","pk_killer_gen","pk_quiets_stage","pk_quiets_total","pk_quiets_popped",
 "pk_quiets_stage_d1","pk_quiets_stage_d2","pk_quiets_stage_d3","pk_quiets_stage_d4p","pk_popped_d1","pk_popped_d2","pk_popped_d3","pk_popped_d4p",
 "pk_killer_ret","pk_cap_ret","pk_tt_ret","pk_moves_searched_cut_killer",
 "qs_calls","qs_tt_cut","qs_standpat_cut","qs_init","qs_cand","qs_searched","qs_tt_move","qs_no_candidates",
 "see_calls","gc_calls","gc_qs","rep_calls","rep_hist_sum","eval_calls","legal_move_calls","royal_calls","lesser_calls","lesser_groups","rep_hit",
 "nm_cut_at_tt","nm_cut_at_cap","nm_cut_at_killer","nm_cut_at_quiet","nm_fail_low","nm_exact",
 "quiet_score_calls","check_see_calls",
];
pub const N: usize = 57;
pub static C: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
#[inline] pub fn add(name: usize, v: u64) { C[name].fetch_add(v, Ordering::Relaxed); }
pub fn dump() { for (i,n) in NAMES.iter().enumerate() { eprintln!("probe {} {}", n, C[i].load(Ordering::Relaxed)); } }
''')
s=open(root+'mod.rs').read()
s=s.replace('pub mod ','pub mod probe_counters;\npub mod ',1) if 'probe_counters' not in s else s
open(root+'mod.rs','w').write(s)
