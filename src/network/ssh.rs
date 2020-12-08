use thrussh_keys::key::PublicKey;
use futures::{future, future::Ready};
//use itertools::Itertools;
use std::{
    sync::Arc,
    net::Ipv4Addr,
    io::{
        Read,
        Cursor
    },
    path::Path,
    time::{Duration, Instant},
};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Could not connect to robot")]
    ConnectionFailure,
    #[error("Could not login to robot")]
    LoginFailure,
    #[error("Could not create robot channel")]
    ChannelFailure,
    #[error("Could not communicate with robot")]
    IoFailure,
    #[error("Not connected to robot")]
    Disconnected,
    /*
    #[error("Connection timed out")]
    Timeout,
    */
}

pub type Result<T> = std::result::Result<T, Error>;

pub enum State {
    Disconnected,
    Connected {
        handle: thrussh::client::Handle,
        shell: Shell,
    }
}

pub struct Device {
    pub addr: Ipv4Addr,
    pub state: State,
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device")
         .field("address", &self.addr)
         .finish()
    }
}

const CONFIRM: &'static [u8] = &[0];

impl Device {
    pub fn new(addr: Ipv4Addr) -> Self {
        Device {addr, state: State::Disconnected}
    }

    pub async fn connect(&mut self) -> Result<()> {
        let config = Arc::new(thrussh::client::Config::default());
        let mut handle = 
            thrussh::client::connect(config, (self.addr, 22), Callbacks::default()).await
            .map_err(|_| Error::ConnectionFailure)?;
        let login_success = handle.authenticate_password("root", "").await
            .map_err(|_| Error::ConnectionFailure)?;
        if login_success == false {
            return Err(Error::LoginFailure);
        }
        let shell = Shell::new(&mut handle).await?;
        self.state = State::Connected{ handle, shell };
        Ok(())
    }

    pub fn disconnect(&mut self) {
        self.state = State::Disconnected;
    }

    pub fn default_shell(&mut self) -> Result<&mut Shell> {
        match self.state {
            State::Connected { ref mut shell, .. } => Ok(shell),
            State::Disconnected => Err(Error::Disconnected)
        }
    }

    /*
    pub async fn set_hostname(&mut self, hostname: &str) -> Result<bool> {
        // use hostname XYZ and not hostnamectl XYZ since the former is not persistent
        Ok(false)
    }
    */
    
    pub async fn upload<P, F, C>(&mut self, path: P, filename: F, contents: C, permissions: usize) -> Result<()>
        where P: AsRef<Path>, F: AsRef<Path>, C: AsRef<[u8]> {
        if let State::Connected { handle, .. } = &mut self.state {
            let path = path.as_ref();
            let filename = filename.as_ref();
            let contents = contents.as_ref();
            let mut channel = handle.channel_open_session().await
                .map_err(|_| Error::ChannelFailure)?;
            let command = format!("scp -t {}", path.to_string_lossy());
            channel.exec(false, command).await
                .map_err(|_| Error::IoFailure)?;
            let header = format!("C0{:o} {} {}\n",
                permissions,
                contents.len(),
                filename.to_string_lossy());
            /* chain the data together */
            let mut wrapped_data = Cursor::new(header)
                .chain(Cursor::new(contents))
                .chain(Cursor::new(CONFIRM));
            /* create an intermediate buffer for moving the data */
            let mut buffer = [0; 32];
            /* transfer the data */
            while let Ok(count) = wrapped_data.read(&mut buffer) {
                if count != 0 {
                    channel.data(&buffer[0..count]).await
                        .map_err(|_| Error::IoFailure)?;
                }
                else {
                    break;
                }
            }               
            channel.eof().await.map_err(|_| Error::ChannelFailure)
        }
        else {
            Err(Error::Disconnected)
        }
    }

    pub async fn _ping(&mut self) -> Result<Duration> {
        let start = Instant::now();
        self.hostname().await?;
        Ok(start.elapsed())
    }

    pub async fn hostname(&mut self) -> Result<String> {
        if let State::Connected { shell, .. } = &mut self.state {
            shell.exec("hostname").await
        }
        else {
            Err(Error::Disconnected)
        }
    }
}

pub struct Shell(thrussh::client::Channel);

impl Shell {
    pub async fn new(handle: &mut thrussh::client::Handle) -> Result<Self> {
        let mut channel = handle.channel_open_session().await
            .map_err(|_| Error::ChannelFailure)?;
        channel.request_shell(true).await
            .map_err(|_| Error::IoFailure)?;
        Ok(Self(channel))
    }

    pub async fn exec<A: AsRef<str>>(&mut self, command: A) -> Result<String> {
        /* wrap command so that its output is wrapped with SOT and EOT markers */
        let padded_command =
            format!("printf \"%b%s%b\" \"\\002\" \"`{}`\" \"\\003\"\n", command.as_ref());
        /* write command */
        self.0.data(padded_command.as_bytes()).await
            .map_err(|e| { log::error!("Failed to execute command over SSH: {}", e); Error::IoFailure})?;
        /* get reply */
        let mut buffer = Vec::new();
        while let Some(message) = self.0.wait().await {
            if let thrussh::ChannelMsg::Data { data } = message {
                buffer.extend(data.to_vec());
                /* search for SOT and EOT in stream */
                if let Some(start) = buffer.iter().position(|b| b == &2_u8) {
                    if let Some(end) = buffer.iter().rposition(|b| b == &3_u8) {
                        if start < end {
                            return std::str::from_utf8(&buffer[start+1..end])
                                .map_err(|_| Error::IoFailure)
                                .map(String::from);
                        }
                    }
                }
            }
        }
        Err(Error::IoFailure)
    }
}

#[derive(Default)]
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
