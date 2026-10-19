//! Reading and writing the binary formats the game uses (recordings and network packets):
//! little-endian numbers, strings, variable-length integers, and a player's `Input`, stored as
//! what changed since the one before.

use anyhow::{Result, bail, ensure};

use crate::sim::Input;

/// What the input before this one was, which each input is stored against
#[derive(Default)]
pub struct Before {
    pub buttons: u16,
    pub yaw: u16,
    pub pitch: u16,
}

/// The buttons as bits
pub fn buttons_of(i: &Input) -> u16 {
    [i.forward, i.back, i.left, i.right, i.run, i.fire, i.use_key, i.reload, i.gun, i.grenade, i.toggle_culling, i.crouch]
        .iter().enumerate().fold(0, |acc, (n, &b)| acc | (b as u16) << n)
}

/// How far to turn from one angle to another by the short way round: -32768 to 32767
pub fn turn(from: u16, to: u16) -> i32 {
    to.wrapping_sub(from) as i16 as i32
}

/// Small numbers, positive or negative, as small positive ones: 0, -1, 1, -2, 2 become 0, 1, 2, 3, 4
pub fn zigzag(v: i32) -> u32 {
    ((v << 1) ^ (v >> 31)) as u32
}

pub fn unzigzag(v: u32) -> i32 {
    (v >> 1) as i32 ^ -((v & 1) as i32)
}

pub struct Writer(pub Vec<u8>);

impl Writer {
    pub fn new() -> Writer {
        Writer(Vec::new())
    }
    pub fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }
    pub fn v3(&mut self, v: [f32; 3]) {
        v.iter().for_each(|&c| self.f32(c));
    }
    pub fn opt_v3(&mut self, v: Option<[f32; 3]>) {
        self.bool(v.is_some());
        if let Some(v) = v {
            self.v3(v);
        }
    }

    pub fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    pub fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
    pub fn opt_u32(&mut self, v: Option<u32>) {
        self.u8(v.is_some() as u8);
        self.u32(v.unwrap_or(0));
    }
    pub fn opt_f32(&mut self, v: Option<f32>) {
        self.u8(v.is_some() as u8);
        self.f32(v.unwrap_or(0.0));
    }
    pub fn string(&mut self, s: &str) {
        self.u16(s.len() as u16);
        self.0.extend_from_slice(s.as_bytes());
    }
    /// A number in as few bytes as it needs: seven bits a byte, with the top bit saying more follow
    pub fn varint(&mut self, mut v: u32) {
        while v >= 0x80 {
            self.u8(v as u8 | 0x80);
            v >>= 7;
        }
        self.u8(v as u8);
    }
    pub fn input(&mut self, i: &Input, before: &mut Before) {
        let buttons = buttons_of(i);
        let control = (buttons != before.buttons) as u8
            | ((i.yaw != before.yaw) as u8) << 1
            | ((i.pitch != before.pitch) as u8) << 2
            | (i.observer.is_some() as u8) << 3;
        self.u8(control);
        if control & 1 != 0 {
            self.u16(buttons);
        }
        if control & 2 != 0 {
            self.varint(zigzag(turn(before.yaw, i.yaw)));
        }
        if control & 4 != 0 {
            self.varint(zigzag(turn(before.pitch, i.pitch)));
        }
        if let Some(o) = i.observer {
            o.iter().for_each(|&c| self.f32(c));
        }
        *before = Before { buttons, yaw: i.yaw, pitch: i.pitch };
    }
}

pub struct Reader<'a> {
    pub bytes: &'a [u8],
    pub at: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes, at: 0 }
    }
    /// How many bytes are left
    pub fn left(&self) -> usize {
        self.bytes.len() - self.at
    }
    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? != 0)
    }
    pub fn v3(&mut self) -> Result<[f32; 3]> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }
    pub fn opt_v3(&mut self) -> Result<Option<[f32; 3]>> {
        Ok(if self.bool()? { Some(self.v3()?) } else { None })
    }

    pub fn take(&mut self, n: usize) -> Result<&[u8]> {
        if self.at + n > self.bytes.len() {
            bail!("the recording ends in the middle of an entry");
        }
        let out = &self.bytes[self.at..self.at + n];
        self.at += n;
        Ok(out)
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    pub fn opt_u32(&mut self) -> Result<Option<u32>> {
        let (some, v) = (self.u8()? != 0, self.u32()?);
        Ok(some.then_some(v))
    }
    pub fn opt_f32(&mut self) -> Result<Option<f32>> {
        let (some, v) = (self.u8()? != 0, self.f32()?);
        Ok(some.then_some(v))
    }
    pub fn string(&mut self) -> Result<String> {
        let n = self.u16()? as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
    pub fn varint(&mut self) -> Result<u32> {
        let mut v = 0u32;
        for shift in (0..35).step_by(7) {
            let b = self.u8()?;
            v |= ((b & 0x7F) as u32) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        bail!("a number in the recording is too long")
    }
    pub fn input(&mut self, before: &mut Before) -> Result<Input> {
        let control = self.u8()?;
        ensure!(control & 0xF0 == 0, "an input in the recording makes no sense");
        let buttons = if control & 1 != 0 { self.u16()? } else { before.buttons };
        let yaw = if control & 2 != 0 { before.yaw.wrapping_add(unzigzag(self.varint()?) as u16) } else { before.yaw };
        let pitch = if control & 4 != 0 { before.pitch.wrapping_add(unzigzag(self.varint()?) as u16) } else { before.pitch };
        let observer = if control & 8 != 0 { Some([self.f32()?, self.f32()?, self.f32()?]) } else { None };
        *before = Before { buttons, yaw, pitch };
        let on = |n: usize| buttons >> n & 1 != 0;
        Ok(Input {
            yaw, pitch,
            forward: on(0), back: on(1), left: on(2), right: on(3), run: on(4), fire: on(5),
            use_key: on(6), reload: on(7), gun: on(8), grenade: on(9), toggle_culling: on(10), crouch: on(11),
            observer,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_stored_in_as_few_bytes_as_they_need() {
        for v in [0, 1, 127, 128, 16383, 16384, u32::MAX] {
            let mut w = Writer(Vec::new());
            w.varint(v);
            assert_eq!(Reader { bytes: &w.0, at: 0 }.varint().unwrap(), v);
            assert_eq!(w.0.len(), if v < 128 { 1 } else if v < 16384 { 2 } else if v == u32::MAX { 5 } else { 3 });
        }
        for d in [-32768, -64, -1, 0, 1, 63, 32767] {
            assert_eq!(unzigzag(zigzag(d)), d);
        }
        assert_eq!(zigzag(-1), 1);
        assert_eq!(zigzag(1), 2);
    }
}
