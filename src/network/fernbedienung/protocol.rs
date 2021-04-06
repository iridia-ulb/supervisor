use serde::{Deserialize, Deserializer, Serialize, Serializer};
use bytes::BytesMut;
use std::path::PathBuf;
use uuid::Uuid;

fn bytesmut_serialize<S: Serializer>(bytes: &BytesMut, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&base64::encode(bytes))
}

fn bytesmut_deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BytesMut, D::Error> {
    use serde::de::Error;
    let input: String = Deserialize::deserialize(deserializer)?;
    base64::decode(input)
        .map(|vec| BytesMut::from(&vec[..]))
        .map_err(D::Error::custom)
}

pub mod process {
    use std::path::PathBuf;
    use bytes::BytesMut;
    use serde::{Deserialize, Serialize};
    

    #[derive(Debug, Serialize)]
    pub struct Process {
        pub target: PathBuf,
        pub working_dir: Option<PathBuf>,
        pub args: Vec<String>,
    }

    #[derive(Debug, Serialize)]
    pub enum Request {
        Run(Process),
        #[serde(serialize_with = "super::bytesmut_serialize")]
        StandardInput(BytesMut),
        Terminate,
    }

    #[derive(Debug, Deserialize)]
    pub enum Response {
        Terminated(bool),
        #[serde(deserialize_with = "super::bytesmut_deserialize")]
        StandardOutput(BytesMut),
        #[serde(deserialize_with = "super::bytesmut_deserialize")]
        StandardError(BytesMut),
    }
}

#[derive(Debug, Serialize)]
pub struct Upload {
    pub filename: PathBuf,
    pub path: PathBuf,
    pub contents: Vec<u8>,
}

#[derive(Debug, Serialize)]
pub enum RequestKind {
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
