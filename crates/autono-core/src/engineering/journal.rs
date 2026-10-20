use super::EngineeringRun;
use crate::{Store, persistence::retry};
use anyhow::Result;
use chrono::Utc;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Serializes checkpoints, never process cancellation. Publish memory only after commit.
#[derive(Clone)]
pub(super) struct Journal {
    pub store: Store,
    state: Arc<Mutex<EngineeringRun>>,
}
impl Journal {
    pub fn new(store: Store, run: EngineeringRun) -> Self {
        Self {
            store,
            state: Arc::new(Mutex::new(run)),
        }
    }
    pub async fn read(&self) -> EngineeringRun {
        self.state.lock().await.clone()
    }
    pub async fn change(
        &self,
        kind: &str,
        detail: &str,
        change: impl FnOnce(&mut EngineeringRun) -> Result<()>,
    ) -> Result<()> {
        let mut state = self.state.lock().await;
        let mut next = state.clone();
        change(&mut next)?;
        next.revision += 1;
        next.updated_at = Utc::now();
        retry(|| self.store.engineering_save(&next, kind, detail)).await?;
        *state = next;
        Ok(())
    }
}
