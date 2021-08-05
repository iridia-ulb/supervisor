
use macaddr::MacAddr6;
use std::{net::Ipv4Addr, time::Duration};
use ipnet::Ipv4Net;

use tokio::sync::{mpsc, oneshot};
use tokio_stream::StreamExt;
use futures::stream::FuturesUnordered;

pub mod xbee;
pub mod fernbedienung;
pub mod fernbedienung_ext;

use crate::arena;

/// This function represents the main task of the network module. It takes a network and a channel for
/// making requests to the arena. IP addresses belonging to this network are repeated probed for an
/// xbee or for the fernbedienung service until they are associated
pub async fn new(network: Ipv4Net, arena_request_tx: &mpsc::Sender<arena::Request>) {
    /* probe for xbees on all addresses */
    let (mut xbee_returned_addrs, mut probe_xbee_queue) : (FuturesUnordered<_>, FuturesUnordered<_>) = network
        .hosts()
        .map(|addr| {
            let (return_addr_tx, return_addr_rx) = oneshot::channel();
            (return_addr_rx, probe_xbee(return_addr_tx, addr))
        }).unzip();
    /* empty collections for the fernbedienung tasks */
    let mut fernbedienung_returned_addrs : FuturesUnordered<oneshot::Receiver<Ipv4Addr>> = Default::default();
    let mut probe_fernbedienung_queue: FuturesUnordered<_> = Default::default();
    /* main task loop */
    loop {
        tokio::select!{
            Some(result) = probe_xbee_queue.next() => {
                if let Ok((mac_addr, device)) = result {
                    let _ = arena_request_tx.send(arena::Request::AddXbee(device, mac_addr));
                }
            },
            Some(result) = xbee_returned_addrs.next() => match result {
                Ok(addr) => {
                    let (return_addr_tx, return_addr_rx) = oneshot::channel();
                    fernbedienung_returned_addrs.push(return_addr_rx);
                    probe_fernbedienung_queue.push(probe_fernbedienung(return_addr_tx, addr));
                },
                Err(_) => {
                    log::error!("IPV4 address not returned");
                }
            },
            Some(result) = probe_fernbedienung_queue.next() => {
                if let Ok((mac_addr, device)) = result {
                    let _ = arena_request_tx.send(arena::Request::AddFernbedienung(device, mac_addr));
                }
            },
            Some(result) = fernbedienung_returned_addrs.next() => match result {
                Ok(addr) => {
                    let (return_addr_tx, return_addr_rx) = oneshot::channel();
                    xbee_returned_addrs.push(return_addr_rx);
                    probe_xbee_queue.push(probe_xbee(return_addr_tx, addr));
                },
                Err(_) => {
                    log::error!("IPV4 address not returned");
                }
            },
            else => break
        }
    }
}

/// This function attempts to associate an xbee device with a given Ipv4Addr. The function starts the async 
/// xbee::Device function `new` inside of a tokio::timeout which attempts the connection.
async fn probe_xbee(return_addr_tx: oneshot::Sender<Ipv4Addr>,
                    addr: Ipv4Addr) -> anyhow::Result<(MacAddr6, xbee::Device)> {
    /* assume address is an xbee and attempt to connect for 500 ms */
    tokio::time::timeout(Duration::from_millis(500), async {
        let device = xbee::Device::new(addr, return_addr_tx).await?;
        let mac_addr = device.mac().await?;
        Ok((mac_addr, device))
    }).await?
}

/// This function attempts to associate an instance of the fernbedienung service with a given Ipv4Addr. The
/// function starts the async fernbedienung::Device function `new` inside of a tokio::timeout which attempts
/// the connection.
async fn probe_fernbedienung(return_addr_tx: oneshot::Sender<Ipv4Addr>,
                             addr: Ipv4Addr) -> anyhow::Result<(MacAddr6, fernbedienung::Device)> {
    /* assume there is a fernbedienung instance running on `addr` and attempt to connect to it for 500 ms */
    tokio::time::timeout(Duration::from_millis(500), async {
        let device = fernbedienung::Device::new(addr, return_addr_tx).await?;
        let mac_addr = device.mac().await?;
        Ok((mac_addr, device))
    }).await?
}
