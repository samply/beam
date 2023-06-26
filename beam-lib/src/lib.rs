mod messages;
mod ids;
#[cfg(feature = "sockets")]
mod sockets;
#[cfg(feature = "sockets")]
pub use sockets::*;

pub use ids::*;
#[cfg(feature = "strict-ids")]
pub use ids::app_or_proxy::AppOrProxyId;

pub use messages::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        // let result = add(2, 2);
        // assert_eq!(result, 4);
    }
}
