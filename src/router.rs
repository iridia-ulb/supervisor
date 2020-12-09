use tokio_util::codec::{Decoder, Encoder, Framed};
use bytes::{BytesMut, Bytes, BufMut, Buf};
use std::{io, collections::HashMap, sync::Arc, net::{SocketAddr, IpAddr}};
use tokio::{net::TcpListener, sync::{mpsc, RwLock}};
use futures::StreamExt;

#[derive(Debug, Default)]
struct ByteArrayCodec {
    len: Option<usize>
}

impl Decoder for ByteArrayCodec {
    type Item = Bytes;
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Bytes>, io::Error> {
        if let Some(len) = self.len {
            if buf.len() >= len {
                self.len = None;
                return Ok(Some(buf.split_to(len).freeze()));
            }
        }
        else {
            if buf.len() >= 4 {
                self.len = Some(buf.get_u32() as usize);
            }
        }
        Ok(None)
    }
}

impl Encoder<Bytes> for ByteArrayCodec {
    type Error = io::Error;

    fn encode(&mut self, data: Bytes, buf: &mut BytesMut) -> Result<(), io::Error> {
        buf.reserve(data.len() + std::mem::size_of<u32>());
        buf.put_u32(data.len() as u32);
        buf.put(data);
        Ok(())
    }
}

type Peers = Arc<RwLock<HashMap<SocketAddr, mpsc::UnboundedSender<Bytes>>>>;

async fn route() -> io::Result<()> {
    let server_addr = SocketAddr::new(IpAddr::from([127, 0, 0, 1]), 4950);
    let mut listener = TcpListener::bind(server_addr).await?;
    eprintln!("Server started on: {}:{}", server_addr.ip(), server_addr.port());
    /* create an atomic map of all peers */
    let peers = Peers::default();
    /* start the main loop */
    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                /* clone peers so that in can be moved inside of the handler */
                let peers = Arc::clone(&peers);
                /* spawn a handler for the newly connected client */
                tokio::spawn(async move {
                    eprintln!("Client {}:{} connected", addr.ip(), addr.port());
                    /* set up a channel for communicating with other robot sockets */
                    let (tx, rx) = mpsc::unbounded_channel::<Bytes>();
                    /* wrap up socket in our ByteArrayCodec */
                    let (sink, mut stream) = 
                        Framed::new(stream, ByteArrayCodec::default()).split();
                    /* send and receive messages concurrently */
                    let _ = tokio::join!(rx.map(|msg| Ok(msg)).forward(sink), async {
                        peers.write().await.insert(addr, tx);
                        while let Some(msg) = stream.next().await {
                            match msg {
                                Ok(msg) => for (peer_addr, tx) in peers.read().await.iter() {
                                    /* do not send messages to the sending robot */
                                    if peer_addr != &addr {
                                        let _ = tx.send(msg.slice(..));
                                    }
                                },
                                Err(_) => break
                            }
                        }
                        peers.write().await.remove(&addr);
                    });
                    eprintln!("Client {}:{} disconnected", addr.ip(), addr.port());
                });
            }
            Err(err) => {
                eprintln!("Error accepting incoming connection: {}", err);
            }
        }   
    }
    // Ok(())
}
