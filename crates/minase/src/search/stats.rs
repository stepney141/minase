//! ワーカーごとに数え、探索終了後に合算する探索統計。

/// 項目の宣言から構造体、合算、回数の表示をまとめて生成する。
macro_rules! search_stats {
    ($(#[doc = $doc:literal] $field:ident),+ $(,)?) => {
        /// 探索の呼び出しと打ち切りの回数。
        ///
        /// null moveと再探索も呼び出しごとに数える。
        /// 着手の適用を数える[`SearchResult::nodes`](super::SearchResult::nodes)とは定義が異なる。
        /// 通常探索は内部ノードのnegamaxを指し、根の手の走査は含めない。
        /// 中断した反復も含め、実際に発生した事象を数える。
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        pub struct SearchStats {
            $(#[doc = $doc] pub $field: u64,)+
        }

        impl core::ops::AddAssign for SearchStats {
            fn add_assign(&mut self, rhs: Self) {
                $(self.$field += rhs.$field;)+
            }
        }

        impl core::fmt::Display for SearchStats {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "stats:")?;
                $(write!(f, " {}={}", stringify!($field), self.$field)?;)+
                Ok(())
            }
        }
    };
}

search_stats! {
    /// 残り深さが正の通常探索の呼び出し数。
    normal_nodes,
    /// 静止探索の呼び出し数。通常探索の深さ0からの移行を含む。
    quiesce_nodes,
    /// 通常探索で置換表を照会した回数。
    tt_probes,
    /// 通常探索の置換表照会で項目が一致した回数。
    tt_hits,
    /// 通常探索で置換表の値により打ち切った回数。
    tt_cutoffs,
    /// 通常探索と根で置換表の補正前の静的評価を使った回数。
    tt_static_eval_uses,
    /// 静止探索で置換表を照会した回数。
    quiesce_tt_probes,
    /// 静止探索の置換表照会で項目が一致した回数。
    quiesce_tt_hits,
    /// 静止探索で置換表の値により打ち切った回数。
    quiesce_tt_cutoffs,
    /// 静止探索で手数制限により読まなかった候補数。
    quiesce_move_count_prunes,
    /// 静止探索で補正後のstand-patと余裕値を基準にしたSEEの閾値により枝刈りした候補数。
    quiesce_see_threshold_prunes,
    /// 静止探索でSEEの下限により枝刈りした候補数。
    quiesce_see_floor_prunes,
    /// 静止探索で取り返しにより、本来は当たる手数制限、delta pruning、またはSEEの閾値を免れた候補数。
    quiesce_recapture_exemptions,
    /// 静止探索で成る手により、本来は当たる手数制限、delta pruning、またはSEEの閾値を免れた候補数。
    quiesce_promotion_exemptions,
    /// 静止探索で王手になる手により、本来は当たる手数制限、delta pruning、またはSEEの閾値を免れた候補数。
    quiesce_check_exemptions,
    /// 静止探索で獅子の2枚取りにより手数制限を免れた候補数。
    quiesce_lion_double_move_count_exemptions,
    /// 静止探索の枝刈り条件に当たった候補について、王手になるかを判定した回数。
    quiesce_gives_check_calls,
    /// 静止探索のdelta pruningの見積りが最善値を実際に引き上げた回数。
    quiesce_delta_best_score_raises,
    /// 静止探索のSEEの閾値の見積りが最善値を実際に引き上げた回数。
    quiesce_see_threshold_best_score_raises,
    /// 静止探索で入口のαとfutility baseの差以下の価値の群から候補を返したノード数。先頭の置換表手を除き、ノードごとに1回だけ数える。
    quiesce_low_value_group_nodes,
    /// 完了したnull move検証探索の回数。中断した呼び出しは含めない。
    null_move_verifications,
    /// 完了したnull move検証探索のうち、値がβ未満だった回数。
    null_move_verification_failures,
    /// 通常探索で着手の探索結果によって手の走査を打ち切った回数。
    beta_cutoffs,
    /// 通常探索のβカットのうち、最初に探索した手による回数。
    first_move_beta_cutoffs,
    /// 全手の走査を完了し、最後の下限更新が探索した1手目だったノード数。
    best_move_rank_1,
    /// 全手の走査を完了し、最後の下限更新が探索した2手目だったノード数。
    best_move_rank_2,
    /// 全手の走査を完了し、最後の下限更新が探索した3手目だったノード数。
    best_move_rank_3,
    /// 全手の走査を完了し、最後の下限更新が探索した4手目以降だったノード数。
    best_move_rank_4_plus,
}

impl SearchStats {
    /// 全手を走査したノードで、最後に下限を更新した手の探索順位を数える。
    pub(crate) fn record_best_move_rank(&mut self, rank: usize) {
        match rank {
            0 => {}
            1 => self.best_move_rank_1 += 1,
            2 => self.best_move_rank_2 += 1,
            3 => self.best_move_rank_3 += 1,
            _ => self.best_move_rank_4_plus += 1,
        }
    }
}
