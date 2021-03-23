use bytes::{BytesMut, Buf, BufMut};
use bitvec::view::BitView;

use futures::{FutureExt, SinkExt, future};
use tokio::{net::UdpSocket, sync::{oneshot, mpsc}, time::Instant};
use tokio_stream::{StreamExt, wrappers::UnboundedReceiverStream};
use tokio_util::{codec::{Decoder, Encoder}, udp::UdpFramed};

use std::{collections::HashMap, convert::TryFrom, net::SocketAddr, ops::BitXor, time::Duration};
use std::net::Ipv4Addr;

const CONFIG_CMD_LEN: usize = 12;
const CONFIG_CMD_HDR: u16 = 0x4242;
const CONFIG_CMD_REQ_ID: u8 = 0x02;
const CONFIG_CMD_RESP_ID: u8 = 0x82;
const CONFIG_CMD_RESP_OK: u8 = 0;
const SAMPLE_CMD_RESP_LEN: usize = 4;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    IoError(#[from] std::io::Error),

    #[error("Callback queue full")]
    CallbackQueueFull,

    #[error("Callback dropped")]
    CallbackDropped,

    #[error("Request failed")]
    RequestFailed,

    #[error("Decode error")]
    DecodeError,

    #[error("Response corrupt")]
    CorruptResponse,

    #[error("Remote error: {status}")]
    RemoteError{ frame_id: u8, status: u8},
}

#[repr(u8)]
pub enum PinMode {
    Disable = 0,
    Alternate = 1,
    Input = 3,
    OutputDefaultLow = 4,
    OutputDefaultHigh = 5,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Pin {
    DIO0 = 0,
    DIO1 = 1,
    DIO2 = 2,
    DIO3 = 3,
    DIO4 = 4,
    DIO5 = 5,
    DIO6 = 6,
    DIO7 = 7,
    DIO8 = 8,
    DIO9 = 9,
    DIO10 = 10,
    DIO11 = 11,
    DIO12 = 12,
    DIN = 13,
    DOUT = 14,
}

impl From<Pin> for [u8; 2] {
    fn from(pin: Pin) -> Self {
        match pin {
            Pin::DIO0 =>  [b'D', b'0'],
            Pin::DIO1 =>  [b'D', b'1'],
            Pin::DIO2 =>  [b'D', b'2'],
            Pin::DIO3 =>  [b'D', b'3'],
            Pin::DIO4 =>  [b'D', b'4'],
            Pin::DIO5 =>  [b'D', b'5'],
            Pin::DIO6 =>  [b'D', b'6'],
            Pin::DIO7 =>  [b'D', b'7'],
            Pin::DIO8 =>  [b'D', b'8'],
            Pin::DIO9 =>  [b'D', b'9'],
            Pin::DIO10 => [b'P', b'0'],
            Pin::DIO11 => [b'P', b'1'],
            Pin::DIO12 => [b'P', b'2'],
            Pin::DIN =>   [b'P', b'3'],
            Pin::DOUT =>  [b'P', b'4'],
        }
    }
}

impl TryFrom<usize> for Pin {
    type Error = Error;

    fn try_from(value: usize) -> Result<Self> {
        match value {
            0 => Ok(Pin::DIO0),
            1 => Ok(Pin::DIO1),
            2 => Ok(Pin::DIO2),
            3 => Ok(Pin::DIO3),
            4 => Ok(Pin::DIO4),
            5 => Ok(Pin::DIO5),
            6 => Ok(Pin::DIO6),
            7 => Ok(Pin::DIO7),
            8 => Ok(Pin::DIO8),
            9 => Ok(Pin::DIO9),
            10 => Ok(Pin::DIO10),
            11 => Ok(Pin::DIO11),
            12 => Ok(Pin::DIO12),
            13 => Ok(Pin::DIN),
            14 => Ok(Pin::DOUT),
            _ => Err(Error::DecodeError)
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

struct Codec;

pub struct Device {
    request_tx: mpsc::UnboundedSender<Request>,
    pub addr: Ipv4Addr
}

enum Request {
    SetParameter([u8; 2], BytesMut, bool),
    GetParameter([u8; 2], oneshot::Sender<Result<BytesMut>>),
    ApplyChanges,
}

struct Command {
    frame_id: u8,
    queue: bool,
    at_command: [u8; 2],
    data: Option<BytesMut>
}

struct CommandResponse {
    frame_id: u8,
    at_command: [u8; 2],
    data: Option<BytesMut>,
}

impl Encoder<Command> for Codec {
    type Error = Error;

    fn encode(&mut self, command: Command, destination: &mut BytesMut) -> Result<()> {
        /* destructure */
        let Command { frame_id, queue, at_command, data } = command;
        /* reserve sufficient space */
        let length = CONFIG_CMD_LEN + match data {
            Some(ref data) => data.len(),
            None => 0,
        };
        destination.reserve(length);
        /* header */
        destination.put_u32(0x4242_0000);
        /* packet id */
        destination.put_u8(0x00);
        /* encryption */
        destination.put_u8(0x00);
        /* command type (remote AT command) */
        destination.put_u8(CONFIG_CMD_REQ_ID);
        /* command options (none) */
        destination.put_u8(0x00);
        destination.put_u8(frame_id);
        match queue {
            true => destination.put_u8(0x00),
            false => destination.put_u8(0x02),
        };
        destination.put(&at_command[..]);
        if let Some(data) = data {
            destination.put(&data[..])
        }
        Ok(())
    }
}

impl Decoder for Codec {
    type Item = CommandResponse;
    type Error = Error;

    fn decode(&mut self, source: &mut BytesMut) -> Result<Option<Self::Item>> {
        if source.len() < CONFIG_CMD_LEN {
            return Ok(None);
        }
        if source.get_u16().bitxor(source.get_u16()) != CONFIG_CMD_HDR {
            return Err(Error::CorruptResponse);
        }
        /* skip packet id and encryption pad */
        source.advance(2); 
        if source.get_u8() != CONFIG_CMD_RESP_ID {
            return Err(Error::CorruptResponse);
        }
        /* skip command options */
        source.advance(1);
        let frame_id = source.get_u8();
        let at_command = [source.get_u8(), source.get_u8()];
        let status = source.get_u8();
        if status != CONFIG_CMD_RESP_OK {
            return Err(Error::RemoteError{frame_id, status});
        }
        let data = match source.has_remaining() {
            true => Some(source.split()),
            false => None,
        };
        Ok(Some(CommandResponse { frame_id, at_command, data}))
    }
}

impl Device {
    pub async fn new(addr: Ipv4Addr, return_addr_tx: mpsc::UnboundedSender<Ipv4Addr>) -> Result<Device> {
        /* bind to a random port on any interface */
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
        let (request_tx, mut request_rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let socket_addr = SocketAddr::new(addr.into(), 0xBEE);
            let mut framed = UdpFramed::new(socket, Codec);
            let mut callbacks: HashMap<u8, (Instant, oneshot::Sender<Result<BytesMut>>)> = HashMap::new();
            let delay_task = future::ready(()).left_future();
            tokio::pin!(delay_task);
            loop {
                tokio::select!{
                    _ = &mut delay_task => {
                        callbacks.retain(|_, (timestamp, callback)| {
                            !(callback.is_closed() || timestamp.elapsed().as_millis() > 500)
                        });
                        delay_task.set(tokio::time::sleep(Duration::from_millis(500)).right_future());
                    },
                    Some(frame) = framed.next() => match frame {
                        Ok((CommandResponse { frame_id, data, .. }, recv_addr)) => {
                            if recv_addr == socket_addr {
                                if let Some((_, callback)) = callbacks.remove(&frame_id) {
                                    if let Some(data) = data {
                                        let _ = callback.send(Ok(data));
                                    }
                                }
                            }
                        }
                        Err(error) => if let Error::RemoteError{frame_id, status} = error {
                            if let Some((_, callback)) = callbacks.remove(&frame_id) {
                                let _ = callback.send(Err(Error::RemoteError{frame_id, status}));
                            }
                        }
                    },
                    request = request_rx.recv() => match request {
                        Some(request) => match request {
                            Request::ApplyChanges => {
                                let command = Command {
                                    frame_id: 0,
                                    queue: false,
                                    at_command: [b'A', b'C'],
                                    data: None
                                };
                                let _ = framed.send((command, socket_addr)).await;
                            },
                            Request::SetParameter(parameter, data, queue) => {
                                let command = Command {
                                    frame_id: 0,
                                    queue: queue,
                                    at_command: parameter,
                                    data: Some(data)
                                };
                                let _ = framed.send((command, socket_addr)).await;
                            },
                            Request::GetParameter(parameter, callback) => {
                                /* find an unused key */
                                let unused_id = (1..u8::MAX).into_iter()
                                    .find(|id| !callbacks.contains_key(id));
                                if let Some(unused_id) = unused_id {
                                    callbacks.insert(unused_id, (Instant::now(), callback));
                                    let command = Command {
                                        frame_id: unused_id,
                                        queue: false,
                                        at_command: parameter,
                                        data: None
                                    };
                                    let _ = framed.send((command, socket_addr)).await;
                                }
                                else {
                                    let _ = callback.send(Err(Error::CallbackQueueFull));
                                }
                            }
                        },
                        /* this should cause this task to shutdown as soon as Device is dropped */
                        None => {
                            let _ = return_addr_tx.send(addr);
                            break
                        },
                    }
                }
            }
        });
        Ok(Device { request_tx, addr })
    }

    pub async fn ip(&self) -> Result<Ipv4Addr> {
        let (response_tx, response_rx) = oneshot::channel();
        let request = Request::GetParameter([b'M',b'Y'], response_tx);
        self.request_tx.send(request).map_err(|_| Error::RequestFailed)?;
        let value = response_rx.await.map_err(|_| Error::CallbackDropped)??;
        <[u8; 4]>::try_from(&value[..])
            .map_err(|_| Error::DecodeError)
            .map(|addr| Ipv4Addr::from(addr))
    }

    pub async fn ap(&self) -> Result<String> {
        let (response_tx, response_rx) = oneshot::channel();
        let request = Request::GetParameter([b'I',b'D'], response_tx);
        self.request_tx.send(request).map_err(|_| Error::RequestFailed)?;
        let value = response_rx.await.map_err(|_| Error::CallbackDropped)??;
        String::from_utf8(value.to_vec()).map_err(|_| Error::DecodeError)
    }

    pub async fn link_state(&self) -> Result<u8> {
        let (response_tx, response_rx) = oneshot::channel();
        let request = Request::GetParameter([b'L',b'M'], response_tx);
        self.request_tx.send(request).map_err(|_| Error::RequestFailed)?;
        let value = response_rx.await.map_err(|_| Error::CallbackDropped)??;
        value.first().cloned().ok_or(Error::DecodeError)
    }

    pub async fn pin_states(&self) -> Result<Vec<(Pin, bool)>> {
        let (response_tx, response_rx) = oneshot::channel();
        let request = Request::GetParameter([b'I',b'S'], response_tx);
        self.request_tx.send(request).map_err(|_| Error::RequestFailed)?;
        let mut value = response_rx.await.map_err(|_| Error::CallbackDropped)??;
        if value.len() < SAMPLE_CMD_RESP_LEN {
            return Err(Error::DecodeError);
        }
        let _sample_sets = value.get_u8();
        let digital_mask = value.get_u16();
        let _analog_mask = value.get_u8();
        if digital_mask != 0 && value.remaining() >= 2 {
            let digital_samples = value.get_u16();
            let digital_mask = digital_mask.view_bits::<bitvec::order::Lsb0>();
            let digital_samples = digital_samples.view_bits::<bitvec::order::Lsb0>();
            digital_mask.iter_ones().map(|index| {
                <Pin>::try_from(index).map(|pin| (pin, digital_samples[index]))
            }).collect::<Result<Vec<_>>>()
        }
        else {
            Err(Error::DecodeError)
        }
    }

    pub fn write_outputs(&self, config: Vec<(Pin, bool)>) -> Result<()> {
        let mut om_config: u16 = 0;
        let mut io_config: u16 = 0;
        for (pin, mode) in config.into_iter() {
            let bit: u16 = 1 << pin as usize;
            om_config |= bit;
            match mode {
                true => io_config |= bit,
                false => io_config &= !bit,
            }
        }
        self.request_tx.send(Request::SetParameter(
            [b'O', b'M'], 
            BytesMut::from(&om_config.to_be_bytes()[..]),
            true
        )).map_err(|_| Error::RequestFailed)?;
        self.request_tx.send(Request::SetParameter(
            [b'I', b'O'],
            BytesMut::from(&io_config.to_be_bytes()[..]),
            false
        )).map_err(|_| Error::RequestFailed)
    }

    pub fn set_pin_modes(&self, modes: Vec<(Pin, PinMode)>) -> Result<()> {
        for (pin, mode) in modes.into_iter() {
            let request = 
                Request::SetParameter(<[u8; 2]>::from(pin),
                                      BytesMut::from(&[mode as u8][..]),
                                      true);
            self.request_tx.send(request).map_err(|_| Error::RequestFailed)?
        }
        self.request_tx.send(Request::ApplyChanges).map_err(|_| Error::RequestFailed)
    }
}
