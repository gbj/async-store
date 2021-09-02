# async-store
A Redux-inspired Rust state container with an async Stream API.

## Example

```rust
use async_store::Stream;
use futures::stream::StreamExt;

// Define state, action, and reducer
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

// Create the actual store
let store = Store::new(State::new(), reducer);

// Derive a Stream from the store. Conceptually this is similar to a Redux slice; you'd
// typically use it to isolate a single field in your state.
// The function here is given &State, so you may need to clone or copy the field.
let mut stream = store.stream(|state| state.a.clone());

store.dispatch(Msg::SetStr(String::from("string 2")));
store.dispatch(Msg::ClearStr);

// Poll the stream
// It will always give the latest value of the state, without intervening values
assert_eq!(stream.next().await, Some(String::from("")));
```

## Advantages
- Simple, predictable state changes
- Async interface via `Stream`s
- State does not need to be `Clone`; the store itself owns it.

## Caveat
There's no change detection on individual fields of a state struct. If any action has been dispatched, every streamm will think it has a new value when it is polled, which means you should limit expensive operations in your `.stream()` selectors.

## Prior art
The notion of a state container that combines a `State` type, an `Action` enum, and a `reducer` function is, of course, inspired by the model promoted by Elm, Redux, and company.

The design of the `Store` type is inspired by [`redux-rs`](https://github.com/redux-rs/redux-rs) and the async implementation of the `Store` type is heavily influenced by the excellent [`futures_signals` crate](https://github.com/Pauan/rust-signals).