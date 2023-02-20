use std::{collections::HashMap, time::Duration};

use aws_sdk_dynamodb::{model::AttributeValue, Client};
use futures::{channel::mpsc, SinkExt, StreamExt};
use tokio::time::sleep;

use crate::{Config, HITS, KEY};

pub fn hit_tracker<'a>(
    client: &'a Client,
    config: &'a Config,
) -> (HitTrackerSender, HitTrackerReceiver<'a>) {
    let (tx, rx) = mpsc::channel(128);
    (
        HitTrackerSender { tx },
        HitTrackerReceiver { rx, client, config },
    )
}

#[derive(Clone)]
pub struct HitTrackerSender {
    tx: mpsc::Sender<String>,
}

impl HitTrackerSender {
    pub async fn track(&mut self, key: String) {
        let _ = self.tx.send(key).await;
    }

    pub fn close(mut self) {
        self.tx.disconnect();
    }
}

pub struct HitTrackerReceiver<'a> {
    rx: mpsc::Receiver<String>,
    client: &'a Client,
    config: &'a Config,
}

impl<'a> HitTrackerReceiver<'a> {
    pub async fn run(&mut self) {
        // Wait until a hit has been made.
        while let Some(initial) = self.rx.next().await {
            for (key, hits) in [initial]
                .into_iter()
                .chain(
                    // Listen for hits for up to 5 seconds, then just immediately
                    // send the results.
                    // This is to try and prevent an update for every single hit,
                    // and instead batch same-key hits together, while making sure
                    // we don't lose too many hits if the lambda is killed.
                    self.rx
                        .by_ref()
                        .take_until(sleep(Duration::from_secs(5)))
                        .collect::<Vec<_>>()
                        .await,
                )
                .fold(HashMap::new(), |mut acc, key| {
                    *acc.entry(key).or_insert(0) += 1;
                    acc
                })
            {
                // Don't do this in parallel to avoid smashing DynamoDB.
                let res = self
                    .client
                    .update_item()
                    .table_name(&self.config.table_name)
                    .key(KEY, AttributeValue::S(key))
                    .update_expression("SET #hits = #hits + :hits")
                    .expression_attribute_names("#hits", HITS)
                    .expression_attribute_values(":hits", AttributeValue::N(hits.to_string()))
                    .send()
                    .await;

                if let Err(e) = res {
                    tracing::error!("Error updating hit count: {:?}", e);
                }
            }
        }
    }
}
