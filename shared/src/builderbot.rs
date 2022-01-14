use std::{fmt::Display, net::Ipv4Addr};
use bytes::Bytes;
use serde::{Serialize, Deserialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct Descriptor {
    pub id: String,
    pub duovero_macaddr: macaddr::MacAddr6,
    pub optitrack_id: Option<i32>,
    pub apriltag_id: Option<u8>,
}

impl Display for Descriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.id)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Update {
    Battery(i32),
    Camera {
        camera: String,
        result: Result<Bytes, String>
    },
    FernbedienungConnected(Ipv4Addr),
    FernbedienungDisconnected,
    FernbedienungSignal(i32),
    Bash(String),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Request {
    BashTerminalStart,
    BashTerminalStop,
    BashTerminalRun(String),
    CameraStreamEnable(bool),
    Identify,
    DuoVeroHalt,
    DuoVeroReboot,
}

