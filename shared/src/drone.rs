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

#[derive(Debug, Deserialize, Serialize)]
pub enum Action {
    UpCorePowerOn,
    UpCoreHalt,
    UpCorePowerOff,
    UpCoreReboot,
    PixhawkPowerOn,
    PixhawkPowerOff,
    StartCameraStream,
    StopCameraStream,
    GetKernelMessages,
    Identify,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Update {
    // sends camera footage
    Camera {
        camera: String,
        result: Result<Bytes, String>
    },
    // indicates whether the ferbedienung connection is up or down
    FernbedienungConnected(Ipv4Addr),
    FernbedienungDisconnected,
    FernbedienungSignal(i32),
    // indicates whether the xbee connection is up or down
    XbeeConnected(Ipv4Addr),
    XbeeDisconnected,
    XbeeSignal(i32)
}

