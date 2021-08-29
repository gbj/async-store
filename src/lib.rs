#![feature(async_stream)]

use std::pin::Pin;
use std::stream::Stream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::task::{Context, Poll, Waker};

pub struct Store<T, A> {
    inner: Arc<RwLock<StoreState<T>>>,
    reducer: Arc<dyn Fn(&mut T, A) + Send + Sync>,
}

impl<T, A> Store<T, A> {
    pub fn new(value: T, reducer: impl Fn(&mut T, A) + Send + Sync + 'static) -> Self {
        Self {
            inner: Arc::new(RwLock::new(StoreState {
                value,
                senders: 1,
                streams: Vec::new(),
            })),
            reducer: Arc::new(reducer),
        }
    }

    pub fn dispatch(&self, action: A) {
        let mut inner = self.inner.write().unwrap();
        let value = &mut inner.value;
        (self.reducer)(value, action);
        inner.notify(true);
    }

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
