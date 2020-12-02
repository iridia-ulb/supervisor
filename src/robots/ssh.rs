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
};

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
    handle: thrussh::client::Handle,
    shell: Mutex<thrussh::client::Channel>,
    ctrl_software_path: Option<PathBuf>,
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device")
         .field("addr", &self.addr)
         .finish()
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
        let mut shell = handle.channel_open_session().await
            .map_err(|_| Error::ChannelFailure)?;
        shell.request_shell(true).await
            .map_err(|_| Error::IoFailure)?;
        Ok(Device { addr, handle, shell: Mutex::new(shell), ctrl_software_path: None })
    }

    pub async fn clear_ctrl_software(&mut self) -> Result<()> {
        let reply = self.exec("mktemp -d").await?;
        self.ctrl_software_path = Some(PathBuf::from(reply));
        Ok(())
    }

    pub async fn add_ctrl_software<N, C>(&mut self, name: N, contents: C) -> Result<()>
        where N: AsRef<Path>, C: AsRef<[u8]> {
        if let Some(ctrl_software_path) = &self.ctrl_software_path {
            let upload_path = ctrl_software_path.join(name);
            log::info!("uploading: {:?}", upload_path);
            self.upload(&contents, &upload_path, 0o644).await?;
        }
        Ok(())
    }

    pub async fn ctrl_software(&mut self) -> Result<Vec<(String, String)>> {
        if let Some(path) = &self.ctrl_software_path {
            let query = format!("find {} -type f -exec md5sum '{{}}' \\;", path.to_string_lossy());
            let response = self.exec(query).await?
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
    
    pub async fn upload<D, P>(&mut self, data: D, path: P, permissions: usize) -> Result<()>
        where D: AsRef<[u8]>, P: AsRef<Path> {
        let path = path.as_ref();
        let data = data.as_ref();
        if let Some(directory) = path.parent() {
            if let Some(file_name) = path.file_name() {
                let mut channel = self.handle.channel_open_session().await
                    .map_err(|_| Error::ChannelFailure)?;
                let command = format!("scp -t {}", directory.to_string_lossy());
                channel.exec(false, command).await
                    .map_err(|_| Error::IoFailure)?;
                let header = format!("C0{:o} {} {}\n",
                    permissions,
                    data.len(),
                    file_name.to_string_lossy());
                /* chain the data together */
                let mut wrapped_data = Cursor::new(header)
                    .chain(Cursor::new(data))
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
                channel.eof().await
                    .map_err(|_| Error::ChannelFailure)?;
            }
            else {
                log::error!("Could not extract filename from {}", path.to_string_lossy());
            }
        }
        else {
            log::error!("Could not extract directory from {}", path.to_string_lossy());
        }
        Ok(())
    }

    pub async fn exec<A: AsRef<str>>(&mut self, command: A) -> Result<String> {
        /* lock the shell */
        let mut shell = self.shell.lock().await;
        /* wrap command so that its output is wrapped with SOT and EOT markers */
        let padded_command =
            format!("printf \"%b%s%b\" \"\\002\" \"`{}`\" \"\\003\"\n", command.as_ref());
        /* write command */
        shell.data(padded_command.as_bytes()).await
            .map_err(|_| Error::IoFailure)?;
        /* get reply */
        let mut buffer = Vec::new();
        while let Some(message) = shell.wait().await {
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

    pub async fn hostname(&mut self) -> Result<String> {
        self.exec("hostname").await
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


