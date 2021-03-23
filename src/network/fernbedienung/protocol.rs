pub mod process {
    use std::path::PathBuf;
    use bytes::BytesMut;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use base64;

    #[derive(Debug, Serialize)]
    pub struct Run {
        pub target: PathBuf,
        pub working_dir: PathBuf,
        pub args: Vec<String>,
    }

    #[derive(Debug, Serialize)]
    pub enum Request {
        Run(Run),
        #[serde(serialize_with = "bytesmut_serialize")]
        StandardInput(BytesMut),
        Terminate,
    }

    #[derive(Debug, Deserialize)]
    pub enum Response {
        Terminated(bool),
        #[serde(deserialize_with = "bytesmut_deserialize")]
        StandardOutput(BytesMut),
        #[serde(deserialize_with = "bytesmut_deserialize")]
        StandardError(BytesMut),
    }

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
