pub mod process {
    use std::path::PathBuf;
    use bytes::BytesMut;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Copy, Debug, Deserialize)]
    pub enum Source {
        Stdout, Stderr
    }

    #[derive(Debug, Serialize)]
    pub struct Run {
        pub target: PathBuf,
        pub working_dir: PathBuf,
        pub args: Vec<String>,
    }

    #[derive(Debug, Serialize)]
    pub enum Request {
        Run(Run),
        StandardInput(BytesMut),
        Signal(u32),
    }

    #[derive(Debug, Deserialize)]
    pub enum Response {
        Started,
        Terminated(bool),
        StandardOutput(BytesMut),
        StandardError(BytesMut),
    }
}

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct Upload {
    pub filename: PathBuf,
    pub path: PathBuf,
    pub contents: Vec<u8>,
}

#[derive(Debug, Serialize)]
pub enum RequestKind {
    Ping,
    Upload(Upload),
    Process(process::Request),
}

#[derive(Debug, Serialize)]
pub struct Request(pub Uuid, pub RequestKind);

#[derive(Debug, Deserialize)]
pub enum ResponseKind {
    Ok,
    Error(String),
    Process(process::Response),
}

#[derive(Debug, Deserialize)]
pub struct Response(pub Option<Uuid>, pub ResponseKind);
