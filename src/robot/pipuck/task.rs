use std::{net::SocketAddr, time::Duration};
use anyhow::Context;
use bytes::BytesMut;
use tokio::{net::UdpSocket, sync::{broadcast, mpsc, oneshot}};
use futures::{FutureExt, Stream, StreamExt, TryStreamExt};
use tokio_stream::{self, wrappers::ReceiverStream};
use tokio_util::sync::PollSender;

use crate::network::{fernbedienung, fernbedienung_ext::MjpegStreamerStream};
use crate::robot::{FernbedienungAction, TerminalAction};
use crate::journal;

pub use shared::{
    pipuck::{Descriptor, Update},
    experiment::software::Software
};

const IDENTIFY_PIPUCK_ARGOS: (&'static str, &'static [u8]) = 
    ("identify_pipuck.argos", include_bytes!("identify_pipuck.argos"));
const IDENTIFY_PIPUCK_LUA: (&'static str, &'static [u8]) = 
    ("identify_pipuck.lua", include_bytes!("identify_pipuck.lua"));

const PIPUCK_CAMERAS_CONFIG: &[(&str, u16, u16, u16)] = &[];

#[derive(Debug)]
pub enum Action {
    AssociateFernbedienung(fernbedienung::Device),
    ExecuteFernbedienungAction(oneshot::Sender<anyhow::Result<()>>, FernbedienungAction),
    Subscribe(oneshot::Sender<broadcast::Receiver<Update>>),
    // its good to keep this one seperate since start exp need to interact with xbee and fernbedienung
    SetupExperiment(oneshot::Sender<anyhow::Result<()>>, String, Software, mpsc::Sender<journal::Action>),
    StartExperiment(oneshot::Sender<anyhow::Result<()>>),
    StopExperiment,
}

pub type Sender = mpsc::Sender<Action>;
pub type Receiver = mpsc::Receiver<Action>;

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
                log::info!("{:?}", update);
                let _ = updates_tx.send(update);
            },
            Some(stderr) = stderr.next() => {
                let update = Update::Bash(String::from_utf8_lossy(&stderr).into_owned());
                log::info!("{:?}", update);
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
    journal: impl Into<Option<mpsc::Sender<journal::Action>>>,
    wait_rx: impl Into<Option<oneshot::Receiver<()>>>,
    stop_rx: oneshot::Receiver<()>,
) {
    let id = id.into();
    let router_socket = router_socket.into();
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
                            for &(camera, width, height, port) in PIPUCK_CAMERAS_CONFIG {
                                let stream = MjpegStreamerStream::new(&device, camera, width, height, port);
                                let stream = tokio_stream::StreamExt::throttle(stream, Duration::from_millis(200));
                                cameras_stream.insert(camera.to_owned(), Box::pin(stream));
                            }
                        }
                        let _ = callback.send(Ok(()));
                    },
                    FernbedienungAction::Halt => {
                        let result = device.halt().await
                            .context("Could not halt Raspberry Pi");
                        let _ = callback.send(result);
                    },
                    FernbedienungAction::Reboot => {
                        let result = device.reboot().await
                            .context("Could not reboot Raspberry Pi");
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
                        },
                        None => {
                            let _ = callback.send(Err(anyhow::anyhow!("Experiment has not been set up")));
                        }
                    },
                    FernbedienungAction::Identify => match argos_stop_tx.as_ref() {
                        Some(_) => {
                            let _ = callback.send(Err(anyhow::anyhow!("ARGoS is already running")));
                        }
                        None => {
                            let software = Software(vec![
                                (IDENTIFY_PIPUCK_ARGOS.0.to_owned(), IDENTIFY_PIPUCK_ARGOS.1.to_vec()),
                                (IDENTIFY_PIPUCK_LUA.0.to_owned(), IDENTIFY_PIPUCK_LUA.1.to_vec())
                            ]);
                            match software.check_config() {
                                Err(error) => {
                                    let _ = callback.send(Err(error).context("Identify software error"));
                                }
                                Ok(_) => {
                                    let (start_tx, start_rx) = oneshot::channel();
                                    start_tx.send(()).unwrap();
                                    let (stop_tx, stop_rx) = oneshot::channel();
                                    let task = argos(&device, callback, software, None, None, None, start_rx, stop_rx);
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
    /* updates_tx is for sending changes in state to subscribers (e.g., the webui) */
    let (updates_tx, _) = broadcast::channel(16);
    

    // TODO: for a clean shutdown we may want to consider the case where updates_tx hangs up
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
                    if let Err(error) = terminate_argos.await {
                        log::error!("{}", error);
                    }
                },
            },
            _ = &mut fernbedienung_task => {
                fernbedienung_tx = None;
                fernbedienung_addr = None;
                fernbedienung_task.set(futures::future::pending().left_future());
                let _ = updates_tx.send(Update::FernbedienungDisconnected);
            },
        }
    }
}