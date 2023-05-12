use ::rplc::export::serde::{self, Deserialize, Serialize};
use std::time::Instant;

pub use std::time::Duration;

#[derive(Default, Serialize, Deserialize)]
#[serde(crate = "self::serde")]
pub struct Timers {
    #[serde(skip)]
    pub t1: Option<Instant>,
    pub enabled: bool,
}
