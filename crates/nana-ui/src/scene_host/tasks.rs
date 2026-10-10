//! Program task workers behind `RuntimeProgramContext::run_task`.
//!
//! A fixed pool of threads drains one bounded queue. Each task runs under
//! `catch_unwind`: a task that panics loses only its own message, and its
//! worker goes back to the queue. Without that, every panic retired a worker
//! for good; once the last one was gone the queue's receiver dropped and every
//! later `run_task` failed with `HostStopped`.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{Sender, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};

use nana_ui_runtime::Task;

use super::schedule;
use super::{TASK_QUEUE_CAPACITY, TASK_WORKERS};

pub(super) fn spawn_task_workers<Message: Send + 'static>(
    message_tx: Sender<Message>,
    wake: Arc<schedule::HostWorkWake>,
) -> SyncSender<Task<Message>> {
    spawn_task_pool(TASK_WORKERS, move |message| {
        if message_tx.send(message).is_err() {
            return false;
        }
        wake.wake();
        true
    })
}

/// Start `workers` threads that run queued tasks and hand each result to
/// `deliver`. A worker stops when the queue closes or `deliver` returns
/// `false` (the host is gone); a panicking task only records
/// `host.task_panicked` and is never delivered.
fn spawn_task_pool<Message: Send + 'static>(
    workers: usize,
    deliver: impl Fn(Message) -> bool + Clone + Send + 'static,
) -> SyncSender<Task<Message>> {
    let (sender, receiver) = sync_channel::<Task<Message>>(TASK_QUEUE_CAPACITY);
    let receiver = Arc::new(Mutex::new(receiver));
    for _ in 0..workers {
        let receiver = Arc::clone(&receiver);
        let deliver = deliver.clone();
        std::thread::spawn(move || {
            loop {
                let task = {
                    let Ok(receiver) = receiver.lock() else {
                        return;
                    };
                    let Ok(task) = receiver.recv() else {
                        return;
                    };
                    task
                };
                // The future is dropped while unwinding, so nothing it owned
                // is observed again; the worker only reuses the queue.
                match catch_unwind(AssertUnwindSafe(|| pollster::block_on(task.into_future()))) {
                    Ok(message) => {
                        if !deliver(message) {
                            return;
                        }
                    }
                    Err(payload) => {
                        let reason = panic_reason(payload.as_ref());
                        nana_diagnostics::fault!(
                            nana_diagnostics::framework::host::TASK_PANICKED;
                            "program task panicked and its message was dropped: {reason}"
                        );
                    }
                }
            }
        });
    }
    sender
}

fn panic_reason(payload: &(dyn Any + Send)) -> &str {
    if let Some(text) = payload.downcast_ref::<&str>() {
        return text;
    }
    if let Some(text) = payload.downcast_ref::<String>() {
        return text;
    }
    "non-string panic payload"
}

#[cfg(test)]
mod tests {
    use super::spawn_task_pool;
    use nana_ui_runtime::Task;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn panicking_tasks_do_not_retire_workers() {
        let (results, received) = mpsc::channel();
        let tasks = spawn_task_pool(2, move |value: u32| results.send(value).is_ok());
        for _ in 0..3 {
            tasks
                .send(Task::new(async { panic!("task failed on purpose") }))
                .expect("queue accepts the panicking task");
        }
        tasks.send(Task::ready(7)).expect("queue stays open");
        tasks.send(Task::ready(8)).expect("queue stays open");

        let mut delivered = [
            received
                .recv_timeout(Duration::from_secs(10))
                .expect("a worker is still alive"),
            received
                .recv_timeout(Duration::from_secs(10))
                .expect("a worker is still alive"),
        ];
        delivered.sort_unstable();
        assert_eq!(delivered, [7, 8]);
        assert!(
            received.recv_timeout(Duration::from_millis(100)).is_err(),
            "panicked tasks deliver nothing"
        );
    }
}
