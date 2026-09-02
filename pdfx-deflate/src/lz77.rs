//! Hash-chain match finder + optimal parse over longest-match candidates.

pub const MIN_MATCH: usize = 3;
pub const MAX_MATCH: usize = 258;
pub const WINDOW: usize = 32768;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Literal(u8),
    Match { length: u16, distance: u16 },
}

const HASH_BITS: u32 = 15;
const HASH_SIZE: usize = 1 << HASH_BITS;
const MAX_CHAIN: usize = 64;

pub fn parse(data: &[u8]) -> Vec<Token> {
    if data.is_empty() {
        return Vec::new();
    }
    let longest = find_longest_matches(data);
    // DP is O(n) extra memory and slower; use lazy greedy on big streams.
    if data.len() > 16 * 1024 {
        greedy_parse(data, &longest)
    } else {
        optimal_parse(data, &longest)
    }
}

#[derive(Clone, Copy)]
struct MatchCand {
    length: u16,
    distance: u16,
}

fn find_longest_matches(data: &[u8]) -> Vec<Option<MatchCand>> {
    let n = data.len();
    let mut out = vec![None; n];
    if n < MIN_MATCH {
        return out;
    }

    let mut head = vec![-1i32; HASH_SIZE];
    let mut prev = vec![-1i32; n];

    for i in 0..n.saturating_sub(MIN_MATCH - 1) {
        let h = hash3(data, i);
        let mut p = head[h];
        prev[i] = p;
        head[h] = i as i32;

        let mut best_len = 0u16;
        let mut best_dist = 0u16;
        let mut chain = 0usize;
        let chain_limit = if n > 64 * 1024 { 16 } else { MAX_CHAIN };
        while p >= 0 && chain < chain_limit {
            let pos = p as usize;
            if i - pos > WINDOW {
                break;
            }
            let dist = (i - pos) as u16;
            let len = match_len(data, pos, i);
            if len >= MIN_MATCH as u16 && len > best_len {
                best_len = len;
                best_dist = dist;
                if best_len as usize == MAX_MATCH {
                    break;
                }
            }
            p = prev[pos];
            chain += 1;
        }
        if best_len >= MIN_MATCH as u16 {
            out[i] = Some(MatchCand {
                length: best_len,
                distance: best_dist,
            });
        }
    }
    out
}

fn hash3(data: &[u8], i: usize) -> usize {
    let v = u32::from(data[i]) | (u32::from(data[i + 1]) << 8) | (u32::from(data[i + 2]) << 16);
    (v.wrapping_mul(0x1E35_A7BD) >> (32 - HASH_BITS)) as usize
}

fn match_len(data: &[u8], a: usize, b: usize) -> u16 {
    let max = (data.len() - b).min(MAX_MATCH);
    let mut n = 0usize;
    while n < max && data[a + n] == data[b + n] {
        n += 1;
    }
    n as u16
}

fn greedy_parse(data: &[u8], longest: &[Option<MatchCand>]) -> Vec<Token> {
    let n = data.len();
    let mut tokens = Vec::new();
    let mut i = 0usize;
    while i < n {
        if let Some(m) = &longest[i] {
            let take = if i + 1 < n {
                match &longest[i + 1] {
                    Some(next) if next.length > m.length => false,
                    _ => true,
                }
            } else {
                true
            };
            if take && m.length >= MIN_MATCH as u16 {
                tokens.push(Token::Match {
                    length: m.length,
                    distance: m.distance,
                });
                i += m.length as usize;
                continue;
            }
        }
        tokens.push(Token::Literal(data[i]));
        i += 1;
    }
    tokens
}

/// Forward DP: at each position, take a literal or a prefix of the longest match.
fn optimal_parse(data: &[u8], longest: &[Option<MatchCand>]) -> Vec<Token> {
    let n = data.len();
    let mut cost = vec![u32::MAX / 4; n + 1];
    let mut prev: Vec<Option<(usize, Token)>> = vec![None; n + 1];
    cost[0] = 0;

    for i in 0..n {
        let c = cost[i];
        let lit = Token::Literal(data[i]);
        let lit_c = c + literal_bits(data[i]);
        if lit_c < cost[i + 1] {
            cost[i + 1] = lit_c;
            prev[i + 1] = Some((i, lit));
        }
        if let Some(m) = &longest[i] {
            for len in interesting_lengths(m.length as usize) {
                let next = i + len;
                if next > n {
                    continue;
                }
                let tok = Token::Match {
                    length: len as u16,
                    distance: m.distance,
                };
                let mc = c + match_bits(len as u16, m.distance);
                if mc < cost[next] {
                    cost[next] = mc;
                    prev[next] = Some((i, tok));
                }
            }
        }
    }

    let mut tokens = Vec::new();
    let mut i = n;
    while i > 0 {
        let (p, tok) = prev[i].expect("DP reached start");
        tokens.push(tok);
        i = p;
    }
    tokens.reverse();
    tokens
}

fn interesting_lengths(max_len: usize) -> impl Iterator<Item = usize> {
    const CANDS: [usize; 8] = [3, 4, 5, 8, 16, 32, 64, 128];
    CANDS
        .into_iter()
        .filter(move |&l| l < max_len)
        .chain(std::iter::once(max_len))
}

fn literal_bits(b: u8) -> u32 {
    // Fixed Huffman: 0-143 = 8 bits, 144-255 = 9 bits
    if b <= 143 {
        8
    } else {
        9
    }
}

fn match_bits(length: u16, distance: u16) -> u32 {
    let (len_code, extra_l) = length_code(length);
    let (_dist_code, extra_d) = distance_code(distance);
    let len_bits = if len_code <= 279 { 7 } else { 8 };
    // distance fixed Huffman is 5 bits
    len_bits + extra_l + 5 + extra_d
}

pub fn length_code(length: u16) -> (u16, u32) {
    const TABLE: [(u16, u16, u32); 29] = [
        (3, 257, 0),
        (4, 258, 0),
        (5, 259, 0),
        (6, 260, 0),
        (7, 261, 0),
        (8, 262, 0),
        (9, 263, 0),
        (10, 264, 0),
        (11, 265, 1),
        (13, 266, 1),
        (15, 267, 1),
        (17, 268, 1),
        (19, 269, 2),
        (23, 270, 2),
        (27, 271, 2),
        (31, 272, 2),
        (35, 273, 3),
        (43, 274, 3),
        (51, 275, 3),
        (59, 276, 3),
        (67, 277, 4),
        (83, 278, 4),
        (99, 279, 4),
        (115, 280, 4),
        (131, 281, 5),
        (163, 282, 5),
        (195, 283, 5),
        (227, 284, 5),
        (258, 285, 0),
    ];
    let mut best = TABLE[0];
    for &(base, code, extra) in &TABLE {
        if length >= base {
            best = (base, code, extra);
        }
    }
    (best.1, best.2)
}

pub fn length_base(code: u16) -> u16 {
    const BASES: [u16; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
        131, 163, 195, 227, 258,
    ];
    BASES[(code - 257) as usize]
}

pub fn distance_code(distance: u16) -> (u16, u32) {
    const TABLE: [(u16, u16, u32); 30] = [
        (1, 0, 0),
        (2, 1, 0),
        (3, 2, 0),
        (4, 3, 0),
        (5, 4, 1),
        (7, 5, 1),
        (9, 6, 2),
        (13, 7, 2),
        (17, 8, 3),
        (25, 9, 3),
        (33, 10, 4),
        (49, 11, 4),
        (65, 12, 5),
        (97, 13, 5),
        (129, 14, 6),
        (193, 15, 6),
        (257, 16, 7),
        (385, 17, 7),
        (513, 18, 8),
        (769, 19, 8),
        (1025, 20, 9),
        (1537, 21, 9),
        (2049, 22, 10),
        (3073, 23, 10),
        (4097, 24, 11),
        (6145, 25, 11),
        (8193, 26, 12),
        (12289, 27, 12),
        (16385, 28, 13),
        (24577, 29, 13),
    ];
    let mut best = TABLE[0];
    for &(base, code, extra) in &TABLE {
        if distance >= base {
            best = (base, code, extra);
        }
    }
    (best.1, best.2)
}

pub fn distance_base(code: u16) -> u16 {
    const BASES: [u16; 30] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
        2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
    ];
    BASES[code as usize]
}

pub fn length_extra(code: u16) -> u32 {
    const EXTRA: [u32; 29] = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
    ];
    EXTRA[(code - 257) as usize]
}

pub fn distance_extra(code: u16) -> u32 {
    const EXTRA: [u32; 30] = [
        0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
        13, 13,
    ];
    EXTRA[code as usize]
}
