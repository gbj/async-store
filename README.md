# async-store
A Redux-inspired Rust state container with an async Stream API.

## Prior art
The notion of a state container that combines a `State` type, an `Action` enum, and a `reducer` function is, of course, inspired by the model promoted by Elm, Redux, and company.

The design of the `Store` type is inspired by [`redux-rs`](https://github.com/redux-rs/redux-rs) and the async implementation of the `Store` type is heavily influenced by the excellent [`futures_signals` crate](https://github.com/Pauan/rust-signals).

## Todo
- [ ] Middleware
- [ ] Logging middleware
- [ ] Examples
  - [ ] Simple binary
  - [ ] Web
  - [ ] LibUI
- [ ] Docs
  - [ ] Doc comments
  - [ ] Readme
- [ ] Ask for input on API
