use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::{net::Ipv4Addr, time::Duration};
use tokio::{sync::{Mutex, mpsc, oneshot}, time::timeout};
use tokio_stream::{StreamExt, wrappers::UnboundedReceiverStream};
use crate::network::{fernbedienung, xbee};

pub struct State {
    pub xbee: Ipv4Addr,
    pub linux: Option<Ipv4Addr>,
    pub actions: Vec<Action>,
}

pub enum Request {
    GetState(oneshot::Sender<State>),
    Pair(fernbedienung::Device),
    Execute(Action),
    //Upload(crate::software::Software),
    GetId(oneshot::Sender<Result<u8>>),
}

pub type Sender = mpsc::UnboundedSender<Request>;
pub type Receiver = mpsc::UnboundedReceiver<Request>;

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

    #[error("Could not communicate with Xbee")]
    XbeeTimeout,

    #[error("{0:?} has not been implemented")]
    UnimplementedAction(Action),

    #[error("{0:?} is not currently valid")]
    InvalidAction(Action),
}

pub type Result<T> = std::result::Result<T, Error>;

pub async fn new(uuid: Uuid, mut rx: Receiver, mut xbee: xbee::Device) -> (Uuid, Ipv4Addr, Option<Ipv4Addr>) {
    /* initialize the xbee pin and mux */
    init(&mut xbee);
    /* connection to the fernbedienung service */
    let mut fernbedienung : Option<fernbedienung::Device> = None;
    
    /* vector for holding the current pin states of the xbee */
    let xbee_pin_states : Mutex<Vec<(xbee::Pin, bool)>> = Default::default();
    let poll_pin_states_task = poll_xbee_pin_states(&xbee, &xbee_pin_states);
    tokio::pin!(poll_pin_states_task);
    /* task main loop */
    loop {
        tokio::select! {
            /* update pin states */
            task_result = &mut poll_pin_states_task => {
                /* this future can only complete with an error */
                /* TODO is there anything more meaningful we can do here? */
                if let Err(error) = task_result {
                    log::warn!("Could not poll pin states: {}", error);
                }
                break;
            },
            /* wait for requests */
            request = rx.recv() => match request {
                None => break,
                Some(request) => match request {
                    Request::GetState(callback) => {
                        let state = State {
                            xbee: xbee.addr,
                            linux: fernbedienung.as_ref().map(|dev| dev.addr),
                            actions: actions(&xbee_pin_states).await,
                        };
                        let _ = callback.send(state);
                    }
                    Request::Pair(device) => {
                        fernbedienung = Some(device);
                    }
                    Request::Execute(requested_action) => {
                        /* prevent the execution of invalid actions */
                        let result = actions(&xbee_pin_states).await
                            .into_iter().find(|action| action == &requested_action)
                            .ok_or(Error::InvalidAction(requested_action))
                            .and_then(|action| match action {
                                Action::UpCorePowerOn => set_upcore_power(&xbee, true),
                                Action::UpCorePowerOff => set_upcore_power(&xbee, false),
                                Action::PixhawkPowerOn => set_pixhawk_power(&xbee, true),
                                Action::PixhawkPowerOff => set_pixhawk_power(&xbee, false),
                                _ => Err(Error::UnimplementedAction(action)),
                            });
                        if let Err(error) = result {
                            log::warn!("Could not execute {:?}: {}", requested_action, error);
                        }
                    },
                    Request::GetId(callback) => {
                        let _ = callback.send(get_id(&xbee).await);
                    },
                },
            }
        }
    }
    /* if paired with a fernbedienung service, map it to just its ip address. This causes
       the structure to drop (including the tx channel) which should then shutdown the
       internal task that was created with tokio::spawn */
    let fernbedienung_addr = fernbedienung.map(|dev| dev.addr);
    /* return the uuid and ip addresses for clean up/reuse */
    (uuid, xbee.addr, fernbedienung_addr)
}

async fn poll_xbee_pin_states(xbee: &xbee::Device, pin_states: &Mutex<Vec<(xbee::Pin, bool)>>) -> Result<()> {
    let mut remaining_timeouts = 3u32;
    loop {
        match tokio::time::timeout(Duration::from_millis(500), xbee.read_inputs()).await {
            Ok(read_inputs_result) => {
                *pin_states.lock().await = read_inputs_result?;
                tokio::time::sleep(Duration::from_millis(500)).await;
                remaining_timeouts = 3;
            }
            Err(_) => match remaining_timeouts {
                0 => break Err(Error::XbeeTimeout),
                _ => remaining_timeouts -= 1,
            }
        }
        
    }
}

async fn actions(pin_states: &Mutex<Vec<(xbee::Pin, bool)>>) -> Vec<Action> {
    let mut actions = Vec::new();
    let pin_states = pin_states.lock().await;
    if let Some((_, state)) = pin_states.iter().find(|(pin, _)| pin == &xbee::Pin::DIO11) {
        match state {
            true => actions.push(Action::UpCorePowerOff),
            false => actions.push(Action::UpCorePowerOn),
        }
    }
    if let Some((_, state)) = pin_states.iter().find(|(pin, _)| pin == &xbee::Pin::DIO12) {
        match state {
            true => actions.push(Action::PixhawkPowerOff),
            false => actions.push(Action::PixhawkPowerOn),
        }
    }
    actions
}


// todo read inputs in a loop and use as a ping/means of tracking the 
// state of the xbee
// if no response in X ms, disconnect (?)
async fn get_id(xbee: &xbee::Device) -> Result<u8> {
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

fn init(xbee: &xbee::Device) -> Result<()> {
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

pub fn set_upcore_power(xbee: &xbee::Device, enable: bool) -> Result<()> {
    xbee.write_outputs(vec![(xbee::Pin::DIO11, enable)])?;
    Ok(())
}

pub fn set_pixhawk_power(xbee: &xbee::Device, enable: bool) -> Result<()> {
    xbee.write_outputs(vec![(xbee::Pin::DIO12, enable)])?;
    Ok(())
}
