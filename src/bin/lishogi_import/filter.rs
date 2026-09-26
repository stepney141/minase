//! 対局の除外判定と理由別の集計。

use super::input::{InputGame, time_control};
use minase::Color;
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};

/// 最初に該当した理由だけに集計する。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Exclusion {
    Variant,
    InitialSfen,
    Bot,
    Unrated,
    MissingSpeed,
    Rating,
    StatusOutoftime,
    StatusTimeout,
    StatusAborted,
    StatusDraw,
    StatusRepetition,
    StatusPerpetualCheck,
    StatusBareKing,
    StatusCreated,
    StatusStarted,
    StatusMate,
    StatusStalemate,
    StatusCheat,
    StatusNoStart,
    StatusUnknownFinish,
    UnknownStatus,
    Winner,
    Duplicate,
    IllegalDefaultRules,
    WinnerMismatch,
    ResignerMismatch,
    DeferredPromotion,
}

impl std::fmt::Display for Exclusion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for Exclusion {}

impl Exclusion {
    const ALL: [Self; 27] = [
        Self::Variant,
        Self::InitialSfen,
        Self::Bot,
        Self::Unrated,
        Self::MissingSpeed,
        Self::Rating,
        Self::StatusOutoftime,
        Self::StatusTimeout,
        Self::StatusAborted,
        Self::StatusDraw,
        Self::StatusRepetition,
        Self::StatusPerpetualCheck,
        Self::StatusBareKing,
        Self::StatusCreated,
        Self::StatusStarted,
        Self::StatusMate,
        Self::StatusStalemate,
        Self::StatusCheat,
        Self::StatusNoStart,
        Self::StatusUnknownFinish,
        Self::UnknownStatus,
        Self::Winner,
        Self::Duplicate,
        Self::IllegalDefaultRules,
        Self::WinnerMismatch,
        Self::ResignerMismatch,
        Self::DeferredPromotion,
    ];
}

#[derive(Default, Serialize)]
pub(super) struct Excluded {
    pub(super) count: usize,
    ids: Vec<String>,
}

#[derive(Serialize)]
pub(super) struct Report {
    pub(super) accepted: usize,
    pub(super) accepted_ids: Vec<String>,
    pub(super) excluded: BTreeMap<Exclusion, Excluded>,
    pub(super) candidates: usize,
    pub(super) excluded_score: usize,
    pub(super) excluded_no_legal_moves: usize,
    pub(super) retained: usize,
    pub(super) retained_plies: BTreeMap<String, Vec<u32>>,
    pub(super) truncated_illegal: BTreeMap<String, u32>,
    nodes: u32,
    start_origin_note: &'static str,
}

impl Report {
    pub(super) fn new(nodes: u32) -> Self {
        Self {
            accepted: 0,
            accepted_ids: Vec::new(),
            excluded: Exclusion::ALL
                .into_iter()
                .map(|r| (r, Excluded::default()))
                .collect(),
            candidates: 0,
            excluded_score: 0,
            excluded_no_legal_moves: 0,
            retained: 0,
            retained_plies: BTreeMap::new(),
            truncated_illegal: BTreeMap::new(),
            nodes,
            start_origin_note: "Human games begin at the standard initial position; games provenance uses start_origin=random under the existing enum contract.",
        }
    }
    pub(super) fn exclude(&mut self, reason: Exclusion, id: String) {
        let entry = self.excluded.entry(reason).or_default();
        entry.count += 1;
        entry.ids.push(id);
    }
}

pub(super) fn metadata_exclusion(
    game: &InputGame,
    openings: bool,
    seen: &mut HashSet<String>,
) -> Option<Exclusion> {
    use Exclusion::*;
    if game.variant.as_deref() != Some("chushogi") {
        return Some(Variant);
    }
    if game.initial_sfen.is_some() {
        return Some(InitialSfen);
    }
    let players = game
        .players
        .as_ref()
        .and_then(|p| Some([p.sente.as_ref()?, p.gote.as_ref()?]));
    let Some(players) = players else {
        return Some(Bot);
    };
    if players.iter().any(|p| {
        p.ai_level.is_some()
            || p.user
                .as_ref()
                .is_none_or(|u| u.title.as_deref() == Some("BOT"))
    }) {
        return Some(Bot);
    }
    if !openings {
        if game.rated != Some(true) {
            return Some(Unrated);
        }
        if time_control(game).is_none() {
            return Some(MissingSpeed);
        }
        if players
            .iter()
            .any(|p| p.rating.is_none() || p.provisional == Some(true))
        {
            return Some(Rating);
        }
        let status = match game.status.as_deref() {
            Some("resign" | "royalsLost") => None,
            Some("outoftime") => Some(StatusOutoftime),
            Some("timeout") => Some(StatusTimeout),
            Some("aborted") => Some(StatusAborted),
            Some("draw") => Some(StatusDraw),
            Some("repetition") => Some(StatusRepetition),
            Some("perpetualCheck") => Some(StatusPerpetualCheck),
            Some("bareKing") => Some(StatusBareKing),
            Some("created") => Some(StatusCreated),
            Some("started") => Some(StatusStarted),
            Some("mate") => Some(StatusMate),
            Some("stalemate") => Some(StatusStalemate),
            Some("cheat") => Some(StatusCheat),
            Some("noStart") => Some(StatusNoStart),
            Some("unknownFinish") => Some(StatusUnknownFinish),
            _ => Some(UnknownStatus),
        };
        if status.is_some() {
            return status;
        }
        if winner(game).is_none() {
            return Some(Winner);
        }
    }
    if !seen.insert(game.id.clone()) {
        return Some(Duplicate);
    }
    None
}

pub(super) fn winner(game: &InputGame) -> Option<Color> {
    match game.winner.as_deref() {
        Some("sente") => Some(Color::Black),
        Some("gote") => Some(Color::White),
        _ => None,
    }
}
