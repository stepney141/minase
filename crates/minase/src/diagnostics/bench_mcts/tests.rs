use super::*;
use std::error::Error as _;

#[test]
fn output_failure_keeps_the_io_error_and_stops() {
    let arguments = Arguments::new(NonZeroU32::MIN, NonZeroUsize::MIN).unwrap();
    let mut calls = 0;
    let error = run(arguments, &mut |_| {
        calls += 1;
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed output"))
    })
    .unwrap_err();
    assert_eq!(calls, 1);
    assert!(matches!(&error, Error::Output(source) if source.kind() == io::ErrorKind::BrokenPipe));
    assert_eq!(
        error
            .source()
            .unwrap()
            .downcast_ref::<io::Error>()
            .unwrap()
            .kind(),
        io::ErrorKind::BrokenPipe
    );
}

#[test]
fn search_failure_retains_the_original_typed_error_in_io_errors() {
    for failure in [
        SearchJoinError::Inference(crate::nn::NnError::MissingOutput { name: "policy" }),
        SearchJoinError::Mcts(mcts::Error::Tree(mcts::tree::TreeError::InvalidState)),
    ] {
        let error = io::Error::other(Error::from(failure));
        let error = error.get_ref().unwrap().downcast_ref::<Error>().unwrap();
        let source = error.source().unwrap();
        match error {
            Error::Inference(_) => assert!(matches!(
                source.downcast_ref::<crate::nn::NnError>(),
                Some(crate::nn::NnError::MissingOutput { name: "policy" })
            )),
            Error::Search(_) => assert_eq!(
                source.downcast_ref::<mcts::Error>(),
                Some(&mcts::Error::Tree(mcts::tree::TreeError::InvalidState))
            ),
            _ => panic!("the search failure must retain its variant"),
        }
    }
}

#[test]
fn search_panic_resumes_with_the_original_payload() {
    let payload =
        std::panic::catch_unwind(|| Error::from(SearchJoinError::Panic(Box::new(42_u64))))
            .unwrap_err();
    assert_eq!(*payload.downcast::<u64>().unwrap(), 42);
}
