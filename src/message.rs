use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
pub enum StateUpdate {
    Added(Uuid),
    Removed(Uuid),
}
