use minase::search::{SearchSnapshot, mcts::{Driver, DrawValue, Step, puct::PuctConfig, tree::SelectionRule}};
use minase_core::{Color, PieceKind, Game, Rules, Move};
use minase_core::test_util::{sq, position};

fn run_case(name: &str, pieces: &[(minase_core::Square, Color, PieceKind)], mv: Move, expected: f32) {
    let p = position(Color::Black, pieces);
    let mut game = Game::from_position(Rules::ENGINE_DEFAULT, p);
    assert!(game.legal_moves().contains(&mv));
    let mut driver = Driver::new(SearchSnapshot::from_game(&game).unwrap(), 1024*1024, DrawValue::SelfPlay).unwrap();
    let cfg = PuctConfig::default();
    let Step::Evaluate(req) = driver.select_leaf(SelectionRule::Puct(&cfg)).unwrap() else {panic!("root")};
    let logits: Vec<_> = req.query().moves.iter().map(|m| if *m == mv {0.0} else {-100.0}).collect();
    driver.evaluate(&req, &logits, 0.5).unwrap();
    let status = game.play(mv).unwrap();
    assert!(matches!(status, minase_core::GameStatus::Finished(_)), "case is not terminal: {status:?}");
    println!("{name}: real game: {status:?}");
    match driver.select_leaf(SelectionRule::Puct(&cfg)).unwrap() {
        Step::Evaluate(req) => {
            println!("MCTS requests NN evaluation with {} moves", req.query().moves.len());
            let logits = vec![0.0; req.query().moves.len()];
            driver.evaluate(&req, &logits, 0.9).unwrap();
            let edge = driver.tree().root().edges.iter().find(|e| e.mv == mv).unwrap();
            println!("backed up Q={:?}; root state={:?}; expected terminal Q={expected}", edge.q(), driver.tree().root().state);
        },
        _ => panic!("Terminal was detected, hypothesis disproven"),
    }
}
fn main() {
    run_case("E0 win", &[
        (sq(0,0), Color::Black, PieceKind::King),
        (sq(11,11), Color::White, PieceKind::King),
        (sq(4,4), Color::Black, PieceKind::Rook),
        (sq(4,5), Color::White, PieceKind::Pawn),
    ], Move {from:sq(4,4), mid:None, to:sq(4,5), promote:false}, 1.0);
    run_case("E0 draw", &[
        (sq(0,0), Color::Black, PieceKind::King),
        (sq(11,11), Color::White, PieceKind::King),
        (sq(0,1), Color::White, PieceKind::Pawn),
    ], Move {from:sq(0,0), mid:None, to:sq(0,1), promote:false}, 0.5);
    run_case("Checkmate", &[
        (sq(2,2), Color::Black, PieceKind::King),
        (sq(0,0), Color::White, PieceKind::King),
        (sq(1,2), Color::Black, PieceKind::FreeKing),
        (sq(11,5), Color::White, PieceKind::Pawn),
    ], Move {from:sq(1,2), mid:None, to:sq(1,1), promote:false}, 1.0);
}
