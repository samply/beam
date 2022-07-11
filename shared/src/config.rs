use static_init::dynamic;

use crate::{config_proxy, config_central, config_shared, errors::SamplyBrokerError};

pub(crate) trait Config: Sized{
    fn load() -> Result<Self,SamplyBrokerError>;
}

fn load<T: Config>() -> T where {
    let config = T::load()
        .unwrap_or_else(|e| panic!("Unable to read config: {}", e));
    config
}

#[dynamic(lazy)]
pub static CONFIG_PROXY: config_proxy::Config = {
    eprintln!("CONFIG_PROXY");
    load()
};

#[dynamic(lazy)]
pub static CONFIG_CENTRAL: config_central::Config = {
    eprintln!("CONFIG_CENTRAL");
    load()
};

#[dynamic(lazy)]
pub(crate) static CONFIG_SHARED: config_shared::Config = {
    eprintln!("CONFIG_SHARED");
    load()
};
