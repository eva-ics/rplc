use bmart_derive::EnumStr;
use eva_common::{EResult, Error};
use serde::{Deserialize, Deserializer};
use std::str::FromStr;

#[derive(Eq, PartialEq, Copy, Clone, Debug, EnumStr)]
#[enumstr(rename_all = "lowercase")]
pub(crate) enum Kind {
    Coil,
    Discrete,
    Input,
    Holding,
}

impl Kind {
    pub fn as_helper_type_str(self) -> &'static str {
        match self {
            Kind::Coil | Kind::Discrete => "Coils",
            Kind::Holding | Kind::Input => "Registers",
        }
    }
    pub fn as_type_str(self) -> &'static str {
        match self {
            Kind::Coil | Kind::Discrete => "bool",
            Kind::Holding | Kind::Input => "u16",
        }
    }
    pub fn as_type_default_value_str(self) -> &'static str {
        match self {
            Kind::Coil | Kind::Discrete => "false",
            Kind::Holding | Kind::Input => "0",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Reg {
    #[serde(deserialize_with = "deserialize_reg_base")]
    reg: RegBase,
    number: Option<u16>,
}

#[inline]
fn deserialize_reg_base<'de, D>(deserializer: D) -> Result<RegBase, D::Error>
where
    D: Deserializer<'de>,
{
    let buf = String::deserialize(deserializer)?;
    buf.parse().map_err(serde::de::Error::custom)
}

impl Reg {
    #[inline]
    pub fn kind(&self) -> Kind {
        self.reg.kind
    }
    #[inline]
    pub fn number(&self) -> u16 {
        if let Some(number) = self.number {
            number
        } else {
            self.reg.number
        }
    }
    #[inline]
    pub fn offset(&self) -> u16 {
        self.reg.offset
    }
    #[allow(dead_code)]
    pub fn update(&mut self) {
        if self.number.is_none() {
            self.number.replace(self.reg.number);
        }
    }
}

#[derive(Debug, Clone)]
struct RegBase {
    kind: Kind,
    offset: u16,
    number: u16,
}

fn parse_kind_offset(r: &str) -> EResult<(Kind, u16)> {
    if let Some(v) = r.strip_prefix('c') {
        Ok((Kind::Coil, v.parse()?))
    } else if let Some(v) = r.strip_prefix('d') {
        Ok((Kind::Discrete, v.parse()?))
    } else if let Some(v) = r.strip_prefix('i') {
        Ok((Kind::Input, v.parse()?))
    } else if let Some(v) = r.strip_prefix('h') {
        Ok((Kind::Holding, v.parse()?))
    } else {
        Err(Error::invalid_params(format!(
            "invalid register kind: {}",
            r
        )))
    }
}

impl FromStr for RegBase {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut sp = s.split('-');
        let reg_str = sp.next().unwrap();
        let (kind, offset) = parse_kind_offset(reg_str)?;
        let next_offset = if let Some(next_reg) = sp.next() {
            if let Ok(v) = next_reg.parse::<u16>() {
                Some(v)
            } else {
                let (kind2, offset2) = parse_kind_offset(next_reg)?;
                if kind != kind2 {
                    return Err(Error::invalid_params(format!(
                        "invalid register range: {}",
                        s
                    )));
                }
                Some(offset2)
            }
        } else {
            None
        };
        let number = if let Some(no) = next_offset {
            if no < offset {
                return Err(Error::invalid_params(format!(
                    "invalid register range: {}",
                    s
                )));
            }
            no - offset + 1
        } else {
            1
        };
        Ok(RegBase {
            kind,
            offset,
            number,
        })
    }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
enum Offset {
    Str(String),
    Num(u16),
}

impl Default for Offset {
    fn default() -> Self {
        Self::Num(0)
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct MapOffset {
    #[serde(default)]
    offset: Offset,
}

fn calc_offset(s: &str) -> EResult<u16> {
    let mut o = 0;
    for offset in s.split('+') {
        o += offset.parse::<u16>()?;
    }
    Ok(o)
}

fn calc_offset_base(s: &str, base_offset: u16) -> EResult<u16> {
    if let Some(v) = s.strip_prefix('=') {
        let o = calc_offset(v)?;
        if o < base_offset {
            Err(Error::invalid_params(format!(
                "invalid offset {}, base: {}",
                s, base_offset
            )))
        } else {
            Ok(o - base_offset)
        }
    } else {
        calc_offset(s)
    }
}

impl MapOffset {
    pub fn normalize(&mut self, base_offset: u16) -> EResult<()> {
        if let Offset::Str(ref s) = self.offset {
            self.offset = Offset::Num(calc_offset_base(s, base_offset)?);
        }
        Ok(())
    }
    pub fn offset(&self) -> u16 {
        if let Offset::Num(v) = self.offset {
            v
        } else {
            panic!("offset has been not normailized");
        }
    }
}
