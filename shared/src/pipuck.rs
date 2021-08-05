use std::net::Ipv4Addr;
use bytes::Bytes;

#[derive(Debug)]
pub enum Update {
    // sends camera footage
    Camera(String, Result<Bytes, String>),
    // indicates whether the connection is up or down
    FernbedienungConnection(Option<Ipv4Addr>),
    // indicates the signal strength
    FernbedienungSignal(Result<u64, String>)
}



