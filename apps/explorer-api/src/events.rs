use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use async_stream::stream;
use axum::response::sse::{Event, KeepAlive, Sse};
use serde::{Serialize, Serializer};
use serde_json::{json, Value};
use tokio::sync::broadcast;

pub(crate) const EVENT_QUEUE_CAPACITY: usize = 256;

#[derive(Debug, Clone)]
pub(crate) struct EventQueue {
    sender: broadcast::Sender<AppEvent>,
    next_id: Arc<AtomicU64>,
}

#[derive(Debug, Clone, Serialize)]
struct AppEvent {
    event_id: u64,
    #[serde(rename = "type")]
    event_type: AppEventType,
    payload: Value,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum AppEventType {
    TransactionSubmitted,
    RollupBatchSequenced,
    RollupBatchSettlementUpdated,
    L1DepositObserved,
}

impl EventQueue {
    pub(crate) fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    pub(crate) fn publish(&self, event_type: AppEventType, payload: Value) {
        let event = AppEvent {
            event_id: self.next_id.fetch_add(1, Ordering::Relaxed),
            event_type,
            payload,
        };

        let _ = self.sender.send(event);
    }

    pub(crate) fn stream(
        &self,
    ) -> Sse<impl futures_core::Stream<Item = Result<Event, std::convert::Infallible>>> {
        let mut receiver = self.sender.subscribe();
        let stream = stream! {
            loop {
                match receiver.recv().await {
                    Ok(notification) => {
                        yield Ok(Event::default()
                            .id(notification.event_id.to_string())
                            .event(notification.event_type.name())
                            .json_data(notification)
                            .unwrap_or_else(|error| {
                                Event::default()
                                    .event("serialization_error")
                                    .data(error.to_string())
                            }));
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        yield Ok(Event::default()
                            .event("event_queue_lagged")
                            .data(json!({ "skipped": skipped }).to_string()));
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        };

        Sse::new(stream).keep_alive(KeepAlive::default())
    }
}

impl AppEventType {
    fn name(self) -> &'static str {
        match self {
            AppEventType::TransactionSubmitted => "transaction_submitted",
            AppEventType::RollupBatchSequenced => "rollup_batch_sequenced",
            AppEventType::RollupBatchSettlementUpdated => "rollup_batch_settlement_updated",
            AppEventType::L1DepositObserved => "l1_deposit_observed",
        }
    }
}

impl Serialize for AppEventType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_envelope_uses_explicit_event_id_field() {
        let event = AppEvent {
            event_id: 7,
            event_type: AppEventType::L1DepositObserved,
            payload: json!({
                "l1_deposit_row_id": 85,
                "deposit_id": 1
            }),
        };

        let value = serde_json::to_value(event).unwrap();

        assert_eq!(value["event_id"], json!(7));
        assert_eq!(value["type"], json!("l1_deposit_observed"));
        assert!(value.get("id").is_none());
        assert_eq!(value["payload"]["l1_deposit_row_id"], json!(85));
        assert_eq!(value["payload"]["deposit_id"], json!(1));
    }
}
