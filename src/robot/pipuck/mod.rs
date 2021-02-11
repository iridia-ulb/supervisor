use futures::FutureExt;
use uuid;
use std::{pin::Pin, task::{Context, Poll}, future::Future};
use tokio::sync::mpsc;
use crate::network::fernbedienung;

mod task;

pub use task::{
    Action, Error, Receiver, Request, Result, Response, Sender, State
};

pub struct PiPuck {
    pub uuid: uuid::Uuid,
    pub tx: Sender,
    pub task: Pin<Box<dyn Future<Output = Result<()>> + Send>>,
}

impl PiPuck {
    pub fn new(device: fernbedienung::Device) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            uuid: uuid::Uuid::new_v4(), 
            task: task::new(device, rx).boxed(),
            // note that holding tx here is not ideal since it prevents
            // task and hence this active object from completing
            tx
        }
    }
}

impl Future for PiPuck {
    type Output = Result<()>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().task.as_mut().poll(cx)
    }
}
