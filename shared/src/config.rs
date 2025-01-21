use once_cell::sync::{Lazy, OnceCell};
use tracing::debug;

use crate::{
    config_broker, config_proxy,
    config_shared::{self, ConfigCrypto},
    crypto,
    errors::SamplyBeamError,
};

pub(crate) trait Config: Sized {
    fn load() -> Result<Self, SamplyBeamError>;
}

fn load<T: Config>() -> T where {
    T::load()
        .unwrap_or_else(|e| {
            eprintln!("Unable to start as there was an error reading the config:\n{}\n\nTerminating -- please double-check your startup parameters with --help and refer to the documentation.", e);
            std::process::exit(1);
        })
}

pub static CONFIG_PROXY: Lazy<config_proxy::Config> = Lazy::new(|| {
    debug!("Loading config CONFIG_PROXY");
    load()
});

pub static CONFIG_CENTRAL: Lazy<config_broker::Config> = Lazy::new(|| {
    debug!("Loading config CONFIG_CENTRAL");
    load()
});

pub static CONFIG_SHARED: Lazy<config_shared::Config> = Lazy::new(|| {
    debug!("Loading config CONFIG_SHARED");
    load()
});

pub(crate) static CONFIG_SHARED_CRYPTO: OnceCell<ConfigCrypto> = OnceCell::new();
