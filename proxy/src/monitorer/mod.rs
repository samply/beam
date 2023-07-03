
#[cfg(feature = "monitor")]
mod monitorer;

#[cfg(feature = "monitor")]
pub use monitorer::*;

#[macro_export]
macro_rules! monitor {
    ($update:expr) => {
        #[cfg(feature="monitor")]
        $crate::monitorer::MONITORER.send($update);
    };
}

