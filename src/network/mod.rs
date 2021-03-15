use futures::stream::FuturesUnordered;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use std::{time::Duration, net::Ipv4Addr};

pub mod xbee;
pub mod fernbedienung;

use crate::arena;

#[derive(thiserror::Error, Debug)]
enum Error {
    #[error("Could not associate {0} with a robot type")]
    AssociateError(Ipv4Addr),
}

type Result<T> = std::result::Result<T, Error>;

pub async fn new(mut network_addr_rx: mpsc::UnboundedReceiver<Ipv4Addr>,
                 arena_request_tx: &mpsc::UnboundedSender<arena::Request>) {
    let mut associate_queue : FuturesUnordered<_> = Default::default();
    loop {
        tokio::select!{
            recv_addr = network_addr_rx.recv() => match recv_addr {
                Some(addr) => {
                    let assoc_attempt = associate(&arena_request_tx, addr, None);
                    associate_queue.push(assoc_attempt);
                },
                /* terminate loop when the tx end of network_addr_rx is dropped */
                None => break,
            },
            Some(associate_result) = associate_queue.next() => {
                if let Err(Error::AssociateError(addr)) = associate_result {
                    let delay = Duration::from_millis(1000);
                    let assoc_attempt = associate(&arena_request_tx, addr, Some(delay));
                    associate_queue.push(assoc_attempt);
                }
            }
            else => break
        }
    }
}

async fn associate(arena_request_tx: &mpsc::UnboundedSender<arena::Request>,
                   addr: Ipv4Addr,
                   delay: Option<Duration>) -> Result<()> {
    /* optional sleep before attempting association */
    if let Some(duration) = delay {
        tokio::time::sleep(duration).await;
    }
    /* assume address is an xbee and attempt to connect for 500 ms */
    let xbee_attempt = tokio::time::timeout(Duration::from_millis(500), async {
        let device = xbee::Device::new(addr).await?;
        let addr = device.ip().await?;
        std::result::Result::<_, xbee::Error>::Ok((addr, device))
    }).await;
    /* inspect result */
    if let Ok(xbee_result) = xbee_attempt {
        if let Ok((xbee_addr, xbee_device)) = xbee_result {
            if xbee_addr == addr {
                return arena_request_tx.send(arena::Request::AddDrone(xbee_device))
                    .map_err(|_| Error::AssociateError(addr));
            }
        }
    }
    /* assume address is a device running the fernbedienung service and 
       attempt to connect for 500 ms */
    let fernbedienung_attempt = tokio::time::timeout(Duration::from_millis(500), async {
        let device = fernbedienung::Device::new(addr).await?;
        let hostname = device.hostname().await?;
        std::result::Result::<_, fernbedienung::Error>::Ok((hostname, device))
    }).await;
    /* inspect result */
    if let Ok(fernbedienung_result) = fernbedienung_attempt {
        if let Ok((hostname, device)) = fernbedienung_result {
            return match &hostname[..] {
                "raspberrypi0-wifi" | "ToshibaLaptop" =>
                    arena_request_tx.send(arena::Request::AddPiPuck(device))
                        .map_err(|_| Error::AssociateError(addr)),
                "up-core" =>
                    arena_request_tx.send(arena::Request::PairWithDrone(device))
                        .map_err(|_| Error::AssociateError(addr)),
                _ => Err(Error::AssociateError(addr)),
            };
        }
    }
    Err(Error::AssociateError(addr))
}
    
    
    
