mod xbee;
pub mod ssh;
pub mod drone;
pub mod pipuck;

use futures::stream::FuturesUnordered;

use tokio::{
    time::timeout,
    stream::StreamExt,
    sync::mpsc::{
        UnboundedSender,
        UnboundedReceiver
    },
};

use std::{
    time::Duration,
    net::Ipv4Addr,
};

#[derive(thiserror::Error, Debug)]
enum Error {
    #[error("Error communicating with SSH server")]
    SshConnectionError {
        source: ssh::Error,
    },
    #[error("Error communicating with Xbee")]
    XbeeConnectionError {
        source: xbee::Error,
    },
    #[error("Association timed out")]
    Timeout,
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Device {
    Ssh(ssh::Device),
    Xbee(xbee::Device),
}

/// The discover task recieves IPv4 addresses and sends back associations
pub async fn discover(mut addr_rx: UnboundedReceiver<Ipv4Addr>,
                      assoc_tx: UnboundedSender<Device>) {
    let mut probing = FuturesUnordered::default();
    loop {
        tokio::select! {
            Some(addr) = addr_rx.recv() => {
                probing.push(probe(addr, None));
            },
            Some((addr, probe_result)) = probing.next() => {
                if let Ok(assoc) = probe_result {
                    assoc_tx.send(assoc).unwrap();
                }
                else {
                    /* TODO: perhaps match on different error types and 
                             delay accordingly */
                    probing.push(probe(addr, Some(Duration::new(1,0))));
                }
            }
            else => {
                break;
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


