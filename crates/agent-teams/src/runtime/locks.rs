//! Process-wide keyed locks.
//!
//! Every captain in this process shares them: two sessions in the same
//! workspace must not interleave a read-modify-write of the same team file.
//! Not reentrant — an operation never takes the same key twice. Different
//! processes are not serialized (the reference makes the same promise).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;

#[derive(Default)]
pub struct KeyedLocks {
    slots: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl KeyedLocks {
    /// Run `f` while holding `key`.
    pub fn with<T>(&self, key: &str, f: impl FnOnce() -> T) -> T {
        let slot = {
            let mut slots = self.slots.lock();
            slots
                .entry(key.to_owned())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = slot.lock();
        f()
    }
}

/// The locks every runtime in this process uses.
pub fn global_locks() -> &'static KeyedLocks {
    static LOCKS: OnceLock<KeyedLocks> = OnceLock::new();
    LOCKS.get_or_init(KeyedLocks::default)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn one_key_serializes_and_other_keys_do_not_block() {
        let locks = Arc::new(KeyedLocks::default());
        let inside = Arc::new(AtomicUsize::new(0));
        let overlap = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let locks = locks.clone();
                let inside = inside.clone();
                let overlap = overlap.clone();
                std::thread::spawn(move || {
                    locks.with("a", || {
                        if inside.fetch_add(1, Ordering::SeqCst) > 0 {
                            overlap.fetch_add(1, Ordering::SeqCst);
                        }
                        std::thread::sleep(std::time::Duration::from_millis(2));
                        inside.fetch_sub(1, Ordering::SeqCst);
                    });
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(overlap.load(Ordering::SeqCst), 0);
        locks.with("a", || locks.with("b", || ()));
    }
}
