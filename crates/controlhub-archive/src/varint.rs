//! LEB128 varints, zigzag signed integers and length-prefixed byte strings.
//! The reader treats its input as untrusted: every length is checked against the bytes left.

use crate::{ArchiveError, Result};

#[derive(Debug, Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub fn uvar(&mut self, mut v: u64) {
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                self.buf.push(byte);
                return;
            }
            self.buf.push(byte | 0x80);
        }
    }

    pub fn ivar(&mut self, v: i64) {
        self.uvar(((v << 1) ^ (v >> 63)) as u64);
    }

    pub fn bytes(&mut self, b: &[u8]) {
        self.uvar(b.len() as u64);
        self.buf.extend_from_slice(b);
    }

    pub fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
}

#[derive(Debug)]
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    pub fn u8(&mut self) -> Result<u8> {
        let b = *self.buf.get(self.pos).ok_or(ArchiveError::Corrupt("unexpected end of data"))?;
        self.pos += 1;
        Ok(b)
    }

    pub fn uvar(&mut self) -> Result<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.u8()?;
            let part = u64::from(b & 0x7f);
            if shift == 63 && part > 1 {
                return Err(ArchiveError::Corrupt("varint overflow"));
            }
            v |= part << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(ArchiveError::Corrupt("varint too long"))
    }

    pub fn ivar(&mut self) -> Result<i64> {
        let u = self.uvar()?;
        Ok(((u >> 1) as i64) ^ -((u & 1) as i64))
    }

    /// A count of items that each take at least one byte, so it can never exceed the bytes left.
    pub fn count(&mut self) -> Result<usize> {
        let n = self.uvar()?;
        usize::try_from(n).ok().filter(|n| *n <= self.remaining()).ok_or(ArchiveError::Corrupt("count exceeds data"))
    }

    pub fn bytes(&mut self) -> Result<&'a [u8]> {
        let len = usize::try_from(self.uvar()?).map_err(|_| ArchiveError::Corrupt("length overflow"))?;
        if len > self.remaining() {
            return Err(ArchiveError::Corrupt("length exceeds data"));
        }
        let out = &self.buf[self.pos..self.pos + len];
        self.pos += len;
        Ok(out)
    }

    pub fn str(&mut self) -> Result<&'a str> {
        std::str::from_utf8(self.bytes()?).map_err(|_| ArchiveError::Corrupt("invalid utf-8"))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn small_values_take_one_byte() {
        let mut w = Writer::default();
        w.uvar(0);
        w.uvar(127);
        w.ivar(-1);
        w.ivar(63);
        assert_eq!(w.buf.len(), 4);
    }

    #[test]
    fn truncated_and_oversized_input_is_an_error() {
        assert!(Reader::new(&[]).u8().is_err());
        assert!(Reader::new(&[0x80]).uvar().is_err());
        assert!(Reader::new(&[0xff; 11]).uvar().is_err(), "more than 64 bits");
        assert!(Reader::new(&[0x05, b'a']).bytes().is_err(), "length beyond the end");
        assert!(Reader::new(&[0x03, 0x00]).count().is_err(), "count beyond the end");
        assert!(Reader::new(&[0x02, 0xff, 0xfe]).str().is_err(), "not utf-8");
    }

    proptest! {
        #[test]
        fn roundtrip(u in any::<u64>(), i in any::<i64>(), s in ".*", b in prop::collection::vec(any::<u8>(), 0..64)) {
            let mut w = Writer::default();
            w.uvar(u);
            w.ivar(i);
            w.str(&s);
            w.bytes(&b);
            let mut r = Reader::new(&w.buf);
            prop_assert_eq!(r.uvar().unwrap(), u);
            prop_assert_eq!(r.ivar().unwrap(), i);
            prop_assert_eq!(r.str().unwrap(), s.as_str());
            prop_assert_eq!(r.bytes().unwrap(), b.as_slice());
            prop_assert!(r.is_empty());
        }

        #[test]
        fn arbitrary_bytes_never_panic(b in prop::collection::vec(any::<u8>(), 0..64)) {
            let mut r = Reader::new(&b);
            while !r.is_empty() {
                if r.str().is_err() && r.uvar().is_err() {
                    break;
                }
            }
        }
    }
}
