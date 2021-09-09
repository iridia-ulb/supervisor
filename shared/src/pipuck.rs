use std::{fmt::Display, net::Ipv4Addr};
use bytes::Bytes;
use serde::{Serialize, Deserialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct Descriptor {
    pub id: String,
    pub rpi_macaddr: macaddr::MacAddr6,
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
    // sends camera footage
    Camera {
        camera: String,
        result: Result<Bytes, String>
    },
    // describes static information about Pi-Puck
    Descriptor(Descriptor),
    // indicates whether the connection is up or down
    FernbedienungConnection(Option<Ipv4Addr>),
    // indicates the signal strength
    FernbedienungSignal(Result<i32, String>)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Request {
    BashTerminalStart,
    BashTerminalStop,
    BashTerminalRun(String),
    CameraStreamEnable(bool),
    RaspberryPiHalt,
    RaspberryPiReboot,
}


