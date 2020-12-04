use thrussh_keys::key::PublicKey;
use futures::{future, future::Ready};
use itertools::Itertools;
use std::{
    sync::Arc,
    net::Ipv4Addr,
    io::{
        Read,
        Cursor
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
    ops::{Deref, DerefMut},
};
use tokio::sync::MutexGuard;
use tokio::sync::Mutex;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Could not connect to server")]
    ConnectionFailure,
    #[error("Could not login to server")]
    LoginFailure,
    #[error("Could not create channel")]
    ChannelFailure,
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
    pub shell: Shell,
    handle: thrussh::client::Handle,
    ctrl_software_path: Option<PathBuf>,
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device")
         .field("addr", &self.addr)
         .finish()
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

const CONFIRM: &'static [u8] = &[0];

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
        let shell = Shell::new(&mut handle).await?;
        Ok(Device { addr, shell, handle, ctrl_software_path: None })
    }

    /* add an ARGoS trait which requires the implementation of a method that gives the SSH device
       back and then provides these methods on top of it */
    pub async fn clear_ctrl_software(&mut self) -> Result<()> {
        let path = self.shell.exec("mktemp -d").await?;
        self.ctrl_software_path = Some(path).map(PathBuf::from);
        Ok(())
    }

    pub async fn add_ctrl_software<F, C>(&mut self, filename: F, contents: C) -> Result<()>
        where F: AsRef<Path>, C: AsRef<[u8]> {
        let directory = self.ctrl_software_path.as_ref()
            .map(PathBuf::to_owned)
            .ok_or(Error::IoFailure)?;
        self.upload(filename, directory, contents, 0o644).await
    }

    pub async fn ctrl_software(&mut self) -> Result<Vec<(String, String)>> {
        if let Some(path) = &self.ctrl_software_path {
            let query = format!("find {} -type f -exec md5sum '{{}}' \\;", path.to_string_lossy());
            let response = self.shell.exec(query).await?
                .split_whitespace()
                .tuples::<(_, _)>()
                .map(|(c, p)| (String::from(c), String::from(p)))
                .collect::<Vec<_>>();
            Ok(response)
        }
        else {
            Ok(Vec::new())
        }
    }

    /*
    pub async fn set_hostname(&mut self, hostname: &str) -> Result<bool> {
        // use hostname XYZ and not hostnamectl XYZ since the former is not persistent
        Ok(false)
    }
    */
    
    pub async fn upload<F, D, C>(&mut self, filename: F, directory: D, contents: C, permissions: usize) -> Result<()>
        where F: AsRef<Path>, D: AsRef<Path>, C: AsRef<[u8]> {
        let directory = directory.as_ref();
        let filename = filename.as_ref();
        let contents = contents.as_ref();
        let mut channel = self.handle.channel_open_session().await
            .map_err(|_| Error::ChannelFailure)?;
        let command = format!("scp -t {}", directory.to_string_lossy());
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

    pub async fn ping(&mut self) -> Result<Duration> {
        let start = Instant::now();
        self.hostname().await?;
        Ok(start.elapsed())
    }

    pub async fn hostname(&mut self) -> Result<String> {
        self.shell.exec("hostname").await
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


