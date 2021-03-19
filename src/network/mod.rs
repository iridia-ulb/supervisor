use futures::stream::FuturesUnordered;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use std::{collections::HashMap, net::Ipv4Addr, time::Duration};
use ipnet::Ipv4Net;

pub mod xbee;
pub mod fernbedienung;

use crate::arena;

#[derive(thiserror::Error, Debug)]
enum Error {
    #[error("Could not associate address")]
    AssociateError,
}

type Result<T> = std::result::Result<T, Error>;

enum AddrState {
    ProbeXbee,
    ProbeFernbedienung,
    InUse,
    Sleep,
}

pub async fn new(network: Ipv4Net, arena_request_tx: &mpsc::UnboundedSender<arena::Request>) {
    let (return_addr_tx, mut return_addr_rx) = mpsc::unbounded_channel::<Ipv4Addr>();
    let mut addr_in_use_map = network.clone().hosts()
        .map(|addr| (addr, false))
        .collect::<HashMap<_,_>>();
    let mut associate_xbee_queue = network.hosts()
        .map(|addr| associate_xbee(&arena_request_tx, &return_addr_tx, addr))
        .collect::<FuturesUnordered<_>>();
    let mut associate_fernbedienung_queue: FuturesUnordered<_> = Default::default();
    loop {
        tokio::select!{
            Some(recv_addr) = return_addr_rx.recv() => {
                /* check if received address was in-use */
                if let Some(true) = addr_in_use_map.get(&recv_addr) {
                    addr_in_use_map.insert(recv_addr, false);
                    let association =
                        associate_xbee(&arena_request_tx, &return_addr_tx, recv_addr);
                    associate_xbee_queue.push(association);
                }
            },
            Some((addr, result)) = associate_xbee_queue.next() => match result {
                Ok(_) => { 
                    addr_in_use_map.insert(addr, true);
                },
                Err(_) => {
                    let association =
                        associate_fernbedienung(&arena_request_tx, &return_addr_tx, addr);
                    associate_fernbedienung_queue.push(association);
                }
            },
            Some((addr, result)) = associate_fernbedienung_queue.next() => match result {
                Ok(_) => { 
                    addr_in_use_map.insert(addr, true);
                },
                Err(_) => {
                    let association =
                        associate_xbee(&arena_request_tx, &return_addr_tx, addr);
                    associate_xbee_queue.push(association);
                }
            },
            else => break
        }
    }
}

// The problem here is that during probing, I create a new device which will return its IP address if it
// fails. Solutions:

// 1. set a flag (or give the device the channel only after initialization is complete and
// we send it to the arena). This could even be done from the arena? device.set_in_arena()

// 2. keep track locally which IP addresses have been given out, what stage in probing they are up to.

// keep all addresses locally


async fn associate_xbee(arena_request_tx: &mpsc::UnboundedSender<arena::Request>,
                        return_addr_tx: &mpsc::UnboundedSender<Ipv4Addr>,
                        addr: Ipv4Addr) -> (Ipv4Addr, Result<()>) {
    /* assume address is an xbee and attempt to connect for 500 ms */
    let xbee_attempt = tokio::time::timeout(Duration::from_millis(500), async {
        let device = xbee::Device::new(addr, return_addr_tx.clone()).await?;
        let addr = device.ip().await?;
        std::result::Result::<_, xbee::Error>::Ok((addr, device))
    }).await;
    /* inspect result */
    if let Ok(xbee_result) = xbee_attempt {
        if let Ok((xbee_addr, xbee_device)) = xbee_result {
            if xbee_addr == addr {
                let result = arena_request_tx.send(arena::Request::AddDrone(xbee_device))
                    .map_err(|_| Error::AssociateError);
                return (addr, result);
            }
        }
    }
    (addr, Err(Error::AssociateError))
}

async fn associate_fernbedienung(arena_request_tx: &mpsc::UnboundedSender<arena::Request>,
                                 return_addr_tx: &mpsc::UnboundedSender<Ipv4Addr>,
                                 addr: Ipv4Addr) -> (Ipv4Addr, Result<()>) {
    /* assume address is a device running the fernbedienung service and 
       attempt to connect for 500 ms */
    let fernbedienung_attempt = tokio::time::timeout(Duration::from_millis(500), async {
        let device = fernbedienung::Device::new(addr, return_addr_tx.clone()).await?;
        let hostname = device.hostname().await?;
        std::result::Result::<_, fernbedienung::Error>::Ok((hostname, device))
    }).await;
    /* inspect result */
    if let Ok(fernbedienung_result) = fernbedienung_attempt {
        if let Ok((hostname, device)) = fernbedienung_result {
            let result = match &hostname[..] {
                "raspberrypi0-wifi" | "ToshibaLaptop" =>
                    arena_request_tx.send(arena::Request::AddPiPuck(device))
                        .map_err(|_| Error::AssociateError),
                "up-core" =>
                    arena_request_tx.send(arena::Request::PairWithDrone(device))
                        .map_err(|_| Error::AssociateError),
                _ => Err(Error::AssociateError),
            };
            return (addr, result);
        }
    }
    (addr, Err(Error::AssociateError))
}
    
    
    
