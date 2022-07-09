use static_init::dynamic;
use tracing::debug;

use crate::{config_proxy, config_central, config_shared, errors::SamplyBrokerError};

pub(crate) trait Config: Sized{
    fn load() -> Result<Self,SamplyBrokerError>;
}

fn load<T: Config>() -> T where {
    T::load()
        .unwrap_or_else(|e| panic!("Unable to read config: {}", e))
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
