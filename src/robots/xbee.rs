use bytes::BufMut;
use tokio::net::UdpSocket;
use std::convert::TryFrom;
use std::net::Ipv4Addr;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Xbee IP address mismatch")]
    Ipv4Mismatch,
    #[error(transparent)]
    IoError(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct Device {
    pub addr: Ipv4Addr,
    handle: UdpSocket,
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
        let mut handle = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
        handle.connect((addr, 3054)).await?;
        let command = Command::new("MY", &[]);
        handle.send(&command.0).await?;
        let mut rx_buffer = [0; 16];
        handle.recv_from(&mut rx_buffer).await?;
        if let Ok(reply_addr) = <[u8; 4]>::try_from(&rx_buffer[12..16]) {
            if Ipv4Addr::from(reply_addr) == addr {
                return Ok(Device { addr, handle })
            }
        }
        Err(Error::Ipv4Mismatch)
    }

    pub async fn send(&mut self, cmd: Command) -> Result<()> {
        self.handle.send(&cmd.0).await?;
        Ok(())
    }
}

pub struct Command(Vec<u8>);

impl Command {
    pub fn new(command: &str, arguments: &[u8]) -> Command {
        let mut packet = Vec::with_capacity(10 + command.len() + arguments.len());
        /* preamble */
        packet.put_u16(0x4242);
        packet.put_u16(0x0000);
        /* packet id */
        packet.put_u8(0x00);
        /* encryption */
        packet.put_u8(0x00);
        /* command id (remote AT command) */
        packet.put_u8(0x02);
        /* command options (none) */
        packet.put_u8(0x00);
        /* frame id */
        packet.put_u8(0x01);
        /* config options (apply immediately) */
        packet.put_u8(0x02);
        /* at command */
        packet.put(command.as_bytes());
        /* at command arguments */
        packet.put(arguments);
        /* return the vector as the result */
        Command(packet)
    }
}






