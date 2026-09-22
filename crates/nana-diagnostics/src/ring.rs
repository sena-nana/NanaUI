//! Bounded single-producer / single-consumer ring.
//!
//! The producer never waits: a full ring rejects the new item and bumps
//! `dropped`. Dropping the newest item (rather than overwriting the oldest)
//! keeps the producer from ever touching a slot the consumer may be reading.

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Keeps the producer and consumer indices on separate cache lines.
#[repr(align(128))]
struct Padded<T>(T);

pub(crate) struct Ring<T> {
    slots: Box<[UnsafeCell<MaybeUninit<T>>]>,
    mask: usize,
    /// Next slot the consumer reads. Written only by the consumer.
    head: Padded<AtomicUsize>,
    /// Next slot the producer writes. Written only by the producer.
    tail: Padded<AtomicUsize>,
    dropped: AtomicU64,
}

// SAFETY: slots are handed between exactly one producer and one consumer
// through the Release/Acquire pairs on `head`/`tail`; `T: Send` is all a
// cross-thread move needs.
unsafe impl<T: Send> Send for Ring<T> {}
unsafe impl<T: Send> Sync for Ring<T> {}

impl<T> Ring<T> {
    /// Capacity rounds up to a power of two (minimum 2).
    pub(crate) fn new(capacity: usize) -> Self {
        let capacity = capacity.max(2).next_power_of_two();
        let slots = (0..capacity)
            .map(|_| UnsafeCell::new(MaybeUninit::uninit()))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            slots,
            mask: capacity - 1,
            head: Padded(AtomicUsize::new(0)),
            tail: Padded(AtomicUsize::new(0)),
            dropped: AtomicU64::new(0),
        }
    }

    #[cfg(test)]
    pub(crate) fn capacity(&self) -> usize {
        self.mask + 1
    }

    /// # Safety
    /// Only one thread may call `push` on a given ring at a time.
    #[inline]
    pub(crate) unsafe fn push(&self, value: T) -> bool {
        let tail = self.tail.0.load(Ordering::Relaxed);
        let head = self.head.0.load(Ordering::Acquire);
        if tail.wrapping_sub(head) > self.mask {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        // SAFETY: the slot at `tail` is outside [head, tail), so the consumer
        // is not reading it, and the caller guarantees we are the only writer.
        unsafe { (*self.slots[tail & self.mask].get()).write(value) };
        self.tail.0.store(tail.wrapping_add(1), Ordering::Release);
        true
    }

    /// # Safety
    /// Only one thread may call `pop` on a given ring at a time.
    #[inline]
    pub(crate) unsafe fn pop(&self) -> Option<T> {
        let head = self.head.0.load(Ordering::Relaxed);
        let tail = self.tail.0.load(Ordering::Acquire);
        if head == tail {
            return None;
        }
        // SAFETY: the Acquire load of `tail` published the producer's write to
        // this slot, and the caller guarantees we are the only reader.
        let value = unsafe { (*self.slots[head & self.mask].get()).assume_init_read() };
        self.head.0.store(head.wrapping_add(1), Ordering::Release);
        Some(value)
    }

    /// Consumer-side view: nothing left to pop.
    pub(crate) fn is_empty(&self) -> bool {
        self.head.0.load(Ordering::Acquire) == self.tail.0.load(Ordering::Acquire)
    }

    pub(crate) fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl<T> Drop for Ring<T> {
    fn drop(&mut self) {
        // `&mut self`: no producer or consumer is left.
        while unsafe { self.pop() }.is_some() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn full_ring_drops_newest_and_counts_it() {
        let ring = Ring::new(4);
        for value in 0..6 {
            unsafe { ring.push(value) };
        }
        assert_eq!(ring.dropped(), 2);
        let drained: Vec<_> = std::iter::from_fn(|| unsafe { ring.pop() }).collect();
        assert_eq!(drained, [0, 1, 2, 3]);
    }

    #[test]
    fn capacity_rounds_up_to_power_of_two() {
        assert_eq!(Ring::<u8>::new(5).capacity(), 8);
        assert_eq!(Ring::<u8>::new(0).capacity(), 2);
    }

    #[test]
    fn drop_releases_unread_items() {
        let marker = Arc::new(());
        let ring = Ring::new(4);
        unsafe {
            ring.push(marker.clone());
            ring.push(marker.clone());
        }
        drop(ring);
        assert_eq!(Arc::strong_count(&marker), 1);
    }

    #[test]
    fn concurrent_producer_and_consumer_preserve_order() {
        const N: u64 = 200_000;
        let ring = Arc::new(Ring::new(64));
        let producer = {
            let ring = ring.clone();
            std::thread::spawn(move || {
                let mut sent = 0;
                for value in 0..N {
                    if unsafe { ring.push(value) } {
                        sent += 1;
                    }
                }
                sent
            })
        };
        let mut last = None;
        let mut received = 0u64;
        let mut take = |value: u64| {
            if let Some(last) = last {
                assert!(value > last, "order broke: {value} after {last}");
            }
            last = Some(value);
            received += 1;
        };
        while !producer.is_finished() {
            match unsafe { ring.pop() } {
                Some(value) => take(value),
                None => std::hint::spin_loop(),
            }
        }
        let sent = producer.join().unwrap();
        while let Some(value) = unsafe { ring.pop() } {
            take(value);
        }
        assert_eq!(sent, received);
        assert_eq!(sent + ring.dropped(), N);
    }
}
