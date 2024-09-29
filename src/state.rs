use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use uuid::Uuid;

pub type CallbackMap = BTreeMap<Uuid, tokio::sync::oneshot::Sender<String>>;

#[derive(Clone, Debug)]
pub struct AppState {
    pub redis: Arc<redis::Client>,
    pub callback_map: Arc<Mutex<CallbackMap>>,
}
