use std::{fmt::Display, net::Ipv4Addr};
use bytes::Bytes;
use serde::{Serialize, Deserialize};
use crate::{TerminalAction, FernbedienungAction};

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

#[derive(Debug, Deserialize, Serialize)]
pub enum Action {
    Fernbedienung(FernbedienungAction),
    Xbee(XbeeAction)
}

#[derive(Debug, Deserialize, Serialize)]
pub enum XbeeAction {
    SetUpCorePower(bool),
    SetPixhawkPower(bool),
    Mavlink(TerminalAction),
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

