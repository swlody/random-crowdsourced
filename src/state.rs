use std::sync::{atomic::AtomicUsize, Arc};

use async_channel::{Receiver, Sender};

#[allow(clippy::module_name_repetitions)]
#[derive(Clone, Debug)]
pub struct AppState {
    pub tx: Sender<i64>,
    pub rx: Receiver<i64>,
    pub waiting_calls: Arc<AtomicUsize>,
}
