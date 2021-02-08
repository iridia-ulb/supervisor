pub mod xbee;
pub mod fernbedienung;

use futures::stream::FuturesUnordered;

use tokio::{stream::StreamExt, sync::mpsc, time::timeout};

use ipnet::Ipv4Net;

use std::{
    time::Duration,
    net::Ipv4Addr,
};

#[derive(Debug)]
pub enum Device {
    Fernbedienung(fernbedienung::Device),
    Xbee(xbee::Device),
}


use crate::robot::{Robot, drone::Drone, pipuck::PiPuck};
use crate::arena;

#[derive(thiserror::Error, Debug)]
enum Error {
    /*
    #[error("Error communicating with SSH server")]
    SshConnectionError {
        source: ssh::Error,
    },
    #[error("Error communicating with Xbee")]
    XbeeConnectionError {
        source: xbee::Error,
    },
    */
    #[error("Association timed out")]
    Timeout,
}

type Result<T> = std::result::Result<T, Error>;

pub async fn new(network: Ipv4Net, arena_request_tx: mpsc::UnboundedSender<arena::Request>) {
    let mut queue = network.hosts()
        // .inspect(|addr| log::info!("{}", addr))
        .map(|addr| probe(addr, None))
        .collect::<FuturesUnordered<_>>();

    loop {
        tokio::select! {
            /*
            /* TODO address received for probing */
            Some(addr) = addr_rx.recv() => {
                probing.push(probe(addr, None));
            },
            */
            /* probe from the FuturesUnordered completed */
            Some((addr, probe_result)) = queue.next() => {
                /* association was sucessful */
                if let Ok(device) = probe_result {
                    associate(device, &arena_request_tx).await;
                }
                else {
                    /* TODO: perhaps match on different error types and 
                                delay accordingly */
                    queue.push(probe(addr, Some(Duration::new(1,0))));
                }
            }
            else => {
                break;
            }
        }
    }
}
    
async fn associate(device: Device, arena_request_tx: &mpsc::UnboundedSender<arena::Request>) {
    match device {
        Device::Fernbedienung(device) => {
            let pipuck = Robot::PiPuck(PiPuck::new(device));
            //arena_request_tx.send(arena::Request::AddRobot(pipuck));
            /*
            if let Ok(hostname) = device.hostname().await {
                match &hostname[..] {
                    "raspberrypi0-wifi" => {
                        
                    },
                    _ => {
                        log::warn!("{} accepted SSH connection with root login, but the \
                                    hostname ({}) was not recognised", hostname, device.addr);
                        // place back in the pool with 5 second delay
                    }
                }
            }
            else {
                // getting hostname failed
                // place back in the pool with 1 second delay
            }
            */
        },
        Device::Xbee(device) => {
            let drone = Robot::Drone(Drone::new(device));
            //arena_request_tx.send(arena::Request::AddRobot(drone));
        }
    }
}



async fn probe(addr: Ipv4Addr, delay: Option<Duration>) -> (Ipv4Addr, Result<Device>) {
    /* wait delay before probing */
    if let Some(delay) = delay {
        tokio::time::delay_for(delay).await;
    }
    /* attempt to connect to Xbee for 500 ms */
    let assoc_xbee_attempt =
        timeout(Duration::from_millis(500), xbee::Device::new(addr));
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
    
    
    