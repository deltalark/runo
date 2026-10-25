//! Answers about the disk that drawing asks for every frame, such as whether a project's
//! folder is still there, remembered for a few seconds so a frame doesn't wait on the
//! disk, or start `git`, to draw the same answer again.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long an answer is kept.
const KEEP: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Ask {
    Dir,
    Repo,
}

thread_local! {
    static ANSWERS: RefCell<HashMap<(Ask, String), (bool, Instant)>> = RefCell::default();
}

fn answer(ask: Ask, path: &str, work: impl FnOnce() -> bool) -> bool {
    let key = (ask, path.to_string());
    if let Some(yes) = ANSWERS.with(|a| {
        a.borrow()
            .get(&key)
            .filter(|(_, at)| at.elapsed() < KEEP)
            .map(|(yes, _)| *yes)
    }) {
        return yes;
    }
    let yes = work();
    ANSWERS.with(|a| {
        let mut a = a.borrow_mut();
        a.retain(|_, (_, at)| at.elapsed() < KEEP);
        a.insert(key, (yes, Instant::now()));
    });
    yes
}

/// Whether `path` is a folder.
pub(crate) fn is_dir(path: &str) -> bool {
    answer(Ask::Dir, path, || std::path::Path::new(path).is_dir())
}

/// Whether `path` is in a git repository.
pub(crate) fn is_repo(path: &str) -> bool {
    answer(Ask::Repo, path, || crate::git::is_repo(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn answers_are_kept_for_a_while() {
        let dir = std::env::temp_dir().join(format!("runo-checks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.to_str().unwrap();
        assert!(is_dir(path));
        std::fs::remove_dir(&dir).unwrap();
        // Still the answer from a moment ago.
        assert!(is_dir(path));
        ANSWERS.with(|a| a.borrow_mut().clear());
        assert!(!is_dir(path));
    }
}
