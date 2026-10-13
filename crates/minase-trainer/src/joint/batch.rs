use super::Result;
use crate::data::Sample;
use crate::{
    data::{Dataset, build_targets_from_arrays},
    fm, pst,
};

/// 同じ鏡映後の局面を、全特徴のFMと鏡映共有のPSTで表すバッチ。
#[derive(Debug)]
pub struct Batch {
    pub(super) pst: pst::Batch,
    pub(super) columns: Vec<[u32; 145]>,
    pub(super) targets: Vec<f32>,
}
impl Batch {
    /// 入力番号順に教師値を作り、指定された鏡映を両モデルへ適用する。
    pub fn from_indices(
        dataset: &Dataset,
        indices: &[u64],
        mirrors: &[u8],
        ks: &[f64],
    ) -> Result<Self> {
        let records = indices
            .iter()
            .map(|&i| dataset.record(i))
            .collect::<crate::data::Result<Vec<_>>>()?;
        let scores = indices
            .iter()
            .map(|&i| dataset.teacher_score(i))
            .collect::<crate::data::Result<Vec<_>>>()?;
        let classes = indices
            .iter()
            .map(|&i| dataset.teacher_class(i))
            .collect::<crate::data::Result<Vec<_>>>()?;
        let pst = pst::Batch::from_records(&records, &scores, &classes)?;
        let targets = build_targets_from_arrays(
            &scores,
            &classes,
            pst.outcomes(),
            ks,
            dataset.teacher_lambdas(),
        )?;
        Self::assemble(pst, &records, mirrors, &targets)
    }
    /// 読み込み済み局面から両モデルに共通の特徴と教師値を作る。
    pub fn from_records(records: &[Sample], mirrors: &[u8], targets: &[f32]) -> Result<Self> {
        let pst =
            pst::Batch::from_records(records, &vec![0.0; records.len()], &vec![0; records.len()])?;
        Self::assemble(pst, records, mirrors, targets)
    }
    fn assemble(
        mut pst: pst::Batch,
        records: &[Sample],
        mirrors: &[u8],
        targets: &[f32],
    ) -> Result<Self> {
        if records.len() != mirrors.len()
            || records.len() != targets.len()
            || mirrors.iter().any(|&m| m > 1)
            || targets
                .iter()
                .any(|&t| !t.is_finite() || !(0.0..=1.0).contains(&t))
        {
            return Err("joint batch mirror or target shape/range".into());
        }
        let columns = records
            .iter()
            .zip(mirrors)
            .map(|(r, &m)| fm::batch::features(r, m != 0).map(|(c, _)| c))
            .collect::<Result<_>>()?;
        pst.reflect(mirrors);
        Ok(Self {
            pst,
            columns,
            targets: targets.to_vec(),
        })
    }
    /// バッチの局面数。
    pub fn len(&self) -> usize {
        self.targets.len()
    }
    /// 構築済みバッチは常に非空。
    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }
    /// 鏡映後の盤面の升順の全特徴番号。末尾は先獅子。
    pub fn columns(&self) -> &[[u32; 145]] {
        &self.columns
    }
}
