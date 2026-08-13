use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use iced::futures::{Stream, stream};
use nana_ui_runtime::{Subscription, Task};

type RuntimeSubscription<T> = Pin<Box<dyn Stream<Item = T> + Send + 'static>>;
type SharedSubscription<T> = Arc<Mutex<Option<RuntimeSubscription<T>>>>;

pub fn run_task<T: Send + 'static>(task: Task<T>) -> iced::Task<T> {
    iced::Task::perform(task.into_future(), |output| output)
}

pub fn run_subscription<T: Send + 'static>(subscription: Subscription<T>) -> iced::Subscription<T> {
    let id = subscription.id().to_owned();
    iced::Subscription::run_with(
        RuntimeStream {
            id,
            stream: Arc::new(Mutex::new(Some(subscription.into_stream()))),
            marker: PhantomData,
        },
        take_stream::<T>,
    )
}

struct RuntimeStream<T> {
    id: String,
    stream: SharedSubscription<T>,
    marker: PhantomData<fn() -> T>,
}

impl<T> Hash for RuntimeStream<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

fn take_stream<T: Send + 'static>(runtime: &RuntimeStream<T>) -> RuntimeSubscription<T> {
    runtime
        .stream
        .lock()
        .expect("runtime subscription lock")
        .take()
        .unwrap_or_else(|| Box::pin(stream::empty()))
}
