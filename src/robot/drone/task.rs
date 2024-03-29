use std::{collections::HashMap, net::SocketAddr, sync::atomic::{AtomicU8, Ordering}, time::Duration};
use anyhow::Context;
use ansi_parser::{Output, AnsiParser};
use bytes::BytesMut;
use mavlink::{MavHeader, common::{self, MavMessage, SerialControlDev, SerialControlFlag}, error::MessageReadError};
use tokio::{net::{TcpStream, UdpSocket}, sync::{broadcast, mpsc, oneshot}};
use futures::{FutureExt, Sink, SinkExt, Stream, StreamExt, TryStreamExt};
use tokio_stream::{self, wrappers::ReceiverStream};
use tokio_util::{codec::Framed, sync::PollSender};

use crate::network::{fernbedienung, fernbedienung_ext::MjpegStreamerStream, xbee};
use crate::robot::{FernbedienungAction, XbeeAction, TerminalAction};
use crate::journal;
use super::codec;

pub use shared::{
    drone::{Descriptor, Update},
    experiment::software::Software
};

const IDENTIFY_DRONE_ARGOS: (&'static str, &'static [u8]) = 
    ("identify_drone.argos", include_bytes!("identify_drone.argos"));
const IDENTIFY_DRONE_LUA: (&'static str, &'static [u8]) = 
    ("identify_drone.lua", include_bytes!("identify_drone.lua"));

const DRONE_BATT_FULL_MV: f32 = 4050.0;
const DRONE_BATT_EMPTY_MV: f32 = 3500.0;
const DRONE_BATT_NUM_CELLS: f32 = 3.0;
const DRONE_CAMERAS_CONFIG: &[(&str, u16, u16, u16)] = &[
    ("/dev/camera0", 1024, 768, 8000),
    ("/dev/camera1", 1024, 768, 8001),
    ("/dev/camera2", 1024, 768, 8002),
    ("/dev/camera3", 1024, 768, 8003),
];

const PIXHAWK_PORT: &'static str = "/dev/ttyS1:921600";

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

#[derive(Debug)]
pub enum Action {
    AssociateFernbedienung(fernbedienung::Device),
    AssociateXbee(xbee::Device),
    ExecuteXbeeAction(oneshot::Sender<anyhow::Result<()>>, XbeeAction),
    ExecuteFernbedienungAction(oneshot::Sender<anyhow::Result<()>>, FernbedienungAction),
    Subscribe(oneshot::Sender<broadcast::Receiver<Update>>),
    // its good to keep this one seperate since start exp need to interact with xbee and fernbedienung
    SetupExperiment(oneshot::Sender<anyhow::Result<()>>, String, Software, mpsc::Sender<journal::Action>),
    StartExperiment(oneshot::Sender<anyhow::Result<()>>),
    StopExperiment,
}

pub type Sender = mpsc::Sender<Action>;
pub type Receiver = mpsc::Receiver<Action>;

async fn mavlink<'dev>(
    device: &'dev xbee::Device
) -> anyhow::Result<impl Stream<Item = Result<(MavHeader, MavMessage), MessageReadError>> + Sink<MavMessage> + 'dev> {
    /* set the baud rate to match the baud rate of the Pixhawk */
    device.set_baud_rate(921600).await
        .context("Could not set serial baud rate")?;
    /* set the serial communication service to TCP mode */
    device.set_scs_mode(true).await
        .context("Could not enable serial communication service")?;
    /* try to connect */
    let connection = TcpStream::connect((device.addr, 9750))
        .map(|result| result
            .context("Could not connect to serial communication service"));
    let connection = tokio::time::timeout(Duration::from_secs(1), connection)
        .map(|result| result
            .context("Timeout while connecting to serial communication service")
            .and_then(|result| result)).await?;
    let framed = Framed::new(connection, codec::MavMessageCodec::<MavMessage>::new());
    /* automatically add headers to outbound mavlink messages */
    let mavlink_sequence = AtomicU8::new(0);
    let framed = framed.with(move |message| {
        let sequence = mavlink_sequence.fetch_add(1, Ordering::Relaxed);
        async move {
            let header = MavHeader {
                system_id: 255,
                component_id: 0,
                sequence
            };
            anyhow::Result::<_>::Ok((header, message))
        }
    });
    Ok(framed)
}

fn xbee_pin_states_stream<'dev>(
    device: &'dev xbee::Device
) -> impl Stream<Item = anyhow::Result<HashMap<xbee::Pin, bool>>> + 'dev {
    async_stream::stream! {
        let mut attempts: u8 = 0;
        loop {
            let link_margin_task = tokio::time::timeout(Duration::from_millis(1000), device.pin_states()).await
                .context("Timeout while communicating with Xbee")
                .and_then(|result| result.context("Could not communicate with Xbee"));
            match link_margin_task {
                Ok(response) => {
                    attempts = 0;
                    yield Ok(response);
                },
                Err(error) => match attempts {
                    0..=4 => attempts += 1,
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
            let link_margin_task = tokio::time::timeout(Duration::from_millis(1000), device.link_margin()).await
                .context("Timeout while communicating with Xbee")
                .and_then(|result| result.context("Xbee communication error"));
            match link_margin_task {
                Ok(response) => {
                    attempts = 0;
                    yield Ok(response);
                },
                Err(error) => match attempts {
                    0..=4 => attempts += 1,
                    _ => yield Err(error)
                }
            }
        }
    }
}

async fn xbee(
    device: xbee::Device,
    mut rx: mpsc::Receiver<(oneshot::Sender<anyhow::Result<()>>, XbeeAction)>,
    updates_tx: broadcast::Sender<Update>
) -> anyhow::Result<()> {
    /* autonomous mode: this variable tracks whether or not we are in autonomous mode */
    let mut autonomous_mode = false;
    /* mavlink sink and stream */
    let (mut mavlink_sink, mut mavlink_stream) = mavlink(&device).await
        .context("Could not connect to MAVLink")?
        .split();
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
    /* since we may be just reconnecting to the xbee, do not turn off the upcore and
       pixhawk power if they are currently switched on */
    if let Some(Ok(pin_states)) = pin_states_stream_throttled.next().await {
        /* initialise autonomous mode based on current pin states */
        autonomous_mode =
            pin_states.get(&xbee::Pin::DIO4).cloned().unwrap_or_default();
        let upcore_power =
            pin_states.get(&xbee::Pin::DIO11).cloned().unwrap_or_default();
        let pixhawk_power =
            pin_states.get(&xbee::Pin::DIO12).cloned().unwrap_or_default();
        let pin_modes = XBEE_DEFAULT_PIN_CONFIG.iter()
            .filter(|&(pin, _)| match pin {
                /* if a pin is already set to true, then it should be removed from
                   the default pin configuration */
                xbee::Pin::DIO4 => !autonomous_mode,
                xbee::Pin::DIO11 => !upcore_power,
                xbee::Pin::DIO12 => !pixhawk_power,
                _ => true,
            });
        device.set_pin_modes(pin_modes).await
            .context("Could not set Xbee pin modes")?;
    }
    else {
        device.set_pin_modes(XBEE_DEFAULT_PIN_CONFIG.into_iter()).await
            .context("Could not set Xbee pin modes")?;
    }
    /* mavlink heartbeat stream */
    let mavlink_heartbeat_stream = futures::stream::iter(std::iter::repeat(
        MavMessage::HEARTBEAT(common::HEARTBEAT_DATA {
            custom_mode: 0,
            mavtype: common::MavType::MAV_TYPE_GENERIC,
            autopilot: common::MavAutopilot::MAV_AUTOPILOT_INVALID,
            base_mode: common::MavModeFlag::empty(),
            system_status: common::MavState::MAV_STATE_UNINIT,
            mavlink_version: 3,
        })
    ));
    let mavlink_heartbeat_stream_throttled =
        tokio_stream::StreamExt::throttle(mavlink_heartbeat_stream, Duration::from_millis(500));
    tokio::pin!(mavlink_heartbeat_stream_throttled);
    /* poll all streams, sinks, channels, and futures */
    loop {
        tokio::select! {
            Some(heartbeat) = mavlink_heartbeat_stream_throttled.next() => {
                /* only send heartbeats if we are not in autonomous mode */
                if !autonomous_mode {
                    let _ = mavlink_sink.send(heartbeat).await;
                }
            },
            Some(Ok((_header, body))) = mavlink_stream.next() => match body {
                MavMessage::BATTERY_STATUS(data) => {
                    let mut battery_reading = data.voltages[0] as f32;
                    battery_reading /= DRONE_BATT_NUM_CELLS;
                    battery_reading -= DRONE_BATT_EMPTY_MV;
                    battery_reading /= DRONE_BATT_FULL_MV - DRONE_BATT_EMPTY_MV;
                    let battery_reading = (battery_reading.max(0.0).min(1.0) * 100.0) as i32;
                    let _ = updates_tx.send(Update::Battery(battery_reading));
                },
                MavMessage::SERIAL_CONTROL(common::SERIAL_CONTROL_DATA { data, count, .. }) => {
                    let data = match std::str::from_utf8(&data[..count as usize]) {
                        Ok(data) => data,
                        Err(error) => {
                            std::str::from_utf8(&data[..error.valid_up_to()]).unwrap()
                        }
                    };
                    let parsed: String = data
                        .ansi_parse()
                        .fold(String::new(), |output, item| match item {
                            Output::TextBlock(text) => format!("{}{}", output, text),
                            Output::Escape(_) => output,
                        });
                    let _  = updates_tx.send(Update::Mavlink(parsed));
                },
                /* ignore other MAVLink messages */
                _ => {}
            },
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
                Some((callback, action)) => match action {
                    XbeeAction::SetAutonomousMode(enable) => {
                        let result = device.write_outputs(&[(xbee::Pin::DIO4, enable)]).await
                            .context("Could not configure autonomous mode");
                        /* if successful update the state of the autonomous mode variable */
                        if result.is_ok() {
                            autonomous_mode = enable;
                        }
                        let _ = callback.send(result);
                    }
                    XbeeAction::SetUpCorePower(enable) => {
                        let result = device.write_outputs(&[(xbee::Pin::DIO11, enable)]).await
                            .context("Could not configure Up Core power");
                        let _ = callback.send(result);
                    },
                    XbeeAction::SetPixhawkPower(enable) => {
                        let result = device.write_outputs(&[(xbee::Pin::DIO12, enable)]).await
                            .context("Could not configure Pixhawk power");
                        let _ = callback.send(result);
                    },
                    XbeeAction::Mavlink(action) => {
                        match autonomous_mode {
                            true => {
                                let error =
                                    anyhow::anyhow!("MAVLink terminal is not available in autonomous mode");
                                let _ = callback.send(Err(error));
                            }
                            false => match action {
                                TerminalAction::Start => {
                                    let command = vec![0x0au8];
                                    let data = common::SERIAL_CONTROL_DATA {
                                        baudrate: 0,
                                        timeout: 0,
                                        device: SerialControlDev::SERIAL_CONTROL_DEV_SHELL,
                                        flags: SerialControlFlag::SERIAL_CONTROL_FLAG_RESPOND |
                                               SerialControlFlag::SERIAL_CONTROL_FLAG_EXCLUSIVE,
                                        count: command.len() as u8,
                                        data: command,
                                    };
                                    let message = MavMessage::SERIAL_CONTROL(data);
                                    let result = mavlink_sink.send(message).await
                                        .map_err(|_| anyhow::anyhow!("Could not start MAVLink terminal"));
                                    let _ = callback.send(result);
                                },
                                TerminalAction::Run(command) => {
                                    let mut command_padded = command.as_bytes().to_vec();
                                    command_padded.push(0x0a); // add a line feed to the command
                                    let data = common::SERIAL_CONTROL_DATA {
                                        baudrate: 0,
                                        timeout: 0,
                                        device: SerialControlDev::SERIAL_CONTROL_DEV_SHELL,
                                        flags: SerialControlFlag::SERIAL_CONTROL_FLAG_RESPOND |
                                               SerialControlFlag::SERIAL_CONTROL_FLAG_EXCLUSIVE,
                                        count: command_padded.len() as u8,
                                        data: command_padded,
                                    };
                                    let message = MavMessage::SERIAL_CONTROL(data);
                                    let result = mavlink_sink.send(message).await
                                        .map_err(|_| anyhow::anyhow!("Could not run command in MAVLink terminal"));
                                    let _ = callback.send(result);
                                },
                                TerminalAction::Stop => {
                                    /* nothing to do */
                                    let _ = callback.send(Ok(()));
                                },
                            }
                        }
                    }
                },
                None => break Ok(()), // normal shutdown
            },
        }
    }
}

fn fernbedienung_link_strength_stream<'dev>(
    device: &'dev fernbedienung::Device
) -> impl Stream<Item = anyhow::Result<i32>> + 'dev {
    async_stream::stream! {
        let mut attempts : u8 = 0;
        loop {
            let link_strength_task = tokio::time::timeout(Duration::from_millis(1000), device.link_strength()).await
                .context("Timeout while communicating with Up Core")
                .and_then(|result| result.context("Could not communicate with Up Core"));
            match link_strength_task {
                Ok(response) => {
                    attempts = 0;
                    yield Ok(response);
                },
                Err(error) => match attempts {
                    0..=4 => attempts += 1,
                    _ => yield Err(error)
                }
            }
        }
    }
}

async fn bash(
    device: &fernbedienung::Device,
    mut rx: mpsc::Receiver<(oneshot::Sender<anyhow::Result<()>>, TerminalAction)>,
    updates_tx: broadcast::Sender<Update>,
) {   
    let process = futures::future::pending().left_future();
    let stdout = futures::stream::pending().left_stream();
    let stderr = futures::stream::pending().left_stream();
    let mut stdin = None;
    let mut terminate = None;
    tokio::pin!(process);
    tokio::pin!(stdout);
    tokio::pin!(stderr);
    loop {
        tokio::select! {
            Some((callback, action)) = rx.recv() => match action {
                TerminalAction::Start => {
                    /* set up channels */
                    let (stdout_tx, stdout_rx) = mpsc::channel(8);
                    stdout.set(ReceiverStream::new(stdout_rx).right_stream());
                    let (stderr_tx, stderr_rx) = mpsc::channel(8);
                    stderr.set(ReceiverStream::new(stderr_rx).right_stream());
                    let (stdin_tx, stdin_rx) = mpsc::channel(8);
                    stdin = Some(stdin_tx);
                    let (terminate_tx, terminate_rx) = oneshot::channel();
                    terminate = Some(terminate_tx);
                    /* start process */
                    let bash = fernbedienung::Process {
                        target: "bash".into(),
                        working_dir: None,
                        args: vec!["-li".to_owned()],
                    };
                    process.set(device.run(bash, terminate_rx, stdin_rx, stdout_tx, stderr_tx).right_future());
                    let _ = callback.send(Ok(()));
                },
                TerminalAction::Run(mut command) => if let Some(tx) = stdin.as_ref() {
                    command.push_str("\r");
                    let result = tx.send(BytesMut::from(command.as_bytes())).await
                        .map_err(|_| {
                            /* remove the "\r" before including the command in the error message */
                            command.pop();
                            anyhow::anyhow!("Could not send \"{}\" to Bash terminal", command)
                        });
                    let _ = callback.send(result);
                },
                TerminalAction::Stop => if let Some(tx) = terminate.take() {
                    let _ = tx.send(());
                    let _ = callback.send(Ok(()));
                }
            },
            result = &mut process => {
                process.set(futures::future::pending().left_future());
                stdout.set(futures::stream::pending().left_stream());
                stderr.set(futures::stream::pending().left_stream());
                stdin = None;
                terminate = None;
                log::info!("Remote Bash instance terminated with {:?}", result);
            }
            Some(stdout) = stdout.next() => {
                let update = Update::Bash(String::from_utf8_lossy(&stdout).into_owned());
                let _ = updates_tx.send(update);
            },
            Some(stderr) = stderr.next() => {
                let update = Update::Bash(String::from_utf8_lossy(&stderr).into_owned());
                let _ = updates_tx.send(update);
            },
        }
    }
}

async fn argos(device: &fernbedienung::Device,
    callback: oneshot::Sender<anyhow::Result<()>>,
    software: Software,
    id: impl Into<Option<String>>,
    router_socket: impl Into<Option<SocketAddr>>,
    pixhawk_port: impl Into<Option<String>>,
    journal: impl Into<Option<mpsc::Sender<journal::Action>>>,
    wait_rx: impl Into<Option<oneshot::Receiver<()>>>,
    stop_rx: oneshot::Receiver<()>,
) {
    let id = id.into();
    let router_socket = router_socket.into();
    let pixhawk_port = pixhawk_port.into();
    let journal = journal.into();
    let wait_rx = wait_rx.into();
    /* create temp directory */
    let path = match device.create_temp_dir().await {
        Ok(path) => path,
        Err(error) => {
            let result = Err(error).context("Could not create temporary directory");
            let _ = callback.send(result);
            return;
        }
    };
    /* get the name of the configuration file */
    let (config, _) = match software.argos_config() {
        Ok(config) => config,
        Err(error) => {
            let result = Err(error).context("Could not get ARGoS configuration file");
            let _ = callback.send(result);
            return;
        }
    };
    /* upload the control software */
    for (filename, contents) in software.0.iter() {
        match device.upload(&path, filename, contents.clone()).await {
            Ok(_) => continue,
            Err(error) => {
                let result = Err(error).context("Could not upload software");
                let _ = callback.send(result);
                return;
            }
        }
    }
    if let Err(_) = callback.send(Ok(())) {
        /* abort if the callback was dropped before we
           could signal that we are ready */
        return;
    }
    /* if wait_tx was provided, wait for this signal before starting ARGoS */
    tokio::pin!(stop_rx);
    if let Some(wait_rx) = wait_rx {
        tokio::pin!(wait_rx);
        tokio::select! {
            result = &mut wait_rx => match result {
                Ok(_) => {} /* proceed with running ARGoS */
                Err(_) => return, /* abort */
            },
            _ = &mut stop_rx => {
                return; /* abort */
            },
        }
    }
    /* start ARGoS */
    let mut args = vec!["--config".to_owned(), config.to_owned()];
    args.extend(router_socket.into_iter().flat_map(|socket| vec!["--router".to_owned(), socket.to_string()]));
    args.extend(id.iter().flat_map(|id| vec!["--id".to_owned(), id.clone()]));
    args.extend(pixhawk_port.into_iter().flat_map(|port| vec!["--pixhawk".to_owned(), port]));
    let process = fernbedienung::Process {
        target: "argos3".into(),
        working_dir: Some(path.into()),
        args,
    };
    let (stdout_tx, mut forward_stdout, stderr_tx, mut forward_stderr) = match (journal, id) {
        (Some(journal), Some(id)) => {
            use journal::{ARGoS, Event, Action};
            let (stdout_tx, stdout_rx) = mpsc::channel(8);
            let (stderr_tx, stderr_rx) = mpsc::channel(8);
            let stdout_stream = ReceiverStream::new(stdout_rx);
            let stderr_stream = ReceiverStream::new(stderr_rx);
            let journal_sink = PollSender::new(journal.clone());
            let stdout_robot_id = id.clone();
            let forward_stdout = stdout_stream.map(move |data: BytesMut| 
                Ok(Action::Record(Event::ARGoS(stdout_robot_id.clone(), ARGoS::StandardOutput(data)))))
                    .forward(journal_sink).right_future();
            let journal_sink = PollSender::new(journal);
            let forward_stderr = stderr_stream.map(move |data: BytesMut| 
                Ok(Action::Record(Event::ARGoS(id.clone(), ARGoS::StandardError(data)))))
                    .forward(journal_sink).right_future();
            (Some(stdout_tx), forward_stdout, Some(stderr_tx), forward_stderr)
        },
        (_, _) => {
            (None, futures::future::pending().left_future(),
             None, futures::future::pending().left_future())
        }
    };
    let (terminate_tx, terminate_rx) = oneshot::channel();      
    let argos = device.run(process, terminate_rx, None, stdout_tx, stderr_tx);
    tokio::pin!(argos);
    loop {
        tokio::select! {
            _ = &mut forward_stdout => {
                /* disable while we wait for the other futures to finish */
                forward_stdout = futures::future::pending().left_future();
            },
            _ = &mut forward_stderr => {
                /* disable while we wait for the other futures to finish */
                forward_stderr = futures::future::pending().left_future();
            },
            /* local shutdown */
            _ = &mut stop_rx => {
                let _ = terminate_tx.send(());
                break;
            }
            /* argos finished */
            _ = &mut argos => break,
        }
    }
}

async fn fernbedienung(
    device: fernbedienung::Device,
    mut rx: mpsc::Receiver<(oneshot::Sender<anyhow::Result<()>>, FernbedienungAction)>,
    updates_tx: broadcast::Sender<Update>
) {
    /* ARGos task */
    let argos_task = futures::future::pending().left_future();
    let mut argos_start_tx = Option::default();
    let mut argos_stop_tx = Option::default();
    tokio::pin!(argos_task);
    /* bash task */
    let (mut bash_tx, bash_rx) = mpsc::channel(8);
    let bash_task = bash(&device, bash_rx, updates_tx.clone());
    tokio::pin!(bash_task);
    /* link strength stream */
    let link_strength_stream = fernbedienung_link_strength_stream(&device)
        .map_ok(Update::FernbedienungSignal);
    let link_strength_stream_throttled =
        tokio_stream::StreamExt::throttle(link_strength_stream, Duration::from_millis(1000));
    tokio::pin!(link_strength_stream_throttled);
    /* camera stream */
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
                Some((callback, action)) => match action {
                    FernbedienungAction::SetCameraStream(enable) => {
                        cameras_stream.clear();
                        if enable {
                            for &(camera, width, height, port) in DRONE_CAMERAS_CONFIG {
                                let stream = MjpegStreamerStream::new(&device, camera, width, height, port);
                                let stream = tokio_stream::StreamExt::throttle(stream, Duration::from_millis(200));
                                cameras_stream.insert(camera.to_owned(), Box::pin(stream));
                            }
                        }
                        let _ = callback.send(Ok(()));
                    },
                    FernbedienungAction::Halt => {
                        let result = device.halt().await
                            .context("Could not halt Up Core");
                        let _ = callback.send(result);
                    },
                    FernbedienungAction::Reboot => {
                        let result = device.reboot().await
                            .context("Could not reboot Up Core");
                        let _ = callback.send(result);
                    },
                    /* the Bash future runs on the same task as fernbedienung, so use try_send to send messages
                       and avoid deadlock from await on a full channel */
                    FernbedienungAction::Bash(action) => if let Err(error) = bash_tx.try_send((callback, action)) {
                        let (callback, action, reason) = match error {
                            mpsc::error::TrySendError::Full((callback, action)) => (callback, action, "full"),
                            mpsc::error::TrySendError::Closed((callback, action)) => (callback, action, "closed"),
                        };
                        let error = 
                            anyhow::anyhow!("Could not send {:?} to Bash terminal: channel is {}", action, reason);
                        let _ = callback.send(Err(error));
                    },
                    FernbedienungAction::SetupExperiment(id, software, journal) => match argos_stop_tx.as_ref() {
                        Some(_) => {
                            let _ = callback.send(Err(anyhow::anyhow!("ARGoS is already setup or running")));
                        }
                        None => {
                            /* get the correct local address of the supervisor */
                            let get_local_addr = async {
                                let socket = UdpSocket::bind("0.0.0.0:0").await?;
                                socket.connect((device.addr, 80)).await?;
                                let mut local_addr = socket.local_addr()?;
                                local_addr.set_port(4950);
                                std::io::Result::<SocketAddr>::Ok(local_addr)
                            };
                            match get_local_addr.await {
                                Err(error) => {
                                    let result = Err(error).context("Could not get local address");
                                    let _ = callback.send(result);
                                }
                                Ok(local_addr) => {
                                    let (start_tx, start_rx) = oneshot::channel();
                                    let (stop_tx, stop_rx) = oneshot::channel();
                                    let task = argos(
                                        &device,
                                        callback,
                                        software,
                                        id,
                                        local_addr,
                                        PIXHAWK_PORT.to_owned(),
                                        journal,
                                        start_rx,
                                        stop_rx);
                                    argos_task.set(task.left_future().right_future());
                                    argos_start_tx = Some(start_tx);
                                    argos_stop_tx = Some(stop_tx);
                                },
                            };
                        }
                    },
                    FernbedienungAction::StartExperiment => match argos_start_tx.take() {
                        Some(start_tx) => {
                            let _ = callback.send(
                                start_tx.send(()).map_err(|_| anyhow::anyhow!("Could not start ARGoS")));
                        },
                        None => {
                            let _ = callback.send(Err(anyhow::anyhow!("Experiment has not been set up")));
                        }
                    },
                    FernbedienungAction::StopExperiment => match argos_stop_tx.take() {
                        Some(stop_tx) => {
                            let _ = callback.send(
                                stop_tx.send(()).map_err(|_| anyhow::anyhow!("Could not stop ARGoS")));
                        }
                        None => {
                            let _ = callback.send(Ok(()));
                        }
                    },
                    FernbedienungAction::Identify => match argos_stop_tx.as_ref() {
                        Some(_) => {
                            let _ = callback.send(Err(anyhow::anyhow!("ARGoS is already running")));
                        }
                        None => {
                            let software = Software(vec![
                                (IDENTIFY_DRONE_ARGOS.0.to_owned(), IDENTIFY_DRONE_ARGOS.1.to_vec()),
                                (IDENTIFY_DRONE_LUA.0.to_owned(), IDENTIFY_DRONE_LUA.1.to_vec())
                            ]);
                            match software.check_config() {
                                Err(error) => {
                                    let _ = callback.send(Err(error).context("Identify software error"));
                                }
                                Ok(_) => {
                                    let (start_tx, start_rx) = oneshot::channel();
                                    start_tx.send(()).unwrap();
                                    let (stop_tx, stop_rx) = oneshot::channel();
                                    let task = argos(&device, callback, software, None, None, None, None, start_rx, stop_rx);
                                    argos_task.set(task.right_future().right_future());
                                    argos_stop_tx = Some(stop_tx);
                                }
                            }
                        }
                    },
                },
                None => break,
            },
            _ = &mut bash_task => {
                /* restart task */
                let (tx, rx) = mpsc::channel(8);
                bash_tx = tx;
                bash_task.set(bash(&device, rx, updates_tx.clone()));
            },
            _ = &mut argos_task => {
                /* set task to pending */
                argos_task.set(futures::future::pending().left_future());
                argos_start_tx = None;
                argos_stop_tx = None;
            },
        }
    }
}

pub async fn new(mut action_rx: Receiver) {
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
    loop {
        tokio::select! {
            Some(action) = action_rx.recv() => match action {
                Action::AssociateFernbedienung(device) => {
                    let (tx, rx) = mpsc::channel(8);
                    fernbedienung_tx = Some(tx);
                    fernbedienung_addr = Some(device.addr);
                    let _ = updates_tx.send(Update::FernbedienungConnected(device.addr));
                    let task = tokio::spawn(fernbedienung(device, rx, updates_tx.clone()));
                    fernbedienung_task.set(task.right_future());
                },
                Action::AssociateXbee(device) => {
                    let (tx, rx) = mpsc::channel(8);
                    xbee_tx = Some(tx);
                    xbee_addr = Some(device.addr);
                    let _ = updates_tx.send(Update::XbeeConnected(device.addr));
                    let task = tokio::spawn(xbee(device, rx, updates_tx.clone()));
                    xbee_task.set(task.right_future());
                },
                Action::ExecuteXbeeAction(callback, action) => match xbee_tx.as_ref() {
                    Some(tx) => {
                        if let Err(mpsc::error::SendError((callback, _))) = tx.send((callback, action)).await {
                            let _ = callback.send(Err(anyhow::anyhow!("Could not communicate with Xbee task")));
                        }
                    },
                    None => {
                        let error = anyhow::anyhow!("Could not execute {:?}: Xbee is not connected.", action);
                        let _ = callback.send(Err(error));
                    }
                },
                Action::ExecuteFernbedienungAction(callback, action) => match fernbedienung_tx.as_ref() {
                    Some(tx) => {
                        if let Err(mpsc::error::SendError((callback, _))) = tx.send((callback, action)).await {
                            let _ = callback.send(Err(anyhow::anyhow!("Could not communicate with Fernbedienung task")));
                        }
                    },
                    None => {
                        let error = anyhow::anyhow!("Could not execute {:?}: Fernbedienung is not connected.", action);
                        let _ = callback.send(Err(error));
                    }
                },
                Action::Subscribe(callback) => {
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
                Action::SetupExperiment(callback, id, software, journal) => match fernbedienung_tx.as_ref() {
                    Some(tx) => {
                        let action = FernbedienungAction::SetupExperiment(id, software, journal);
                        if let Err(mpsc::error::SendError((callback, _))) = tx.send((callback, action)).await {
                            let _ = callback.send(Err(anyhow::anyhow!("Could not communicate with Fernbedienung task")));
                        }
                    }
                    None => {
                        let error = anyhow::anyhow!("Fernbedienung is not connected.");
                        let _ = callback.send(Err(error));
                    }
                },
                Action::StartExperiment(callback) => {
                    let result = async {
                        let xbee_tx = xbee_tx.as_ref()
                            .ok_or(anyhow::anyhow!("Xbee is not connected"))?;
                        let (xbee_callback_tx, xbee_callback_rx) = oneshot::channel();
                        xbee_tx.send((xbee_callback_tx, XbeeAction::SetAutonomousMode(true))).await
                            .context("Could not communicate with Xbee task")?;
                        xbee_callback_rx.await
                            .context("Xbee did not respond")??;
                        let fernbedienung_tx = fernbedienung_tx.as_ref()
                            .ok_or(anyhow::anyhow!("Fernbedienung is not connected"))?;
                        let (fernbedienung_callback_tx, fernbedienung_callback_rx) = oneshot::channel();
                        // as above with fernbedienung
                        fernbedienung_tx.send((fernbedienung_callback_tx, FernbedienungAction::StartExperiment)).await
                            .context("Could not communicate with Fernbedienung task")?;
                        fernbedienung_callback_rx.await
                            .context("Fernbedienung did not respond")??;
                        anyhow::Result::<()>::Ok(())
                    };
                    let _ = callback.send(result.await.context("Could not start experiment"));
                },
                Action::StopExperiment => {
                    let terminate_argos = async {
                        let fernbedienung_tx = fernbedienung_tx.as_ref()
                            .ok_or(anyhow::anyhow!("Fernbedienung is not connected"))?;
                        let (fernbedienung_callback_tx, fernbedienung_callback_rx) = oneshot::channel();
                        fernbedienung_tx.send((fernbedienung_callback_tx, FernbedienungAction::StopExperiment)).await
                            .context("Fernbedienung is not available")?;
                        fernbedienung_callback_rx.await
                            .context("Fernbedienung did not respond")??;
                        anyhow::Result::<()>::Ok(())
                    };
                    let disable_autonomous_mode = async {
                        let xbee_tx = xbee_tx.as_ref()
                            .ok_or(anyhow::anyhow!("Xbee is not connected"))?;
                        let (xbee_callback_tx, xbee_callback_rx) = oneshot::channel();
                        xbee_tx.send((xbee_callback_tx, XbeeAction::SetAutonomousMode(false))).await
                            .context("Xbee is not available")?;
                        xbee_callback_rx.await
                            .context("Xbee did not respond")??;
                        anyhow::Result::<()>::Ok(())
                    };
                    // !!! this logic will impact safety during experiments -- modify with caution !!!
                    // the Pixhawk is programmed to go into the off-board fail safe, so just disable autonomous
                    // mode here. Be careful that we are not sending heartbeat messages or the drone will keep
                    // flying. Using tokio::join! below we simulatenously shutdown ARGoS and disable autonomous
                    // mode.
                    let result = tokio::join!(terminate_argos, disable_autonomous_mode);
                    if let Err(error) = result.0 {
                        log::warn!("{}", error);
                    }
                    if let Err(error) = result.1 {
                        log::warn!("{}", error);
                    }
                },
            },
            _ = &mut fernbedienung_task => {
                fernbedienung_tx = None;
                fernbedienung_addr = None;
                fernbedienung_task.set(futures::future::pending().left_future());
                let _ = updates_tx.send(Update::FernbedienungDisconnected);
            },
            join_result = &mut xbee_task => {
                xbee_tx = None;
                xbee_addr = None;
                xbee_task.set(futures::future::pending().left_future());
                let _ = updates_tx.send(Update::XbeeDisconnected);
                match join_result {
                    Ok(task_result) => if let Err(error) = task_result {
                        log::warn!("xbee terminated with: {}", error);
                    }
                    Err(joint_error) => {
                        log::warn!("xbee task failed to rejoin: {}", joint_error);
                    }
                }
            }
        }
    }
}