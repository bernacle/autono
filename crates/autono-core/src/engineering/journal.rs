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
    #[cfg(test)]
    publication_gate: Arc<Mutex<Option<Arc<PublicationGate>>>>,
}
impl Journal {
    pub fn new(store: Store, run: EngineeringRun) -> Self {
        Self {
            store,
            state: Arc::new(Mutex::new(run)),
            #[cfg(test)]
            publication_gate: Default::default(),
        }
    }
    pub async fn read(&self) -> EngineeringRun {
        self.state.lock().await.clone()
    }
    /// Synchronize an owned journal with confirmed SQLite state, without replaying
    /// agents or adopting processes. Used only under the resolution mutation gate.
    pub async fn reconcile(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        let durable = retry(|| self.store.engineering_get(state.id)).await?;
        anyhow::ensure!(
            durable.revision >= state.revision,
            "durable journal revision moved backwards"
        );
        *state = durable;
        Ok(())
    }
    #[cfg(test)]
    pub async fn pause_resolution_publication(&self) -> Arc<PublicationGate> {
        let gate = Arc::new(PublicationGate::default());
        *self.publication_gate.lock().await = Some(gate.clone());
        gate
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
        next.handoff = Some(next.partial_completion());
        next.revision += 1;
        next.updated_at = Utc::now();
        retry(|| self.store.engineering_save(&next, kind, detail)).await?;
        #[cfg(test)]
        if kind == "human_resolution"
            && let Some(gate) = self.publication_gate.lock().await.take()
        {
            gate.committed.notify_one();
            gate.publish.notified().await;
        }
        *state = next;
        Ok(())
    }
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct PublicationGate {
    pub committed: tokio::sync::Notify,
    pub publish: tokio::sync::Notify,
}
