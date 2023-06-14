use std::ops::Deref;

use once_cell::sync::OnceCell;
use static_init::dynamic;
use tracing::debug;

use crate::{
    config_broker, config_proxy,
    config_shared::{self, ConfigCrypto},
    crypto,
    errors::SamplyBeamError,
};

pub trait Config: Sized {
    fn load() -> Result<Self, SamplyBeamError>;
}

fn load<T: Config>() -> T where {
    T::load()
        .unwrap_or_else(|e| {
            eprintln!("Unable to start as there was an error reading the config:\n{}\n\nTerminating -- please double-check your startup parameters with --help and refer to the documentation.", e);
            std::process::exit(1);
        })
}

pub struct ConfigWrapper<T>(OnceCell<T>);

impl<T> ConfigWrapper<T> {
    pub const fn new() -> Self {
        ConfigWrapper(OnceCell::new())
    }

    pub fn set(&self, val: T) -> Result<(), T> {
        self.0.set(val)
    }
}

impl<T> Deref for ConfigWrapper<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0.get().expect("Config to be initialized already!")
    }
}

pub static CONFIG_PROXY: ConfigWrapper<config_proxy::Config> = ConfigWrapper::new();

#[dynamic(lazy)]
pub static CONFIG_CENTRAL: config_broker::Config = {
    debug!("Loading config CONFIG_CENTRAL");
    load()
};

pub static CONFIG_SHARED: ConfigWrapper<config_shared::Config> = ConfigWrapper::new();

pub(crate) static CONFIG_SHARED_CRYPTO: OnceCell<ConfigCrypto> = OnceCell::new();

pub fn prepare_env() {
    for var in ["http_proxy", "https_proxy", "all_proxy", "no_proxy"] {
        for (k, v) in std::env::vars().filter(|(k, _)| k.to_lowercase() == var) {
            std::env::set_var(k.to_uppercase(), v);
        }
    }
}
