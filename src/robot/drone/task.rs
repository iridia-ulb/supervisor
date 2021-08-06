use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::Ipv4Addr, path::PathBuf, sync::Arc, time::Duration};

use futures::{Future, FutureExt, StreamExt, TryFutureExt, TryStreamExt, future::{self, Either}, stream::{FuturesOrdered, FuturesUnordered}};
use tokio::{self, net::{TcpStream, UdpSocket}, sync::{broadcast, mpsc, oneshot}};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::codec::FramedRead;

use crate::network::{fernbedienung, xbee};
use crate::journal;
use crate::software;

pub use shared::drone::Update;

const DRONE_BATT_FULL_MV: f32 = 4050.0;
const DRONE_BATT_EMPTY_MV: f32 = 3500.0;
const DRONE_BATT_NUM_CELLS: f32 = 3.0;
const DRONE_CAMERAS_CONFIG: &[(&str, u16, u16, u16)] = &[
    ("/dev/camera0", 1024, 768, 8000),
    ("/dev/camera1", 1024, 768, 8001),
    ("/dev/camera2", 1024, 768, 8002),
    ("/dev/camera3", 1024, 768, 8003),
];

use crate::robot::drone::codec;

pub struct State {
    pub xbee: (Ipv4Addr, i32),
    pub upcore: Option<(Ipv4Addr, i32)>,
    pub battery_remaining: i8,
    pub actions: Vec<Action>,
    pub cameras: Vec<Bytes>,
    pub devices: Vec<(String, String)>,
    pub kernel_messages: Option<String>,
}

pub enum Request {
    AssociateFernbedienung(fernbedienung::Device),
    AssociateXbee(xbee::Device),
    // when this message is recv, all variants of Update must be sent
    // and then updates are sent only on changes
    Subscribe(oneshot::Sender<broadcast::Receiver<Update>>),

    
    
    GetId(oneshot::Sender<u8>),
    Execute(Action),
    ExperimentStart {
        software: software::Software,
        journal: mpsc::Sender<journal::Request>,
        callback: oneshot::Sender<Result<()>>
    },
    ExperimentStop,
}

pub type Sender = mpsc::Sender<Request>;
pub type Receiver = mpsc::Receiver<Request>;

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
    #[serde(rename = "Start camera stream")]
    StartCameraStream,
    #[serde(rename = "Stop camera stream")]
    StopCameraStream,
    #[serde(rename = "Get kernel messages")]
    GetKernelMessages,
    #[serde(rename = "Identify")]
    Identify,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Operation timed out")]
    Timeout,
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
    FetchError(#[from] reqwest::Error),
    #[error(transparent)]
    SoftwareError(#[from] software::Error),
    #[error(transparent)]
    IoError(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub async fn identify(device: Arc<fernbedienung::Device>) -> Result<()> {
    let identify_script = include_bytes!("../../scripts/drone_identify.sh");
    device.upload("/tmp".into(), "drone_identify.sh".into(), identify_script.to_vec()).await
        .map_err(|error| Error::FernbedienungError(error))?;
    let identify = fernbedienung::Process {
        target: "sh".into(),
        working_dir: Some("/tmp".into()),
        args: vec!["drone_identify.sh".to_owned()],
    };
    let (terminate_tx, terminate_rx) = oneshot::channel();
    tokio::try_join!(
        device.run(identify, Some(terminate_rx), None, None, None),
        async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            terminate_tx.send(()).map_err(|_| fernbedienung::Error::RequestError)
        }
    )?;
    Ok(())
}

pub async fn poll_upcore_devices(device: Arc<fernbedienung::Device>) -> Result<Vec<(String, String)>> {
    tokio::time::sleep(Duration::from_secs(1)).await;
    let query_devices_script = include_bytes!("../../scripts/drone_query_devices.sh");
    device.upload("/tmp".into(), "drone_query_devices.sh".into(), query_devices_script.to_vec()).await
        .map_err(|error| Error::FernbedienungError(error))?;
    let query_hubs = fernbedienung::Process {
        target: "sh".into(),
        working_dir: Some("/tmp".into()),
        args: vec!["drone_query_devices.sh".to_owned()],
    };
    let (stdout_tx, stdout_rx) = mpsc::channel(8);
    let stdout_stream = ReceiverStream::new(stdout_rx);
    let (_, stdout) = tokio::try_join!(
        device.run(query_hubs, None, None, Some(stdout_tx), None),
        stdout_stream.concat().map(fernbedienung::Result::Ok)
    )?;
    let result = String::from_utf8(stdout.to_vec())
        .map_err(|_| Error::FernbedienungError(fernbedienung::Error::DecodeError))?
        .lines()
        .map(|device| {
            let mut device = device.split_whitespace();
            let left = match device.next() {
                Some(left) => left.to_owned(),
                None => String::new()
            };
            let right = match device.next() {
                Some(right) => right.to_owned(),
                None => String::new()
            };
            (left, right)
        }).collect::<Vec<_>>();
    Ok(result)
}

pub async fn poll_upcore_link_strength(fernbedienung: Arc<fernbedienung::Device>) -> Result<i32> {
    tokio::time::sleep(Duration::from_secs(1)).await;
    tokio::time::timeout(Duration::from_secs(1), fernbedienung.link_strength()).await
        .map_err(|_| Error::Timeout)
        .and_then(|inner| inner.map_err(|error| Error::FernbedienungError(error)))
}

async fn poll_xbee_link_margin(xbee: &xbee::Device) -> Result<i32> {
    tokio::time::sleep(Duration::from_secs(1)).await;
    xbee.link_margin().await.map_err(|error| Error::XbeeError(error))
}

// To answers to cleaning this up, either use a futures unordered or refactor all of the
// individual handlers into their own async functions.
// the futures unordered solution sounds nice, but all futures have to have the same type,
// further more, they would have to be the same async function or use dynamic dispatch since
// futures generated by different async expressions are always different futures
// also, putting everything inside an futuresunordered makes it different 

pub async fn new(mut rx: Receiver) {
    // /* initialize the xbee pins and mux */
    // if let Err(error) = init(&xbee).await {
    //     log::error!("Drone {}: failed to initialize Xbee: {}", error);
    // }
    
    // /* try to connect to the xbee scs for one second, using a pending stream on failure */
    // let mavlink_connect = TcpStream::connect((xbee.addr, 9750));
    // let mavlink_connect_timeout = Duration::from_secs(1);
    // let mavlink_connect_result = tokio::time::timeout(mavlink_connect_timeout, mavlink_connect).await
    //     .map_err(|inner| std::io::Error::new(std::io::ErrorKind::TimedOut, inner))
    //     .and_then(|inner| inner);
    // let mut mavlink = match mavlink_connect_result {
    //     Ok(stream) => {
    //         FramedRead::new(stream, codec::MavMessageDecoder::<mavlink::common::MavMessage>::new())
    //             .left_stream()
    //     },
    //     Err(error) => {
    //         log::warn!("Drone {}: failed to connect to the Xbee serial communication service: {}", uuid, error);
    //         futures::stream::pending()
    //             .right_stream()
    //     }
    // };

    // let mut fernbedienung: Option<Arc<fernbedienung::Device>> = None;
    // let poll_upcore_link_strength_task = future::pending().left_future();
    // tokio::pin!(poll_upcore_link_strength_task);
    // let mut upcore_link_strength = -100;

    // let poll_upcore_devices_task = future::pending().left_future();
    // tokio::pin!(poll_upcore_devices_task);
    // let mut upcore_devices = Vec::new();

    // let mut argos_stop_tx = None;
    // let argos_task = future::pending().left_future();
    // tokio::pin!(argos_task);

    // let mut kernel_messages = None;

    // let identify_task = future::pending().left_future();
    // tokio::pin!(identify_task);

    // let poll_xbee_link_margin_task = poll_xbee_link_margin(&xbee);
    // tokio::pin!(poll_xbee_link_margin_task);
    // let mut xbee_link_margin = 0;

    // let mut upcore_camera_stream_stop_tx = None;
    // let mut upcore_camera_stream = futures::stream::pending().left_stream();
    // let upcore_camera_task = futures::future::pending().left_future();
    // tokio::pin!(upcore_camera_task);
    // let mut upcore_camera_frames = Vec::new();

    // let mut battery_remaining = -1i8;

    // loop {
    //     tokio::select! {
    //         Some(frames) = upcore_camera_stream.next() => {
    //             upcore_camera_frames = frames;
    //         },
    //         Some(message) = mavlink.next() => {
    //             if let Ok((_, mavlink::common::MavMessage::BATTERY_STATUS(data))) = message {
    //                 /* voltages: [u16; 10] Battery voltage of cells 1 to 10 in mV. If individual
    //                    cell voltages are unknown or not measured for this battery, then the overall
    //                    battery voltage should be filled in cell 0. */
    //                 let mut battery_reading = data.voltages[0] as f32;
    //                 battery_reading /= DRONE_BATT_NUM_CELLS;
    //                 battery_reading -= DRONE_BATT_EMPTY_MV;
    //                 battery_reading /= DRONE_BATT_FULL_MV - DRONE_BATT_EMPTY_MV;
    //                 battery_remaining = (battery_reading.max(0.0).min(1.0) * 100.0) as i8;
    //             }
    //         },
    //         result = &mut identify_task => {
    //             if let Err(error) = result {
    //                 log::warn!("Identify task returned an error: {}", error);
    //             }
    //             identify_task.set(future::pending().left_future())
    //         }
    //         result = &mut poll_upcore_devices_task => {
    //             poll_upcore_devices_task.set(match fernbedienung {
    //                 Some(ref device) => poll_upcore_devices(device.clone()).right_future(),
    //                 None => future::pending().left_future(),
    //             });
    //             match result {
    //                 Ok(devices) => upcore_devices = devices,
    //                 Err(error) => log::warn!("Could not poll devices: {}", error),
    //             }
    //         },
    //         result = &mut poll_xbee_link_margin_task => match result {
    //             Ok(link_margin) => {
    //                 xbee_link_margin = link_margin;
    //                 poll_xbee_link_margin_task.set(poll_xbee_link_margin(&xbee));
    //             }
    //             Err(error) => {
    //                 log::warn!("Xbee on drone {}: {}", uuid, error);
    //                 /* disconnect here if the upcore is offline, otherwise try to reestablish
    //                    the connection with xbee */
    //                 match fernbedienung {
    //                     Some(_) => poll_xbee_link_margin_task.set(poll_xbee_link_margin(&xbee)),
    //                     None => break,
    //                 }
    //             }
    //         },
    //         result = &mut poll_upcore_link_strength_task => match result {
    //             Ok(link_strength) => {
    //                 upcore_link_strength = link_strength;
    //                 poll_upcore_link_strength_task.set(match fernbedienung {
    //                     Some(ref device) => poll_upcore_link_strength(device.clone()).right_future(),
    //                     None => future::pending().left_future(),
    //                 });
    //             }
    //             Err(error) => {
    //                 log::warn!("UP Core on drone {}: {}", uuid, error);
    //                 /* TODO consider this as a disconnection scenario */
    //                 fernbedienung = None;
    //                 poll_upcore_link_strength_task.set(future::pending().left_future());
    //                 poll_upcore_devices_task.set(future::pending().left_future());
    //                 upcore_devices.clear();
    //                 upcore_camera_frames.clear();
    //             }
    //         },
    //         /* if ARGoS is running, keep forwarding stdout/stderr  */
    //         argos_result = &mut argos_task => {
    //             argos_stop_tx = None;
    //             argos_task.set(futures::future::pending().left_future());
    //             log::info!("ARGoS terminated with {:?}", argos_result);
    //         },
    //         /* clean up for when the streaming process terminates */
    //         upcore_camera_result = &mut upcore_camera_task => {
    //             upcore_camera_stream = futures::stream::pending().left_stream();
    //             upcore_camera_task.set(futures::future::pending().left_future());
    //             upcore_camera_frames.clear();
    //             match upcore_camera_result {
    //                 /* since we use the terminate signal to shutdown mjpg_streamer, report
    //                    AbnormalTerminationError as not an error */
    //                 Ok(_) | Err(Error::FernbedienungError(fernbedienung::Error::AbnormalTerminationError)) =>
    //                     log::info!("Camera stream stopped"),
    //                 Err(error) =>
    //                     log::warn!("Camera stream stopped: {}", error),
    //             }
    //         },
    //         recv_request = rx.recv() => match recv_request {
    //             None => break, /* tx handle dropped, exit the loop */
    //             Some(request) => match request {
    //                 Request::GetState(callback) => {
    //                     /* generate a vector of valid actions */
    //                     let mut actions = xbee_actions(&xbee).await;
    //                     if fernbedienung.is_some() {
    //                         actions.push(Action::UpCoreReboot);
    //                         actions.push(Action::UpCoreHalt);
    //                         actions.push(match *upcore_camera_task {
    //                             Either::Left(_) => Action::StartCameraStream,
    //                             Either::Right(_) => Action::StopCameraStream,
    //                         });
    //                         actions.push(Action::GetKernelMessages);
    //                         actions.push(Action::Identify);
    //                     }
    //                     /* send back the state */
    //                     let state = State {
    //                         xbee: (xbee.addr, xbee_link_margin),
    //                         upcore: fernbedienung.as_ref().map(|dev| (dev.addr, upcore_link_strength)),
    //                         battery_remaining: battery_remaining,
    //                         cameras: upcore_camera_frames.clone(),
    //                         devices: upcore_devices.clone(),
    //                         kernel_messages: kernel_messages.take(),
    //                         actions,
    //                     };
    //                     let _ = callback.send(state);
    //                 },
    //                 Request::Pair(device) => {
    //                     let device = Arc::new(device);
    //                     poll_upcore_link_strength_task.set(poll_upcore_link_strength(device.clone()).right_future());
    //                     poll_upcore_devices_task.set(poll_upcore_devices(device.clone()).right_future());
    //                     fernbedienung = Some(device);
    //                 },
    //                 Request::Execute(action) => {
    //                     let result = match action {
    //                         Action::UpCorePowerOn => set_upcore_power(&xbee, true).await,
    //                         Action::UpCorePowerOff => set_upcore_power(&xbee, false).await,
    //                         Action::PixhawkPowerOn => set_pixhawk_power(&xbee, true).await,
    //                         Action::PixhawkPowerOff => set_pixhawk_power(&xbee, false).await,
    //                         Action::UpCoreReboot => match fernbedienung {
    //                             Some(ref device) => device.reboot().await
    //                                 .map_err(|error| Error::FernbedienungError(error)),
    //                             None => Err(Error::InvalidAction(action)),
    //                         },
    //                         Action::UpCoreHalt => match fernbedienung {
    //                             Some(ref device) => device.halt().await
    //                                 .map_err(|error| Error::FernbedienungError(error)),
    //                             None => Err(Error::InvalidAction(action)),
    //                         },
    //                         Action::GetKernelMessages => match fernbedienung {
    //                             Some(ref device) => {
    //                                 match device.kernel_messages().await {
    //                                     Ok(messages) => {
    //                                         kernel_messages = Some(messages);
    //                                         Ok(())
    //                                     },
    //                                     Err(error) => Err(Error::FernbedienungError(error))
    //                                 }
    //                             }
    //                             None => Err(Error::InvalidAction(action)),
    //                         }
    //                         Action::Identify => match fernbedienung {
    //                             Some(ref device) => {
    //                                 identify_task.set(identify(device.clone()).right_future());
    //                                 Ok(())
    //                             },
    //                             None => Err(Error::InvalidAction(action)),
    //                         },
    //                         Action::StartCameraStream => {
    //                             if let Either::Left(_) = *upcore_camera_task {
    //                                 match fernbedienung {
    //                                     Some(ref device) => {
    //                                         let (task, stop_tx, stream_rx) =
    //                                             handle_stream_start(device.clone(), DRONE_CAMERAS_CONFIG);
    //                                         upcore_camera_stream_stop_tx = Some(stop_tx);
    //                                         upcore_camera_stream = ReceiverStream::new(stream_rx).right_stream();
    //                                         upcore_camera_task.set(task.right_future());
    //                                         log::info!("Camera stream started");
    //                                         Ok(())
    //                                     }
    //                                     None => Err(Error::InvalidAction(action)),
    //                                 }
    //                             }
    //                             else {
    //                                 Err(Error::InvalidAction(action))
    //                             }
    //                         },
    //                         Action::StopCameraStream => {
    //                             if let Either::Right(_) = *upcore_camera_task {
    //                                 if let Some(stop_tx) = upcore_camera_stream_stop_tx.take() {
    //                                     let _ = stop_tx.send(());
    //                                 }
    //                             }
    //                             Ok(())
    //                         }
    //                     };
    //                     if let Err(error) = result {
    //                         log::warn!("Could not execute {:?}: {}", action, error);
    //                     }
    //                 },
    //                 Request::GetId(callback) => {
    //                     /* just drop callback if reading the id failed */
    //                     if let Ok(id) = get_id(&xbee).await {
    //                         let _ = callback.send(id);
    //                     }
    //                 },
    //                 Request::ExperimentStart{software, journal, callback} => {
    //                     match fernbedienung.as_ref() {
    //                         None => {
    //                             let _ = callback.send(Err(Error::RequestError));
    //                         },
    //                         Some(device) => {
    //                             match handle_experiment_start(uuid, device.clone(), software, journal).await {
    //                                 Ok((argos, stop_tx)) => {
    //                                     argos_task.set(argos.right_future());
    //                                     argos_stop_tx = Some(stop_tx);
    //                                     let _ = callback.send(Ok(()));
    //                                 },
    //                                 Err(error) => {
    //                                     let _ = callback.send(Err(error));
    //                                 }
    //                             }
    //                         }
    //                     }
    //                 },
    //                 Request::ExperimentStop => {
    //                     if let Some(stop_tx) = argos_stop_tx.take() {
    //                         let _ = stop_tx.send(());
    //                     }
    //                     /* poll argos to completion */
    //                     let result = (&mut argos_task).await;
    //                     log::info!("ARGoS terminated with {:?}", result);
    //                     argos_task.set(futures::future::pending().left_future());
    //                 },
    //             }
    //         }
    //     }
    // }
    // uuid
    while let Some(_) = rx.recv().await {}
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

async fn init(xbee: &xbee::Device) -> Result<()> {
    /* set pin modes */
    let pin_modes = vec![
        /* UART pins: TX: DOUT, RTS: DIO6, RX: DIN, CTS: DIO7 */
        /* hardware flow control connected but disabled */
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
    xbee.set_pin_modes(pin_modes).await?;
    /* configure mux (upcore controls pixhawk) */
    xbee.write_outputs(vec![(xbee::Pin::DIO4, true)]).await?;

    /* set the serial communication service to TCP mode */
    xbee.set_scs_mode(true).await?;
    /* set the baud rate to match the baud rate of the Pixhawk */
    xbee.set_baud_rate(115200).await?;
    Ok(())
}

pub async fn set_upcore_power(xbee: &xbee::Device, enable: bool) -> Result<()> {
    xbee.write_outputs(vec![(xbee::Pin::DIO11, enable)]).await
        .map_err(|error| Error::XbeeError(error))
}

pub async fn set_pixhawk_power(xbee: &xbee::Device, enable: bool) -> Result<()> {
    xbee.write_outputs(vec![(xbee::Pin::DIO12, enable)]).await
        .map_err(|error| Error::XbeeError(error))
}

fn handle_stream_start(device: Arc<fernbedienung::Device>, configs: &'static [(&str, u16, u16, u16)])
    -> (impl Future<Output = Result<()>>, oneshot::Sender<()>, mpsc::Receiver<Vec<Bytes>>) {
    let (mut processes, mut stop_txs) : (FuturesUnordered<_>, HashMap<_,_>)  = configs.iter()
        .map(|&(camera, width, height, port)| {
            let device = device.clone();
            let process_request = fernbedienung::Process {
                target: "mjpg_streamer".into(),
                working_dir: None,
                args: vec![
                    "-i".to_owned(),
                    format!("input_uvc.so -d {} -r {}x{} -n", camera, width, height),
                    "-o".to_owned(),
                    format!("output_http.so -p {} -l {}", port, device.addr)
                ],
            };
            let (stop_tx, stop_rx) = oneshot::channel::<()>();
            let process = async move {
                (camera, device.run(process_request, Some(stop_rx), None, None, None)
                    .map_err(|e| Error::FernbedienungError(e)).await)
            };
            (process, (camera, stop_tx))
        }).unzip();
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    let (stream_tx, stream_rx) = mpsc::channel::<Vec<Bytes>>(2);
    let task = async move {
        let mut instances = configs.iter()
            .map(|&(camera, _, _, port)| (camera, port))
            .collect::<HashMap<_,_>>();
        /* start mjpg_stream */
        loop {
            tokio::select! {
                Some((camera, _)) = processes.next() => {
                    instances.remove(camera);
                    stop_txs.remove(camera);
                    log::warn!("Streaming from {} failed", camera);
                },
                _ = tokio::time::sleep(Duration::from_secs(1)) => break
            }
        }
        /* poll for frames */
        loop {
            let reqwest_frames = instances.iter()
                .map(|(_, port)| {
                    reqwest::get(format!("http://{}:{}/?action=snapshot", device.addr, port))
                        .and_then(|response| response.bytes())
                })
                .collect::<FuturesOrdered<_>>()
                .try_collect::<Vec<_>>();
            tokio::select! {
                Some((camera, _)) = processes.next() => {
                    instances.remove(camera);
                    stop_txs.remove(camera);
                    log::info!("Streaming from {} terminated", camera);
                },
                _ = &mut stop_rx => {
                    break stop_txs.into_iter()
                        .map(|(_, stop_tx)| stop_tx.send(()).map_err(|_| Error::RequestError))
                        .collect::<Result<Vec<_>>>()
                        .map(|_| ())
                },
                reqwest_result = reqwest_frames => match reqwest_result {
                    Ok(frames) => {
                        if let Err(_) = stream_tx.send(frames).await {
                            break Ok(());
                        }
                    },
                    Err(error) => break Err(Error::FetchError(error)),
                },
            }
        }
    };
    (task, stop_tx, stream_rx)
}

// async fn handle_experiment_start(uuid: Uuid,
//                                  device: Arc<fernbedienung::Device>,
//                                  software: software::Software,
//                                  journal: mpsc::Sender<journal::Request>) 
//     -> Result<(impl Future<Output = fernbedienung::Result<()>>, oneshot::Sender<()>)> {
//     /* extract the name of the config file */
//     let (argos_config, _) = software.argos_config()?;
//     let argos_config = argos_config.to_owned();
//     /* get the relevant ip address of this machine */
//     let message_router_addr = async {
//         let socket = UdpSocket::bind("0.0.0.0:0").await?;
//         socket.connect((device.addr, 80)).await?;
//         socket.local_addr().map(|mut socket| {
//             socket.set_port(4950);
//             socket
//         })
//     }.await?;

//     /* upload the control software */
//     let software_upload_path = device.create_temp_dir()
//         .map_err(|error| Error::FernbedienungError(error))
//         .and_then(|path: String| software.0.into_iter()
//             .map(|(filename, contents)| {
//                 let path = PathBuf::from(&path);
//                 let filename = PathBuf::from(&filename);
//                 device.upload(path, filename, contents)
//             })
//             .collect::<FuturesUnordered<_>>()
//             .map_err(|error| Error::FernbedienungError(error))
//             .try_collect::<Vec<_>>()
//             .map_ok(|_| path)
//         ).await?;

//     /* create a remote instance of ARGoS3 */
//     let process = fernbedienung::Process {
//         target: "argos3".into(),
//         working_dir: Some(software_upload_path.into()),
//         args: vec![
//             "--config".to_owned(), argos_config.to_owned(),
//             "--pixhawk".to_owned(), "/dev/ttyS1:921600".to_owned(),
//             "--router".to_owned(), message_router_addr.to_string(),
//             "--id".to_owned(), uuid.to_string(),
//         ],
//     };

//     /* channel for terminating ARGoS */
//     let (stop_tx, stop_rx) = oneshot::channel();

//     /* create future for running ARGoS */
//     let argos_task_future = async move {
//         /* channels for routing stdout and stderr to the journal */
//         let (stdout_tx, mut stdout_rx) = mpsc::channel(8);
//         let (stderr_tx, mut stderr_rx) = mpsc::channel(8);
//         /* run argos remotely */
//         let argos = device.run(process, Some(stop_rx), None, Some(stdout_tx), Some(stderr_tx));
//         tokio::pin!(argos);
//         loop {
//             tokio::select! {
//                 Some(data) = stdout_rx.recv() => {
//                     let message = journal::Robot::StandardOutput(data);
//                     let event = journal::Event::Robot(uuid, message);
//                     let request = journal::Request::Record(event);
//                     if let Err(error) = journal.send(request).await {
//                         log::warn!("Could not forward standard output of {} to journal: {}", uuid, error);
//                     }
//                 },
//                 Some(data) = stderr_rx.recv() => {
//                     let message = journal::Robot::StandardError(data);
//                     let event = journal::Event::Robot(uuid, message);
//                     let request = journal::Request::Record(event);
//                     if let Err(error) = journal.send(request).await {
//                         log::warn!("Could not forward standard error of {} to journal: {}", uuid, error);
//                     }
//                 },
//                 exit_status = &mut argos => break exit_status,
//             }
//         }
//     };
//     Ok((argos_task_future, stop_tx))
// }
