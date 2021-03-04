use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::net::Ipv4Addr;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::{StreamExt, wrappers::UnboundedReceiverStream};
use crate::network::{fernbedienung, xbee};

pub struct State {
    pub xbee: Ipv4Addr,
    pub linux: Option<Ipv4Addr>,
    pub actions: Vec<Action>,
}

pub enum Request {
    GetState,
    Pair(fernbedienung::Device),
    Execute(Action),
    //Upload(crate::software::Software),
    GetId,
}

pub enum Response {
    State(State),
    Id(u8)
}

pub type Sender = mpsc::UnboundedSender<(Request, Option<oneshot::Sender<Response>>)>;
pub type Receiver = mpsc::UnboundedReceiver<(Request, Option<oneshot::Sender<Response>>)>;

// Note: the power off, shutdown, reboot up core actions
// should change the state to standby which, in turn,
// should move the IP address back to the probing pool
#[derive(Copy, Clone, Serialize, Deserialize, Debug, PartialEq)]
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

pub async fn new(uuid: Uuid, rx: Receiver, mut xbee: xbee::Device) -> (Uuid, Ipv4Addr, Option<Ipv4Addr>) {
    /* initialize the xbee pin and mux */
    init(&mut xbee);
    /* wait for requests */
    let mut requests = UnboundedReceiverStream::new(rx);
    while let Some((request, callback)) = requests.next().await {
        match request {
            Request::GetState => {}
            Request::Pair(fernbedienung) => {

            }
            Request::Execute(_) => {}
            Request::GetId => {
                if let Some(callback) = callback {
                    if let Ok(id) = get_id(&mut xbee).await {
                        let _ = callback.send(Response::Id(id));
                    }
                }
            }
        }

    }
    
    (uuid, xbee.addr, None)

    // if Fernbedienung exits, we should go into emergency mode and take control over the drone using the Xbee
}

async fn get_id(xbee: &mut xbee::Device) -> Result<u8> {
    let mut id: u8 = 0;
    for (pin, value) in xbee.read_inputs().await? {
        let bit_index = pin as usize;
        /* extract identifier from bit indices 0-3 inclusive */
        if bit_index < 4 {
            id |= (value as u8) << bit_index;
        }
    }
    Ok(id)
}

fn init(xbee: &mut xbee::Device) -> Result<()> {
    /* set pin modes */
    let pin_modes = vec![
        /* The UART pins need to be disabled for the moment */
        /* CTS: DIO7, RTS: DIO6, TX: DOUT, RX: DIN */
        (xbee::Pin::DIO7, xbee::PinMode::Disable),
        (xbee::Pin::DIO6, xbee::PinMode::Disable),
        (xbee::Pin::DOUT, xbee::PinMode::Disable),
        (xbee::Pin::DIN,  xbee::PinMode::Disable),
        /* Input pins for reading an identifer */
        (xbee::Pin::DIO0, xbee::PinMode::Input),
        (xbee::Pin::DIO1, xbee::PinMode::Input),
        (xbee::Pin::DIO2, xbee::PinMode::Input),
        (xbee::Pin::DIO3, xbee::PinMode::Input),
        /* Output pins for controlling power and mux */
        (xbee::Pin::DIO4, xbee::PinMode::OutputDefaultLow),
        (xbee::Pin::DIO11, xbee::PinMode::OutputDefaultLow),
        (xbee::Pin::DIO12, xbee::PinMode::OutputDefaultLow),
    ];
    xbee.set_pin_modes(pin_modes)?;
    /* configure mux */
    xbee.write_outputs(vec![(xbee::Pin::DIO4, true)])?;
    Ok(())
}

pub fn set_power(xbee: &mut xbee::Device, upcore: Option<bool>, pixhawk: Option<bool>) -> Result<()> {
    let mut outputs = Vec::new();
    if let Some(enable) = upcore {
        outputs.push((xbee::Pin::DIO11, enable));
    }
    if let Some(enable) = pixhawk {
        outputs.push((xbee::Pin::DIO12, enable));
    }
    xbee.write_outputs(outputs)?;
    Ok(())
}