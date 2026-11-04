root='crates/minase/src/search/'
def rep(path, old, new, count=1):
    s=open(path).read()
    assert s.count(old)>=1, (path, old)
    s=s.replace(old,new) if count==0 else s.replace(old,new,count)
    open(path,'w').write(s)
p=root+'probe_counters.rs'
s=open(p).read()
s=s.replace('"quiet_score_calls","check_see_calls",','"quiet_score_calls","check_see_calls","ch_read","ch_row_cold","ch_rowkind_cold","ch_val_init","ph_read","ph_cold","ph_val_init","cc_read","cc_row_cold","cc_rowkind_cold","cc_val_init","c5_read","c5_val_init","ch_update","cc_update","ph_update","nz_in","nz_in_nonzero",')
s=s.replace('pub const N: usize = 57;','pub const N: usize = 75;')
open(p,'w').write(s)
h=root+'alphabeta/history.rs'
rep(h,"pub(crate) struct ContinuationHistory {\n    values: Box<[AtomicI16]>,\n}","pub(crate) struct ContinuationHistory {\n    values: Box<[AtomicI16]>,\n    masks: Vec<std::sync::atomic::AtomicU32>,\n}")
rep(h,"            values: atomic_history(CONTINUATION_HISTORY_LEN, CONTINUATION_HISTORY_INITIAL),\n","            values: atomic_history(CONTINUATION_HISTORY_LEN, CONTINUATION_HISTORY_INITIAL),\n            masks: (0..CONTINUATION_HISTORY_LEN/(29*144)).map(|_| std::sync::atomic::AtomicU32::new(0)).collect(),\n")
rep(h,"""    pub(super) fn read(&self, index: usize) -> i32 {
        i32::from(self.values[index].load(Ordering::Relaxed))
    }

    /// 指定要素へ上限30000の重力式で更新量を与える。
    pub(super) fn update(&self, index: usize, bonus: i32) {
""","""    pub(super) fn read(&self, index: usize) -> i32 {
        let row = index/(29*144); let kind=(index%(29*144))/144;
        let m=self.masks[row].load(Ordering::Relaxed);
        crate::pc!("ch_read");
        if m==0 { crate::pc!("ch_row_cold"); }
        if m>>kind&1==0 { crate::pc!("ch_rowkind_cold"); }
        let v=i32::from(self.values[index].load(Ordering::Relaxed));
        if v==i32::from(CONTINUATION_HISTORY_INITIAL) { crate::pc!("ch_val_init"); }
        v
    }

    /// 指定要素へ上限30000の重力式で更新量を与える。
    pub(super) fn update(&self, index: usize, bonus: i32) {
        crate::pc!("ch_update");
        { let row = index/(29*144); let kind=(index%(29*144))/144; self.masks[row].fetch_or(1<<kind, Ordering::Relaxed); }
""")
rep(h,"""    pub(super) fn read(&self, key: u64, piece: PieceCode, to: Square) -> i32 {
        i32::from(self.values[self.index(key, piece, to)].load(Ordering::Relaxed))
    }""","""    pub(super) fn read(&self, key: u64, piece: PieceCode, to: Square) -> i32 {
        crate::pc!("ph_read");
        let i=self.index(key, piece, to);
        let row=i/144;
        if !self.touched[row].load(Ordering::Relaxed) { crate::pc!("ph_cold"); }
        let v=i32::from(self.values[i].load(Ordering::Relaxed));
        if v==i32::from(PAWN_HISTORY_INITIAL) { crate::pc!("ph_val_init"); }
        v
    }""")
rep(h,"""pub(crate) struct PawnHistory {
    values: Box<[AtomicI16]>,
""","""pub(crate) struct PawnHistory {
    touched: Vec<std::sync::atomic::AtomicBool>,
    values: Box<[AtomicI16]>,
""")
rep(h,"""        Self {
            values: atomic_history(len, PAWN_HISTORY_INITIAL),""","""        Self {
            touched: (0..len/144).map(|_| std::sync::atomic::AtomicBool::new(false)).collect(),
            values: atomic_history(len, PAWN_HISTORY_INITIAL),""")
rep(h,"""    pub(super) fn update(&self, key: u64, piece: PieceCode, to: Square, bonus: i32) {
""","""    pub(super) fn update(&self, key: u64, piece: PieceCode, to: Square, bonus: i32) {
        crate::pc!("ph_update");
        self.touched[self.index(key, piece, to)/144].store(true, Ordering::Relaxed);
""")
c=root+'alphabeta/correction.rs'
rep(c,"""pub(crate) struct ContinuationCorrectionHistory {
    values: Box<[i16]>,
}""","""pub(crate) struct ContinuationCorrectionHistory {
    values: Box<[i16]>,
    masks: Vec<u32>,
}""")
rep(c,"""            values: vec![CONTINUATION_CORRECTION_INITIAL; CONTINUATION_CORRECTION_LEN]
                .into_boxed_slice(),
""","""            values: vec![CONTINUATION_CORRECTION_INITIAL; CONTINUATION_CORRECTION_LEN]
                .into_boxed_slice(),
            masks: vec![0; CONTINUATION_CORRECTION_LEN/(29*144)],
""")
rep(c,"""    pub(super) fn read(&self, index: usize) -> i32 {
        i32::from(self.values[index])
    }""","""    pub(super) fn read(&self, index: usize) -> i32 {
        crate::pc!("cc_read");
        let row=index/(29*144); let kind=(index%(29*144))/144;
        let m=self.masks[row];
        if m==0 { crate::pc!("cc_row_cold"); }
        if m>>kind&1==0 { crate::pc!("cc_rowkind_cold"); }
        if self.values[index]==CONTINUATION_CORRECTION_INITIAL { crate::pc!("cc_val_init"); }
        i32::from(self.values[index])
    }""")
rep(c,"""    pub(super) fn update(&mut self, index: usize, bonus: i32, multiplier: i32) {
""","""    pub(super) fn update(&mut self, index: usize, bonus: i32, multiplier: i32) {
        crate::pc!("cc_update");
        { let row=index/(29*144); let kind=(index%(29*144))/144; self.masks[row]|=1<<kind; }
""")
rep(c,"""    pub(super) fn read(&self, side: Color, key: u64, table: CorrectionTableKind) -> i32 {
        i32::from(self.values.values[self.values.index(side, key, table)].load(Ordering::Relaxed))
    }""","""    pub(super) fn read(&self, side: Color, key: u64, table: CorrectionTableKind) -> i32 {
        crate::pc!("c5_read");
        let v=i32::from(self.values.values[self.values.index(side, key, table)].load(Ordering::Relaxed));
        if v==i32::from(CORRECTION_INITIAL) { crate::pc!("c5_val_init"); }
        v
    }""")
# head density
hd='crates/minase/src/eval/nnue/head.rs'
rep(hd,"""        let mut u = [0_u8; 30];
""","""        crate::pc!("nz_in", input.len());
        crate::pc!("nz_in_nonzero", input.iter().filter(|&&a| a != 0).count());
        let mut u = [0_u8; 30];
""")
