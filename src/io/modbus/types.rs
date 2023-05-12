use eva_common::{EResult, Error, ErrorKind};
use std::ops::{Deref, DerefMut};

macro_rules! invalid_data {
    () => {
        Error::new0(ErrorKind::InvalidData)
    };
}

#[derive(Debug, Clone)]
pub struct Coils(pub Vec<bool>);

impl Deref for Coils {
    type Target = Vec<bool>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Coils {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Coils {
    pub fn slice_at(&self, idx: usize) -> EResult<CoilSlice> {
        if idx < self.len() {
            Ok(CoilSlice(&self[idx..]))
        } else {
            Err(Error::invalid_data(format!(
                "coils index out of bounds: {idx}"
            )))
        }
    }
}

#[derive(Debug, Clone)]
pub struct CoilSlice<'a>(pub &'a [bool]);

impl<'a> Deref for CoilSlice<'a> {
    type Target = &'a [bool];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct Registers(pub Vec<u16>);

impl Deref for Registers {
    type Target = Vec<u16>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Registers {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Registers {
    pub fn slice_at(&self, idx: usize) -> EResult<RegisterSlice> {
        if idx < self.len() {
            Ok(RegisterSlice(&self[idx..]))
        } else {
            Err(Error::invalid_data(format!(
                "registers index out of bounds: {idx}"
            )))
        }
    }
}

#[derive(Debug, Clone)]
pub struct RegisterSlice<'a>(pub &'a [u16]);

impl<'a> Deref for RegisterSlice<'a> {
    type Target = &'a [u16];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<&bool> for Coils {
    fn from(s: &bool) -> Coils {
        Coils(vec![*s])
    }
}

impl<const N: usize> From<&[bool; N]> for Coils {
    fn from(s: &[bool; N]) -> Coils {
        Coils(s.to_vec())
    }
}

impl<'a> TryFrom<CoilSlice<'a>> for bool {
    type Error = Error;
    fn try_from(s: CoilSlice) -> Result<bool, Self::Error> {
        Ok(*s.first().ok_or_else(|| invalid_data!())?)
    }
}

impl<'a, const N: usize> TryFrom<CoilSlice<'a>> for [bool; N] {
    type Error = Error;
    fn try_from(s: CoilSlice) -> Result<[bool; N], Self::Error> {
        if N > s.len() {
            Err(invalid_data!())
        } else {
            s[..N].try_into().map_err(Error::invalid_data)
        }
    }
}

impl From<&u16> for Registers {
    fn from(s: &u16) -> Registers {
        Registers(vec![*s])
    }
}

impl<const N: usize> From<&[u16; N]> for Registers {
    fn from(v: &[u16; N]) -> Registers {
        Registers(v.to_vec())
    }
}

impl<'a> TryFrom<RegisterSlice<'a>> for u16 {
    type Error = Error;
    fn try_from(s: RegisterSlice) -> Result<u16, Self::Error> {
        Ok(*s.first().ok_or_else(|| invalid_data!())?)
    }
}

impl<'a, const N: usize> TryFrom<RegisterSlice<'a>> for [u16; N] {
    type Error = Error;
    fn try_from(s: RegisterSlice) -> Result<[u16; N], Self::Error> {
        if N > s.len() {
            Err(invalid_data!())
        } else {
            s[..N].try_into().map_err(Error::invalid_data)
        }
    }
}

#[allow(clippy::cast_sign_loss)]
impl From<&i16> for Registers {
    fn from(s: &i16) -> Registers {
        Registers(vec![*s as u16])
    }
}

#[allow(clippy::cast_sign_loss)]
impl<const N: usize> From<&[i16; N]> for Registers {
    fn from(v: &[i16; N]) -> Registers {
        Registers(v.iter().map(|v| *v as u16).collect())
    }
}

#[allow(clippy::cast_possible_wrap)]
impl<'a> TryFrom<RegisterSlice<'a>> for i16 {
    type Error = Error;
    fn try_from(s: RegisterSlice) -> Result<i16, Self::Error> {
        Ok(*s.first().ok_or_else(|| invalid_data!())? as i16)
    }
}

#[allow(clippy::cast_possible_wrap)]
impl<'a, const N: usize> TryFrom<RegisterSlice<'a>> for [i16; N] {
    type Error = Error;
    fn try_from(s: RegisterSlice) -> Result<[i16; N], Self::Error> {
        if N > s.len() {
            Err(invalid_data!())
        } else {
            s[..N]
                .iter()
                .map(|v| *v as i16)
                .collect::<Vec<i16>>()
                .try_into()
                .map_err(|_| invalid_data!())
        }
    }
}

fn u32_to_modbus_array(val: u32) -> [u16; 2] {
    let v = val.to_be_bytes();
    [
        u16::from(v[0]).overflowing_shl(8).0 + u16::from(v[1]),
        u16::from(v[2]).overflowing_shl(8).0 + u16::from(v[3]),
    ]
}

impl From<&u32> for Registers {
    #![allow(arithmetic_overflow)]
    fn from(s: &u32) -> Registers {
        Registers(u32_to_modbus_array(*s).to_vec())
    }
}

impl<const N: usize> From<&[u32; N]> for Registers {
    fn from(s: &[u32; N]) -> Registers {
        let mut result = Vec::with_capacity(N * 2);
        for v in s {
            result.extend(u32_to_modbus_array(*v));
        }
        Registers(result)
    }
}

impl<'a> TryFrom<RegisterSlice<'a>> for u32 {
    type Error = Error;
    fn try_from(s: RegisterSlice) -> Result<u32, Self::Error> {
        let val1 = s.first().ok_or_else(|| invalid_data!())?.to_be_bytes();
        let val2 = s.get(1).ok_or_else(|| invalid_data!())?.to_be_bytes();
        Ok(u32::from_be_bytes([val1[0], val1[1], val2[0], val2[1]]))
    }
}

impl<'a, const N: usize> TryFrom<RegisterSlice<'a>> for [u32; N] {
    type Error = Error;
    fn try_from(s: RegisterSlice) -> Result<[u32; N], Self::Error> {
        let mut result = Vec::with_capacity(N / 2);
        for idx in (0..N * 2).step_by(2) {
            let val1 = s.get(idx).ok_or_else(|| invalid_data!())?.to_be_bytes();
            let val2 = s.get(idx + 1).ok_or_else(|| invalid_data!())?.to_be_bytes();
            result.push(u32::from_be_bytes([val1[0], val1[1], val2[0], val2[1]]));
        }
        result.try_into().map_err(|_| invalid_data!())
    }
}

#[allow(clippy::cast_sign_loss)]
impl From<&i32> for Registers {
    fn from(s: &i32) -> Registers {
        Registers(u32_to_modbus_array(*s as u32).to_vec())
    }
}

#[allow(clippy::cast_sign_loss)]
impl<const N: usize> From<&[i32; N]> for Registers {
    fn from(s: &[i32; N]) -> Registers {
        let mut result = Vec::with_capacity(N * 2);
        for v in s {
            result.extend(u32_to_modbus_array(*v as u32));
        }
        Registers(result)
    }
}

impl<'a> TryFrom<RegisterSlice<'a>> for i32 {
    type Error = Error;
    fn try_from(s: RegisterSlice) -> Result<i32, Self::Error> {
        let val1 = s.first().ok_or_else(|| invalid_data!())?.to_be_bytes();
        let val2 = s.get(1).ok_or_else(|| invalid_data!())?.to_be_bytes();
        Ok(i32::from_be_bytes([val1[0], val1[1], val2[0], val2[1]]))
    }
}

impl<'a, const N: usize> TryFrom<RegisterSlice<'a>> for [i32; N] {
    type Error = Error;
    fn try_from(s: RegisterSlice) -> Result<[i32; N], Self::Error> {
        let mut result = Vec::with_capacity(N / 2);
        for idx in (0..N * 2).step_by(2) {
            let val1 = s.get(idx).ok_or_else(|| invalid_data!())?.to_be_bytes();
            let val2 = s.get(idx + 1).ok_or_else(|| invalid_data!())?.to_be_bytes();
            result.push(i32::from_be_bytes([val1[0], val1[1], val2[0], val2[1]]));
        }
        result.try_into().map_err(|_| invalid_data!())
    }
}

fn u64_to_modbus_array(val: u64) -> [u16; 4] {
    let v = val.to_be_bytes();
    [
        u16::from(v[0]).overflowing_shl(8).0 + u16::from(v[1]),
        u16::from(v[2]).overflowing_shl(8).0 + u16::from(v[3]),
        u16::from(v[4]).overflowing_shl(8).0 + u16::from(v[5]),
        u16::from(v[6]).overflowing_shl(8).0 + u16::from(v[7]),
    ]
}

impl From<&u64> for Registers {
    #![allow(arithmetic_overflow)]
    fn from(s: &u64) -> Registers {
        Registers(u64_to_modbus_array(*s).to_vec())
    }
}

impl<const N: usize> From<&[u64; N]> for Registers {
    fn from(s: &[u64; N]) -> Registers {
        let mut result = Vec::with_capacity(N * 4);
        for v in s {
            result.extend(u64_to_modbus_array(*v));
        }
        Registers(result)
    }
}

impl<'a> TryFrom<RegisterSlice<'a>> for u64 {
    type Error = Error;
    fn try_from(s: RegisterSlice) -> Result<u64, Self::Error> {
        let val1 = s.first().ok_or_else(|| invalid_data!())?.to_be_bytes();
        let val2 = s.get(1).ok_or_else(|| invalid_data!())?.to_be_bytes();
        let val3 = s.get(2).ok_or_else(|| invalid_data!())?.to_be_bytes();
        let val4 = s.get(3).ok_or_else(|| invalid_data!())?.to_be_bytes();
        Ok(u64::from_be_bytes([
            val1[0], val1[1], val2[0], val2[1], val3[0], val3[1], val4[0], val4[1],
        ]))
    }
}

impl<'a, const N: usize> TryFrom<RegisterSlice<'a>> for [u64; N] {
    type Error = Error;
    fn try_from(s: RegisterSlice) -> Result<[u64; N], Self::Error> {
        let mut result = Vec::with_capacity(N / 4);
        for idx in (0..N * 4).step_by(4) {
            let val1 = s.get(idx).ok_or_else(|| invalid_data!())?.to_be_bytes();
            let val2 = s.get(idx + 1).ok_or_else(|| invalid_data!())?.to_be_bytes();
            let val3 = s.get(idx + 2).ok_or_else(|| invalid_data!())?.to_be_bytes();
            let val4 = s.get(idx + 3).ok_or_else(|| invalid_data!())?.to_be_bytes();
            result.push(u64::from_be_bytes([
                val1[0], val1[1], val2[0], val2[1], val3[0], val3[1], val4[0], val4[1],
            ]));
        }
        result.try_into().map_err(|_| invalid_data!())
    }
}

impl From<&i64> for Registers {
    #![allow(clippy::cast_sign_loss)]
    fn from(s: &i64) -> Registers {
        Registers(u64_to_modbus_array(*s as u64).to_vec())
    }
}

#[allow(clippy::cast_sign_loss)]
impl<const N: usize> From<&[i64; N]> for Registers {
    fn from(s: &[i64; N]) -> Registers {
        let mut result = Vec::with_capacity(N * 4);
        for v in s {
            result.extend(u64_to_modbus_array(*v as u64));
        }
        Registers(result)
    }
}

impl<'a> TryFrom<RegisterSlice<'a>> for i64 {
    type Error = Error;
    fn try_from(s: RegisterSlice) -> Result<i64, Self::Error> {
        let val1 = s.first().ok_or_else(|| invalid_data!())?.to_be_bytes();
        let val2 = s.get(1).ok_or_else(|| invalid_data!())?.to_be_bytes();
        let val3 = s.get(2).ok_or_else(|| invalid_data!())?.to_be_bytes();
        let val4 = s.get(3).ok_or_else(|| invalid_data!())?.to_be_bytes();
        Ok(i64::from_be_bytes([
            val1[0], val1[1], val2[0], val2[1], val3[0], val3[1], val4[0], val4[1],
        ]))
    }
}

impl<'a, const N: usize> TryFrom<RegisterSlice<'a>> for [i64; N] {
    type Error = Error;
    fn try_from(s: RegisterSlice) -> Result<[i64; N], Self::Error> {
        let mut result = Vec::with_capacity(N / 4);
        for idx in (0..N * 4).step_by(4) {
            let val1 = s.get(idx).ok_or_else(|| invalid_data!())?.to_be_bytes();
            let val2 = s.get(idx + 1).ok_or_else(|| invalid_data!())?.to_be_bytes();
            let val3 = s.get(idx + 2).ok_or_else(|| invalid_data!())?.to_be_bytes();
            let val4 = s.get(idx + 3).ok_or_else(|| invalid_data!())?.to_be_bytes();
            result.push(i64::from_be_bytes([
                val1[0], val1[1], val2[0], val2[1], val3[0], val3[1], val4[0], val4[1],
            ]));
        }
        result.try_into().map_err(|_| invalid_data!())
    }
}

fn f32_to_modbus_array(val: f32) -> [u16; 2] {
    let v = val.to_be_bytes();
    [
        u16::from(v[2]).overflowing_shl(8).0 + u16::from(v[3]),
        u16::from(v[0]).overflowing_shl(8).0 + u16::from(v[1]),
    ]
}

impl From<&f32> for Registers {
    fn from(s: &f32) -> Registers {
        Registers(f32_to_modbus_array(*s).to_vec())
    }
}

impl<const N: usize> From<&[f32; N]> for Registers {
    fn from(s: &[f32; N]) -> Registers {
        let mut result = Vec::with_capacity(N * 2);
        for v in s {
            result.extend(f32_to_modbus_array(*v));
        }
        Registers(result)
    }
}

impl<'a> TryFrom<RegisterSlice<'a>> for f32 {
    type Error = Error;
    fn try_from(s: RegisterSlice) -> Result<f32, Self::Error> {
        let val1 = s.first().ok_or_else(|| invalid_data!())?.to_be_bytes();
        let val2 = s.get(1).ok_or_else(|| invalid_data!())?.to_be_bytes();
        Ok(f32::from_be_bytes([val2[0], val2[1], val1[0], val1[1]]))
    }
}

impl<'a, const N: usize> TryFrom<RegisterSlice<'a>> for [f32; N] {
    type Error = Error;
    fn try_from(s: RegisterSlice) -> Result<[f32; N], Self::Error> {
        let mut result = Vec::with_capacity(N / 2);
        for idx in (0..N * 2).step_by(2) {
            let val1 = s.get(idx).ok_or_else(|| invalid_data!())?.to_be_bytes();
            let val2 = s.get(idx + 1).ok_or_else(|| invalid_data!())?.to_be_bytes();
            result.push(f32::from_be_bytes([val2[0], val2[1], val1[0], val1[1]]));
        }
        result.try_into().map_err(|_| invalid_data!())
    }
}

fn f64_to_modbus_array(val: f64) -> [u16; 4] {
    let v = val.to_be_bytes();
    [
        u16::from(v[6]).overflowing_shl(8).0 + u16::from(v[7]),
        u16::from(v[4]).overflowing_shl(8).0 + u16::from(v[5]),
        u16::from(v[2]).overflowing_shl(8).0 + u16::from(v[3]),
        u16::from(v[0]).overflowing_shl(8).0 + u16::from(v[1]),
    ]
}

impl From<&f64> for Registers {
    fn from(s: &f64) -> Registers {
        Registers(f64_to_modbus_array(*s).to_vec())
    }
}

impl<const N: usize> From<&[f64; N]> for Registers {
    fn from(s: &[f64; N]) -> Registers {
        let mut result = Vec::with_capacity(N * 4);
        for v in s {
            result.extend(f64_to_modbus_array(*v));
        }
        Registers(result)
    }
}

impl<'a> TryFrom<RegisterSlice<'a>> for f64 {
    type Error = Error;
    fn try_from(s: RegisterSlice) -> Result<f64, Self::Error> {
        let val1 = s.first().ok_or_else(|| invalid_data!())?.to_be_bytes();
        let val2 = s.get(1).ok_or_else(|| invalid_data!())?.to_be_bytes();
        let val3 = s.get(2).ok_or_else(|| invalid_data!())?.to_be_bytes();
        let val4 = s.get(3).ok_or_else(|| invalid_data!())?.to_be_bytes();
        Ok(f64::from_be_bytes([
            val4[0], val4[1], val3[0], val3[1], val2[0], val2[1], val1[0], val1[1],
        ]))
    }
}

impl<'a, const N: usize> TryFrom<RegisterSlice<'a>> for [f64; N] {
    type Error = Error;
    fn try_from(s: RegisterSlice) -> Result<[f64; N], Self::Error> {
        let mut result = Vec::with_capacity(N / 2);
        for idx in (0..N * 4).step_by(4) {
            let val1 = s.get(idx).ok_or_else(|| invalid_data!())?.to_be_bytes();
            let val2 = s.get(idx + 1).ok_or_else(|| invalid_data!())?.to_be_bytes();
            let val3 = s.get(idx + 2).ok_or_else(|| invalid_data!())?.to_be_bytes();
            let val4 = s.get(idx + 3).ok_or_else(|| invalid_data!())?.to_be_bytes();
            result.push(f64::from_be_bytes([
                val4[0], val4[1], val3[0], val3[1], val2[0], val2[1], val1[0], val1[1],
            ]));
        }
        result.try_into().map_err(|_| invalid_data!())
    }
}

#[cfg(test)]
mod test {

    use super::*;

    #[test]
    fn test_coils() {
        let coil = true;
        let coils = Coils::from(&coil);
        assert_eq!(coils.0, vec![true]);
        let coils = Coils::from(&[true, false, true, true]);
        let coil: bool = coils.slice_at(0).unwrap().try_into().unwrap();
        assert_eq!(coil, true);
        let coil_array: [bool; 3] = coils.slice_at(1).unwrap().try_into().unwrap();
        assert_eq!(coil_array, [false, true, true]);
        assert!(TryInto::<[bool; 3]>::try_into(coils.slice_at(2).unwrap()).is_err());
    }

    #[test]
    fn test_u16() {
        let reg = 0xFFAAu16;
        let regs = Registers::from(&reg);
        assert_eq!(regs.0, vec![reg]);
        let regs = Registers::from(&[0xAABBu16, 0xCCDD, 0xEEFF, 0x1122]);
        let reg: u16 = regs.slice_at(0).unwrap().try_into().unwrap();
        assert_eq!(reg, 0xAABB);
        let reg_array: [u16; 3] = regs.slice_at(1).unwrap().try_into().unwrap();
        assert_eq!(reg_array, [0xCCDD, 0xEEFF, 0x1122]);
        assert!(TryInto::<[u16; 3]>::try_into(regs.slice_at(2).unwrap()).is_err());
    }

    #[test]
    fn test_i16() {
        let reg = -1122i16;
        let regs = Registers::from(&reg);
        assert_eq!(regs.0, vec![0xFB9E]);
        let regs = Registers::from(&[0xEE99u16, 0xFB9E, 0xDB96, 0xE42D]);
        let reg: i16 = regs.slice_at(0).unwrap().try_into().unwrap();
        assert_eq!(reg, -4455);
        let reg_array: [i16; 3] = regs.slice_at(1).unwrap().try_into().unwrap();
        assert_eq!(reg_array, [-1122, -9322, -7123]);
        assert!(TryInto::<[i16; 3]>::try_into(regs.slice_at(2).unwrap()).is_err());
    }
    #[test]
    fn test_u32() {
        let reg = 0xFFAA1122u32;
        let regs = Registers::from(&reg);
        assert_eq!(regs.0, vec![0xFFAA, 0x1122]);
        let vals = [
            0x378cu16, 0x50e, 0x2ead, 0xaf67, 0x1302, 0x3cc4, 0x495e, 0xcbdf,
        ];
        let regs = Registers::from(&vals);
        let reg: u32 = regs.slice_at(0).unwrap().try_into().unwrap();
        assert_eq!(reg, 931923214);
        let reg_array: [u32; 3] = regs.slice_at(2).unwrap().try_into().unwrap();
        assert_eq!(reg_array, [783134567, 318913732, 1230949343]);
        assert!(TryInto::<[u32; 3]>::try_into(regs.slice_at(4).unwrap()).is_err());
        let arr = [931923214u32, 783134567, 318913732, 1230949343];
        let regs = Registers::from(&arr);
        assert_eq!(regs.0, vals);
        let arr2: [u32; 4] = regs.slice_at(0).unwrap().try_into().unwrap();
        assert_eq!(arr, arr2);
    }
    #[test]
    fn test_i32() {
        let reg = -1230949343i32;
        let regs = Registers::from(&reg);
        assert_eq!(
            TryInto::<i32>::try_into(regs.slice_at(0).unwrap()).unwrap(),
            -1230949343i32
        );
        assert_eq!(regs.0, vec![0xb6a1u16, 0x3421]);
        let vals = [
            0xb6a1u16, 0x3421, 0xc86d, 0xe06f, 0xd0c6, 0x1b75, 0xd7df, 0x94e2,
        ];
        let regs = Registers::from(&vals);
        let reg: i32 = regs.slice_at(0).unwrap().try_into().unwrap();
        assert_eq!(reg, -1230949343i32);
        let reg_array: [i32; 3] = regs.slice_at(2).unwrap().try_into().unwrap();
        assert_eq!(reg_array, [-932323217, -792323211, -673213214]);
        assert!(TryInto::<[i32; 3]>::try_into(regs.slice_at(4).unwrap()).is_err());
        let arr = [-1230949343i32, -932323217, -792323211, -673213214];
        let regs = Registers::from(&arr);
        assert_eq!(regs.0, vals);
        let arr2: [i32; 4] = regs.slice_at(0).unwrap().try_into().unwrap();
        assert_eq!(arr, arr2);
    }
    #[test]
    fn test_u64() {
        let reg = 0xFFAA112299FF5577u64;
        let regs = Registers::from(&reg);
        assert_eq!(regs.0, vec![0xFFAA, 0x1122, 0x99FF, 0x5577]);
        let vals = [
            0x5d6du16, 0x5685, 0xa5b0, 0x63f7, 0x6b2a, 0x873f, 0xcfcf, 0x91d7, 0xeefe, 0x7662,
            0xde73, 0x1997, 0x70b7, 0x9d21, 0x458b, 0x82a5,
        ];
        let regs = Registers::from(&vals);
        let reg: u64 = regs.slice_at(0).unwrap().try_into().unwrap();
        assert_eq!(reg, 6732132149999199223);
        let reg_array: [u64; 3] = regs.slice_at(4).unwrap().try_into().unwrap();
        assert_eq!(
            reg_array,
            [
                7722133219219313111,
                17221332192122313111,
                8122133219212231333
            ]
        );
        assert!(TryInto::<[u64; 3]>::try_into(regs.slice_at(8).unwrap()).is_err());
        let arr = [
            6732132149999199223u64,
            7722133219219313111,
            17221332192122313111,
            8122133219212231333,
        ];
        let regs = Registers::from(&arr);
        assert_eq!(regs.0, vals);
        let arr2: [u64; 4] = regs.slice_at(0).unwrap().try_into().unwrap();
        assert_eq!(arr, arr2);
    }
    #[test]
    fn test_i64() {
        let reg = -8122133219212231333i64;
        let regs = Registers::from(&reg);
        assert_eq!(regs.0, vec![0x8f48, 0x62de, 0xba74, 0x7d5b]);
        let vals = [
            0x8f48u16, 0x62de, 0xba74, 0x7d5b, 0xd3ad, 0x43e0, 0x9b4, 0xd91b, 0x9a6c, 0x54ac,
            0xd3a1, 0x3ca3, 0xfcca, 0x788a, 0xd88c, 0xd4a3,
        ];
        let regs = Registers::from(&vals);
        let reg: i64 = regs.slice_at(0).unwrap().try_into().unwrap();
        assert_eq!(reg, -8122133219212231333i64);
        let reg_array: [i64; 3] = regs.slice_at(4).unwrap().try_into().unwrap();
        assert_eq!(
            reg_array,
            [
                -3193821931221231333,
                -7319382193122231133,
                -231239893122231133,
            ]
        );
        assert!(TryInto::<[u64; 3]>::try_into(regs.slice_at(8).unwrap()).is_err());
        let arr = [
            -8122133219212231333i64,
            -3193821931221231333,
            -7319382193122231133,
            -231239893122231133,
        ];
        let regs = Registers::from(&arr);
        assert_eq!(regs.0, vals);
        let arr2: [i64; 4] = regs.slice_at(0).unwrap().try_into().unwrap();
        assert_eq!(arr, arr2);
    }
    #[test]
    fn test_f32() {
        let reg = 38321.312f32;
        let regs = Registers::from(&reg);
        assert_eq!(regs.0, vec![0xb150, 0x4715]);
        let vals = [
            0xb150u16, 0x4715, 0xb8e3, 0x45f4, 0x51ec, 0xc49a, 0x3148, 0xc7b2,
        ];
        let regs = Registers::from(&vals);
        let reg: f32 = regs.slice_at(0).unwrap().try_into().unwrap();
        assert_eq!(reg, 38321.312);
        let reg_array: [f32; 3] = regs.slice_at(2).unwrap().try_into().unwrap();
        assert_eq!(reg_array, [7831.111, -1234.56, -91234.56]);
        assert!(TryInto::<[f32; 3]>::try_into(regs.slice_at(4).unwrap()).is_err());
        let arr = [38321.312f32, 7831.111, -1234.56, -91234.56];
        let regs = Registers::from(&arr);
        assert_eq!(regs.0, vals);
        let arr2: [f32; 4] = regs.slice_at(0).unwrap().try_into().unwrap();
        assert_eq!(arr, arr2);
    }
    #[test]
    fn test_f64() {
        let reg1 = 3832194.312f64;
        let regs = Registers::from(&reg1);
        assert_eq!(regs.0, vec![0x9db2, 0x27ef, 0x3cc1, 0x414d,]);
        let reg2 = 9832194.971f64;
        let regs = Registers::from(&reg2);
        assert_eq!(regs.0, vec![0x6e98, 0x5f12, 0xc0e0, 0x4162,]);
        let reg3 = -9732194.121f64;
        let regs = Registers::from(&reg3);
        assert_eq!(regs.0, vec![0x3b64, 0x43df, 0x900c, 0xc162,]);
        let reg4 = -1132194.92192f64;
        let regs = Registers::from(&reg4);
        assert_eq!(regs.0, vec![0xf2fa, 0xec02, 0x46a2, 0xc131,]);
        let vals = [
            0x9db2u16, 0x27ef, 0x3cc1, 0x414d, 0x6e98, 0x5f12, 0xc0e0, 0x4162, 0x3b64, 0x43df,
            0x900c, 0xc162, 0xf2fa, 0xec02, 0x46a2, 0xc131,
        ];
        let regs = Registers::from(&vals);
        let reg: f64 = regs.slice_at(0).unwrap().try_into().unwrap();
        assert_eq!(reg, reg1);
        let reg_array: [f64; 3] = regs.slice_at(4).unwrap().try_into().unwrap();
        assert_eq!(reg_array, [reg2, reg3, reg4]);
        assert!(TryInto::<[f64; 3]>::try_into(regs.slice_at(8).unwrap()).is_err());
        let arr = [reg1, reg2, reg3, reg4];
        let regs = Registers::from(&arr);
        assert_eq!(regs.0, vals);
        let arr2: [f64; 4] = regs.slice_at(0).unwrap().try_into().unwrap();
        assert_eq!(arr, arr2);
    }
}
