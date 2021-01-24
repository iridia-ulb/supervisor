mod process {
    use bytes::BytesMut;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Copy, Debug, Deserialize)]
    pub enum Source {
        Stdout, Stderr
    }

    #[derive(Debug, Serialize)]
    pub enum Request {
        Write(Vec<u8>),
        Kill,
    }

    #[derive(Debug, Deserialize)]
    pub enum Response {
        Started,
        Terminated(bool),
        Output(Source, BytesMut),
    }
}

use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct Upload {
    pub filename: PathBuf,
    pub path: PathBuf,
    pub contents: Vec<u8>,
}

#[derive(Debug, Serialize)]
pub struct Run {
    pub target: PathBuf,
    pub working_dir: PathBuf,
    pub args: Vec<String>,
}

#[derive(Debug, Serialize)]
pub enum Request {
    Ping,
    Upload(Upload),
    Run(Run),    
    Process(u32, process::Request),
}

#[derive(Debug, Deserialize)]
enum Response {
    Ok,
    Error(String),
    Process(u32, process::Response),
}
