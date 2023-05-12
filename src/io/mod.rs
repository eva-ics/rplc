use serde::{Deserialize, Serialize};

#[cfg(feature = "eva")]
pub mod eapi;
#[cfg(feature = "modbus")]
pub mod modbus;
#[cfg(feature = "opcua")]
pub mod opcua;

#[derive(Deserialize, Serialize, Debug, Copy, Clone)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[cfg(feature = "modbus")]
    Modbus,
    #[cfg(feature = "opcua")]
    OpcUa,
    #[cfg(feature = "eva")]
    Eapi,
}
