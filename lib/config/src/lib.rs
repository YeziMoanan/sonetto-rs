include!("../../config/configs/mod.rs");

pub mod destiny;

#[cfg(test)]
#[allow(unused_imports)]
#[path = "../excel_confgen/mod.rs"]
mod excel_confgen;

pub mod configs {
    pub use crate::{GameDB, get, init, try_get};
}
