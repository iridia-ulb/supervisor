use futures::{Future, FutureExt, TryFutureExt, TryStreamExt, future, stream::FuturesUnordered};
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;
use uuid::Uuid;
use std::{net::Ipv4Addr, path::PathBuf, sync::Arc, time::Duration};
use tokio::{net::UdpSocket, sync::{mpsc, oneshot}};
use crate::network::{fernbedienung, xbee};
use crate::journal;
use crate::software;

pub struct State {
    pub xbee_addr: Ipv4Addr,
    pub upcore_addr: Option<Ipv4Addr>,
    pub xbee_link_state: u8,
    pub actions: Vec<Action>,
}

// better design would involve making these methods that use fernbedienung some sort
// of Robot (probably async) trait, that requires the implementation of a
// fernbedienung() getter. Then have a Request::Robot(robot::Request), which includes 
// ExperimentStart, ExperimentStop etc
pub enum Request {
    GetState(oneshot::Sender<State>),
    GetId(oneshot::Sender<u8>),
    Pair(fernbedienung::Device),
    Execute(Action),    
    ExperimentStart {
        software: software::Software,
        journal: mpsc::UnboundedSender<journal::Request>,
        callback: oneshot::Sender<Result<()>>
    },
    ExperimentStop,
}

pub type Sender = mpsc::UnboundedSender<Request>;
pub type Receiver = mpsc::UnboundedReceiver<Request>;

// Note: the power off, shutdown, reboot up core actions
// should change the state to standby which, in turn,
// should move the IP address back to the probing pool
#[derive(Copy, Clone, Serialize, Deserialize, Debug, PartialEq)]
pub enum Action {
    #[serde(rename = "Power on UP Core")]
    UpCorePowerOn,
    #[serde(rename = "Halt UP Core")]
    UpCoreHalt,
    #[serde(rename = "Power off UP Core")]
    UpCorePowerOff,
    #[serde(rename = "Reboot UP Core")]
    UpCoreReboot,
    #[serde(rename = "Power on Pixhawk")]
    PixhawkPowerOn,
    #[serde(rename = "Power off Pixhawk")]
    PixhawkPowerOff,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {

    #[error("Could not communicate with Xbee")]
    XbeeTimeout,

    #[error("{0:?} is not currently valid")]
    InvalidAction(Action),

    #[error("Could not request action")]
    RequestError,
    #[error("Did not receive response")]
    ResponseError,

    #[error(transparent)]
    XbeeError(#[from] xbee::Error),
    #[error(transparent)]
    FernbedienungError(#[from] fernbedienung::Error),
    #[error(transparent)]
    SoftwareError(#[from] software::Error),
    #[error(transparent)]
    IoError(#[from] std::io::Error),

}

pub type Result<T> = std::result::Result<T, Error>;

pub async fn poll_fernbedienung(device: Arc<fernbedienung::Device>) {
    while let Ok(response) = tokio::time::timeout(Duration::from_millis(500), device.ping()).await {
        match response {
            Ok(true) => tokio::time::sleep(Duration::from_millis(500)).await,
            _ => break,
        }
    }
}

pub async fn new(uuid: Uuid, mut rx: Receiver, xbee: xbee::Device) -> Uuid {
    /* attempt to initialize the xbee pins and mux */
    if let Err(error) = init(&xbee) {
        log::error!("Failed to initialize Xbee for drone {}: {}", uuid, error);
        return uuid;
    }
    
    let poll_xbee_link_state_task = poll_xbee_link_state(&xbee);
    tokio::pin!(poll_xbee_link_state_task);
    let mut xbee_link_state = 0u8;

    let mut fernbedienung: Option<Arc<fernbedienung::Device>> = None;
    let poll_ferbedienung_task = future::pending().left_future();
    tokio::pin!(poll_ferbedienung_task);

    let mut terminate: Option<oneshot::Sender<()>> = None;
    let argos_task = futures::future::pending().left_future();
    tokio::pin!(argos_task);

    loop {
        tokio::select! {
            result = &mut poll_xbee_link_state_task => match result {
                Ok(link_state) => {
                    xbee_link_state = link_state;
                    poll_xbee_link_state_task.set(poll_xbee_link_state(&xbee));
                },
                /* exit this loop if the xbee fails to respond */
                Err(_) => {
                    log::warn!("Xbee for drone {} failed to respond", uuid);
                    break;
                },
            },
            _ = &mut poll_ferbedienung_task => {
                /* handler if fernbedienung disconnects */
                fernbedienung = None;
                poll_ferbedienung_task.set(future::pending().left_future());
            },
            /* if ARGoS is running, keep forwarding stdout/stderr  */
            argos_result = &mut argos_task => {
                terminate = None;
                argos_task.set(futures::future::pending().left_future());
                log::info!("ARGoS terminated with {:?}", argos_result);
            },
            recv_request = rx.recv() => match recv_request {
                None => break, /* tx handle dropped, exit the loop */
                Some(request) => match request {
                    Request::GetState(callback) => {
                        /* generate a vector of valid actions */
                        let mut actions = xbee_actions(&xbee).await;
                        if fernbedienung.is_some() {
                            actions.push(Action::UpCoreReboot);
                            actions.push(Action::UpCoreHalt);
                        }
                        /* send back the state */
                        let state = State {
                            xbee_addr: xbee.addr,
                            xbee_link_state: xbee_link_state,
                            upcore_addr: fernbedienung.as_ref().map(|dev| dev.addr),
                            actions,
                        };
                        let _ = callback.send(state);
                    },
                    Request::Pair(device) => {
                        let device = Arc::new(device);
                        fernbedienung = Some(device.clone());
                        poll_ferbedienung_task.set(poll_fernbedienung(device).right_future());
                    },
                    Request::Execute(action) => {
                        let result = match action {
                            Action::UpCorePowerOn => set_upcore_power(&xbee, true),
                            Action::UpCorePowerOff => set_upcore_power(&xbee, false),
                            Action::PixhawkPowerOn => set_pixhawk_power(&xbee, true),
                            Action::PixhawkPowerOff => set_pixhawk_power(&xbee, false),
                            Action::UpCoreReboot => match fernbedienung {
                                Some(ref device) => device.reboot().await
                                    .map(|_| ())
                                    .map_err(|error| Error::FernbedienungError(error)),
                                None => Err(Error::InvalidAction(action)),
                            },
                            Action::UpCoreHalt => match fernbedienung {
                                Some(ref device) => device.halt().await
                                    .map(|_| ())
                                    .map_err(|error| Error::FernbedienungError(error)),
                                None => Err(Error::InvalidAction(action)),
                            },
                        };
                        if let Err(error) = result {
                            log::warn!("Could not execute {:?}: {}", action, error);
                        }
                    },
                    Request::GetId(callback) => {
                        /* just drop callback if reading the id failed */
                        if let Ok(id) = get_id(&xbee).await {
                            let _ = callback.send(id);
                        }
                    },
                    Request::ExperimentStart{software, journal, callback} => {
                        match fernbedienung.as_ref() {
                            None => {
                                let _ = callback.send(Err(Error::RequestError));
                            },
                            Some(device) => {
                                match handle_experiment_start(uuid, device.clone(), software, journal).await {
                                    Ok((argos, terminate_tx)) => {
                                        argos_task.set(argos.right_future());
                                        terminate = Some(terminate_tx);
                                        let _ = callback.send(Ok(()));
                                    },
                                    Err(error) => {
                                        let _ = callback.send(Err(error));
                                    }
                                }
                            }
                        }
                    },
                    Request::ExperimentStop => {
                        if let Some(terminate) = terminate.take() {
                            let _ = terminate.send(());
                        }
                        /* poll argos to completion */
                        let result = (&mut argos_task).await;
                        log::info!("ARGoS terminated with {:?}", result);
                        argos_task.set(futures::future::pending().left_future());
                    },
                }
            }
        }
    }
    uuid
}

async fn poll_xbee_link_state(xbee: &xbee::Device) -> Result<u8> {
    let mut remaining_timeouts = 3usize;
    /* sleep a bit */
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    while remaining_timeouts > 0 {
        let request_with_timeout = 
            tokio::time::timeout(Duration::from_millis(500), xbee.link_state());
        if let Ok(result) = request_with_timeout.await {
            return result.map_err(|error| Error::XbeeError(error))
        }
        else {
            remaining_timeouts -= 1;
        }
    }
    Err(Error::XbeeTimeout)
}
 

async fn xbee_actions(xbee: &xbee::Device) -> Vec<Action> {
    let mut actions = Vec::with_capacity(2);
    if let Ok(pin_states) = xbee.pin_states().await {
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
    }
    actions
}

async fn get_id(xbee: &xbee::Device) -> Result<u8> {
    let pin_states = xbee.pin_states().await?;
    let mut id: u8 = 0;
    for (pin, value) in pin_states.into_iter() {
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

async fn handle_experiment_start(uuid: Uuid,
                                 device: Arc<fernbedienung::Device>,
                                 software: software::Software,
                                 journal: mpsc::UnboundedSender<journal::Request>) 
    -> Result<(impl Future<Output = fernbedienung::Result<bool>>, oneshot::Sender<()>)> {
    /* extract the name of the config file */
    let (argos_config, _) = software.argos_config()?;
    let argos_config = argos_config.to_owned();
    /* get the relevant ip address of this machine */
    let message_router_addr = async {
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        socket.connect((device.addr, 80)).await?;
        socket.local_addr().map(|mut socket| {
            socket.set_port(4950);
            socket
        })
    }.await?;

    /* upload the control software */
    let software_upload_path = device.create_temp_dir()
        .map_err(|error| Error::FernbedienungError(error))
        .and_then(|path: String| software.0.into_iter()
            .map(|(filename, contents)| {
                let path = PathBuf::from(&path);
                let filename = PathBuf::from(&filename);
                device.upload(path, filename, contents)
            })
            .collect::<FuturesUnordered<_>>()
            .map_err(|error| Error::FernbedienungError(error))
            .collect::<Result<Vec<_>>>()
            .map_ok(|_| path)
        ).await?;

    /* create a remote instance of ARGoS3 */
    let task = fernbedienung::Run {
        target: "argos3".into(),
        working_dir: software_upload_path.into(),
        args: vec![
            "--config".to_owned(), argos_config.to_owned(),
            "--router".to_owned(), message_router_addr.to_string(),
            "--id".to_owned(), uuid.to_string(),
        ],
    };

    /* channel for terminating ARGoS */
    let (terminate_tx, terminate_rx) = oneshot::channel();

    /* create future for running ARGoS */
    let argos_task_future = async move {
        /* channels for routing stdout and stderr to the journal */
        let (stdout_tx, mut stdout_rx) = mpsc::unbounded_channel();
        let (stderr_tx, mut stderr_rx) = mpsc::unbounded_channel();
        /* run argos remotely */
        let argos = device.run(task, Some(terminate_rx), None, Some(stdout_tx), Some(stderr_tx));
        tokio::pin!(argos);
        loop {
            tokio::select! {
                Some(data) = stdout_rx.recv() => {
                    let message = journal::Robot::StandardOutput(data);
                    let event = journal::Event::Robot(uuid, message);
                    let request = journal::Request::Record(event);
                    if let Err(error) = journal.send(request) {
                        log::warn!("Could not forward standard output of {} to journal: {}", uuid, error);
                    }
                },
                Some(data) = stderr_rx.recv() => {
                    let message = journal::Robot::StandardError(data);
                    let event = journal::Event::Robot(uuid, message);
                    let request = journal::Request::Record(event);
                    if let Err(error) = journal.send(request) {
                        log::warn!("Could not forward standard error of {} to journal: {}", uuid, error);
                    }
                },
                exit_status = &mut argos => break exit_status,
            }
        }
    };
    Ok((argos_task_future, terminate_tx)) 
}