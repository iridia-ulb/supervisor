use serde::{Deserialize, Serialize};
//use log;
use tokio::sync::{mpsc, oneshot};
use crate::network::{fernbedienung};

pub enum RequestKind {
    GetState,
    Pair(fernbedienung::Device),
}

pub struct Request(RequestKind, oneshot::Sender<Response>);

pub enum Response {}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
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
}

pub type Result<T> = std::result::Result<T, Error>;

pub async fn new(mut device: fernbedienung::Device, mut rx: mpsc::UnboundedReceiver<Request>) -> Result<()> {
    Ok(())
}

