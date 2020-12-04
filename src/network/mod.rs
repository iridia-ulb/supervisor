pub mod xbee;
pub mod ssh;

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
    Ssh(ssh::Device),
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

/// This code can be reorganised without the channels to monitor the network and to
/// add/remove robots from the shared collections

/// TODO:
/// 1. Clean up this code so that it compiles again
/// 1a. Quick investigation into what a robot enum (static dispatch) would look like
/// 2. Add the ping functionality to remove robots if they don't reply
///    a. What if SSH drops from drone, but Xbee is still up? (move back to the standby state?)
///    b. What if Xbee drops, but SSH is still up? (these are difficult problems to solve)
/// 3. Investigate the SSH shell drop outs


/// The discover task recieves IPv4 addresses and sends back associations
pub async fn discover(network: Ipv4Net, robots: Robots) {
        let mut queue = network.hosts()
            .map(|addr| probe(addr, None))
            .collect::<FuturesUnordered<_>>();
    
        loop {
            tokio::select! {
                /*
                /* address received for probing */
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
            Device::Ssh(mut device) => {
                if let Ok(hostname) = device.hostname().await {
                    match &hostname[..] {
                        "raspberrypi0-wifi" => {
                            let pipuck = PiPuck::new(device);
                            robots.write().await.push(Robot::PiPuck(pipuck));
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
        /* attempt a ssh connection for 1000 ms */
        let assoc_ssh_attempt =
            timeout(Duration::from_millis(1000), ssh::Device::new(addr));
        if let Ok(assoc_ssh_result) = assoc_ssh_attempt.await {
            match assoc_ssh_result {
                Ok(device) => {
                    return (addr, Ok(Device::Ssh(device)));
                }
                Err(_error) => {
                    // TODO, _error is the ssh error and could be useful
                    return (addr, Err(Error::Timeout));
                }
            }
        }
        (addr, Err(Error::Timeout))
    }
    
    
    