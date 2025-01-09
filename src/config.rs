use std::{net::IpAddr, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Config {
    pub network: NetworkConfig,
    pub template: TemplateConfig,
    pub service: ServiceConfig,
}

#[derive(Serialize, Deserialize)]
pub struct NetworkConfig {
    pub address: IpAddr,
    pub port: u16,
}

#[derive(Serialize, Deserialize)]
pub struct TemplateConfig {
    pub header_file: PathBuf,
    pub footer_file: PathBuf,
    pub error_file: PathBuf,
}

#[derive(Serialize, Deserialize)]
pub struct ServiceConfig {
    pub limit: u64,
    pub root: PathBuf,
}
