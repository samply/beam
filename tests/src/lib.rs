
use once_cell::sync::Lazy;
use shared::beam_id::{AppOrProxyId, BeamId, AppId};

pub const APP1: Lazy<AppOrProxyId> = Lazy::new(|| {
    AppId::set_broker_id("broker".to_string());
    AppOrProxyId::new(option_env!("APP1_P1").unwrap_or("app1.proxy1.broker")).unwrap()
}); 
pub const APP2: Lazy<AppOrProxyId> = Lazy::new(|| {
    AppId::set_broker_id("broker".to_string());
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

