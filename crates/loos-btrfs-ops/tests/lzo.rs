mod common;
use common::*;
use loos_btrfs_ops::compress::lzo1x_decompress;

const HELPER: &str = r#"
#include <lzo/lzo1x.h>
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
int main(int argc, char **argv) {
    if (lzo_init() != LZO_E_OK) return 2;
    size_t chunk = (size_t)atoi(argv[3]);
    FILE *in = fopen(argv[1], "rb"), *out = fopen(argv[2], "wb");
    unsigned char *ib = malloc(chunk), *ob = malloc(chunk + chunk / 16 + 64 + 3);
    static unsigned char wrk[LZO1X_1_MEM_COMPRESS];
    size_t n;
    while ((n = fread(ib, 1, chunk, in)) > 0) {
        lzo_uint olen = 0;
        if (lzo1x_1_compress(ib, n, ob, &olen, wrk) != LZO_E_OK) return 3;
        uint32_t h[2] = { (uint32_t)n, (uint32_t)olen };
        fwrite(h, 4, 2, out); fwrite(ob, 1, olen, out);
    }
    fclose(out);
    return 0;
}
"#;

fn compressor() -> Option<std::path::PathBuf> {
    if !have("gcc") {
        return None;
    }
    let d = scratch("lzo-helper");
    let src = d.join("lzoc.c");
    std::fs::write(&src, HELPER).unwrap();
    let bin = d.join("lzoc");
    let (ok, out) = run("gcc", &["-O2", "-o", bin.to_str().unwrap(), src.to_str().unwrap(), "-llzo2"]);
    if !ok {
        eprintln!("skip: no liblzo2 development files: {}", out);
        return None;
    }
    Some(bin)
}

fn roundtrip(bin: &std::path::Path, data: &[u8], chunk: usize, tag: &str) {
    let d = scratch(&format!("lzo-{}", tag));
    std::fs::write(d.join("in"), data).unwrap();
    must(bin.to_str().unwrap(), &[d.join("in").to_str().unwrap(), d.join("out").to_str().unwrap(), &chunk.to_string()]);
    let comp = std::fs::read(d.join("out")).unwrap();
    let mut pos = 0;
    let mut got = Vec::new();
    while pos < comp.len() {
        let n = u32::from_le_bytes(comp[pos..pos + 4].try_into().unwrap()) as usize;
        let olen = u32::from_le_bytes(comp[pos + 4..pos + 8].try_into().unwrap()) as usize;
        pos += 8;
        let mut buf = vec![0u8; n + 64];
        let produced = lzo1x_decompress(&comp[pos..pos + olen], &mut buf).unwrap_or_else(|e| panic!("{}: chunk at {}: {:?}", tag, got.len(), e));
        assert_eq!(produced, n, "{} chunk length", tag);
        got.extend_from_slice(&buf[..produced]);
        pos += olen;
    }
    assert_eq!(got, data, "{}", tag);
}

#[test]
fn decoder_matches_liblzo2_on_assorted_data() {
    let bin = match compressor() {
        Some(b) => b,
        None => return,
    };
    let text: Vec<u8> = b"the rain in spain stays mainly in the plain. ".iter().cycle().take(300_000).copied().collect();
    let zeros = vec![0u8; 100_000];
    let mut runs = Vec::new();
    for i in 0..5000u32 {
        runs.extend(std::iter::repeat((i % 251) as u8).take((i % 97) as usize + 1));
    }
    let rnd = payload(5, 200_000);
    let mixed: Vec<u8> = [text[..50_000].to_vec(), rnd[..30_000].to_vec(), zeros[..20_000].to_vec(), text[1000..60_000].to_vec()].concat();
    // also tiny inputs, which exercise the "first literal run" prefix forms
    let tiny: Vec<Vec<u8>> = (0..64usize).map(|n| payload(n as u64 + 100, n)).collect();
    for (name, data) in [("text", &text), ("zeros", &zeros), ("runs", &runs), ("rnd", &rnd), ("mixed", &mixed)] {
        for chunk in [4096usize, 65536, 128 * 1024] {
            roundtrip(&bin, data, chunk, &format!("{}-{}", name, chunk));
        }
    }
    for (i, t) in tiny.iter().enumerate() {
        if !t.is_empty() {
            roundtrip(&bin, t, 4096, &format!("tiny{}", i));
        }
    }
}

#[test]
fn hostile_streams_never_panic() {
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    for _ in 0..20000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let len = (x % 200) as usize;
        let src: Vec<u8> = (0..len).map(|i| ((x >> (i % 56)) as u8).wrapping_mul(i as u8 | 1)).collect();
        let mut dst = vec![0u8; 4096];
        let _ = lzo1x_decompress(&src, &mut dst);
    }
}
