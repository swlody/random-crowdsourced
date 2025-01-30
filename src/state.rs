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

// naughty numbers
pub static BANNED_NUMBERS: OnceLock<HashSet<String>> = OnceLock::new();

pub type CallbackMap = BTreeMap<Uuid, oneshot::Sender<String>>;

#[derive(Clone, Debug)]
pub struct AppState {
    pub redis: Arc<deadpool_redis::Pool>,

    // Allow submitted numbers to be sent to tasks that are awaiting a number by mapping the uuid
    // of the origin request to a response channel
    pub callback_map: Arc<Mutex<CallbackMap>>,

    // State updates sent to all open websocket connections
    pub state_updates: Arc<broadcast::Sender<String>>,
}
