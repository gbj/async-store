#[cfg(test)]
use crate::Store;

#[derive(Clone)]
struct State {
    a: String,
}

impl State {
    pub fn new() -> Self {
        Self {
            a: String::from("Initial value"),
        }
    }
}

enum Msg {
    ClearStr,
    SetStr(String),
}

fn reducer(state: &mut State, action: Msg) {
    match action {
        Msg::ClearStr => state.a = String::from(""),
        Msg::SetStr(s) => state.a = s,
    }
}

#[test]
fn should_update_state_when_dispatched() {
    let s = Store::new(State::new(), reducer);
    s.dispatch(Msg::ClearStr);
    assert_eq!(s.snapshot(|n| n.a.clone()), "");
    assert_eq!(s.snapshot_cloned().a, "");
    s.dispatch(Msg::SetStr(String::from("Test")));
    assert_eq!(s.snapshot(|n| n.a.clone()), "Test");
    assert_eq!(s.snapshot_cloned().a, "Test");
}

#[tokio::test]
async fn should_emit_to_stream_when_dispatched() {
    use futures::stream::StreamExt;
    let s = Store::new(State::new(), reducer);
    s.dispatch(Msg::ClearStr);
    let mut stream = s.stream(|n| n.a.clone());
    assert_eq!(stream.next().await, Some("".to_string()));
    s.dispatch(Msg::SetStr(String::from("Test")));
    assert_eq!(stream.next().await, Some("Test".to_string()));
}

#[tokio::test]
async fn multiple_stores_should_emit_to_stream() {
    use futures::stream::StreamExt;
    let s = Store::new(State::new(), reducer);
    let s2 = s.clone();
    let mut stream = s.stream(|n| n.a.clone());
    s.dispatch(Msg::ClearStr);
    assert_eq!(stream.next().await, Some("".to_string()));
    s2.dispatch(Msg::SetStr(String::from("second one")));
    assert_eq!(stream.next().await, Some("second one".to_string()));
    s2.dispatch(Msg::ClearStr);
    assert_eq!(stream.next().await, Some("".to_string()));
}

#[tokio::test]
async fn stream_should_close_when_all_senders_dropped() {
    use futures::stream::StreamExt;
    let s = Store::new(State::new(), reducer);
    let s2 = s.clone();
    let mut stream = s.stream(|n| n.a.clone());
    s.dispatch(Msg::ClearStr);
    assert_eq!(stream.next().await, Some("".to_string()));
    drop(s);
    drop(s2);
    assert_eq!(stream.next().await, None);
}
