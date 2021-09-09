use std::{fmt::Display, net::Ipv4Addr};
use bytes::Bytes;
use serde::{Serialize, Deserialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct Descriptor {
    pub id: String,
    pub xbee_macaddr: macaddr::MacAddr6,
    pub upcore_macaddr: macaddr::MacAddr6,
    pub optitrack_id: Option<i32>,
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
    XbeeConnected(Ipv4Addr),
    XbeeDisconnected,
    XbeeSignal(i32),
    Mavlink(String),
    Bash(String),
    PowerState {
        pixhawk: bool,
        upcore: bool,
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Request {
    BashTerminalStart,
    BashTerminalStop,
    BashTerminalRun(String),
    CameraStreamEnable(bool),
    PixhawkPowerEnable(bool),
    MavlinkTerminalStart,
    MavlinkTerminalStop,
    MavlinkTerminalRun(String),
    UpCorePowerEnable(bool),
    UpCoreHalt,
    UpCoreReboot,   
}

