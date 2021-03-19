use futures::{FutureExt, TryFutureExt, TryStreamExt, stream::FuturesUnordered};
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;
use uuid::Uuid;
use std::{net::Ipv4Addr, path::PathBuf, time::Duration};
use tokio::{sync::{Mutex, mpsc, oneshot}};
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
    GetId(oneshot::Sender<u8>),
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

    #[error("Could not request action")]
    RequestError,

    #[error("Did not receive response")]
    ResponseError,
}

pub type Result<T> = std::result::Result<T, Error>;

enum FernbedienungRequest {
    Shutdown(oneshot::Sender<fernbedienung::Result<bool>>),
    Reboot(oneshot::Sender<fernbedienung::Result<bool>>),
    Upload(crate::software::Software, oneshot::Sender<fernbedienung::Result<()>>),
}

async fn fernbedienung(device: fernbedienung::Device,
                       mut rx: mpsc::UnboundedReceiver<FernbedienungRequest>) {
    let mut argos_path: Option<String> = Default::default();
    let mut argos_config: Option<String> = Default::default();

    tokio::select! {
        /* this future will complete if the sender is dropped */
        _ = async {
            while let Some(request) = rx.recv().await {
                match request {
                    FernbedienungRequest::Shutdown(callback) => {
                        let _ = callback.send(device.shutdown().await);
                    },
                    FernbedienungRequest::Reboot(callback) => {
                        let _ = callback.send(device.reboot().await);
                    },
                    FernbedienungRequest::Upload(software, callback) => {
                        argos_config = software.argos_config()
                            .map(|(argos_config, _)| argos_config.to_owned())
                            .ok();
                        let upload_result = device.create_temp_dir()
                            .and_then(|path: String| software.0.into_iter()
                                .map(|(filename, contents)| {
                                    let path = PathBuf::from(&path);
                                    let filename = PathBuf::from(&filename);
                                    device.upload(path, filename, contents)
                                })
                                .collect::<FuturesUnordered<_>>()
                                .collect::<fernbedienung::Result<Vec<_>>>()
                                .map_ok(|_| path)
                            ).await;
                        let _ = callback.send(upload_result.map(|path| {
                            /* send Result<(), Error> */
                            argos_path = Some(path);
                        }));
                    }
                }
            }
        } => {},
        /* this future will complete if fernbedienung stops responding */
        _ = async {
            while let Ok(Ok(true)) = tokio::time::timeout(Duration::from_millis(500),  device.ping()).await {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        } => {}
    }
    
}

// fernbedienung dies -> just let it go out of scope
// handle returning ip address separately

pub async fn new(uuid: Uuid, mut rx: Receiver, mut xbee: xbee::Device) -> Uuid {
    /* initialize the xbee pin and mux */
    init(&mut xbee);
    /* vector for holding the current pin states of the xbee */
    let xbee_pin_states : Mutex<Vec<(xbee::Pin, bool)>> = Default::default();
    let poll_pin_states_task = poll_xbee_pin_states(&xbee, &xbee_pin_states);
    tokio::pin!(poll_pin_states_task);

    // this is a pinned box over an either future. Either future allows holding one of two future
    // variants, either Pending or the Poll function
    let mut fernbedienung_tx = None;
    let fernbedienung_task = futures::future::pending().left_future();
    tokio::pin!(fernbedienung_task);

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
                /* note that this will drop the fernbedienung connection if it exists */
                break;
            },
            _ = &mut fernbedienung_task => {
                fernbedienung_tx = None;
                fernbedienung_task.set(futures::future::pending().left_future());
            }
            /* wait for requests */
            request = rx.recv() => match request {
                None => break,
                Some(request) => match request {
                    Request::GetState(callback) => {
                        /* generate a vector of valid actions */
                        let mut actions = xbee_actions(&xbee_pin_states).await;
                        if fernbedienung_tx.is_some() {
                            actions.push(Action::UpCoreReboot);
                            actions.push(Action::UpCoreShutdown);
                        }
                        /* send back the state */
                        let state = State {
                            xbee: xbee.addr,
                            linux: None, //fernbedienung.as_ref().map(|dev| dev.addr),
                            actions,
                        };
                        let _ = callback.send(state);
                    }
                    Request::Pair(device) => {
                        let (tx, rx) = mpsc::unbounded_channel();
                        fernbedienung_tx = Some(tx);
                        fernbedienung_task.set(fernbedienung(device, rx).right_future());
                    }
                    Request::Execute(requested_action) => {
                        /* generate a vector of valid actions */
                        let mut actions = xbee_actions(&xbee_pin_states).await;
                        if fernbedienung_tx.is_some() {
                            actions.push(Action::UpCoreReboot);
                            actions.push(Action::UpCoreShutdown);
                        }
                        /* execute requested action if it was valid */
                        let result = match actions.into_iter().find(|&action| action == requested_action) {
                            Some(action) => match action {
                                Action::UpCorePowerOn => set_upcore_power(&xbee, true),
                                Action::UpCorePowerOff => set_upcore_power(&xbee, false),
                                Action::PixhawkPowerOn => set_pixhawk_power(&xbee, true),
                                Action::PixhawkPowerOff => set_pixhawk_power(&xbee, false),
                                Action::UpCoreReboot => match fernbedienung_tx {
                                    Some(ref tx) => {
                                        let (callback_tx, callback_rx) = oneshot::channel();
                                        match tx.send(FernbedienungRequest::Reboot(callback_tx)) {
                                            Ok(_) => callback_rx.await
                                                .map_err(|_| Error::ResponseError)
                                                .and_then(|result| result
                                                    .map_err(|err| Error::FernbedienungError(err))
                                                    .map(|_| ())),
                                            Err(_) => Err(Error::RequestError)
                                        }
                                    },
                                    None => Err(Error::InvalidAction(requested_action)),
                                },
                                Action::UpCoreShutdown => match fernbedienung_tx {
                                    Some(ref tx) => {
                                        let (callback_tx, callback_rx) = oneshot::channel();
                                        match tx.send(FernbedienungRequest::Shutdown(callback_tx)) {
                                            Ok(_) => callback_rx.await
                                                .map_err(|_| Error::ResponseError)
                                                .and_then(|result| result
                                                    .map_err(|err| Error::FernbedienungError(err))
                                                    .map(|_| ())),
                                            Err(_) => Err(Error::RequestError)
                                        }
                                    },
                                    None => Err(Error::InvalidAction(requested_action)),
                                },
                                _ => Err(Error::UnimplementedAction(action))
                            },
                            None => Err(Error::InvalidAction(requested_action))
                        };
                        if let Err(error) = result {
                            log::warn!("Could not execute {:?}: {}", requested_action, error);
                        }
                    },
                    Request::GetId(callback) => {
                        let _ = callback.send(get_id(&xbee_pin_states).await);
                    },
                },
            }
        }
    }
    /* return the uuid */
    uuid
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

async fn xbee_actions(pin_states: &Mutex<Vec<(xbee::Pin, bool)>>) -> Vec<Action> {
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

async fn get_id(pin_states: &Mutex<Vec<(xbee::Pin, bool)>>) -> u8 {
    let pin_states = pin_states.lock().await;
    let mut id: u8 = 0;
    for (pin, value) in pin_states.clone() {
        let bit_index = pin as usize;
        /* extract identifier from bit indices 0-3 inclusive */
        if bit_index < 4 {
            id |= (value as u8) << bit_index;
        }
    }
    id
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
