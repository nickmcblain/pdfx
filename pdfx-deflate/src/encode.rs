use crate::adler::adler32;
use crate::bits::BitWriter;
use crate::huffman::{build_lengths, encode_code_lengths, HuffTable, CODE_LENGTH_ORDER};
use crate::lz77::{
    distance_base, distance_code, distance_extra, length_base, length_code, length_extra, parse,
    Token,
};

const LITLEN_SIZE: usize = 286;
const DIST_SIZE: usize = 30;

pub fn compress_zlib(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 2 + 16);
    // CMF/FLG: deflate, 32K window, max compression. 0x78DA % 31 == 0.
    out.push(0x78);
    out.push(0xDA);
    out.extend_from_slice(&compress_deflate(data));
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn compress_deflate(data: &[u8]) -> Vec<u8> {
    let tokens = parse(data);
    let mut w = BitWriter::new();
    write_block(&mut w, &tokens, true);
    w.finish()
}

fn write_block(w: &mut BitWriter, tokens: &[Token], bfinal: bool) {
    let (ll_freq, dist_freq) = count_freqs(tokens);
    let ll_lengths = build_lengths(&ll_freq, 15);
    let dist_lengths = build_lengths(&dist_freq, 15);
    let ll_tab = HuffTable::from_lengths(&ll_lengths);
    let dist_tab = HuffTable::from_lengths(&dist_lengths);

    let dyn_ok = ll_lengths.iter().any(|&l| l > 0) && dist_lengths.iter().any(|&l| l > 0);
    if dyn_ok {
        w.write_bits(if bfinal { 1 } else { 0 }, 1);
        w.write_bits(0b10, 2); // dynamic
        write_dynamic_header(w, &ll_lengths, &dist_lengths);
        write_tokens(w, tokens, &ll_tab, &dist_tab);
    } else {
        w.write_bits(if bfinal { 1 } else { 0 }, 1);
        w.write_bits(0b01, 2); // fixed
        write_tokens_fixed(w, tokens);
    }
}

fn count_freqs(tokens: &[Token]) -> ([u32; LITLEN_SIZE], [u32; DIST_SIZE]) {
    let mut ll = [0u32; LITLEN_SIZE];
    let mut dist = [0u32; DIST_SIZE];
    for t in tokens {
        match *t {
            Token::Literal(b) => ll[b as usize] += 1,
            Token::Match { length, distance } => {
                let (lc, _) = length_code(length);
                ll[lc as usize] += 1;
                let (dc, _) = distance_code(distance);
                dist[dc as usize] += 1;
            }
        }
    }
    ll[256] += 1; // EOB
                  // Ensure at least one distance code so the tree is valid
    if dist.iter().all(|&c| c == 0) {
        dist[0] = 1;
    }
    (ll, dist)
}

fn write_dynamic_header(w: &mut BitWriter, ll_lengths: &[u8], dist_lengths: &[u8]) {
    let mut ll_trim = ll_lengths.len();
    while ll_trim > 257 && ll_lengths[ll_trim - 1] == 0 {
        ll_trim -= 1;
    }
    let mut dist_trim = dist_lengths.len();
    while dist_trim > 1 && dist_lengths[dist_trim - 1] == 0 {
        dist_trim -= 1;
    }

    let mut all = Vec::with_capacity(ll_trim + dist_trim);
    all.extend_from_slice(&ll_lengths[..ll_trim]);
    all.extend_from_slice(&dist_lengths[..dist_trim]);
    let rle = encode_code_lengths(&all);

    let mut cl_freq = [0u32; 19];
    for &(sym, _, _) in &rle {
        cl_freq[sym as usize] += 1;
    }
    let cl_lengths = build_lengths(&cl_freq, 7);
    let cl_tab = HuffTable::from_lengths(&cl_lengths);

    let mut hclen = 19;
    while hclen > 4 && cl_lengths[CODE_LENGTH_ORDER[hclen - 1]] == 0 {
        hclen -= 1;
    }

    let hlit = ll_trim - 257;
    let hdist = dist_trim - 1;
    w.write_bits(hlit as u32, 5);
    w.write_bits(hdist as u32, 5);
    w.write_bits((hclen - 4) as u32, 4);

    for &idx in &CODE_LENGTH_ORDER[..hclen] {
        w.write_bits(u32::from(cl_lengths[idx]), 3);
    }

    for &(sym, extra_n, extra_v) in &rle {
        let len = cl_tab.lengths[sym as usize];
        w.write_huffman(cl_tab.codes[sym as usize], u32::from(len));
        if extra_n > 0 {
            w.write_bits(u32::from(extra_v), u32::from(extra_n));
        }
    }
}

fn write_tokens(w: &mut BitWriter, tokens: &[Token], ll: &HuffTable, dist: &HuffTable) {
    for t in tokens {
        match *t {
            Token::Literal(b) => {
                let i = b as usize;
                w.write_huffman(ll.codes[i], u32::from(ll.lengths[i]));
            }
            Token::Match { length, distance } => {
                let (lc, _) = length_code(length);
                w.write_huffman(ll.codes[lc as usize], u32::from(ll.lengths[lc as usize]));
                let extra_l = length_extra(lc);
                if extra_l > 0 {
                    w.write_bits(u32::from(length - length_base(lc)), extra_l);
                }
                let (dc, _) = distance_code(distance);
                w.write_huffman(
                    dist.codes[dc as usize],
                    u32::from(dist.lengths[dc as usize]),
                );
                let extra_d = distance_extra(dc);
                if extra_d > 0 {
                    w.write_bits(u32::from(distance - distance_base(dc)), extra_d);
                }
            }
        }
    }
    w.write_huffman(ll.codes[256], u32::from(ll.lengths[256]));
}

fn write_tokens_fixed(w: &mut BitWriter, tokens: &[Token]) {
    for t in tokens {
        match *t {
            Token::Literal(b) => write_fixed_litlen(w, u16::from(b)),
            Token::Match { length, distance } => {
                let (lc, _) = length_code(length);
                write_fixed_litlen(w, lc);
                let extra_l = length_extra(lc);
                if extra_l > 0 {
                    w.write_bits(u32::from(length - length_base(lc)), extra_l);
                }
                let (dc, _) = distance_code(distance);
                w.write_bits(reverse5(dc), 5);
                let extra_d = distance_extra(dc);
                if extra_d > 0 {
                    w.write_bits(u32::from(distance - distance_base(dc)), extra_d);
                }
            }
        }
    }
    write_fixed_litlen(w, 256);
}

fn write_fixed_litlen(w: &mut BitWriter, sym: u16) {
    // RFC 1951 3.2.6 — codes shown MSB-first
    if sym <= 143 {
        w.write_huffman(0x30 + u32::from(sym), 8);
    } else if sym <= 255 {
        w.write_huffman(0x190 + u32::from(sym - 144), 9);
    } else if sym <= 279 {
        w.write_huffman(u32::from(sym - 256), 7);
    } else {
        w.write_huffman(0xC0 + u32::from(sym - 280), 8);
    }
}

fn reverse5(v: u16) -> u32 {
    let mut x = u32::from(v);
    let mut r = 0u32;
    for _ in 0..5 {
        r = (r << 1) | (x & 1);
        x >>= 1;
    }
    r
}
