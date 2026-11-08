#!/usr/bin/env python3
"""Packs a PEM CA bundle into certs/roots.bin (repeated: u32 big-endian length, DER) for the LoOS native TLS client.
Usage: gen_roots.py [bundle.pem]   (default: the system bundle). Re-run to refresh; the kernel skips key types it cannot verify."""
import base64, re, struct, sys, pathlib
cands = [sys.argv[1]] if len(sys.argv) > 1 else ["/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem", "/etc/ssl/certs/ca-certificates.crt"]
src = next((c for c in cands if pathlib.Path(c).exists()), None)
if not src: sys.exit("no CA bundle found")
pem = pathlib.Path(src).read_text()
ders = [base64.b64decode(re.sub(r"\s+", "", m)) for m in re.findall(r"-----BEGIN CERTIFICATE-----(.*?)-----END CERTIFICATE-----", pem, re.S)]
out = pathlib.Path(__file__).resolve().parent.parent / "certs" / "roots.bin"
out.write_bytes(b"".join(struct.pack(">I", len(d)) + d for d in ders))
print(f"{len(ders)} roots from {src} -> {out} ({out.stat().st_size} bytes)")
