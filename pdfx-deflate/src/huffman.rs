//! Canonical Huffman construction with a 15-bit limit (DEFLATE).

pub struct HuffTable {
    /// code bits (MSB-first as RFC lists them)
    pub codes: Vec<u32>,
    pub lengths: Vec<u8>,
}

impl HuffTable {
    pub fn from_lengths(lengths: &[u8]) -> Self {
        let n = lengths.len();
        let mut bl_count = [0u32; 16];
        for &l in lengths {
            if l > 0 {
                bl_count[l as usize] += 1;
            }
        }
        let mut next_code = [0u32; 16];
        let mut code = 0u32;
        for bits in 1..=15 {
            code = (code + bl_count[bits - 1]) << 1;
            next_code[bits] = code;
        }
        let mut codes = vec![0u32; n];
        for (i, &len) in lengths.iter().enumerate() {
            if len != 0 {
                codes[i] = next_code[len as usize];
                next_code[len as usize] += 1;
            }
        }
        Self {
            codes,
            lengths: lengths.to_vec(),
        }
    }
}

/// Build limited-length Huffman bit lengths from frequencies.
pub fn build_lengths(freqs: &[u32], max_bits: u8) -> Vec<u8> {
    let n = freqs.len();
    let nodes: Vec<(u32, usize)> = freqs
        .iter()
        .enumerate()
        .filter(|(_, &f)| f > 0)
        .map(|(i, &f)| (f, i))
        .collect();

    if nodes.is_empty() {
        let mut lengths = vec![0u8; n];
        if n > 0 {
            lengths[0] = 1;
        }
        if n > 1 {
            lengths[1] = 1;
        }
        return lengths;
    }
    if nodes.len() == 1 {
        let mut lengths = vec![0u8; n];
        lengths[nodes[0].1] = 1;
        // DEFLATE needs at least two codes for lit/len and dist trees
        let other = if nodes[0].1 == 0 { 1.min(n - 1) } else { 0 };
        if n > 1 {
            lengths[other] = 1;
        }
        return lengths;
    }

    // Simple package-merge-ish: Huffman tree then cap lengths.
    #[derive(Clone)]
    enum Node {
        Leaf {
            freq: u32,
            sym: usize,
        },
        Inner {
            freq: u32,
            left: usize,
            right: usize,
        },
    }
    let mut tree: Vec<Node> = Vec::new();
    let mut heap: Vec<(u32, usize)> = Vec::new();
    for (f, s) in nodes {
        let id = tree.len();
        tree.push(Node::Leaf { freq: f, sym: s });
        heap.push((f, id));
    }
    fn freq_of(node: &Node) -> u32 {
        match node {
            Node::Leaf { freq, .. } | Node::Inner { freq, .. } => *freq,
        }
    }
    while heap.len() > 1 {
        heap.sort_by_key(|&(f, _)| f);
        let (_, a) = heap.remove(0);
        let (_, b) = heap.remove(0);
        let f = freq_of(&tree[a]) + freq_of(&tree[b]);
        let id = tree.len();
        tree.push(Node::Inner {
            freq: f,
            left: a,
            right: b,
        });
        heap.push((f, id));
    }
    let root = heap[0].1;
    let mut lengths = vec![0u8; n];
    fn walk(tree: &[Node], id: usize, depth: u8, lengths: &mut [u8], max_bits: u8) {
        match &tree[id] {
            Node::Leaf { sym, .. } => {
                lengths[*sym] = depth.max(1).min(max_bits);
            }
            Node::Inner { left, right, .. } => {
                walk(tree, *left, depth.saturating_add(1), lengths, max_bits);
                walk(tree, *right, depth.saturating_add(1), lengths, max_bits);
            }
        }
    }
    walk(&tree, root, 0, &mut lengths, max_bits);

    // If any length was capped we may have an oversubscribed tree. Redistribute.
    limit_lengths(&mut lengths, max_bits);
    lengths
}

fn limit_lengths(lengths: &mut [u8], max_bits: u8) {
    // Kraft check: sum 2^{-len} <= 1
    loop {
        let mut kraft = 0i32;
        for &l in lengths.iter() {
            if l > 0 {
                kraft += 1 << (max_bits - l);
            }
        }
        let limit = 1 << max_bits;
        if kraft <= limit {
            break;
        }
        // Lengthen a shortest non-zero code
        if let Some((i, _)) = lengths
            .iter()
            .enumerate()
            .filter(|(_, &l)| l > 0 && l < max_bits)
            .min_by_key(|(_, &l)| l)
        {
            lengths[i] += 1;
        } else {
            break;
        }
    }
}

/// Repeat-code order for dynamic Huffman (RFC 1951).
pub const CODE_LENGTH_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// RLE-encode bit lengths for the dynamic header.
pub fn encode_code_lengths(lengths: &[u8]) -> Vec<(u8, u8, u8)> {
    // (symbol, extra_bits, extra_value) where symbol is 0-18
    let mut out = Vec::new();
    let mut i = 0;
    while i < lengths.len() {
        let len = lengths[i];
        if len == 0 {
            let mut run = 1usize;
            i += 1;
            while i < lengths.len() && lengths[i] == 0 && run < 138 {
                run += 1;
                i += 1;
            }
            if run >= 11 {
                out.push((18, 7, (run - 11) as u8));
            } else if run >= 3 {
                out.push((17, 3, (run - 3) as u8));
            } else {
                for _ in 0..run {
                    out.push((0, 0, 0));
                }
            }
        } else {
            out.push((len, 0, 0));
            i += 1;
            let mut run = 0usize;
            while i < lengths.len() && lengths[i] == len && run < 6 {
                run += 1;
                i += 1;
            }
            if run >= 3 {
                out.push((16, 2, (run - 3) as u8));
            } else {
                // rewind the extra copies into literals
                i -= run;
            }
        }
    }
    out
}
