use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, Mutex, OnceLock},
};

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, oneshot};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum StateUpdate {
    Added(Uuid),
    Removed(Uuid),
}

pub static BANNED_NUMBERS: OnceLock<HashSet<String>> = OnceLock::new();

pub type CallbackMap = BTreeMap<Uuid, oneshot::Sender<String>>;

#[derive(Clone, Debug)]
pub struct AppState {
    pub redis: redis::aio::MultiplexedConnection,
    pub callback_map: Arc<Mutex<CallbackMap>>,
    pub state_updates: Arc<broadcast::Sender<StateUpdate>>,
}
