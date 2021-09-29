use bytes::{BytesMut, Buf, BufMut};
use bitvec::view::BitView;
use futures::FutureExt;
use futures::{StreamExt, TryStreamExt, SinkExt, stream::FuturesUnordered};
use macaddr::MacAddr6;
use std::fmt::Debug;
use std::{collections::HashMap, convert::TryFrom, net::SocketAddr, ops::BitXor, time::Duration};
use std::net::Ipv4Addr;
use tokio::{net::UdpSocket, sync::{oneshot, mpsc}, time::Instant};
use tokio_util::{codec::{Decoder, Encoder}, udp::UdpFramed};

const CONFIG_CMD_LEN: usize = 12;
const CONFIG_CMD_HDR: u16 = 0x4242;
const CONFIG_CMD_REQ_ID: u8 = 0x02;
const CONFIG_CMD_RESP_ID: u8 = 0x82;
const CONFIG_CMD_RESP_OK: u8 = 0;
const SAMPLE_CMD_RESP_LEN: usize = 4;

const MAX_RETRIES: usize = 3;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    IoError(#[from] std::io::Error),

    #[error("Callback queue full")]
    CallbackQueueFull,

    #[error("No response")]
    NoResponse,

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
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum PinMode {
    Disable = 0,
    Alternate = 1,
    Input = 3,
    OutputDefaultLow = 4,
    OutputDefaultHigh = 5,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
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
    pub addr: Ipv4Addr,
    request_tx: mpsc::Sender<Request>,
    return_addr_tx: Option<oneshot::Sender<Ipv4Addr>>,
}

impl Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Xbee@{}", self.addr)
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        if let Some(return_addr_tx) = self.return_addr_tx.take() {
            let _ = return_addr_tx.send(self.addr);
        }
    }
}

enum Request {
    SetParameter([u8; 2], BytesMut, bool),
    GetParameter([u8; 2], oneshot::Sender<Result<BytesMut>>),
    ApplyChanges,
}

#[derive(Debug, Clone)]
struct Command {
    frame_id: u8,
    queue: bool,
    at_command: [u8; 2],
    data: Option<BytesMut>
}

struct CommandResponse {
    frame_id: u8,
    //at_command: [u8; 2],
    data: BytesMut,
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
        let _at_command = [source.get_u8(), source.get_u8()];
        let status = source.get_u8();
        if status != CONFIG_CMD_RESP_OK {
            return Err(Error::RemoteError{frame_id, status});
        }
        let data = match source.has_remaining() {
            true => source.split(),
            false => BytesMut::new(),
        };
        Ok(Some(CommandResponse { frame_id, data}))
    }
}

impl Device {
    pub async fn new(addr: Ipv4Addr, return_addr_tx: oneshot::Sender<Ipv4Addr>) -> Result<Device> {
        type RemoteRequest = (Instant, Option<oneshot::Sender<Result<BytesMut>>>, Command, usize);
        /* bind to a random port on any interface */
        let (request_tx, mut request_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let socket = match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await {
                Ok(socket) => socket,
                Err(_) => return,
            };
            let socket_addr = SocketAddr::new(addr.into(), 0xBEE);
            let mut framed = UdpFramed::new(socket, Codec);
            let mut remote_requests: HashMap<u8, RemoteRequest> = HashMap::new();
            let maintain_remote_requests_task = tokio::time::sleep(Duration::from_millis(100));
            tokio::pin!(maintain_remote_requests_task);
            loop {
                tokio::select!{
                    _ = &mut maintain_remote_requests_task => {
                        /* remove all remote requests whose callback has been closed or dropped */
                        remote_requests.retain(|_, (_, callback, _, _)| {
                            callback.as_ref().map_or(false, |callback| !callback.is_closed())
                        });
                        /* iterate over remote requests */
                        for (_, (timestamp, callback, command, retries)) in remote_requests.iter_mut() {
                            if timestamp.elapsed().as_millis() > 300 {
                                match retries {
                                    0 => {
                                        *callback = None;
                                    },
                                    _ => {
                                        *retries -= 1;
                                        *timestamp = Instant::now();
                                        let _ = framed.send((command.clone(), socket_addr)).await;
                                    }
                                }
                            }
                        }
                        maintain_remote_requests_task.set(tokio::time::sleep(Duration::from_millis(100)));
                    },
                    Some(frame) = framed.next() => match frame {
                        Ok((CommandResponse { frame_id, data, .. }, recv_addr)) => {
                            if recv_addr == socket_addr {
                                if let Some((_, Some(callback), _, _)) = remote_requests.remove(&frame_id) {
                                    let _ = callback.send(Ok(data));
                                }
                            }
                        }
                        Err(error) => if let Error::RemoteError{frame_id, status} = error {
                            if let Some((_, Some(callback), _, _)) = remote_requests.remove(&frame_id) {
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
                                    .find(|id| !remote_requests.contains_key(id));
                                if let Some(unused_id) = unused_id {
                                    let command = Command {
                                        frame_id: unused_id,
                                        queue: false,
                                        at_command: parameter,
                                        data: None
                                    };
                                    let remote_request = (Instant::now(), Some(callback), command.clone(), MAX_RETRIES);
                                    remote_requests.insert(unused_id, remote_request);
                                    let _ = framed.send((command, socket_addr)).await;
                                }
                                else {
                                    let _ = callback.send(Err(Error::CallbackQueueFull));
                                }
                            }
                        },
                        /* When Device is dropped the mpsc::Sender is closed  */
                        None => break,
                    }
                }
            }
        });
        Ok(Device { request_tx, addr, return_addr_tx: Some(return_addr_tx) })
    }

    pub async fn ip(&self) -> Result<Ipv4Addr> {
        let (response_tx, response_rx) = oneshot::channel();
        let request = Request::GetParameter([b'M',b'Y'], response_tx);
        self.request_tx.send(request).await.map_err(|_| Error::RequestFailed)?;
        let value = response_rx.await.map_err(|_| Error::NoResponse)??;
        <[u8; 4]>::try_from(&value[..])
            .map_err(|_| Error::DecodeError)
            .map(|addr| Ipv4Addr::from(addr))
    }

    pub async fn mac(&self) -> Result<MacAddr6> {
        /* get the upper 16 bits */
        let (response_tx, response_rx) = oneshot::channel();
        let request = Request::GetParameter([b'S',b'H'], response_tx);
        self.request_tx.send(request).await.map_err(|_| Error::RequestFailed)?;
        let mut value = response_rx.await.map_err(|_| Error::NoResponse)??;
        /* get the lower 32 bits */
        let (response_tx, response_rx) = oneshot::channel();
        let request = Request::GetParameter([b'S',b'L'], response_tx);
        self.request_tx.send(request).await.map_err(|_| Error::RequestFailed)?;
        value.extend_from_slice(&response_rx.await.map_err(|_| Error::NoResponse)??);
        /* try build a MacAddr6 from the responses */
        <[u8; 6]>::try_from(&value[..])
            .map_err(|_| Error::DecodeError)
            .map(|addr| MacAddr6::from(addr))
    }

    pub async fn link_margin(&self) -> Result<i32> {
        let (response_tx, response_rx) = oneshot::channel();
        let request = Request::GetParameter([b'L',b'M'], response_tx);
        self.request_tx.send(request).await.map_err(|_| Error::RequestFailed)?;
        let value = response_rx.await.map_err(|_| Error::NoResponse)??;
        value.first().cloned().map(|state| state as i32).ok_or(Error::DecodeError)
    }

    pub async fn pin_states(&self) -> Result<HashMap<Pin, bool>> {
        let (response_tx, response_rx) = oneshot::channel();
        let request = Request::GetParameter([b'I',b'S'], response_tx);
        self.request_tx.send(request).await.map_err(|_| Error::RequestFailed)?;
        let mut value = response_rx.await.map_err(|_| Error::NoResponse)??;
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
            }).collect::<Result<HashMap<_,_>>>()
        }
        else {
            Err(Error::DecodeError)
        }
    }

    pub async fn write_outputs(&self, config: &[(Pin, bool)]) -> Result<()> {
        let mut om_config: u16 = 0;
        let mut io_config: u16 = 0;
        for (pin, mode) in config.iter() {
            let bit: u16 = 1 << pin.clone() as usize;
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
        )).await.map_err(|_| Error::RequestFailed)?;
        self.request_tx.send(Request::SetParameter(
            [b'I', b'O'],
            BytesMut::from(&io_config.to_be_bytes()[..]),
            false
        )).await.map_err(|_| Error::RequestFailed)?;

        /* read back the pin states */
        let pin_states = self.pin_states().await?;
        /* check if they were set correctly */
        for (target_pin, target_mode) in config.iter() {
            pin_states.get(target_pin)
                .ok_or(Error::RequestFailed)
                .and_then(|mode| match target_mode == mode {
                    true => Ok(()),
                    false => Err(Error::RequestFailed)
                })?;
        }
        Ok(())
    }

    pub async fn set_pin_modes<'m, M>(&self, modes: M) -> Result<()>
        where M: Iterator<Item = &'m (Pin, PinMode)> {
        /* send pin configurations */
        let configuration = modes
            .map(|&(pin, mode)| {
                let request = Request::SetParameter(
                    <[u8; 2]>::from(pin),
                    BytesMut::from(&[mode as u8][..]),
                    true
                );
                self.request_tx.send(request).map(move |result| match result {
                    Ok(_) => Ok((pin, mode)),
                    Err(_) => Err(Error::RequestFailed)
                })
            })
            .collect::<FuturesUnordered<_>>()
            .try_collect::<Vec<_>>().await?;
        /* apply changes */
        self.request_tx.send(Request::ApplyChanges).await
            .map_err(|_| Error::RequestFailed)?;
        /* check if the modes were actually applied */
        configuration.into_iter()
            .map(|(pin, mode)| {
                let (response_tx, response_rx) = oneshot::channel();
                let request = Request::GetParameter(<[u8; 2]>::from(pin), response_tx);
                let result = self.request_tx.send(request);
                async move {
                    match result.await {
                        Ok(_) => match response_rx.await {
                            Ok(response) => match response {
                                Ok(mut response) => match response.has_remaining() {
                                    true => match response.get_u8() == mode as u8 {
                                        true => Ok(()),
                                        _ => Err(Error::RequestFailed) 
                                    },
                                    _ => Err(Error::RequestFailed)
                                },
                                Err(error) => Err(error)
                            },
                            Err(_) => Err(Error::RequestFailed),
                        },
                        Err(_) => Err(Error::RequestFailed),
                    }
                }
            })
            .collect::<FuturesUnordered<_>>()
            .try_collect::<Vec<_>>().await
            .map(|_| ())
    }

    pub async fn set_scs_mode(&self, tcp: bool) -> Result<()> {
        self.request_tx.send(Request::SetParameter(
            [b'I', b'P'],
            BytesMut::from(&(tcp as u8).to_be_bytes()[..]),
            false
        )).await.map_err(|_| Error::RequestFailed)?;
        let (response_tx, response_rx) = oneshot::channel();
        let request = Request::GetParameter([b'I',b'P'], response_tx);
        self.request_tx.send(request).await.map_err(|_| Error::RequestFailed)?;
        let mut response = response_rx.await.map_err(|_| Error::NoResponse)??;
        match response.len() {
            1 => match response.get_u8() == (tcp as u8) {
                true => Ok(()),
                false => Err(Error::RequestFailed)
            },
            _ => Err(Error::RequestFailed)
        }
    }

    pub async fn set_baud_rate(&self, baud_rate: u32) -> Result<()> {
        self.request_tx.send(Request::SetParameter(
            [b'B', b'D'],
            BytesMut::from(&baud_rate.to_be_bytes()[..]),
            false
        )).await.map_err(|_| Error::RequestFailed)?;
        let (response_tx, response_rx) = oneshot::channel();
        let request = Request::GetParameter([b'B',b'D'], response_tx);
        self.request_tx.send(request).await.map_err(|_| Error::RequestFailed)?;
        let mut response = response_rx.await.map_err(|_| Error::NoResponse)??;
        match response.len() {
            4 => {
                let baud_rate = baud_rate as f32;
                let selected_baud_rate = response.get_u32() as f32;
                let error = (baud_rate - selected_baud_rate).abs() / baud_rate;
                log::info!("Selected/target baud rate: {}/{} (error = {:.02}%)",
                    selected_baud_rate, baud_rate, error * 100.0);
                Ok(())
            },
            _ => Err(Error::RequestFailed)
        }
    }
}
