pub mod xbee;
pub mod fernbedienung;

use futures::stream::FuturesUnordered;

use tokio::{
    time::timeout,
    stream::StreamExt,
//    sync::{ mpsc, RwLock },
};

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

use super::Robots;
use super::robot::{Robot, drone::Drone, pipuck::PiPuck};

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

// by testing all connected robots at the same time, we lower the amount of time that we have to lock the collection
// how often should we ping, every 500 ms?

// a more clever way of doing this could involve looking for failed operations/only pinging devices that we not responding
// this would involve keeping a last_error: Option<Error>, last_operation: Option<Instant> variable in the devices

// this would reduce congestion but would not speed up the operation of pinging since we would have the collection locked for
// that duration anyway

// Does it make sense to make individual robots lockable?

// type Robots = Arc<RwLock<Vec<robot::Robot>>>;
// type Robots = Arc<RwLock<Vec<RwLock<robot::Robot>>>>;

// in this way, even if robot state needed to be changed, we would only ever need to lock the outer collection if we were
// adding/removing robots

// if it is determined that a robot is not responding, should we remove it from the collection? This could be dangerous for a
// drone under the control of ARGoS, 

// Each device (ssh/xbee) could have a state online/offline, instead of removing the device from the collection,
// it could be marked as inactive, and we could try here to reestablish that connection

// For long running processes such as ARGoS, it may make sense to run this inside tmux or screen such that SSH disconnects and
// reconnects are not effected. Upon reconnect, ARGoS should be detected and handled appropiately

pub async fn discover(network: Ipv4Net, robots: Robots) {
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
                        associate(device, &robots).await;
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
    
    async fn associate(device: Device, robots: &Robots) {
        match device {
            Device::Fernbedienung(mut device) => {
                let pipuck = PiPuck::new(device);
                robots.write().await.push(Robot::PiPuck(pipuck));
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
                let mut drone = Drone::new(device);
                if let Ok(_) = drone.init().await {
                    robots.write().await.push(Robot::Drone(drone));
                }
                else {
                    // place address back in pool
                }
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
    
    
    