use lazy_static::lazy_static;

#[path = "config_central.rs"]
mod config_central;

#[path = "config_proxy.rs"]
mod config_proxy;

lazy_static!{
    pub static ref CONFIG_CENTRAL: config_central::Config = {
        eprintln!("CONFIG_CENTRAL");
        let config = config_central::get_config()
            .expect("Unable to read config");
        config
    };
    pub static ref CONFIG_PROXY: config_proxy::Config = {
        eprintln!("CONFIG_PROXY");
        let config = config_proxy::get_config()
            .expect("Unable to read config");
        config
    };
}