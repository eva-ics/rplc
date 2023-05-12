use serde::Deserialize;

#[cfg(feature = "modbus")]
pub mod modbus;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[cfg(feature = "modbus")]
    Modbus,
}
