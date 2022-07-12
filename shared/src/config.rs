use static_init::dynamic;
use tracing::debug;

use crate::{config_proxy, config_central, config_shared, errors::SamplyBrokerError};

pub(crate) trait Config: Sized{
    fn load() -> Result<Self,SamplyBrokerError>;
}

fn load<T: Config>() -> T where {
    T::load()
        .unwrap_or_else(|e| {
            eprintln!("Unable to start as there was an error reading the config:\n{}\n\nTerminating -- please double-check your startup parameters with --help and refer to the documentation.", e);
            std::process::exit(1);
        })
}

#[dynamic(lazy)]
pub static CONFIG_PROXY: config_proxy::Config = {
    debug!("Loading config CONFIG_PROXY");
    load()
};

#[dynamic(lazy)]
pub static CONFIG_CENTRAL: config_central::Config = {
    debug!("Loading config CONFIG_CENTRAL");
    load()
};

#[dynamic(lazy)]
pub(crate) static CONFIG_SHARED: config_shared::Config = {
    debug!("Loading config CONFIG_SHARED");
    load()
};

pub fn prepare_env() {
    for var in ["http_proxy"] {
        for (k,v) in std::env::vars().filter(|(k,_)| k.to_lowercase() == var) {
            std::env::set_var(k.to_uppercase(), v);
        }
    }
}