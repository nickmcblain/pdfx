use crate::tables::HuffSpec;

pub struct JpegWriter {
    buf: Vec<u8>,
    bit_buf: u32,
    bit_count: u32,
}

impl JpegWriter {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            bit_buf: 0,
            bit_count: 0,
        }
    }

    pub fn marker(&mut self, code: u8) {
        self.flush_bits();
        self.buf.push(0xFF);
        self.buf.push(code);
    }

    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn bytes(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }

    pub fn huffman(&mut self, spec: &HuffSpec, symbol: u8) {
        let (code, len) = spec.code(symbol);
        self.write_bits(code, len);
    }

    pub fn write_amp(&mut self, amp: u16, bits: u8) {
        self.write_bits(u32::from(amp) & ((1 << bits) - 1), bits);
    }

    fn write_bits(&mut self, bits: u32, n: u8) {
        self.bit_buf = (self.bit_buf << n) | bits;
        self.bit_count += u32::from(n);
        while self.bit_count >= 8 {
            self.bit_count -= 8;
            let byte = (self.bit_buf >> self.bit_count) as u8;
            self.buf.push(byte);
            if byte == 0xFF {
                self.buf.push(0x00); // stuff
            }
            self.bit_buf &= (1 << self.bit_count) - 1;
        }
    }

    pub fn flush_bits(&mut self) {
        if self.bit_count > 0 {
            let pad = 8 - self.bit_count as u8;
            self.write_bits((1 << pad) - 1, pad); // pad 1s
        }
        self.bit_buf = 0;
        self.bit_count = 0;
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.buf
    }
}
