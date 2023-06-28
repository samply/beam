
use once_cell::sync::Lazy;
use beam_lib::{AddressingId, set_broker_id, AppOrProxyId};

#[cfg(all(feature = "sockets", test))]
mod socket_test;

pub static APP1: Lazy<AddressingId> = Lazy::new(|| {
    set_broker_id("broker".into());
    AppOrProxyId::new(option_env!("APP1_P1").unwrap_or("app1.proxy1.broker")).unwrap()
}); 
pub static APP2: Lazy<AddressingId> = Lazy::new(|| {
    set_broker_id("broker".into());
    AppOrProxyId::new(option_env!("APP2_P2").unwrap_or("app2.proxy2.broker")).unwrap()
});

// unwrap_or is not const yet
pub const PROXY1: &str = match option_env!("P1") {
    Some(v) => v,
    _ => "http://localhost:8081"
};
pub const PROXY2: &str = match option_env!("P2") {
    Some(v) => v,
    _ => "http://localhost:8082"
};

pub const APP_KEY: &str = match option_env!("APP_KEY") {
    Some(v) => v,
    None => "App1Secret"
};
