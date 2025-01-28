use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, oneshot};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum StateUpdate {
    Added(Uuid),
    Removed(Uuid),
}

pub type CallbackMap = BTreeMap<Uuid, oneshot::Sender<String>>;

#[derive(Clone, Debug)]
pub struct AppState {
    pub redis: redis::aio::MultiplexedConnection,
    pub callback_map: Arc<Mutex<CallbackMap>>,
    pub state_updates: Arc<broadcast::Sender<StateUpdate>>,
}
