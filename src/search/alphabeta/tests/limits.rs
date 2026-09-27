//! 探索制限とスナップショットを検査する。

use super::*;

// ---------------------------------------------------------------------------
// D7-LIM　SearchLimits
// ---------------------------------------------------------------------------

// D7-LIM-01。search.md「時間管理」節: 深さ制約は1以上、最大探索ply（256）
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

// D7-LIM-02[実装契約]。search.md「時間管理」節の制約列挙からの導出:
// 無期限が明示の制約値として存在するため、全制約の欠落は指定漏れとして
// 拒否する。無期限の明示（D7-LIM-05）とは峻別される。
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
        ClockLimits::new(0, 0, 0, 0),
        Err(SearchError::EmptyClock)
    ));
}

// 監査「探索条件と探索局面の不正状態」: 公開スナップショットはGameが
// 確定した局面、規則、履歴およびルート合法手を一括して複製する。
#[test]
fn snapshot_is_constructed_from_one_game_state() {
    let game = crate::Game::with_default_rules();
    let snapshot = SearchSnapshot::from_game(&game).unwrap();

    assert_eq!(snapshot.position(), game.position());
    assert_eq!(snapshot.rules(), game.rules().moves);
    assert_eq!(snapshot.history_keys(), game.search_key_history());
    assert_eq!(snapshot.root_moves(), game.legal_moves());

    let empty =
        crate::Game::from_position(crate::Rules::ENGINE_DEFAULT, position(Color::Black, &[]));
    assert!(matches!(
        SearchSnapshot::from_game(&empty),
        Err(SearchError::NoLegalMoves)
    ));

    let mut finished = crate::Game::with_default_rules();
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
    let pst = crate::eval::weights().unwrap();
    let result = crate::search::search(
        &pst,
        &snapshot,
        &SearchLimits::infinite(),
        DEFAULT_THREADS,
        &mut small_tt(),
    );
    assert!(matches!(
        result,
        Err(SearchError::InfiniteSynchronousSearch)
    ));
}
