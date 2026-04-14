use std::collections::HashMap;

use tokio::sync::{RwLock, broadcast};
use uuid::Uuid;

pub struct LiveLogHub {
    channels: RwLock<HashMap<Uuid, broadcast::Sender<String>>>,
}

impl LiveLogHub {
    pub fn new() -> Self {
        Self {
            channels: RwLock::new(HashMap::new()),
        }
    }

    pub async fn subscribe(&self, task_id: Uuid) -> broadcast::Receiver<String> {
        let mut channels = self.channels.write().await;
        channels
            .entry(task_id)
            .or_insert_with(|| {
                let (sender, _) = broadcast::channel(512);
                sender
            })
            .subscribe()
    }

    pub async fn publish(&self, task_id: Uuid, chunk: String) {
        let mut channels = self.channels.write().await;
        let sender = channels.entry(task_id).or_insert_with(|| {
            let (sender, _) = broadcast::channel(512);
            sender
        });

        let _ = sender.send(chunk);
    }
}
