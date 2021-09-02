#![feature(async_stream)]

use futures::stream::Stream;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::task::{Context, Poll, Waker};

/// A container that holds a state. The state is not manipulated directly, but updated through
/// actions that are dispatched, then processed by a reducer function.
///
/// The store can generate any number of [Stream](futures::stream::Stream)s, which emit the new state
/// any time an action is dispatched.
///
/// The type of a store is determined by the type of the state it holds, an the type of the actions
/// that can be dispatched (usually an enum).
pub struct Store<T, A> {
    inner: Arc<RwLock<StoreState<T>>>,
    reducer: Arc<dyn Fn(&mut T, A) + Send + Sync>,
    middleware: Arc<Mutex<Vec<Middleware<T, A>>>>,
}

type Middleware<T, A> = Box<dyn Fn(&Store<T, A>, A) -> Option<A>>;

impl<T, A> Store<T, A> {
    /// Creates a new store from an initial value and a reducer function.
    /// ```
    /// # use async_store::Store;
    /// type State = i32;
    /// enum Action {
    ///    Increment,
    ///    Decrement
    /// }
    /// fn reducer(state: &mut State, action: Action) {
    ///    match action {
    ///        Action::Increment => *state += 1,
    ///        Action::Decrement => *state -= 1,
    ///    }
    /// }
    /// let store = Store::new(0, reducer);
    /// ```
    pub fn new(value: T, reducer: impl Fn(&mut T, A) + Send + Sync + 'static) -> Self {
        Self {
            inner: Arc::new(RwLock::new(StoreState {
                value,
                senders: 1,
                streams: Vec::new(),
            })),
            reducer: Arc::new(reducer),
            middleware: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Adds custom middleware to the store, which will intercept and can transform any action dispatched.
    /// ```
    /// # use async_store::Store;
    /// type State = i32;
    /// enum Action {
    ///    Add(i32),
    ///    Subtract(i32)
    /// }
    /// fn reducer(state: &mut State, action: Action) {
    ///    match action {
    ///        Action::Add(n) => *state += n,
    ///        Action::Subtract(n) => *state -= n,
    ///    }
    /// }
    /// let store = Store::new(0, reducer);
    /// fn double_middleware(state: &Store<State, Action>, action: Action) -> Option<Action> {
    ///     match action {
    ///         Action::Add(n) => Some(Action::Add(n*2)),
    ///         Action::Subtract(n) => Some(Action::Subtract(n*2))
    ///     }
    /// }
    /// store.add_middleware(double_middleware);
    /// store.dispatch(Action::Add(10));
    /// assert_eq!(store.snapshot_cloned(), 20);
    /// ```
    pub fn add_middleware(&self, middleware: impl Fn(&Store<T, A>, A) -> Option<A> + 'static) {
        self.middleware.lock().unwrap().push(Box::new(middleware));
    }

    /// Dispatches an event, mutates the contained state by running it through the reducer,
    /// and updates any streams with the latest state.
    /// ```
    /// # use async_store::Store;
    /// # type State = i32;
    /// # enum Action {
    /// #    Increment,
    /// #    Decrement
    /// # }
    /// # fn reducer(state: &mut State, action: Action) {
    /// #   match action {
    /// #        Action::Increment => *state += 1,
    /// #        Action::Decrement => *state -= 1,
    /// #   }
    /// # }
    /// let store = Store::new(0, reducer);
    /// store.dispatch(Action::Increment);
    /// assert_eq!(store.snapshot_cloned(), 1);
    /// ```
    pub fn dispatch(&self, action: A) {
        if self.middleware.lock().unwrap().is_empty() {
            self.dispatch_reducer(action);
        } else {
            self.dispatch_middleware(0, action);
        }
    }

    // Runs the reucer
    pub fn dispatch_reducer(&self, action: A) {
        let mut inner = self.inner.write().unwrap();
        let value = &mut inner.value;
        (self.reducer)(value, action);
        inner.notify(true);
    }

    // Runs one middleware
    fn dispatch_middleware(&self, index: usize, action: A) {
        if index == self.middleware.lock().unwrap().len() {
            self.dispatch_reducer(action);
            return;
        }

        let next = self.middleware.lock().unwrap()[index](self, action);

        if next.is_none() {
            return;
        }

        self.dispatch_middleware(index + 1, next.unwrap());
    }

    /// Returns a [Stream](futures::stream::Stream), which, whenever polled, yields the callback
    /// function applied to the current state. This can be used to select a member of a complex state struct,
    /// or to clone values to be used elsewhere.
    /// ```
    /// # use async_store::Store;
    /// # use futures::stream::StreamExt;
    /// struct State {
    ///     x: i32,
    ///     y: i32,
    /// };
    /// enum Action {
    ///     SetX(i32),
    ///     SetY(i32)
    /// }
    /// # fn reducer(state: &mut State, action: Action) {
    /// #   match action {
    /// #        Action::SetX(n) => state.x = n,
    /// #        Action::SetY(n) => state.y = n,
    /// #   }
    /// # }
    /// # tokio_test::block_on(async {
    /// let store = Store::new(State { x: 0, y: 0 }, reducer);
    /// let mut x_stream = store.stream(|state| state.x);
    /// let mut y_stream = store.stream(|state| state.y);
    /// store.dispatch(Action::SetX(10));
    /// store.dispatch(Action::SetY(-10));
    /// assert_eq!(x_stream.next().await, Some(10));
    /// assert_eq!(y_stream.next().await, Some(-10));
    /// # });
    /// ```
    pub fn stream<U, F>(&self, f: F) -> StoreStream<T, U, F>
    where
        F: Fn(&T) -> U,
    {
        let waker = Arc::new(ChangedWaker::new());
        self.inner.write().unwrap().push_stream(&waker);
        StoreStream {
            state: self.inner.clone(),
            transform: f,
            waker,
        }
    }
}

impl<T, A> Store<T, A> {
    /// Calls the given function with the current value of the store's state and returns the value.
    ///
    /// ```
    /// # use async_store::Store;
    /// # struct State {
    /// #     x: i32,
    /// #     y: i32,
    /// # };
    /// # enum Action {
    /// #    SetX(i32),
    /// #    SetY(i32)
    /// # }
    /// # fn reducer(state: &mut State, action: Action) {
    /// #   match action {
    /// #        Action::SetX(n) => state.x = n,
    /// #        Action::SetY(n) => state.y = n,
    /// #   }
    /// # }
    /// let store = Store::new(State { x: 0, y: 0 }, reducer);
    /// store.dispatch(Action::SetX(10));
    /// store.dispatch(Action::SetY(-10));
    /// assert_eq!(store.snapshot(|state| state.x), 10);
    /// assert_eq!(store.snapshot(|state| state.y), -10);
    /// ```
    pub fn snapshot<F, U>(&self, f: F) -> U
    where
        F: Fn(&T) -> U,
    {
        f(&self.inner.read().unwrap().value)
    }
}

impl<T, A> Store<T, A>
where
    T: Clone,
{
    /// Returns the current value of the contained state by cloning it.
    ///
    /// ```
    /// # use async_store::Store;
    /// #[derive(Clone, PartialEq, Debug)]
    /// struct State {
    ///     x: i32,
    ///     y: i32,
    /// };
    /// enum Action {
    ///     SetX(i32),
    ///     SetY(i32)
    /// }
    /// fn reducer(state: &mut State, action: Action) {
    ///     match action {
    ///         Action::SetX(n) => state.x = n,
    ///         Action::SetY(n) => state.y = n,
    ///     }
    /// }
    /// let store = Store::new(State { x: 0, y: 0 }, reducer);
    /// store.dispatch(Action::SetX(10));
    /// store.dispatch(Action::SetY(-10));
    /// assert_eq!(store.snapshot_cloned(), State { x: 10, y: -10 });
    /// ```
    pub fn snapshot_cloned(&self) -> T {
        self.inner.read().unwrap().value.clone()
    }
}

impl<T, A> Clone for Store<T, A> {
    fn clone(&self) -> Self {
        self.inner.write().unwrap().senders += 1;
        Store {
            inner: self.inner.clone(),
            reducer: self.reducer.clone(),
            middleware: self.middleware.clone(),
        }
    }
}

impl<T, A> Drop for Store<T, A> {
    fn drop(&mut self) {
        let mut state = self.inner.write().unwrap();

        state.senders -= 1;

        if state.senders == 0 && !state.streams.is_empty() {
            state.notify(false);
            state.streams = vec![];
        }
    }
}

struct StoreState<T> {
    value: T,
    senders: usize,
    streams: Vec<Weak<ChangedWaker>>,
}

impl<T> StoreState<T> {
    fn push_stream(&mut self, state: &Arc<ChangedWaker>) {
        if self.senders != 0 {
            self.streams.push(Arc::downgrade(state));
        }
    }

    fn notify(&mut self, has_changed: bool) {
        self.streams.retain(|stream| {
            if let Some(stream) = stream.upgrade() {
                stream.wake(has_changed);
                true
            } else {
                false
            }
        });
    }
}

pub struct StoreStream<T, B, F>
where
    F: Fn(&T) -> B,
{
    waker: Arc<ChangedWaker>,
    state: Arc<RwLock<StoreState<T>>>,
    transform: F,
}

impl<T, B, F> Stream for StoreStream<T, B, F>
where
    F: Fn(&T) -> B,
{
    type Item = B;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let lock = self.state.read().unwrap();

        if self.waker.is_changed() {
            Poll::Ready(Some((self.transform)(&lock.value)))
        } else if lock.senders == 0 {
            Poll::Ready(None)
        } else {
            self.waker.set_waker(cx);
            Poll::Pending
        }
    }
}

#[derive(Debug)]
pub(crate) struct ChangedWaker {
    changed: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

impl ChangedWaker {
    pub(crate) fn new() -> Self {
        Self {
            changed: AtomicBool::new(true),
            waker: Mutex::new(None),
        }
    }

    pub(crate) fn wake(&self, changed: bool) {
        let waker = {
            let mut lock = self.waker.lock().unwrap();

            if changed {
                self.changed.store(true, Ordering::SeqCst);
            }

            lock.take()
        };

        if let Some(waker) = waker {
            waker.wake();
        }
    }

    pub(crate) fn set_waker(&self, cx: &Context) {
        *self.waker.lock().unwrap() = Some(cx.waker().clone());
    }

    pub(crate) fn is_changed(&self) -> bool {
        self.changed.swap(false, Ordering::SeqCst)
    }
}

//mod tests;
