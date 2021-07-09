use futures::FutureExt;
use uuid::Uuid;
use std::{future::Future, pin::Pin, task::{Context, Poll}};
use tokio::{sync::mpsc, task::JoinHandle};
use crate::network::xbee;

mod task;
mod codec;

pub use task::{
    Action, Error, Receiver, Request, Sender, State
};

pub struct Drone(JoinHandle<Uuid>);

impl Drone {
    pub fn new(device: xbee::Device) -> (Uuid, Sender, Self) {
        let uuid = Uuid::new_v4();
        let (tx, rx) = mpsc::channel(32);
        let handle = tokio::spawn(task::new(uuid, rx, device));
        (uuid, tx, Self(handle))
    }
}

impl Future for Drone {
    type Output = Result<Uuid, tokio::task::JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().0.poll_unpin(cx)
    }
}
