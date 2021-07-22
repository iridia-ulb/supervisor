use std::net::Ipv4Addr;
use bytes::Bytes;
use moonlight::serde_lite::{Deserialize, Serialize, Error, Intermediate, Number};

// #[derive(Debug, Clone)]
// pub enum Intermediate {
//     None,
//     Bool(bool),
//     Number(Number),
//     String(String),
//     Array(Vec<Intermediate>),
//     Map(Map),
// }

#[derive(Debug)]
pub enum Update {
    // sends camera footage
    Cameras(Vec<Bytes>),
    // indicates whether the connection is up or down
    FernbedienungConnection(Option<Ipv4Addr>),
    // indicates the signal strength
    FernbedienungSignal(u8)
}

impl Deserialize for Update {
    fn deserialize(val: &Intermediate) -> Result<Self, Error>
    where
        Self: Sized {
        match val {
            Intermediate::Array(vec) => {
                if let [Intermediate::String(key), value] = &vec[..] {
                    match &key[..] {
                        "cameras" => match value {
                            Intermediate::Array(cameras_data) => cameras_data
                                .into_iter()
                                .map(|data| match data {
                                    Intermediate::String(data) => base64::decode(data)
                                        .map_err(|inner| Error::InvalidValue(inner.to_string()))
                                        .map(|data| Bytes::from(data)),
                                    _ => Err(Error::InvalidValue(String::new()))
                                })
                                .collect::<Result<Vec<_>, _>>()
                                .map_err(|inner| Error::InvalidValue(inner.to_string()))
                                .map(|inner| Update::Cameras(inner)),
                            _ => Err(Error::InvalidValue(String::new()))
                        },
                        "fernbedienung_connection" => match value {
                            Intermediate::None => Ok(Update::FernbedienungConnection(None)),
                            Intermediate::String(address) => address
                                .parse::<Ipv4Addr>()
                                .map_err(|inner| Error::InvalidValue(inner.to_string()))
                                .map(|inner| Update::FernbedienungConnection(Some(inner))),
                            _ => Err(Error::InvalidValue(String::new()))
                        },
                        "fernbedienung_signal" => match value {
                            Intermediate::String(strength) => strength
                                .parse::<u8>()
                                .map_err(|inner| Error::InvalidValue(inner.to_string()))
                                .map(|inner| Update::FernbedienungSignal(inner)),
                            _ => Err(Error::InvalidValue(String::new()))
                        },
                        _ => Err(Error::InvalidKey(key.to_owned()))
                    }
                }
                else {
                    Err(Error::InvalidKey(String::new()))
                }
            },
            _ => Err(Error::InvalidValue(String::new()))
        }
    }
}

impl Serialize for Update {
    fn serialize(&self) -> Result<Intermediate, Error> {
        let ir = match self {
            Update::Cameras(cameras_data) => {
                vec![
                    "cameras".to_owned().into(),
                    Intermediate::Array(cameras_data
                        .into_iter()
                        .map(base64::encode)
                        .map(Intermediate::String)
                        .collect::<Vec<_>>())
                ]
            },
            Update::FernbedienungConnection(state) => {
                vec![
                    "fernbedienung_connection".to_owned().into(),
                    match state {
                        Some(addr) => Intermediate::String(addr.to_string()),
                        None => Intermediate::None,
                    }
                ]
            },
            Update::FernbedienungSignal(strength) => {
                vec![
                    "fernbedienung_signal".to_owned().into(),
                    Intermediate::Number(Number::UnsignedInt(u64::from(*strength)))
                ]
            },
        };
        Ok(Intermediate::Array(ir))
    }
}

