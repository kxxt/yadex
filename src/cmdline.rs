use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
pub struct Cmdline {
    #[clap(
        short,
        long,
        help = "path to configuration file",
        default_value = "/etc/yadex/config.toml"
    )]
    pub config: PathBuf,
}
