pub mod xbee;
pub mod fernbedienung;

use futures::stream::FuturesUnordered;

use tokio::{sync::mpsc, time::timeout};
use tokio_stream::{StreamExt, wrappers::UnboundedReceiverStream};

use std::{
    time::Duration,
    net::Ipv4Addr,
};

use crate::arena;

pub enum Device {
    Fernbedienung(fernbedienung::Device),
    Xbee(xbee::Device),
}

#[derive(thiserror::Error, Debug)]
enum Error {
    #[error("Association timed out")]
    Timeout,

    #[error("Non-matching IP address")]
    IpMismatch,

    #[error(transparent)]
    XbeeError(#[from] xbee::Error),
}

type Result<T> = std::result::Result<T, Error>;

pub async fn new(network_addr_rx: mpsc::UnboundedReceiver<Ipv4Addr>,
                 arena_request_tx: &mpsc::UnboundedSender<arena::Request>) {
    let mut probe_queue : FuturesUnordered<_> = Default::default();
    let mut addresses = UnboundedReceiverStream::new(network_addr_rx);

    loop {
        tokio::select!{
            Some(network_addr) = addresses.next() => {
                probe_queue.push(probe(network_addr, None));
            },
            Some((probe_addr, probe_result)) = probe_queue.next() => {
                if let Ok(device) = probe_result {
                    associate(device, &arena_request_tx).await;
                }
                else {
                    /* TODO: perhaps match on different error types and delay accordingly */
                    probe_queue.push(probe(probe_addr, Some(Duration::new(1,0))));
                }
            }
            else => break
        }
    }
}
    
async fn associate(device: Device, arena_request_tx: &mpsc::UnboundedSender<arena::Request>) {
    match device {
        Device::Fernbedienung(device) => {
            /* the task of the device needs to be run in order for hostname to resolve */
            let (mut task, interface, addr) = device.split();
            tokio::select! {
                _ = &mut task => {},
                hostname = interface.clone().hostname() => match hostname {
                    Ok(hostname) => {
                        let device = fernbedienung::Device::unite(task, interface, addr);
                        match &hostname[..] {
                            "ToshibaLaptop" => {
                                if let Err(error) = arena_request_tx.send(arena::Request::AddPiPuck(device)) {
                                    log::error!("Could not add Pi-Puck to the arena: {}", error);
                                }
                            },
                            "raspberrypi0-wifi" => {
                                if let Err(error) = arena_request_tx.send(arena::Request::AddPiPuck(device)) {
                                    log::error!("Could not add Pi-Puck to the arena: {}", error);
                                }
                            },
                            "upcore" => {
                                if let Err(error) = arena_request_tx.send(arena::Request::PairWithDrone(device)) {
                                    log::error!("Could not pair drone to it's Xbee: {}", error);
                                }
                            },
                            _ => log::warn!("Unrecognized fernbedienung device {} detected", hostname),
                        }
                    },
                    Err(error) => {
                        // the IP address should be returned to our pool here
                    }
                }
            }
        },
        Device::Xbee(device) => {
            if let Err(error) = arena_request_tx.send(arena::Request::AddDrone(device)) {
                log::error!("Could not add drone to the arena: {}", error);
            }
        }
    }
}



async fn probe(addr: Ipv4Addr, delay: Option<Duration>) -> (Ipv4Addr, Result<Device>) {
    /* wait delay before probing */
    if let Some(delay) = delay {
        tokio::time::sleep(delay).await;
    }
    /* attempt to connect to Xbee for 500 ms */
    let assoc_xbee_attempt = timeout(Duration::from_millis(500), async {
        let device = xbee::Device::new(addr).await?;
        /* validate the connection by checking if the remote IP address matches
           the IP address that we connected to */
        if addr == device.ip().await? {
            Ok(device)
        }
        else {
            Err(Error::IpMismatch)
        }
    });
    if let Ok(assoc_xbee_result) = assoc_xbee_attempt.await {
        /* TODO consider the Xbee error variant? */
        if let Ok(device) = assoc_xbee_result {
            return (addr, Ok(Device::Xbee(device)));
        }
    }
    /* xbee connection timed out/failed */
    /* attempt a fernbedienung connection for 500 ms */
    let assoc_fernbedienung_attempt =
        timeout(Duration::from_millis(500), fernbedienung::Device::new(addr));
    if let Ok(assoc_fernbedienung_result) = assoc_fernbedienung_attempt.await {
        if let Ok(device) = assoc_fernbedienung_result {
            return (addr, Ok(Device::Fernbedienung(device)));
        }
    }
    (addr, Err(Error::Timeout))
}
    
    
    
