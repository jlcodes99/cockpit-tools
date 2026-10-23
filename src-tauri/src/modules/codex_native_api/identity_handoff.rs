//! Per-database handoff fence for work that can outlive its async waiter.
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock, Mutex, Weak,
    },
};

static HANDOFFS: LazyLock<Mutex<HashMap<PathBuf, Weak<AtomicBool>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) struct HandoffGuard(Arc<AtomicBool>);

impl Drop for HandoffGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Callers use the same identity path as `handoff_scope`, without filesystem
/// scans or canonicalization. Move this guard into the actual blocking worker.
pub(crate) fn begin_handoff(path: &Path) -> Result<HandoffGuard, String> {
    let mut registry = HANDOFFS
        .lock()
        .map_err(|_| "API continuity handoff: identity fence is unavailable".to_owned())?;
    registry.retain(|_, work| work.strong_count() != 0);
    let work = match registry.get(path).and_then(Weak::upgrade) {
        Some(work) => {
            work.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .map_err(|_| "API continuity handoff: identity state is busy".to_owned())?;
            work
        }
        None => {
            let work = Arc::new(AtomicBool::new(true));
            registry.insert(path.to_owned(), Arc::downgrade(&work));
            work
        }
    };
    Ok(HandoffGuard(work))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_fences_only_the_matching_identity_path_and_releases_on_drop() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first.sqlite");
        let second = temp.path().join("second.sqlite");
        let one = begin_handoff(&first).unwrap();
        assert!(begin_handoff(&first).is_err());
        let two = begin_handoff(&second).unwrap();
        assert!(begin_handoff(&second).is_err());
        drop(one);
        let again = begin_handoff(&first).unwrap();
        assert!(begin_handoff(&first).is_err());
        drop(again);
        drop(two);
        assert!(begin_handoff(&second).is_ok());
    }
}
