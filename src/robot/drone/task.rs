use std::{collections::HashMap, time::Duration};
use anyhow::Context;
use bytes::BytesMut;
use mavlink::{MavHeader, common::{self, MavMessage, SerialControlDev, SerialControlFlag}};
use shared::drone::{BashAction, MavlinkAction};
use tokio::{net::TcpStream, sync::{broadcast, mpsc, oneshot}};
use futures::{FutureExt, SinkExt, Stream, StreamExt, TryStreamExt};
use tokio_stream;
use tokio_util::codec::Framed;

use crate::network::{fernbedienung, fernbedienung_ext::MjpegStreamerStream, xbee};
use crate::journal;
use crate::software;
use super::codec;

pub use shared::drone::{Action, FernbedienungAction, XbeeAction, Descriptor, Update};

const DRONE_BATT_FULL_MV: f32 = 4050.0;
const DRONE_BATT_EMPTY_MV: f32 = 3500.0;
const DRONE_BATT_NUM_CELLS: f32 = 3.0;
const DRONE_CAMERAS_CONFIG: &[(&str, u16, u16, u16)] = &[
    ("/dev/camera0", 1024, 768, 8000),
    ("/dev/camera1", 1024, 768, 8001),
    ("/dev/camera2", 1024, 768, 8002),
    ("/dev/camera3", 1024, 768, 8003),
];

const XBEE_DEFAULT_PIN_CONFIG: &[(xbee::Pin, xbee::PinMode)] = &[
    /* UART pins: TX: DOUT, RTS: DIO6, RX: DIN, CTS: DIO7 */
    /* UART enabled without hardware flow control */
    (xbee::Pin::DOUT, xbee::PinMode::Alternate),
    (xbee::Pin::DIO6, xbee::PinMode::Disable),
    (xbee::Pin::DIO7, xbee::PinMode::Disable),
    (xbee::Pin::DIN,  xbee::PinMode::Alternate),
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

/* hardware flow control connected but disabled */

#[derive(Debug)]
pub enum Request {
    AssociateFernbedienung(fernbedienung::Device),
    AssociateXbee(xbee::Device),
    Execute(Action),
    Subscribe(oneshot::Sender<broadcast::Receiver<Update>>),
    StartExperiment {
        software: software::Software,
        journal: mpsc::Sender<journal::Request>,
        callback: oneshot::Sender<anyhow::Result<()>>
    },
    StopExperiment,
}

pub type Sender = mpsc::Sender<Request>;
pub type Receiver = mpsc::Receiver<Request>;

fn xbee_pin_states_stream<'dev>(
    device: &'dev xbee::Device
) -> impl Stream<Item = anyhow::Result<HashMap<xbee::Pin, bool>>> + 'dev {
    async_stream::stream! {
        let mut attempts: u8 = 0;
        loop {
            let link_margin_task = tokio::time::timeout(Duration::from_millis(500), device.pin_states()).await
                .context("Timeout while communicating with Xbee")
                .and_then(|result| result.context("Could not communicate with Xbee"));
            match link_margin_task {
                Ok(response) => {
                    attempts = 0;
                    yield Ok(response);
                },
                Err(error) => match attempts {
                    0..=2 => attempts += 1,
                    _ => yield Err(error)
                }
            }
        }
    }
}

fn xbee_link_margin_stream<'dev>(
    device: &'dev xbee::Device
) -> impl Stream<Item = anyhow::Result<i32>> + 'dev {
    async_stream::stream! {
        let mut attempts: u8 = 0;
        loop {
            let link_margin_task = tokio::time::timeout(Duration::from_millis(500), device.link_margin()).await
                .context("Timeout while communicating with Xbee")
                .and_then(|result| result.context("Xbee communication error"));
            match link_margin_task {
                Ok(response) => {
                    attempts = 0;
                    yield Ok(response);
                },
                Err(error) => match attempts {
                    0..=2 => attempts += 1,
                    _ => yield Err(error)
                }
            }
        }
    }
}

// To be set before starting ARGoS, and cleared after ARGoS finishes
/* configure mux (upcore controls pixhawk) */
//device.write_outputs(vec![(xbee::Pin::DIO4, true)]).await?;

async fn mavlink(
    device: &xbee::Device,
    mut rx: mpsc::Receiver<MavlinkAction>,
    updates_tx: broadcast::Sender<Update>
) -> anyhow::Result<()> {
    let connection = TcpStream::connect((device.addr, 9750))
        .map(|result| result
            .context("Could not connect to serial communication service"));
    let connection = tokio::time::timeout(Duration::from_secs(1), connection)
        .map(|result| result
            .context("Timeout while connecting to serial communication service")
            .and_then(|result| result)).await?;
    let (mut sink, mut stream) = 
        Framed::new(connection, codec::MavMessageCodec::<MavMessage>::new()).split();

    let mut mavlink_target = None;
    let mut mavlink_sequence = 0;
    
    loop {
        tokio::select! {
            Some(message) = rx.recv() => match message {
                MavlinkAction::KeyUp(_) => {},
                MavlinkAction::KeyDown(_) => {
                    match mavlink_target {
                        Some((system_id, component_id)) => {
                            log::info!("sending top");
                            let data = common::SERIAL_CONTROL_DATA {
                                baudrate: 0,
                                timeout: 3,
                                device: SerialControlDev::SERIAL_CONTROL_DEV_SHELL,
                                flags: SerialControlFlag::SERIAL_CONTROL_FLAG_RESPOND | SerialControlFlag::SERIAL_CONTROL_FLAG_MULTI,
                                count: "top".as_bytes().len() as u8,
                                data: "top".as_bytes().to_owned(),
                            };
                            let message = MavMessage::SERIAL_CONTROL(data);
                            let header = MavHeader { system_id, component_id, sequence: mavlink_sequence };
                            
                            match sink.send((header, message)).await {
                                Ok(_) => {
                                    mavlink_sequence = mavlink_sequence.wrapping_add(1);
                                }
                                Err(error) => log::error!("{}", error)
                            }
                        },
                        None => {
                            log::warn!("Can not send, target not ready");
                        }
                    }
                },
                MavlinkAction::Close => {},
            },
            Some(Ok((header, body))) = stream.next() => {
                match body {
                    MavMessage::HEARTBEAT(_) => {
                        if let None = mavlink_target {
                            mavlink_target = Some((header.system_id, header.component_id));
                        }
                    },
                    MavMessage::BATTERY_STATUS(data) => {
                        let mut battery_reading = data.voltages[0] as f32;
                        battery_reading /= DRONE_BATT_NUM_CELLS;
                        battery_reading -= DRONE_BATT_EMPTY_MV;
                        battery_reading /= DRONE_BATT_FULL_MV - DRONE_BATT_EMPTY_MV;
                        let battery_reading = (battery_reading.max(0.0).min(1.0) * 100.0) as i32;
                        let _ = updates_tx.send(Update::Battery(battery_reading));
                    },
                    MavMessage::SERIAL_CONTROL(data) => {
                        log::info!("serial control data => {:?}", data);
                        let s = String::from_utf8_lossy(&data.data).into_owned();
                        let _  = updates_tx.send(Update::Mavlink(s));
                    },
                    _ => {}
                }
            }
        }
    }
}

async fn xbee(
    device: xbee::Device,
    mut rx: mpsc::Receiver<XbeeAction>,
    updates_tx: broadcast::Sender<Update>
) -> anyhow::Result<()> {
    device.set_pin_modes(XBEE_DEFAULT_PIN_CONFIG).await
        .context("Could not set Xbee pin modes")?;
    /* set the serial communication service to TCP mode */
    device.set_scs_mode(true).await
        .context("Could not enable serial communication service")?;
    /* set the baud rate to match the baud rate of the Pixhawk */
    device.set_baud_rate(115200).await
        .context("Could not set serial baud rate")?;
    /* link margin stream */
    let link_margin_stream = xbee_link_margin_stream(&device);
    let link_margin_stream_throttled =
        tokio_stream::StreamExt::throttle(link_margin_stream, Duration::from_millis(1000));       
    tokio::pin!(link_margin_stream_throttled);
    /* pin states stream */
    let pin_states_stream = xbee_pin_states_stream(&device);
    let pin_states_stream_throttled =
        tokio_stream::StreamExt::throttle(pin_states_stream, Duration::from_millis(1000));       
    tokio::pin!(pin_states_stream_throttled);
    /* mavlink task */
    let (mut mavlink_tx, mavlink_rx) = mpsc::channel(8);
    let mavlink_task = mavlink(&device, mavlink_rx, updates_tx.clone());
    tokio::pin!(mavlink_task);

    loop {
        tokio::select! {
            Some(response) = link_margin_stream_throttled.next() => {
                let update = Update::XbeeSignal(response?);
                let _ = updates_tx.send(update);
            },
            Some(response) = pin_states_stream_throttled.next() => {
                let response = response?;
                let upcore = response.get(&xbee::Pin::DIO11);
                let pixhawk = response.get(&xbee::Pin::DIO12);
                match (upcore, pixhawk) {
                    (Some(&upcore), Some(&pixhawk)) => {
                        let _ = updates_tx.send(Update::PowerState { upcore, pixhawk });
                    },
                    _ => log::warn!("Could not update power state")
                }
            },
            recv = rx.recv() => match recv {
                Some(request) => match request {
                    XbeeAction::SetUpCorePower(enable) => {
                        if let Err(error) = device.write_outputs(&[(xbee::Pin::DIO11, enable)]).await {
                            log::error!("Could not configure Up Core power: {}", error);
                        }
                    },
                    XbeeAction::SetPixhawkPower(enable) => {
                        if let Err(error) = device.write_outputs(&[(xbee::Pin::DIO12, enable)]).await {
                            log::error!("Could not configure Pixhawk power: {}", error);
                        }
                    },
                    XbeeAction::Mavlink(action) => {
                        let _ = mavlink_tx.send(action).await;
                    },
                },
                None => break Ok(()), // normal shutdown
            },
            result = &mut mavlink_task => {
                if let Err(error) = result {
                    log::error!("Mavlink task terminated: {}", error);
                }
                /* restart task */
                let (tx, rx) = mpsc::channel(8);
                mavlink_tx = tx;
                mavlink_task.set(mavlink(&device, rx, updates_tx.clone()));
            }
        }
    }
}

fn fernbedienung_link_strength_stream<'dev>(
    device: &'dev fernbedienung::Device
) -> impl Stream<Item = anyhow::Result<i32>> + 'dev {
    async_stream::stream! {
        let mut attempts : u8 = 0;
        loop {
            let link_strength_task = tokio::time::timeout(Duration::from_millis(500), device.link_strength()).await
                .context("Timeout while communicating with Up Core")
                .and_then(|result| result.context("Could not communicate with Up Core"));
            match link_strength_task {
                Ok(response) => {
                    attempts = 0;
                    yield Ok(response);
                },
                Err(error) => match attempts {
                    0..=2 => attempts += 1,
                    _ => yield Err(error)
                }
            }
        }
    }
}

async fn bash(
    device: &fernbedienung::Device,
    mut rx: mpsc::Receiver<BashAction>,
    updates_tx: broadcast::Sender<Update>,
) {
    let process = fernbedienung::Process {
        target: "bash".into(),
        working_dir: None,
        args: vec!["-i".to_owned()],
    };
    let (stdout_tx, mut stdout_rx) = mpsc::channel(8);
    let (stderr_tx, mut stderr_rx) = mpsc::channel(8);
    let (stdin_tx, stdin_rx) = mpsc::channel(8);
    let (terminate_tx, terminate_rx) = oneshot::channel();

    let process = device.run(process, Some(terminate_rx), Some(stdin_rx), Some(stdout_tx), Some(stderr_tx));
    tokio::pin!(process);

    loop {
        tokio::select! {
            result = &mut process => {
                log::info!("Remote bash instance terminated with {:?}", result);
                break;
            }
            Some(stdout) = stdout_rx.recv() => {
                let update = Update::Bash(String::from_utf8_lossy(&stdout).into_owned());
                log::info!("{:?}", update);
                let _ = updates_tx.send(update);
            },
            Some(stderr) = stderr_rx.recv() => {
                let update = Update::Bash(String::from_utf8_lossy(&stderr).into_owned());
                log::info!("{:?}", update);
                let _ = updates_tx.send(update);
            },
            Some(action) = rx.recv() => match action {
                BashAction::KeyUp(_key) => {}, // stdin_tx
                BashAction::KeyDown(key) => {
                    let input = match key.as_ref() {
                        "Enter" => "\r",
                        "Backspace" => "\x08",
                        other => other,
                    };
                    if let Err(error) = stdin_tx.send(BytesMut::from(input)).await {
                        log::warn!("Could not send keystokes to remote Bash instance: {}", error)
                    }
                },
                BashAction::Close => {
                    let _ = terminate_tx.send(());
                    break;
                }
            }
        }
    }
}

async fn fernbedienung(
    device: fernbedienung::Device,
    mut rx: mpsc::Receiver<FernbedienungAction>,
    updates_tx: broadcast::Sender<Update>
) {
    /* bash task */
    let bash_task = futures::future::pending().left_future();
    let mut bash_tx : Option<mpsc::Sender<BashAction>> = Option::default();
    tokio::pin!(bash_task);
    /* link strength */
    let link_strength_stream = fernbedienung_link_strength_stream(&device)
        .map_ok(Update::FernbedienungSignal);
    let link_strength_stream_throttled =
        tokio_stream::StreamExt::throttle(link_strength_stream, Duration::from_millis(1000));
    tokio::pin!(link_strength_stream_throttled);
    /* handle for the camera stream */
    let mut cameras_stream: tokio_stream::StreamMap<String, _> =
        tokio_stream::StreamMap::new();
    loop {
        tokio::select! {
            Some((camera, result)) = cameras_stream.next() => {
                let result: reqwest::Result<bytes::Bytes> = result;
                let update = Update::Camera { camera, result: result.map_err(|e| e.to_string()) };
                let _ = updates_tx.send(update);
            },
            Some(response) = link_strength_stream_throttled.next() => match response {
                Ok(update) => {
                    let _ = updates_tx.send(update);
                },
                Err(error) => {
                    log::warn!("{}", error);
                    break;
                },
            },
            recv = rx.recv() => match recv {
                Some(request) => match request {
                    FernbedienungAction::SetCameraStream(enable) => {
                        cameras_stream.clear();
                        if enable {
                            for &(camera, width, height, port) in DRONE_CAMERAS_CONFIG {
                                let stream = MjpegStreamerStream::new(&device, camera, width, height, port);
                                let stream = tokio_stream::StreamExt::throttle(stream, Duration::from_millis(200));
                                cameras_stream.insert(camera.to_owned(), Box::pin(stream));
                            }
                        }
                    },
                    FernbedienungAction::Halt => if let Err(error) = device.halt().await {
                        log::warn!("Could not halt Up Core: {}", error);
                    },
                    FernbedienungAction::Reboot => if let Err(error) = device.reboot().await {
                        log::warn!("Could not reboot Up Core: {}", error);
                    },
                    FernbedienungAction::Bash(action) => match &bash_tx {
                        Some(tx) => {
                            let _ = tx.send(action).await;    
                        },
                        None => {
                            let (tx, rx) = mpsc::channel(8);
                            let _ = tx.send(action).await;
                            bash_tx = Some(tx);
                            bash_task.set(bash(&device, rx, updates_tx.clone()).right_future());
                        }
                    },
                    FernbedienungAction::GetKernelMessages => {},
                    FernbedienungAction::Identify => {},
                },
                None => break,
            },
            _ = &mut bash_task => {
                bash_tx = None;
                bash_task.set(futures::future::pending().left_future());
            },
        }
    }
}

pub async fn new(mut request_rx: Receiver) {
    /* fernbedienung task state */
    let fernbedienung_task = futures::future::pending().left_future();
    let mut fernbedienung_tx = Option::default();
    let mut fernbedienung_addr = Option::default();
    tokio::pin!(fernbedienung_task);
    /* xbee task state */
    let xbee_task = futures::future::pending().left_future();
    let mut xbee_tx = Option::default();
    let mut xbee_addr = Option::default();
    tokio::pin!(xbee_task);
    /* updates_tx is for sending changes in state to subscribers (e.g., the webui) */
    let (updates_tx, _) = broadcast::channel(16);
    

    // TODO: for a clean shutdown we may want to consider the case where updates_tx hangs up
    loop {
        tokio::select! {
            Some(request) = request_rx.recv() => match request {
                Request::AssociateFernbedienung(device) => {
                    let (tx, rx) = mpsc::channel(8);
                    fernbedienung_tx = Some(tx);
                    fernbedienung_addr = Some(device.addr);
                    let _ = updates_tx.send(Update::FernbedienungConnected(device.addr));
                    fernbedienung_task.set(fernbedienung(device, rx, updates_tx.clone()).right_future());
                },
                Request::AssociateXbee(device) => {
                    let (tx, rx) = mpsc::channel(8);
                    xbee_tx = Some(tx);
                    xbee_addr = Some(device.addr);
                    let _ = updates_tx.send(Update::XbeeConnected(device.addr));
                    xbee_task.set(xbee(device, rx, updates_tx.clone()).right_future());
                },
                Request::Execute(action) => match action {
                    Action::Fernbedienung(action) => match &fernbedienung_tx {
                        Some(tx) => {
                            let _ = tx.send(action).await;
                        },
                        None => log::error!("Could not execute {:?}: Fernbedienung is not connected.", action)
                    },
                    Action::Xbee(action) => match &xbee_tx {
                        Some(tx) => {
                            let _ = tx.send(action).await;
                        },
                        None => log::error!("Could not execute {:?}: Xbee is not connected.", action)
                    },
                },
                Request::Subscribe(callback) => {
                    /* note that upon subscribing all updates should be sent to ensure
                       that new clients are in sync */
                    if let Ok(_) = callback.send(updates_tx.subscribe()) {
                        if let Some(addr) = xbee_addr {
                            let _ = updates_tx.send(Update::XbeeConnected(addr));
                        }
                        if let Some(addr) = fernbedienung_addr {
                            let _ = updates_tx.send(Update::FernbedienungConnected(addr));
                        }
                    }
                },
                Request::StartExperiment { software, journal, callback } => log::warn!("not implemented"),
                Request::StopExperiment => log::warn!("not implemented"),
            },
            _ = &mut fernbedienung_task => {
                fernbedienung_tx = None;
                fernbedienung_addr = None;
                fernbedienung_task.set(futures::future::pending().left_future());
                let _ = updates_tx.send(Update::FernbedienungDisconnected);
            },
            result = &mut xbee_task => {
                xbee_tx = None;
                xbee_addr = None;
                xbee_task.set(futures::future::pending().left_future());
                let _ = updates_tx.send(Update::XbeeDisconnected);
                if let Err(error) = result {
                    log::warn!("{}", error);
                }
            }
        }
    }
}