//! Waiting without polling, and how much of Runo the person can see.
//!
//! - [`Inbox`] takes over a channel's receiving end, so the UI wakes when something
//!   arrives instead of checking the channel on a timer.
//! - [`shown`] says whether the window can be seen, so periodic work can slow down or
//!   stop while Runo is hidden, minimized or covered.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

static SHOWN: AtomicBool = AtomicBool::new(true);

/// Whether any part of the main window is on screen: not hidden, minimized, covered or
/// on a sleeping display.
pub fn shown() -> bool {
    SHOWN.load(Ordering::Relaxed)
}

pub fn set_shown(on: bool) {
    SHOWN.store(on, Ordering::Relaxed);
}

/// `front` while the window is on screen, `hidden` otherwise.
pub fn pace(front: Duration, hidden: Duration) -> Duration {
    if shown() {
        front
    } else {
        hidden
    }
}

struct Shared<T> {
    queue: VecDeque<T>,
    closed: bool,
    waker: Option<Waker>,
}

/// The receiving end of a channel, moved to a thread that blocks on it and hands what
/// arrives to [`Inbox::wait`]. Senders stay plain `mpsc::Sender`s.
pub struct Inbox<T> {
    shared: Arc<Mutex<Shared<T>>>,
}

impl<T: Send + 'static> Inbox<T> {
    pub fn new(rx: Receiver<T>) -> Self {
        let shared = Arc::new(Mutex::new(Shared {
            queue: VecDeque::new(),
            closed: false,
            waker: None,
        }));
        let feed = shared.clone();
        std::thread::Builder::new()
            .name("inbox".into())
            .spawn(move || {
                let ring = |push: Option<T>| {
                    let mut s = feed.lock().unwrap_or_else(|e| e.into_inner());
                    match push {
                        Some(item) => s.queue.push_back(item),
                        None => s.closed = true,
                    }
                    if let Some(w) = s.waker.take() {
                        w.wake();
                    }
                };
                while let Ok(item) = rx.recv() {
                    ring(Some(item));
                }
                ring(None);
            })
            .expect("start the inbox thread");
        Self { shared }
    }
}

impl<T> Inbox<T> {
    /// Resolves once something is waiting, or the senders are all gone.
    pub fn wait(&self) -> Wait<'_, T> {
        Wait { inbox: self }
    }

    /// Everything waiting now; `None` once the senders are gone and nothing is left.
    pub fn take(&self) -> Option<Vec<T>> {
        let mut s = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if s.queue.is_empty() && s.closed {
            return None;
        }
        Some(s.queue.drain(..).collect())
    }
}

pub struct Wait<'a, T> {
    inbox: &'a Inbox<T>,
}

impl<T> Future for Wait<'_, T> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut s = self.inbox.shared.lock().unwrap_or_else(|e| e.into_inner());
        if !s.queue.is_empty() || s.closed {
            return Poll::Ready(());
        }
        s.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Polls `fut` once with a waker that records whether it was woken.
    fn poll_once<F: Future + Unpin>(fut: &mut F, woken: &Arc<AtomicBool>) -> bool {
        struct Flag(Arc<AtomicBool>);
        impl std::task::Wake for Flag {
            fn wake(self: Arc<Self>) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let waker = Waker::from(Arc::new(Flag(woken.clone())));
        let mut cx = Context::from_waker(&waker);
        Pin::new(fut).poll(&mut cx).is_ready()
    }

    #[test]
    fn inbox_wakes_on_send_and_on_close() {
        let (tx, rx) = std::sync::mpsc::channel::<u32>();
        let inbox = Inbox::new(rx);
        let woken = Arc::new(AtomicBool::new(false));
        assert!(!poll_once(&mut inbox.wait(), &woken));
        tx.send(1).unwrap();
        tx.send(2).unwrap();
        for _ in 0..200 {
            if woken.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(woken.load(Ordering::SeqCst));
        while inbox.shared.lock().unwrap().queue.len() < 2 {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(poll_once(&mut inbox.wait(), &woken));
        assert_eq!(inbox.take(), Some(vec![1, 2]));
        assert_eq!(inbox.take(), Some(vec![]));
        drop(tx);
        while !inbox.shared.lock().unwrap().closed {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(poll_once(&mut inbox.wait(), &woken));
        assert_eq!(inbox.take(), None);
    }
}
