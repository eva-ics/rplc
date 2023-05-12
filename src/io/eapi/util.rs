use eva_common::{Error, ItemStatus};

pub struct ItemStatusX(pub ItemStatus);

macro_rules! impl_sx_for_smaller {
    ($t: ty) => {
        impl TryFrom<ItemStatusX> for $t {
            type Error = Error;
            fn try_from(s: ItemStatusX) -> Result<Self, Self::Error> {
                if s.0 > ItemStatus::from(<$t>::MAX) {
                    Err(Self::Error::invalid_data("unable to convert item status"))
                } else {
                    Ok(s.0 as $t)
                }
            }
        }
    };
}

macro_rules! impl_sx {
    ($t: ty) => {
        impl TryFrom<ItemStatusX> for $t {
            type Error = Error;
            fn try_from(s: ItemStatusX) -> Result<Self, Self::Error> {
                Ok(s.0 as $t)
            }
        }
    };
}

impl TryFrom<ItemStatusX> for bool {
    type Error = Error;
    fn try_from(s: ItemStatusX) -> Result<Self, Self::Error> {
        if s.0 > 1 {
            Err(Self::Error::invalid_data("unable to convert item status"))
        } else {
            Ok(s.0 == 1)
        }
    }
}

impl_sx_for_smaller!(u8);
impl_sx!(u16);
impl_sx!(u32);
impl_sx!(u64);
impl_sx_for_smaller!(i8);
impl_sx_for_smaller!(i16);
impl_sx!(i32);
impl_sx!(i64);
