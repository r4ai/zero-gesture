use std::cell::UnsafeCell;
use std::ops::Deref;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use super::RuntimeConfig;

pub(crate) const MAX_GENERATION: u64 = u64::MAX >> 1;

struct Publication<T> {
    slots: [UnsafeCell<Option<T>>; 2],
    readers: [AtomicUsize; 2],
    state: AtomicU64,
}

// The sole ConfigOwner writes only the inactive slot after observing its
// reader count at zero. Readers form a reference only after incrementing that
// slot's count and rechecking the combined generation/index state. Because
// generations never wrap, a delayed reader cannot mistake a reused index for
// the generation it first observed.
unsafe impl<T: Send + Sync> Sync for Publication<T> {}

impl<T> Publication<T> {
    fn new(active: Option<T>, generation: u64) -> Self {
        debug_assert!(generation <= MAX_GENERATION);
        Self {
            slots: [UnsafeCell::new(active), UnsafeCell::new(None)],
            readers: [AtomicUsize::new(0), AtomicUsize::new(0)],
            state: AtomicU64::new(generation << 1),
        }
    }

    fn reserve(&self, value: T) -> Result<usize, T> {
        let inactive = (self.state.load(Ordering::Acquire) as usize & 1) ^ 1;
        if self.readers[inactive].load(Ordering::SeqCst) != 0 {
            return Err(value);
        }
        // SAFETY: ConfigOwner is the sole writer. A validated reader can only
        // reference the active slot. A delayed reader for this inactive slot
        // must fail its generation/index recheck before forming a reference.
        unsafe {
            *self.slots[inactive].get() = Some(value);
        }
        Ok(inactive)
    }

    fn abort(&self, slot: usize) {
        debug_assert_eq!(slot, (self.state.load(Ordering::Acquire) as usize & 1) ^ 1);
        debug_assert_eq!(self.readers[slot].load(Ordering::SeqCst), 0);
        // SAFETY: the prepared slot is inactive and unobservable to readers.
        unsafe {
            *self.slots[slot].get() = None;
        }
    }

    fn publish(&self, slot: usize, generation: u64) {
        debug_assert!(generation <= MAX_GENERATION);
        debug_assert_eq!(slot, (self.state.load(Ordering::Acquire) as usize & 1) ^ 1);
        // The candidate was installed during Prepare. Publication is one
        // release store and therefore cannot fail after durable replacement.
        self.state
            .store((generation << 1) | slot as u64, Ordering::Release);
    }

    fn read(&self) -> Option<Guard<'_, T>> {
        loop {
            let observed = self.state.load(Ordering::Acquire);
            let slot = observed as usize & 1;
            self.readers[slot].fetch_add(1, Ordering::SeqCst);
            if self.state.load(Ordering::Acquire) == observed {
                // SAFETY: a matching state recheck makes this the active slot.
                // Its reader count remains non-zero for the guard lifetime.
                let present = unsafe { (&*self.slots[slot].get()).is_some() };
                if present {
                    return Some(Guard {
                        publication: self,
                        slot,
                        generation: observed >> 1,
                    });
                }
            }
            self.readers[slot].fetch_sub(1, Ordering::SeqCst);
            if observed >> 1 == 0 {
                return None;
            }
        }
    }
}

struct Guard<'a, T> {
    publication: &'a Publication<T>,
    slot: usize,
    generation: u64,
}

impl<T> Guard<'_, T> {
    fn generation(&self) -> u64 {
        self.generation
    }
}

impl<T> Deref for Guard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: read() validated this slot and the guard keeps its reader
        // count non-zero, so the sole writer cannot reuse the slot.
        unsafe {
            (&*self.publication.slots[self.slot].get())
                .as_ref()
                .expect("an active publication slot must be populated")
        }
    }
}

impl<T> Drop for Guard<'_, T> {
    fn drop(&mut self) {
        self.publication.readers[self.slot].fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Clone)]
pub(crate) struct ConfigSnapshotReader(Arc<Publication<Arc<RuntimeConfig>>>);

pub(crate) struct ConfigSnapshotGuard<'a>(Guard<'a, Arc<RuntimeConfig>>);

impl ConfigSnapshotReader {
    pub(crate) fn read(&self) -> Option<ConfigSnapshotGuard<'_>> {
        self.0.read().map(ConfigSnapshotGuard)
    }
}

impl ConfigSnapshotGuard<'_> {
    pub(crate) fn generation(&self) -> u64 {
        self.0.generation()
    }
}

impl Deref for ConfigSnapshotGuard<'_> {
    type Target = RuntimeConfig;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub(super) struct ConfigPublication {
    inner: Arc<Publication<Arc<RuntimeConfig>>>,
}

impl ConfigPublication {
    pub(super) fn new(active: Option<Arc<RuntimeConfig>>, generation: u64) -> Self {
        Self {
            inner: Arc::new(Publication::new(active, generation)),
        }
    }

    pub(super) fn reader(&self) -> ConfigSnapshotReader {
        ConfigSnapshotReader(Arc::clone(&self.inner))
    }

    pub(super) fn reserve(&self, value: Arc<RuntimeConfig>) -> Result<usize, ()> {
        self.inner.reserve(value).map_err(|_| ())
    }

    pub(super) fn abort(&self, slot: usize) {
        self.inner.abort(slot);
    }

    pub(super) fn publish(&self, slot: usize, generation: u64) {
        self.inner.publish(slot, generation);
    }
}

#[cfg(test)]
mod tests {
    use super::{Publication, MAX_GENERATION};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn two_slot_publication_preserves_guarded_generation() {
        let publication = Publication::new(Some(11_u64), 1);
        let old = publication.read().unwrap();
        let slot = publication.reserve(22).unwrap();
        publication.publish(slot, 2);

        assert_eq!(*old, 11);
        assert!(publication.reserve(33).is_err());
        drop(old);
        let slot = publication.reserve(33).unwrap();
        publication.publish(slot, 3);
        let current = publication.read().unwrap();
        assert_eq!((*current, current.generation()), (33, 3));
    }

    #[test]
    fn concurrent_readers_never_observe_torn_generation_or_value() {
        let publication = Arc::new(Publication::new(Some(1_u64), 1));
        let stop = Arc::new(AtomicBool::new(false));
        let reader_publication = Arc::clone(&publication);
        let reader_stop = Arc::clone(&stop);
        let reader = thread::spawn(move || {
            while !reader_stop.load(Ordering::Acquire) {
                let guard = reader_publication.read().unwrap();
                assert_eq!(*guard, guard.generation());
            }
        });

        for generation in 2..=2_000 {
            loop {
                if let Ok(slot) = publication.reserve(generation) {
                    publication.publish(slot, generation);
                    break;
                }
                thread::yield_now();
            }
        }
        stop.store(true, Ordering::Release);
        reader.join().unwrap();
    }

    #[test]
    fn generation_limit_has_no_wrap_value() {
        let maximum = std::hint::black_box(MAX_GENERATION);
        assert_eq!(maximum.checked_add(1), Some(1_u64 << 63));
        assert_eq!((maximum << 1) | 1, u64::MAX);
    }
}
