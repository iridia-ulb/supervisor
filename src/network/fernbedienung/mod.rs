use std::fmt::Debug;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::collections::HashMap;

use bytes::BytesMut;
use macaddr::MacAddr6;
use regex::Regex;
use once_cell::sync::Lazy;

use futures::{self, FutureExt, StreamExt, stream::FuturesUnordered};
use tokio::{net::TcpStream, sync::{mpsc, oneshot}};
use tokio_stream::wrappers::ReceiverStream;
use tokio_serde::{SymmetricallyFramed, formats::SymmetricalJson};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
use uuid::Uuid;

mod protocol;
pub use protocol::{Upload, process::Process};

static REGEX_LINK_STRENGTH: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"signal:\s+(-\d+)\s+dBm+").unwrap()
});

static REGEX_MAC_ADDR: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"addr\s+([0-9a-fA-F:.-]+)").unwrap()
});

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error("Could not send request")]
    RequestError,
    #[error("Process terminated abnormally")]
    AbnormalTerminationError,
    #[error("Remote error: {0}")]
    RemoteError(String),
    #[error("Did not receive response")]
    ResponseError,
    #[error("Could not decode data")]
    DecodeError,
}

pub type Result<T> = std::result::Result<T, Error>;

type RemoteResponses = SymmetricallyFramed<
    FramedRead<tokio::io::ReadHalf<TcpStream>, LengthDelimitedCodec>,
    protocol::Response,
    SymmetricalJson<protocol::Response>>;

pub type RemoteRequests = SymmetricallyFramed<
    FramedWrite<tokio::io::WriteHalf<TcpStream>, LengthDelimitedCodec>,
    protocol::Request,
    SymmetricalJson<protocol::Request>>;

pub struct Device {
    pub addr: Ipv4Addr,
    request_tx: mpsc::Sender<Request>,
    return_addr_tx: Option<oneshot::Sender<Ipv4Addr>>,
}

impl Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Fernbedienung@{}", self.addr)
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        if let Some(return_addr_tx) = self.return_addr_tx.take() {
            let _ = return_addr_tx.send(self.addr);
        }
    }
}

enum Request {
    Halt {
        result_tx: oneshot::Sender<Result<()>>,
    },
    Reboot {
        result_tx: oneshot::Sender<Result<()>>,
    },
    Run {
        process: protocol::process::Process,
        terminate_rx: Option<oneshot::Receiver<()>>,
        stdin_rx: Option<mpsc::Receiver<BytesMut>>,
        stdout_tx: Option<mpsc::Sender<BytesMut>>,
        stderr_tx: Option<mpsc::Sender<BytesMut>>,
        result_tx: oneshot::Sender<Result<()>>,
    },
    Upload {
        upload: protocol::Upload,
        result_tx: oneshot::Sender<Result<()>>
    },
}

impl Device {
    pub async fn new(addr: Ipv4Addr, return_addr_tx: oneshot::Sender<Ipv4Addr>) -> Result<Self> {
        let (local_request_tx, mut local_request_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let stream = match TcpStream::connect((addr, 17653)).await {
                Ok(stream) => stream,
                Err(_) => return,
            };
            /* requests and responses from remote */
            let (read, write) = tokio::io::split(stream);
            let remote_requests: RemoteRequests = SymmetricallyFramed::new(
                FramedWrite::new(write, LengthDelimitedCodec::new()),
                SymmetricalJson::<protocol::Request>::default(),
            );
            let mut remote_responses: RemoteResponses = SymmetricallyFramed::new(
                FramedRead::new(read, LengthDelimitedCodec::new()),
                SymmetricalJson::<protocol::Response>::default(),
            );
            /* create an mpsc channel to share for remote_requests */
            let (remote_requests_tx, remote_requests_rx) = mpsc::channel(32);           
            let mut forward_remote_requests = ReceiverStream::new(remote_requests_rx)
                .map(|request| Ok(request))
                .forward(remote_requests);
            /* collections for tracking state */
            let mut status_txs: HashMap<Uuid, mpsc::Sender<protocol::ResponseKind>> = Default::default();
            let mut tasks: FuturesUnordered<_> = Default::default();
            /* event loop */
            loop {
                tokio::select! {
                    Some(response) = remote_responses.next() => match response {
                        Ok(protocol::Response(uuid, response)) => {
                            if let Some(uuid) = uuid {
                                if let Some(status_tx) = status_txs.get(&uuid) {
                                    let _ = status_tx.send(response).await;
                                }
                            }
                            else {
                                log::warn!("Received message without identifier: {:?}", response);
                            }
                        },
                        Err(error) => {
                            log::warn!("Could not deserialize response from remote: {}", error);
                        }
                    },
                    request = local_request_rx.recv() => match request {
                        Some(request) => {
                            let task = match request {
                                Request::Halt { result_tx } => {
                                    let uuid = Uuid::new_v4();
                                    let request = protocol::RequestKind::Halt;
                                    let (halt_status_tx, mut halt_status_rx) = mpsc::channel(8);
                                    status_txs.insert(uuid, halt_status_tx);
                                    let request_result = remote_requests_tx
                                        .send(protocol::Request(uuid, request)).await;
                                    async move {
                                        let result = match request_result {
                                            Ok(_) => match halt_status_rx.recv().await {
                                                Some(protocol::ResponseKind::Ok) => Ok(()),
                                                _ => Err(Error::ResponseError),
                                            }
                                            _ => Err(Error::RequestError),
                                        };
                                        let _ = result_tx.send(result);
                                        uuid
                                    }.boxed()
                                }
                                Request::Reboot { result_tx } => {
                                    let uuid = Uuid::new_v4();
                                    let request = protocol::RequestKind::Reboot;
                                    let (reboot_status_tx, mut reboot_status_rx) = mpsc::channel(8);
                                    status_txs.insert(uuid, reboot_status_tx);
                                    let request_result = remote_requests_tx
                                        .send(protocol::Request(uuid, request)).await;
                                    async move {
                                        let result = match request_result {
                                            Ok(_) => match reboot_status_rx.recv().await {
                                                Some(protocol::ResponseKind::Ok) => Ok(()),
                                                _ => Err(Error::ResponseError),
                                            }
                                            _ => Err(Error::RequestError),
                                        };
                                        let _ = result_tx.send(result);
                                        uuid
                                    }.boxed()
                                }
                                Request::Upload { upload, result_tx } => {
                                    let uuid = Uuid::new_v4();
                                    let request = protocol::RequestKind::Upload(upload);
                                    /* subscribe to updates */
                                    let (upload_status_tx, mut upload_status_rx) = mpsc::channel(8);
                                    status_txs.insert(uuid, upload_status_tx);
                                    /* send the request */
                                    let request_result = remote_requests_tx
                                        .send(protocol::Request(uuid, request)).await;
                                    /* process responses */
                                    async move {
                                        let result = match request_result {
                                            Ok(_) => match upload_status_rx.recv().await {
                                                Some(protocol::ResponseKind::Ok) => Ok(()),
                                                _ => Err(Error::ResponseError),
                                            }
                                            _ => Err(Error::RequestError),
                                        };
                                        let _ = result_tx.send(result);
                                        uuid
                                    }.boxed()
                                },
                                Request::Run { process, terminate_rx, stdin_rx, stdout_tx, stderr_tx, result_tx } => {
                                    let uuid = Uuid::new_v4();
                                    let request = protocol::RequestKind::Process(protocol::process::Request::Run(process));
                                    /* subscribe to updates */
                                    let (run_status_tx, run_status_rx) = mpsc::channel(8);
                                    status_txs.insert(uuid, run_status_tx);
                                    /* send the request */
                                    match remote_requests_tx.send(protocol::Request(uuid, request)).await {
                                        Ok(_) => {
                                            let remote_requests_tx = remote_requests_tx.clone();
                                            Device::handle_run_request(uuid, run_status_rx, remote_requests_tx,
                                                terminate_rx, stdin_rx, stdout_tx, stderr_tx, result_tx).left_future()
                                        }
                                        _ => async move {
                                            let _ = result_tx.send(Err(Error::RequestError));
                                            uuid
                                        }.right_future()
                                    }.boxed()
                                },
                            };
                            tasks.push(task);
                        },
                        None => break,
                    },
                    Some(uuid) = tasks.next() => {
                        status_txs.remove(&uuid);
                    },
                    _ = &mut forward_remote_requests => {}
                }
            }
        });
        Ok(Device { request_tx: local_request_tx, addr, return_addr_tx: Some(return_addr_tx) })
    }

    async fn handle_run_request(uuid: Uuid,
                                mut run_status_rx: mpsc::Receiver<protocol::ResponseKind>,
                                remote_requests_tx: mpsc::Sender<protocol::Request>,
                                terminate_rx: Option<oneshot::Receiver<()>>,
                                stdin_rx: Option<mpsc::Receiver<BytesMut>>,
                                stdout_tx: Option<mpsc::Sender<BytesMut>>,
                                stderr_tx: Option<mpsc::Sender<BytesMut>>,
                                exit_status_tx: oneshot::Sender<Result<()>>) -> Uuid {
        let mut terminate_rx = match terminate_rx {
            Some(terminate_rx) => terminate_rx.into_stream().left_stream(),
            None => futures::stream::pending().right_stream(),
        };
        let mut stdin_rx = match stdin_rx {
            Some(stdin_rx) => ReceiverStream::new(stdin_rx).left_stream(),
            None => futures::stream::pending().right_stream(),
        };

        loop {
            tokio::select! {
                Some(_) = terminate_rx.next() => {
                    let request = protocol::Request(uuid, protocol::RequestKind::Process(
                        protocol::process::Request::Terminate)
                    );
                    let _ = remote_requests_tx.send(request).await;
                },
                Some(stdin) = stdin_rx.next() => {
                    let request = protocol::Request(uuid, protocol::RequestKind::Process(
                        protocol::process::Request::StandardInput(stdin))
                    );
                    let _ = remote_requests_tx.send(request).await;

                },
                Some(response) = run_status_rx.recv() => match response {
                    protocol::ResponseKind::Ok => {},
                    protocol::ResponseKind::Error(error) => {
                        let status = Err(Error::RemoteError(error));
                        let _ = exit_status_tx.send(status);
                        break;
                    }
                    protocol::ResponseKind::Process(response) => match response {
                        protocol::process::Response::Terminated(result) => {
                            let status = match result {
                                true => Ok(()),
                                false => Err(Error::AbnormalTerminationError),
                            };
                            let _ = exit_status_tx.send(status);
                            break;
                        },
                        protocol::process::Response::StandardOutput(data) => {
                            if let Some(stdout_tx) = &stdout_tx {
                                let _ = stdout_tx.send(data).await;
                            }
                        },
                        protocol::process::Response::StandardError(data) => {
                            if let Some(stderr_tx) = &stderr_tx {
                                let _ = stderr_tx.send(data).await;
                            }
                        },
                    },
                },
                else => break
            }
        }
        /* return the uuid so it can be removed from the hashmap */
        uuid
    }

    pub async fn upload<P, F, C>(
        &self,
        path: P,
        filename: F,
        contents: C
    ) -> Result<()> where P: Into<PathBuf>, F: Into<PathBuf>, C: Into<Vec<u8>> {
        let upload = protocol::Upload {
            path: path.into(), filename: filename.into(), contents: contents.into(),
        };
        let (result_tx, result_rx) = oneshot::channel();
        self.request_tx
            .send(Request::Upload { upload, result_tx }).await
            .map_err(|_| Error::RequestError)?;
        result_rx.await.map_err(|_| Error::ResponseError).and_then(|result| result)
    }

    pub async fn halt(&self) -> Result<()> {
        let (result_tx, result_rx) = oneshot::channel();
        self.request_tx
            .send(Request::Halt { result_tx }).await
            .map_err(|_| Error::RequestError)?;
        result_rx.await.map_err(|_| Error::ResponseError).and_then(|result| result)
    }

    pub async fn reboot(&self) -> Result<()> {
        let (result_tx, result_rx) = oneshot::channel();
        self.request_tx
            .send(Request::Reboot { result_tx }).await
            .map_err(|_| Error::RequestError)?;
        result_rx.await.map_err(|_| Error::ResponseError).and_then(|result| result)
    }

    pub async fn run(&self,
                     process: protocol::process::Process,
                     terminate_rx: impl Into<Option<oneshot::Receiver<()>>>,
                     stdin_rx: impl Into<Option<mpsc::Receiver<BytesMut>>>,
                     stdout_tx: impl Into<Option<mpsc::Sender<BytesMut>>>,
                     stderr_tx: impl Into<Option<mpsc::Sender<BytesMut>>>) -> Result<()> {
        let (result_tx, result_rx) = oneshot::channel();
        let request = Request::Run {
            process,
            terminate_rx: terminate_rx.into(),
            stdin_rx: stdin_rx.into(),
            stdout_tx: stdout_tx.into(),
            stderr_tx: stderr_tx.into(),
            result_tx: result_tx.into()
        };
        self.request_tx.send(request).await.map_err(|_ | Error::RequestError)?;
        result_rx.await.map_err(|_| Error::ResponseError).and_then(|result| result)
    }

    pub async fn create_temp_dir(&self) -> Result<String> {
        let process = protocol::process::Process {
            target: "mktemp".into(),
            working_dir: None,
            args: vec!["-d".to_owned()],
        };
        let (stdout_tx, stdout_rx) = mpsc::channel(8);
        let stdout_stream = ReceiverStream::new(stdout_rx);
        let (_, stdout) = tokio::try_join!(
            self.run(process, None, None, stdout_tx, None),
            stdout_stream.concat().map(Result::Ok)
        )?;
        let temp_dir = std::str::from_utf8(stdout.as_ref())
            .map_err(|_| Error::DecodeError)?;
        Ok(temp_dir.trim().to_owned())
    }

    pub async fn hostname(&self) -> Result<String> {
        let process = protocol::process::Process {
            target: "hostname".into(),
            working_dir: None,
            args: vec![],
        };
        let (stdout_tx, stdout_rx) = mpsc::channel(8);
        let stdout_stream = ReceiverStream::new(stdout_rx);
        let (_, stdout) = tokio::try_join!(
            self.run(process, None, None, stdout_tx, None),
            stdout_stream.concat().map(Result::Ok)
        )?;
        let hostname = std::str::from_utf8(stdout.as_ref())
            .map_err(|_| Error::DecodeError)?;
        Ok(hostname.trim().to_owned())
    }

    pub async fn kernel_messages(&self) -> Result<String> {
        let process = protocol::process::Process {
            target: "dmesg".into(),
            working_dir: None,
            args: vec![],
        };
        let (stdout_tx, stdout_rx) = mpsc::channel(8);
        let stdout_stream = ReceiverStream::new(stdout_rx);
        let (_, stdout) = tokio::try_join!(
            self.run(process, None, None, stdout_tx, None),
            stdout_stream.concat().map(Result::Ok)
        )?;
        let messages = std::str::from_utf8(stdout.as_ref())
            .map_err(|_| Error::DecodeError)?;
        Ok(messages.trim().to_owned())
    }

    pub async fn link_strength(&self) -> Result<i32> {
        let process = protocol::process::Process {
            target: "iw".into(),
            working_dir: None,
            args: "dev wlan0 link"
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
        };
        let (stdout_tx, stdout_rx) = mpsc::channel(8);
        let stdout_stream = ReceiverStream::new(stdout_rx);
        let (_, stdout) = tokio::try_join!(
            self.run(process, None, None, stdout_tx, None),
            stdout_stream.concat().map(Result::Ok)
        )?;
        let link = std::str::from_utf8(stdout.as_ref())
            .map_err(|_| Error::DecodeError)?;
        REGEX_LINK_STRENGTH.captures(link)
            .and_then(|captures| captures.get(1))
            .map(|capture| capture.as_str())
            .ok_or(Error::DecodeError)
            .and_then(|strength| strength.parse().map_err(|_| Error::DecodeError))
    }

    pub async fn mac(&self) -> Result<MacAddr6> {
        let process = protocol::process::Process {
            target: "iw".into(),
            working_dir: None,
            args: "dev wlan0 info"
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
        };
        let (stdout_tx, stdout_rx) = mpsc::channel(8);
        let stdout_stream = ReceiverStream::new(stdout_rx);
        let (_, stdout) = tokio::try_join!(
            self.run(process, None, None, stdout_tx, None),
            stdout_stream.concat().map(Result::Ok)
        )?;
        let info = std::str::from_utf8(stdout.as_ref())
            .map_err(|_| Error::DecodeError)?;
        REGEX_MAC_ADDR.captures(info)
            .and_then(|captures| captures.get(1))
            .map(|capture| capture.as_str())
            .ok_or(Error::DecodeError)
            .and_then(|mac_addr| mac_addr.parse().map_err(|_| Error::DecodeError))
    }
}

