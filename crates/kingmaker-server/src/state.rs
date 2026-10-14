use kingmaker_core::WorldState;
use kingmaker_sim::ProductionRecipe;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AppState {
    pub world: Arc<RwLock<WorldState>>,
    pub recipes: Arc<HashMap<String, ProductionRecipe>>,
}
