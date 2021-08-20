use std::net::Ipv4Addr;
use bytes::Bytes;
use serde::{Serialize, Deserialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Update {
    // sends camera footage
    Camera {
        camera: String,
        result: Result<Bytes, String>
    },
    // indicates whether the connection is up or down
    FernbedienungConnection(Option<Ipv4Addr>),
    // indicates the signal strength
    FernbedienungSignal(Result<i32, String>)
}

#[derive(Debug, Deserialize, Serialize)]
pub enum Action {
    RpiHalt,
    RpiReboot,
    StartCameraStream,
    StopCameraStream,
    GetKernelMessages,
}

