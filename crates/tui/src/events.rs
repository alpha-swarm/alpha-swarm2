use async_nats::Client;
use futures::StreamExt;
use swarm_events::SwarmEvent;
use tokio::sync::mpsc::Sender;

pub async fn subscribe_nats(url: String, tx: Sender<SwarmEvent>) {
    let client = match async_nats::connect(&url).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("NATS connect failed: {e}");
            return;
        }
    };

    let mut sub = match client.subscribe("alpha-swarm.>").await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("NATS subscribe failed: {e}");
            return;
        }
    };

    while let Some(msg) = sub.next().await {
        if let Ok(event) = serde_json::from_slice::<SwarmEvent>(&msg.payload) {
            let _ = tx.send(event).await;
        }
    }
}
