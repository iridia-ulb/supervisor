use std::net::Ipv4Addr;
use bytes::Bytes;
use serde::{Serialize, Deserialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Descriptor {
    pub id: String,
    pub xbee_macaddr: macaddr::MacAddr6,
    pub upcore_macaddr: macaddr::MacAddr6,
    pub optitrack_id: Option<i32>,
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
    // describes static information about drone
    Descriptor(Descriptor),
    // indicates whether the connection is up or down
    FernbedienungConnection(Option<Ipv4Addr>),
    // indicates the fernbedienung signal strength
    FernbedienungSignal(Result<i32, String>),
    // indicates the xbee signal strength
    XbeeSignal(Result<i32, String>)
}

