//! 探索制限とスナップショットを検査する。

use super::*;

// 深さ制約は1以上、最大探索ply（256）
// 以下だけを受理し、範囲外は`SearchLimits`の検証で拒否する。
#[test]
fn depth_limit_accepts_the_1_to_256_range_only() {
    assert!(matches!(
        SearchLimits::new(Some(0), None, None, None),
        Err(SearchError::InvalidDepth { depth: 0 })
    ));
    assert!(SearchLimits::new(Some(1), None, None, None).is_ok());
    assert!(SearchLimits::new(Some(256), None, None, None).is_ok());
    assert!(matches!(
        SearchLimits::new(Some(257), None, None, None),
        Err(SearchError::InvalidDepth { depth: 257 })
    ));
}

// 制約を1つも指定しない探索は拒否し、無期限の探索は明示的な指定だけを受理する。
#[test]
fn limits_without_any_constraint_are_rejected_but_explicit_infinite_is_accepted() {
    assert!(matches!(
        SearchLimits::new(None, None, None, None),
        Err(SearchError::MissingLimit)
    ));
    assert!(infinite_limits().is_infinite());
}

// 監査「探索条件と探索局面の不正状態」: 0ノード、0ms、および全要素が
// 0msの時計は、探索開始前の検証済みコンストラクタで拒否する。
#[test]
fn zero_search_budgets_are_rejected_at_construction() {
    assert!(matches!(
        SearchLimits::new(None, Some(0), None, None),
        Err(SearchError::ZeroNodeLimit)
    ));
    assert!(matches!(
        SearchLimits::new(None, None, Some(0), None),
        Err(SearchError::ZeroMoveTime)
    ));
    assert!(matches!(
        ClockLimits::new(0, 0, 0, None, 0),
        Err(SearchError::EmptyClock)
    ));
    assert!(matches!(
        ClockLimits::new(0, 0, 0, Some(1), 0),
        Err(SearchError::EmptyClock)
    ));
}

// 設計書T5の入力契約: 秒読み0かつmovetimeなしの時計だけが相手の残り時間を必須とする。
#[test]
fn time_advantage_clock_requires_opponent_remaining_time() {
    let without_opponent = ClockLimits::new(1_000, 0, 0, None, 0).unwrap();
    for (depth, nodes) in [(None, None), (Some(2), None), (None, Some(100))] {
        assert!(matches!(
            SearchLimits::new(depth, nodes, None, Some(without_opponent)),
            Err(SearchError::MissingOpponentClock)
        ));
    }

    let opponent_at_zero = ClockLimits::new(1_000, 0, 0, Some(0), 0).unwrap();
    assert_eq!(opponent_at_zero.opponent_remaining_ms(), Some(0));
    assert!(SearchLimits::new(None, None, None, Some(opponent_at_zero)).is_ok());

    let byoyomi = ClockLimits::new(1_000, 0, 100, None, 0).unwrap();
    assert!(SearchLimits::new(None, None, None, Some(byoyomi)).is_ok());
    assert!(SearchLimits::new(None, None, Some(20), Some(without_opponent)).is_ok());
}

// 監査「探索条件と探索局面の不正状態」: 公開スナップショットはGameが
// 確定した局面、規則、履歴およびルート合法手を一括して複製する。
#[test]
fn snapshot_is_constructed_from_one_game_state() {
    let game = minase_core::Game::with_default_rules();
    let snapshot = SearchSnapshot::from_game(&game).unwrap();

    assert_eq!(snapshot.position(), game.position());
    assert_eq!(snapshot.rules(), game.rules().moves);
    assert_eq!(snapshot.history_keys(), game.repetition_key_history());
    assert_eq!(snapshot.root_moves(), game.legal_moves());

    let empty = minase_core::Game::from_position(
        minase_core::Rules::ENGINE_DEFAULT,
        position(Color::Black, &[]),
    );
    assert!(matches!(
        SearchSnapshot::from_game(&empty),
        Err(SearchError::NoLegalMoves)
    ));

    let mut finished = minase_core::Game::with_default_rules();
    finished.agree_draw().unwrap();
    assert!(matches!(
        SearchSnapshot::from_game(&finished),
        Err(SearchError::FinishedGame)
    ));
}

// 監査「start_search/searchの公開パニック」: 外部停止手段のない同期APIは
// 無期限指定をpanicせず型付きエラーとして返す。
#[test]
fn synchronous_search_rejects_infinite_limits_without_panicking() {
    let snapshot = snapshot_for(&Position::initial());
    let nnue = test_nnue();
    let result = crate::search::search(
        nnue,
        &snapshot,
        &SearchLimits::infinite(),
        DEFAULT_THREADS,
        &mut small_tt(),
        &mut crate::search::HistoryTables::new(DEFAULT_THREADS),
    );
    assert!(matches!(
        result,
        Err(SearchError::InfiniteSynchronousSearch)
    ));
}
