const BASE: u32 = 65521;

pub fn adler32(data: &[u8]) -> u32 {
    let mut s1: u32 = 1;
    let mut s2: u32 = 0;
    for &b in data {
        s1 += u32::from(b);
        if s1 >= BASE {
            s1 -= BASE;
        }
        s2 += s1;
        if s2 >= BASE {
            s2 -= BASE;
        }
    }
    (s2 << 16) | s1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_one() {
        assert_eq!(adler32(&[]), 1);
    }

    #[test]
    fn wikipedia_example() {
        // Adler-32("Wikipedia") = 0x11E60398
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }
}
