//! Hello
//use futures::{FutureExt, stream::FuturesOrdered};
use serde::{Deserialize, Serialize};
//use log;
use tokio::sync::{mpsc, oneshot};
use crate::network::{fernbedienung, xbee};

pub enum RequestKind {
    GetState,
    Pair(fernbedienung::Device),
}

pub struct Request(RequestKind, oneshot::Sender<Response>);

pub enum Response {

}

const UPCORE_POWER_BIT_INDEX: u8 = 11;
const PIXHAWK_POWER_BIT_INDEX: u8 = 12;
const MUX_CONTROL_BIT_INDEX: u8 = 4;


// Note: the power off, shutdown, reboot up core actions
// should change the state to standby which, in turn,
// should move the IP address back to the probing pool
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub enum Action {
    #[serde(rename = "Power on UpCore")]
    UpCorePowerOn,
    #[serde(rename = "Shutdown UpCore")]
    UpCoreShutdown,
    #[serde(rename = "Power off UpCore")]
    UpCorePowerOff,
    #[serde(rename = "Reboot UpCore")]
    UpCoreReboot,
    #[serde(rename = "Power on Pixhawk")]
    PixhawkPowerOn,
    #[serde(rename = "Power off Pixhawk")]
    PixhawkPowerOff,
    #[serde(rename = "Identify")]
    Identify,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    XbeeError(#[from] xbee::Error),
    #[error(transparent)]
    FernbedienungError(#[from] fernbedienung::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub async fn new(mut xbee: xbee::Device, mut rx: mpsc::UnboundedReceiver<Request>) -> Result<()> {
    //init(&mut xbee).await?;
    Ok(())
}

async fn init(xbee: &mut xbee::Device) -> Result<()> {
    /* pin configuration */
    let pin_disable_output: u8 = 0;
    let pin_digital_output: u8 = 4;
    /* mux configuration */
    let mut dio_config: u16 = 0b0000_0000_0000_0000;
    let mut dio_set: u16 = 0b0000_0000_0000_0000;
    dio_config |= 1 << MUX_CONTROL_BIT_INDEX;
    dio_set |= 1 << MUX_CONTROL_BIT_INDEX;
    /* prepare commands to be sent */
    // TODO this could all be placed into a futures ordered
    let init_commands = vec![
        /* The UART pins need to be disabled for the moment */
        /* D7 -> CTS, D6 -> RTS, P3 -> DOUT, P4 -> DIN */
        /* disabled pins */
        xbee::Command::new("D7", &pin_disable_output.to_be_bytes()),
        xbee::Command::new("D6", &pin_disable_output.to_be_bytes()),
        xbee::Command::new("P3", &pin_disable_output.to_be_bytes()),
        xbee::Command::new("P4", &pin_disable_output.to_be_bytes()),
        /* digital output pins */
        xbee::Command::new("D4", &pin_digital_output.to_be_bytes()),
        xbee::Command::new("D1", &pin_digital_output.to_be_bytes()),
        xbee::Command::new("D2", &pin_digital_output.to_be_bytes()),
        /* mux configuration */
        xbee::Command::new("OM", &dio_config.to_be_bytes()),
        xbee::Command::new("IO", &dio_set.to_be_bytes()),
    ];   
    /* send commands */
    for command in init_commands.into_iter() {
        xbee.send(command).await?;
    }
    Ok(())
}

pub async fn set_power(xbee: &mut xbee::Device, upcore: Option<bool>, pixhawk: Option<bool>) -> Result<()> {
    let mut dio_config: u16 = 0b0000_0000_0000_0000;
    let mut dio_set: u16 = 0b0000_0000_0000_0000;
    /* enable upcore power? */
    if let Some(enable_upcore_power) = upcore {
        dio_config |= 1 << UPCORE_POWER_BIT_INDEX;
        if enable_upcore_power {
            dio_set |= 1 << UPCORE_POWER_BIT_INDEX;
        }
    }
    /* enable pixhawk power? */
    if let Some(enable_pixhawk_power) = pixhawk {
        dio_config |= 1 << PIXHAWK_POWER_BIT_INDEX;
        if enable_pixhawk_power {
            dio_set |= 1 << PIXHAWK_POWER_BIT_INDEX;
        }
    }
    let cmd_om = xbee::Command::new("OM", &dio_config.to_be_bytes());
    let cmd_io = xbee::Command::new("IO", &dio_set.to_be_bytes());
    xbee.send(cmd_om).await?;
    xbee.send(cmd_io).await?;
    Ok(())
}