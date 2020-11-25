use bytes::{buf::ext::BufExt, BytesMut};
use futures::StreamExt;
use natnet_decode::{
    NatNet,
    NatNetResponse,
    ParseError,
    FrameOfData
};
use semver::Version;
use std::{
    io::{
        self, BufReader
    },
    net::Ipv4Addr,
};
use tokio::net::UdpSocket;
use tokio_util::{
    udp::UdpFramed,
    codec::Decoder,
};

#[derive(Debug)]
struct NatNetCodec {
    version: Version,
}

impl NatNetCodec {
    fn new(version: &str) -> Self {
        NatNetCodec {
            version: Version::parse(version).unwrap(),
        }
    }
}

impl Decoder for NatNetCodec {
    type Item = NatNetResponse;
    type Error = ParseError;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<NatNetResponse>, ParseError> {
        let buf = buf.split().freeze();
        let mut buf = BufReader::new(buf.reader());
        NatNet::unpack_with(&self.version, &mut buf).map(Option::from)
    }
}

pub async fn once() -> io::Result<FrameOfData> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 1511)).await?;
    socket.join_multicast_v4(Ipv4Addr::new(239,255,42,99), Ipv4Addr::UNSPECIFIED)?;
    let mut responses = UdpFramed::new(socket, NatNetCodec::new("2.9.0"));
    while let Some(Ok(response)) = responses.next().await {
        if let NatNetResponse::FrameOfData(frame_of_data) = response.0 {
            return Ok(frame_of_data);
        }
    }
    Err(io::Error::new(io::ErrorKind::ConnectionReset, "No more data"))
}

pub async fn stream() -> io::Result<()> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 1511)).await?;
    socket.join_multicast_v4(Ipv4Addr::new(239,255,42,99), Ipv4Addr::UNSPECIFIED)?;
    let mut responses = UdpFramed::new(socket, NatNetCodec::new("2.9.0"));
    while let Some(response) = responses.next().await {
        eprintln!("response = {:?}", response)
    }
    Ok(())
}
