use futures::FutureExt;
use serde::{Deserialize, Serialize};
use uuid;
use log;
use std::{pin::Pin, task::{Context, Poll}, future::Future};
use tokio::sync::{mpsc, oneshot};
use crate::network::{fernbedienung, xbee};

mod task;

pub use task::{Error, Result, Request, Action};

pub struct Drone {
    pub uuid: uuid::Uuid,
    pub tx: mpsc::UnboundedSender<Request>,
    pub task: Pin<Box<dyn Future<Output = Result<()>> + Send>>,
}

impl Drone {
    pub fn new(xbee: xbee::Device) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            uuid: uuid::Uuid::new_v4(), 
            task: task::new(xbee, rx).boxed(),
            tx
        }
    }
}

impl Future for Drone {
    type Output = Result<()>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().task.as_mut().poll(cx)
    }
}
