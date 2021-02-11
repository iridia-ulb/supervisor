use futures::FutureExt;
use uuid;
use std::{pin::Pin, task::{Context, Poll}, future::Future};
use tokio::sync::mpsc;
use crate::network::xbee;

mod task;

pub use task::{
    Action, Error, Receiver, Request, Result, Response, Sender, State
};

pub struct Drone {
    pub uuid: uuid::Uuid,
    pub tx: Sender,
    pub task: Pin<Box<dyn Future<Output = Result<()>> + Send>>,
}

impl Drone {
    pub fn new(xbee: xbee::Device) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            uuid: uuid::Uuid::new_v4(), 
            task: task::new(xbee, rx).boxed(),
            // note that holding tx here is not ideal since it prevents
            // task and hence this active object from completing
            tx
            // put tx and uuid in a HashMap inside of arena?
            // future should also return the UUID so that the hashmap entry can be removed
        }
    }
}

// ideally the output here would be the use IP addresses so that we can return them
// to the network module
impl Future for Drone {
    type Output = Result<()>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().task.as_mut().poll(cx)
    }
}
