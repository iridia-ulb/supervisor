use thrussh_keys::key::PublicKey;
use futures::{future, future::Ready};
use std::{sync::Arc, net::Ipv4Addr};
use tokio::sync::Mutex;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Could not connect to server")]
    ConnectionFailure,
    #[error("Could not login to server")]
    LoginFailure,
    #[error("Could not communicate with server")]
    IoFailure,
    /*
    #[error("Connection timed out")]
    Timeout,
    */
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct Device {
    pub addr: Ipv4Addr,
    handle: thrussh::client::Handle,
    shell: Mutex<thrussh::client::Channel>
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
        let mut handle = 
            thrussh::client::connect(config, (addr, 22), callbacks).await
            .map_err(|_| Error::ConnectionFailure)?;
        let login_success = handle.authenticate_password("root", "").await
            .map_err(|_| Error::ConnectionFailure)?;
        if login_success == false {
            return Err(Error::LoginFailure);
        }
        let mut shell = handle.channel_open_session().await
            .map_err(|_| Error::IoFailure)?;
        shell.request_shell(true).await
            .map_err(|_| Error::IoFailure)?;
        Ok(Device { addr, handle, shell: Mutex::new(shell) })
    }

    /*
    pub async fn set_hostname(&mut self, hostname: &str) -> Result<bool> {
        // use hostname XYZ and not hostnamectl XYZ since the former is not persistent
        Ok(false)
    }
    */

    pub async fn exec<A: Into<String>>(&mut self, command: A, want_reply: bool) -> Result<Option<String>> {
        /* lock the shell */
        let mut shell = self.shell.lock().await;
        /* write command */
        shell.data(format!("{}\n", command.into()).as_bytes()).await
            .map_err(|_| Error::IoFailure)?;
        /* get reply */
        if want_reply {
            while let Some(message) = shell.wait().await {
                if let thrussh::ChannelMsg::Data { data } = message {
                    if let Ok(data) = String::from_utf8(data.to_vec()) {
                        return Ok(Some(data));
                    }
                }
            }
            return Err(Error::IoFailure);
        }
        Ok(None)       
    }

    pub async fn hostname(&mut self) -> Result<String> {
        match self.exec("hostname", true).await? {
            Some(mut hostname) => {
                hostname.retain(|c| !c.is_whitespace());
                Ok(hostname)
            }
            None => {
                Err(Error::IoFailure)
            }
        }        
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


