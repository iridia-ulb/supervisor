use thrussh_keys::key::PublicKey;
use futures::{future, future::Ready};
use std::{sync::Arc, net::Ipv4Addr};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Could not connect to server")]
    ConnectionFailure,
    #[error("Could not login to server")]
    LoginFailure,
    /*
    #[error("Connection timed out")]
    Timeout,
    */
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct Device {
    pub addr: Ipv4Addr,
    handle: thrussh::client::Handle
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device")
         .field("addr", &self.addr)
         .finish()
    }
}

impl Device {
    pub async fn new(addr: Ipv4Addr) -> Result<Self> {
        let config : thrussh::client::Config = Default::default();
        let config = Arc::new(config);
        let callbacks = Callbacks {};
        let mut handle = thrussh::client::connect(config, (addr, 22), callbacks)
            .await
            .map_err(|_| Error::ConnectionFailure)?;
        if handle.authenticate_password("root", "")
            .await
            .map_err(|_| Error::ConnectionFailure)? {
            Ok(Device { addr, handle })
        }
        else {
            Err(Error::LoginFailure)
        }
    }

    pub async fn set_hostname(&mut self, hostname: &str) -> Result<bool> {
        // use hostname XYZ and not hostnamectl XYZ since the former is not persistent
        Ok(false)
    }
    
    // TODO: perhaps change this into a generic exec method
    pub async fn hostname(&mut self) -> Result<String> {
        let mut channel = self.handle.channel_open_session()
            .await
            .map_err(|_| Error::ConnectionFailure)?;
        channel.exec(true, "hostname")
            .await
            .map_err(|_| Error::ConnectionFailure)?;
        while let Some(message) = channel.wait().await {
            if let thrussh::ChannelMsg::Data { data } = message {
                if let Ok(mut hostname) = String::from_utf8(data.to_vec()) {
                    /* strip away any newlines/spacing */
                    hostname.retain(|c| !c.is_whitespace());
                    return Ok(hostname);
                }
            }
        }
        return Err(Error::ConnectionFailure)
    }
}

struct Callbacks {}

impl thrussh::client::Handler for Callbacks {
    type FutureUnit = 
        Ready<anyhow::Result<(Self, thrussh::client::Session)>>;
    type FutureBool = 
        Ready<anyhow::Result<(Self, bool)>>;

    fn finished_bool(self, b: bool) -> Self::FutureBool {
        future::ready(Ok((self, b)))
    }

    fn finished(self, session: thrussh::client::Session) -> Self::FutureUnit {
        future::ready(Ok((self, session)))
    }

    fn check_server_key(self, _server_public_key: &PublicKey) -> Self::FutureBool {
        self.finished_bool(true)
    }
}


