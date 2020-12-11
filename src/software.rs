
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug)]
pub enum Action {
    Upload,
    Clear,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("ARGoS configuration file missing")]
    MissingConfigurationFile,

    #[error("More than one ARGoS configuration file provided")]
    MultipleConfigurationFiles,

    #[error("Could not find referenced file {0}")]
    MissingReferencedFile(String),

    #[error("Configuration file was not valid UTF-8")]
    DecodeError(#[from] std::str::Utf8Error),

    #[error("Configuration file was not valid XML")]
    ParseError(#[from] roxmltree::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Default, Debug)]
pub struct Software(pub Vec<(PathBuf, Vec<u8>)>);

impl Software {
    pub fn add<F: Into<PathBuf>, C: Into<Vec<u8>>>(&mut self, new_filename: F, new_contents: C) {
        let new_filename = new_filename.into();
        let new_contents = new_contents.into();
        if let Some((_, contents)) = self.0.iter_mut()
            .find(|(filename, _)| filename == &new_filename) {
            contents.splice(.., new_contents.into_iter());
        }
        else {
            self.0.push((new_filename, new_contents));
        }
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn argos_config(&self) -> Result<&(PathBuf, Vec<u8>)> {
        let config = self.0.iter()
            .filter(|entry| {
                entry.0.to_string_lossy().contains(".argos")
            })
            .collect::<Vec<_>>();
        match config.len() {
            0 => Err(Error::MissingConfigurationFile),
            1 => Ok(config[0]),
            _ => Err(Error::MultipleConfigurationFiles)
        }
    }
   
    pub fn check_config(&self) -> Result<()> {
        let config = self.argos_config()?;
        let config = std::str::from_utf8(&config.1[..])?;
        let config = roxmltree::Document::parse(&config)?;
        /* extract lua scripts */
        config.root().descendants()
            .filter_map(|node| match node.tag_name().name() {
                "controllers" => Some(node.children()),
                _ => None
            }).flatten()
            .filter_map(|node| match node.tag_name().name() {
                "lua_controller" => Some(node.children()),
                _ => None,
            }).flatten()
            .filter_map(|node| match node.tag_name().name() {
                "params" => Some(node.attributes()),
                _ => None,
            }).flatten()
            .filter_map(|attr| match attr.name() {
                "script" => Some(attr.value()),
                _ => None,
            })
            .map(|script| self.0.iter()
                .find(|(filename, _)| filename.to_string_lossy() == script )
                .ok_or(Error::MissingReferencedFile(script.to_owned())))
            .collect::<Result<Vec<_>>>()?;
        Ok(())
    }
}
