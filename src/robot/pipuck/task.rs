use futures::{Future, stream::FuturesUnordered};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::{net::Ipv4Addr, sync::Arc};
use tokio::sync::{mpsc, oneshot};
use tokio_stream::{StreamExt, wrappers::UnboundedReceiverStream};
use crate::network::fernbedienung;

#[derive(Debug)]
pub struct State {
    pub linux: Ipv4Addr, // rename to addr
    pub actions: Vec<Action>,
}

pub enum Request {
    State,
    Execute(Action),
    Upload(crate::software::Software),
}

#[derive(Debug)]
pub enum Response {
    State(State),
    Ok,
    Error(Error),
}

pub type Sender = mpsc::UnboundedSender<(Request, Option<oneshot::Sender<Response>>)>;
pub type Receiver = mpsc::UnboundedReceiver<(Request, Option<oneshot::Sender<Response>>)>;

#[derive(Copy, Clone, Serialize, Deserialize, Debug, PartialEq)]
pub enum Action {
    #[serde(rename = "Shutdown RPi")]
    RpiShutdown,
    #[serde(rename = "Reboot RPi")]
    RpiReboot,
    /*
    #[serde(rename = "Identify")]
    Identify,
    */
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    FernbedienungError(#[from] fernbedienung::Error),

    #[error(transparent)]
    JoinError(#[from] tokio::task::JoinError),
}

pub async fn new(uuid: Uuid, rx: Receiver, device: fernbedienung::Device) -> (Uuid, Ipv4Addr) {


    let (device_task, device_interface, device_addr) = device.split();


    let software = crate::software::Software::default();
    let path = "/tmp";

    


// Ok(path) => {
//     for (filename, contents) in software.0.iter() {
//         if let Err(err) = device_interface.clone().upload(&path, filename, contents.clone()).await {

//         }
//     }
//     Response::Ok
// }







    let request_task = async move {
        let mut requests = UnboundedReceiverStream::new(rx);
        //let processes : FuturesUnordered<_> = Default::default();
        loop {
            tokio::select! {
                Some((request, callback)) = requests.next() => {
                    match request {
                        Request::State => if let Some(callback) = callback {
                            let state = State {
                                linux: device_addr,
                                actions: vec![Action::RpiShutdown, Action::RpiReboot]
                            };
                            if let Err(_) = callback.send(Response::State(state)) {
                                log::error!("Could not respond with state");
                            }
                        }
                        Request::Execute(action) => match action {
                            Action::RpiReboot => {
                                if let Err(error) = device_interface.clone().reboot().await {
                                    log::error!("Reboot failed: {}", error);
                                }
                                break;
                            },
                            Action::RpiShutdown => {
                                if let Err(error) = device_interface.clone().shutdown().await {
                                    log::error!("Shut down failed: {}", error);
                                }
                                break;
                            }
                        }
                        Request::Upload(software) => if let Some(callback) = callback {
                            let response = match device_interface.clone().create_temp_dir().await {
                                Ok(path) => {
                                    let upload_result = software.0.into_iter()
                                        .map(|(filename, contents)| {
                                            device_interface.clone().upload(&path, filename, contents)
                                        })
                                        .collect::<FuturesUnordered<_>>()
                                        .collect::<Result<Vec<_>, _>>().await;
                                    match upload_result {
                                        Ok(_) => Response::Ok,
                                        Err(error) => Response::Error(Error::FernbedienungError(error))
                                    }
                                }
                                Err(error) => Response::Error(Error::FernbedienungError(error))
                            };
                            if let Err(response) = callback.send(response) {
                                log::error!("Could not respond with {:?}", response);
                            }
                        }
                        /*
                        Request::Experiment(experiment) => match experiment {
                            Experiment::Start(working_dir, args) => {
                                let task = fernbedienung::process::Run {
                                    target: "argos3".into(),
                                    working_dir: working_dir.into(),
                                    args: args,
                                };
                                let process = device_interface.clone().run(task, None, None, None, None);
                                processes.push(process);
                            }
                            Experiment::Stop => {
                                //process.signal().await;
            
                            }
                        }
                        */
                    }
                }
            }
        }
    };
    /* select here instead of join since if one future completes the
       other should be dropped */
    tokio::select! {
        _ = device_task => {},
        _ = request_task => {},
    }
    (uuid, device_addr)
}
