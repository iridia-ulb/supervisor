use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use futures::{StreamExt, SinkExt, TryStreamExt, TryFutureExt};

use tokio::net::TcpStream;
use tokio_serde::formats::SymmetricalJson;
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

mod protocol;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    IoError(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct Device {
    pub addr: Ipv4Addr,
    stream: TcpStream,
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device")
         .field("addr", &self.addr)
         .finish()
    }
}

impl Device {
    pub async fn new(addr: Ipv4Addr) -> Result<Device> {
        TcpStream::connect((addr, 17653)).await
            .map_err(|error| Error::IoError(error))
            .map(|stream| Device { addr, stream})
    }

    pub async fn upload<P, F, C>(&mut self, path: P, filename: F, contents: C, permissions: usize) -> Result<()>
    where P: Into<PathBuf>, F: Into<PathBuf>, C: Into<Vec<u8>> {
        Ok(())
    }

    pub async fn hostname(&self) -> Result<String> {
        Ok("Bob".to_owned())
    }

    pub async fn create_temp_dir(&self) -> Result<PathBuf> {
        Ok("/".into())
    }

    pub async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    pub async fn reboot(&self) -> Result<()> {
        Ok(())
    }

    pub async fn run<T, W, A>(&mut self, target: T, working_dir: W, args: A) -> Result<String> 
        where T: Into<PathBuf>, W: Into<PathBuf>, A: Into<Vec<String>> {
        let task = protocol::Run {
            target: target.into(),
            working_dir: working_dir.into(),
            args: args.into()
        };
        Ok("Bob".to_owned())
    }
}





