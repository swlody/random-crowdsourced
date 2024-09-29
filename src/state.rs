use std::sync::Arc;

#[allow(clippy::module_name_repetitions)]
#[derive(Clone, Debug)]
pub struct AppState {
    pub redis: Arc<redis::Client>,
}
