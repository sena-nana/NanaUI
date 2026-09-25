//! Host-owned persistence lane. Only the worker calls the physical backend.
use nana_ui_core::{KvBackend, SharedStore, StoreError};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Debug, Default, Clone, Copy)]
pub struct PersistenceWork {
    pub generation: u64,
    pub completed_generation: u64,
    pub writes_started: u64,
    pub writes_completed: u64,
    pub writes_coalesced: u64,
    pub failures: u64,
}
#[derive(Debug)]
struct State {
    entries: BTreeMap<String, String>,
    work: PersistenceWork,
    deadline: Option<Instant>,
    first_dirty: Option<Instant>,
    force: bool,
    stop: bool,
    error: Option<StoreError>,
}
#[derive(Debug)]
struct Lane {
    state: Mutex<State>,
    wake: Condvar,
}
/// One coordinator per physical backend, shared by all windows and domains.
/// Construct before entering the event loop. Dropping requests a final write
/// and waits at most two seconds; a blocked IO call is never joined on the UI.
#[derive(Debug)]
pub struct PersistenceCoordinator {
    owner: Arc<Owner>,
}
#[derive(Debug)]
struct Owner {
    lane: Arc<Lane>,
    identity: usize,
    hosts: std::sync::atomic::AtomicUsize,
    stopping: AtomicBool,
    finished: AtomicBool,
}
fn registry() -> &'static Mutex<HashMap<usize, Arc<Owner>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<usize, Arc<Owner>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}
impl PersistenceCoordinator {
    pub fn new(backend: SharedStore) -> Result<Self, StoreError> {
        let identity = Arc::as_ptr(&backend) as *const () as usize;
        let mut known = registry().lock().map_err(|_| StoreError::poisoned())?;
        if let Some(owner) = known.get(&identity).cloned() {
            if owner.stopping.load(Ordering::Acquire) {
                if owner.finished.load(Ordering::Acquire) {
                    known.remove(&identity);
                } else {
                    return Err(StoreError::new("persistence backend is stopping"));
                }
            }
            if !owner.stopping.load(Ordering::Acquire) {
                owner.hosts.fetch_add(1, Ordering::AcqRel);
                return Ok(Self { owner });
            }
            if !owner.finished.load(Ordering::Acquire) {
                return Err(StoreError::new("persistence backend is stopping"));
            }
        }
        let mut entries = BTreeMap::new();
        for key in backend.keys()? {
            if let Some(value) = backend.get(&key)? {
                entries.insert(key, value);
            }
        }
        let lane = Arc::new(Lane {
            state: Mutex::new(State {
                entries,
                work: PersistenceWork::default(),
                deadline: None,
                first_dirty: None,
                force: false,
                stop: false,
                error: None,
            }),
            wake: Condvar::new(),
        });
        let owner = Arc::new(Owner {
            lane,
            identity,
            hosts: std::sync::atomic::AtomicUsize::new(1),
            stopping: AtomicBool::new(false),
            finished: AtomicBool::new(false),
        });
        let worker = owner.clone();
        std::thread::Builder::new()
            .name("nana-persistence".into())
            .spawn(move || run(worker, backend))
            .map_err(|e| StoreError::new(e.to_string()))?;
        known.insert(identity, owner.clone());
        Ok(Self { owner })
    }
    pub fn work(&self) -> PersistenceWork {
        self.owner
            .lane
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .work
    }
    pub fn flush_timeout(&self, timeout: Duration) -> Result<(), StoreError> {
        let until = Instant::now() + timeout;
        let mut state = self
            .owner
            .lane
            .state
            .lock()
            .map_err(|_| StoreError::poisoned())?;
        let target = state.work.generation;
        if state.work.completed_generation >= target {
            return Ok(());
        }
        state.force = true;
        state.error = None;
        self.owner.lane.wake.notify_all();
        loop {
            if state.work.completed_generation >= target {
                return Ok(());
            }
            if let Some(error) = state.error.clone() {
                return Err(error);
            }
            let remaining = until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(StoreError::new("persistence flush timed out"));
            }
            state = self
                .owner
                .lane
                .wake
                .wait_timeout(state, remaining)
                .map_err(|_| StoreError::poisoned())?
                .0;
        }
    }
    fn mutate(
        &self,
        change: impl FnOnce(&mut BTreeMap<String, String>) -> bool,
    ) -> Result<(), StoreError> {
        let mut state = self
            .owner
            .lane
            .state
            .lock()
            .map_err(|_| StoreError::poisoned())?;
        if !change(&mut state.entries) {
            return Ok(());
        }
        if state.work.generation > state.work.completed_generation {
            state.work.writes_coalesced += 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::persistence::WRITES_COALESCED);
        }
        state.work.generation += 1;
        nana_diagnostics::metric!(
            nana_diagnostics::framework::persistence::PENDING_GENERATION,
            state.work.generation
        );
        let now = Instant::now();
        let first = *state.first_dirty.get_or_insert(now);
        state.deadline =
            Some((now + Duration::from_millis(250)).min(first + Duration::from_secs(2)));
        self.owner.lane.wake.notify_all();
        Ok(())
    }
}
impl KvBackend for PersistenceCoordinator {
    fn get(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .owner
            .lane
            .state
            .lock()
            .map_err(|_| StoreError::poisoned())?
            .entries
            .get(key)
            .cloned())
    }
    fn keys(&self) -> Result<Vec<String>, StoreError> {
        Ok(self
            .owner
            .lane
            .state
            .lock()
            .map_err(|_| StoreError::poisoned())?
            .entries
            .keys()
            .cloned()
            .collect())
    }
    fn set(&self, key: &str, value: String) -> Result<(), StoreError> {
        self.mutate(|entries| {
            if entries.get(key) == Some(&value) {
                false
            } else {
                entries.insert(key.into(), value);
                true
            }
        })
    }
    fn remove(&self, key: &str) -> Result<(), StoreError> {
        self.mutate(|entries| entries.remove(key).is_some())
    }
    fn clear(&self) -> Result<(), StoreError> {
        self.mutate(|entries| {
            let changed = !entries.is_empty();
            entries.clear();
            changed
        })
    }
    fn flush(&self) -> Result<(), StoreError> {
        self.flush_timeout(Duration::from_secs(2))
    }
}
impl Drop for PersistenceCoordinator {
    fn drop(&mut self) {
        if self.owner.hosts.fetch_sub(1, Ordering::AcqRel) != 1 {
            return;
        }
        self.owner.stopping.store(true, Ordering::Release);
        {
            let mut state = self
                .owner
                .lane
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            state.stop = true;
            state.force = true;
        }
        self.owner.lane.wake.notify_all();
        let _ = self.flush_timeout(Duration::from_secs(2));
        if self.owner.finished.load(Ordering::Acquire) {
            let mut known = registry().lock().unwrap_or_else(|e| e.into_inner());
            if known
                .get(&self.owner.identity)
                .is_some_and(|registered| Arc::ptr_eq(registered, &self.owner))
            {
                known.remove(&self.owner.identity);
            }
        }
    }
}
fn run(owner: Arc<Owner>, backend: SharedStore) {
    let lane = owner.lane.clone();
    loop {
        let (snapshot, generation, stopping) = {
            let mut state = lane.state.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if state.work.generation == state.work.completed_generation {
                    if state.stop {
                        // The owner remains registered until this point so a
                        // new host cannot race the worker's final exit.
                        finish_owner(&owner);
                        return;
                    }
                    state = lane.wake.wait(state).unwrap_or_else(|e| e.into_inner());
                    continue;
                }
                if !state.force && !state.stop {
                    let wait = state
                        .deadline
                        .unwrap_or_else(Instant::now)
                        .saturating_duration_since(Instant::now());
                    if !wait.is_zero() {
                        state = lane
                            .wake
                            .wait_timeout(state, wait)
                            .unwrap_or_else(|e| e.into_inner())
                            .0;
                        continue;
                    }
                }
                break;
            }
            state.force = false;
            state.first_dirty = None;
            state.deadline = None;
            state.work.writes_started += 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::persistence::WRITES_STARTED);
            (state.entries.clone(), state.work.generation, state.stop)
        };
        let started = Instant::now();
        let bytes = snapshot
            .iter()
            .map(|(key, value)| key.len() as u64 + value.len() as u64)
            .sum::<u64>();
        let result = (|| {
            for key in backend.keys()? {
                if !snapshot.contains_key(&key) {
                    backend.remove(&key)?;
                }
            }
            for (key, value) in &snapshot {
                backend.set(key, value.clone())?;
            }
            backend.flush()
        })();
        let mut state = lane.state.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            Ok(()) => {
                state.work.completed_generation = generation;
                state.work.writes_completed += 1;
                nana_diagnostics::metric!(
                    nana_diagnostics::framework::persistence::WRITES_COMPLETED
                );
                nana_diagnostics::metric!(
                    nana_diagnostics::framework::persistence::BYTES_WRITTEN,
                    bytes
                );
                nana_diagnostics::metric!(
                    nana_diagnostics::framework::persistence::FLUSH_NS,
                    started.elapsed()
                );
                state.error = None;
            }
            Err(error) => {
                state.work.failures += 1;
                state.error = Some(error);
                nana_diagnostics::event!(nana_diagnostics::framework::persistence::FAILURE);
                state.deadline = Some(Instant::now() + Duration::from_secs(2));
            }
        }
        lane.wake.notify_all();
        if stopping && state.error.is_some() {
            finish_owner(&owner);
            return;
        }
    }
}

fn finish_owner(owner: &Arc<Owner>) {
    owner.finished.store(true, Ordering::Release);
    let mut known = registry().lock().unwrap_or_else(|e| e.into_inner());
    if known
        .get(&owner.identity)
        .is_some_and(|registered| Arc::ptr_eq(registered, owner))
    {
        known.remove(&owner.identity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui_core::{memory_store, shared_store};
    #[test]
    fn bursts_coalesce_and_idle_does_no_work() {
        for count in [100, 1000] {
            let backend = memory_store();
            let lane = PersistenceCoordinator::new(backend.clone()).unwrap();
            for i in 0..count {
                lane.set("window", i.to_string()).unwrap();
            }
            lane.flush_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(
                backend.get("window").unwrap(),
                Some((count - 1).to_string())
            );
            assert_eq!(lane.work().writes_started, 1);
            assert_eq!(lane.work().writes_completed, 1);
            let settled = lane.work();
            lane.set("window", (count - 1).to_string()).unwrap();
            lane.flush_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(lane.work().writes_started, settled.writes_started);
        }
    }
    #[test]
    fn dropping_flushes_latest_generation() {
        let backend = memory_store();
        {
            let lane = PersistenceCoordinator::new(backend.clone()).unwrap();
            lane.set("last", "saved".into()).unwrap();
        }
        assert_eq!(backend.get("last").unwrap().as_deref(), Some("saved"));
    }
    #[test]
    fn hosts_sharing_a_backend_share_one_snapshot_and_worker() {
        let backend = memory_store();
        let first = PersistenceCoordinator::new(backend.clone()).unwrap();
        let second = PersistenceCoordinator::new(backend.clone()).unwrap();
        first.set("profile/a", "one".into()).unwrap();
        second.set("profile/b", "two".into()).unwrap();
        assert_eq!(first.work().generation, 2);
        drop(first);
        second.flush_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(backend.get("profile/a").unwrap().as_deref(), Some("one"));
        assert_eq!(backend.get("profile/b").unwrap().as_deref(), Some("two"));
        assert_eq!(second.work().writes_started, 1);
    }
    #[test]
    fn stopping_backend_cannot_be_taken_over_during_final_flush() {
        let backend = memory_store();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let shared = shared_store(Blocked {
            store: backend,
            entered: entered_tx,
            release: Mutex::new(release_rx),
        });
        let lane = PersistenceCoordinator::new(shared.clone()).unwrap();
        lane.set("key", "value".into()).unwrap();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            drop(lane);
            done_tx.send(()).unwrap();
        });
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(PersistenceCoordinator::new(shared.clone()).is_err());
        release_tx.send(()).unwrap();
        done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(PersistenceCoordinator::new(shared).is_ok());
    }
    #[derive(Debug)]
    struct Failing {
        store: SharedStore,
        fail: Arc<AtomicBool>,
    }
    impl KvBackend for Failing {
        fn get(&self, key: &str) -> Result<Option<String>, StoreError> {
            self.store.get(key)
        }
        fn set(&self, key: &str, value: String) -> Result<(), StoreError> {
            self.store.set(key, value)
        }
        fn remove(&self, key: &str) -> Result<(), StoreError> {
            self.store.remove(key)
        }
        fn clear(&self) -> Result<(), StoreError> {
            self.store.clear()
        }
        fn keys(&self) -> Result<Vec<String>, StoreError> {
            self.store.keys()
        }
        fn flush(&self) -> Result<(), StoreError> {
            if self.fail.load(Ordering::Acquire) {
                Err(StoreError::new("injected persistence failure"))
            } else {
                self.store.flush()
            }
        }
    }
    #[test]
    fn failed_write_keeps_memory_and_retries_after_recovery() {
        let backend = memory_store();
        let fail = Arc::new(AtomicBool::new(true));
        let shared = shared_store(Failing {
            store: backend.clone(),
            fail: fail.clone(),
        });
        let lane = PersistenceCoordinator::new(shared).unwrap();
        lane.set("latest", "memory-value".into()).unwrap();
        assert!(lane.flush_timeout(Duration::from_secs(2)).is_err());
        assert_eq!(lane.get("latest").unwrap().as_deref(), Some("memory-value"));
        assert_eq!(lane.work().failures, 1);
        fail.store(false, Ordering::Release);
        lane.flush_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            backend.get("latest").unwrap().as_deref(),
            Some("memory-value")
        );
        assert_eq!(lane.work().completed_generation, 1);
    }
    #[derive(Debug)]
    struct Blocked {
        store: SharedStore,
        entered: std::sync::mpsc::Sender<()>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl KvBackend for Blocked {
        fn get(&self, k: &str) -> Result<Option<String>, StoreError> {
            self.store.get(k)
        }
        fn set(&self, k: &str, v: String) -> Result<(), StoreError> {
            self.store.set(k, v)
        }
        fn remove(&self, k: &str) -> Result<(), StoreError> {
            self.store.remove(k)
        }
        fn clear(&self) -> Result<(), StoreError> {
            self.store.clear()
        }
        fn keys(&self) -> Result<Vec<String>, StoreError> {
            self.store.keys()
        }
        fn flush(&self) -> Result<(), StoreError> {
            self.entered.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
            Ok(())
        }
    }
    #[test]
    fn in_flight_write_cannot_acknowledge_newer_state() {
        let backend = memory_store();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let lane = PersistenceCoordinator::new(shared_store(Blocked {
            store: backend.clone(),
            entered: entered_tx,
            release: Mutex::new(release_rx),
        }))
        .unwrap();
        lane.set("key", "old".into()).unwrap();
        assert!(lane.flush_timeout(Duration::from_millis(10)).is_err());
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        lane.set("key", "new".into()).unwrap();
        assert_eq!(lane.get("key").unwrap().as_deref(), Some("new"));
        release_tx.send(()).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(lane.work().completed_generation, 1);
        release_tx.send(()).unwrap();
        lane.flush_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(backend.get("key").unwrap().as_deref(), Some("new"));
        assert_eq!(lane.work().completed_generation, 2);
    }
}
