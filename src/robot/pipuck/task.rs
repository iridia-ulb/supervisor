use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::net::Ipv4Addr;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::{StreamExt, wrappers::UnboundedReceiverStream};
use crate::network::fernbedienung;

pub struct State {
    pub linux: Ipv4Addr,
    pub actions: Vec<Action>,
}

#[derive(Copy, Clone)]
pub enum Request {
    GetState,
    Execute(Action)
}

pub enum Response {
    State(State),
    ToBeRemoved
}

pub type Sender = mpsc::UnboundedSender<(Request, Option<oneshot::Sender<Response>>)>;
pub type Receiver = mpsc::UnboundedReceiver<(Request, Option<oneshot::Sender<Response>>)>;

#[derive(Copy, Clone, Serialize, Deserialize, Debug, PartialEq)]
pub enum Action {
    #[serde(rename = "Shutdown RPi")]
    RpiShutdown,
    #[serde(rename = "Reboot RPi")]
    RpiReboot,
    #[serde(rename = "Identify")]
    Identify,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    FernbedienungError(#[from] fernbedienung::Error),

    #[error(transparent)]
    JoinError(#[from] tokio::task::JoinError),
}

// pub type Result<T> = std::result::Result<T, Error>;

pub async fn new(uuid: Uuid, rx: Receiver, device: fernbedienung::Device) -> (Uuid, Ipv4Addr) {
    let mut requests = UnboundedReceiverStream::new(rx);

    while let Some((request, callback)) = requests.next().await {
        match request {
            Request::GetState => {
                if let Some(callback) = callback {
                    let state = State {
                        linux: device.addr,
                        actions: vec![]
                    };
                    if let Err(_) = callback.send(Response::State(state)) {
                        log::error!("Could not respond with state");
                    }
                }
            }
            Request::Execute(_action) => {}
        }
    }
    (uuid, device.addr)
}

