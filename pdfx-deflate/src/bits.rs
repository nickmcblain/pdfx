/// LSB-first bit writer used by DEFLATE.
pub struct BitWriter {
    pub buf: Vec<u8>,
    bit_buf: u32,
    bit_count: u32,
}

impl BitWriter {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            bit_buf: 0,
            bit_count: 0,
        }
    }

    pub fn write_bits(&mut self, bits: u32, n: u32) {
        debug_assert!(n <= 16);
        self.bit_buf |= bits << self.bit_count;
        self.bit_count += n;
        while self.bit_count >= 8 {
            self.buf.push(self.bit_buf as u8);
            self.bit_buf >>= 8;
            self.bit_count -= 8;
        }
    }

    /// Huffman codes are sent MSB-first; the stream is LSB-first. Reverse.
    pub fn write_huffman(&mut self, code: u32, n: u32) {
        self.write_bits(reverse_bits(code, n), n);
    }

    pub fn align_byte(&mut self) {
        if self.bit_count > 0 {
            self.buf.push(self.bit_buf as u8);
            self.bit_buf = 0;
            self.bit_count = 0;
        }
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.align_byte();
        self.buf
    }
}

fn reverse_bits(mut v: u32, n: u32) -> u32 {
    let mut r = 0u32;
    for _ in 0..n {
        r = (r << 1) | (v & 1);
        v >>= 1;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_lsb_first() {
        let mut w = BitWriter::new();
        w.write_bits(0b1, 1);
        w.write_bits(0b10, 2);
        w.write_bits(0b011, 3);
        w.align_byte();
        // bits sent: 1, 0, 1, 1, 1, 0  -> byte 0b0001_1101 = 0x1D (lsb first)
        assert_eq!(w.buf, vec![0b0001_1101]);
    }
}
