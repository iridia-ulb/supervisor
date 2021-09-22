use anyhow::Context;
use bytes::{Buf, BytesMut};
use natnet_decode::{
    NatNet,
    NatNetResponse,
    ParseError,
};
use semver::Version;
use std::{io::Cursor, net::Ipv4Addr};
use futures::StreamExt;
use tokio::{net::UdpSocket, sync::{broadcast, mpsc, oneshot}};
use tokio_util::{udp::UdpFramed, codec::Decoder};
use shared::tracking_system::Update;

#[derive(Debug)]
struct NatNetCodec {
    version: Version,
}

impl NatNetCodec {
    fn new(version: Version) -> Self {
        NatNetCodec { version }
    }
}

#[derive(Debug)]
pub struct Configuration {
    pub version: semver::Version,
    pub bind_addr: Ipv4Addr,
    pub bind_port: u16,
    pub multicast_addr: Ipv4Addr,
    pub iface_addr: Ipv4Addr,
}

impl Decoder for NatNetCodec {
    type Item = NatNetResponse;
    type Error = ParseError;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<NatNetResponse>, ParseError> {
        let mut cursor = Cursor::new(buf.as_ref());
        match NatNet::unpack_with(&self.version, &mut cursor) {
            Ok(response) => {
                let position = cursor.position() as usize;
                buf.advance(position);
                Ok(Some(response))
            }
            Err(ParseError::NotEnoughBytes) => {
                Ok(None)
            }
            Err(inner) => Err(inner)
        }
    }
}

pub enum Action {
    Subscribe(oneshot::Sender<broadcast::Receiver<Vec<Update>>>),
}

pub async fn new(config: Configuration, mut requests: mpsc::Receiver<Action>) -> anyhow::Result<()> {
    let socket = UdpSocket::bind((config.bind_addr, config.bind_port)).await
        .context("Could not bind to port")?;
    socket.join_multicast_v4(config.multicast_addr, config.iface_addr)
        .context("Could not join multicast group")?;
    let (updates_tx, _) = broadcast::channel(32);
    let mut stream = UdpFramed::new(socket, NatNetCodec::new(config.version));
    loop {
        tokio::select! {
            request = requests.recv() => match request {
                Some(action) => match action {
                    Action::Subscribe(callback) => {
                        let _ = callback.send(updates_tx.subscribe());
                    }
                },
                None => break,
            },
            Some(data) = stream.next() => match data {
                Ok(decoded) => if let (NatNetResponse::FrameOfData(frame), _) = decoded {
                    let updates = frame.rigid_bodies.iter()
                        .map(|body| Update {
                            id: body.id,
                            position: [
                                body.position.x,
                                body.position.y,
                                body.position.z
                            ],
                            orientation: [
                                body.orientation.w,
                                body.orientation.i,
                                body.orientation.j,
                                body.orientation.k
                            ],
                        })
                        .collect::<Vec<_>>();
                    let _ = updates_tx.send(updates);
                }
                Err(error) => {
                    log::warn!("Could not decode optitrack data: {}", error);
                }
            }
        }
    }
    Ok(())
}
