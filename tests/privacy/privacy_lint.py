#!/usr/bin/env python3
"""Reject direct socket and network-client calls in LoOS source and apps."""

import argparse
from pathlib import Path
import re
import sys


ROOT = Path(__file__).resolve().parents[2]
SOURCE_SUFFIXES = {
    ".c", ".h", ".cc", ".cpp", ".cxx", ".hpp", ".hxx",
    ".s", ".S", ".asm", ".def", ".cmake",
}
NETWORK_CALL = re.compile(
    r"\b(?:"
    r"socket|connect|bind|listen|accept|send|sendto|sendmsg|recv|recvfrom|recvmsg|"
    r"shutdown|closesocket|WSAStartup|WSASocket(?:A|W)?|WSAConnect(?:ByName)?|"
    r"WSASend(?:To)?|WSARecv(?:From)?|getaddrinfo|getnameinfo|gethostbyname|"
    r"GetAddrInfo(?:A|W)?|GetNameInfo(?:A|W)?|InternetOpen(?:A|W)?|"
    r"InternetConnect(?:A|W)?|InternetOpenUrl(?:A|W)?|HttpSendRequest(?:A|W)?|"
    r"WinHttpOpen|WinHttpConnect|WinHttpSendRequest|WinHttpReceiveResponse|"
    r"URLDownloadToFile(?:A|W)?|curl_easy_perform|curl_multi_perform"
    r")\s*\(",
    re.IGNORECASE,
)
NETWORK_LIBRARY_LINK = re.compile(
    r"(?:-l\s*|/defaultlib:|target_link_libraries\s*\([^)]*|"
    r"#\s*pragma\s+comment\s*\(\s*lib\s*,\s*[\"'])"
    r"\b(ws2_32|wininet|winhttp|urlmon)(?:\.lib|\.dll)?\b",
    re.IGNORECASE,
)
DYNAMIC_NETWORK_LOAD = re.compile(
    r"\b(?:LoadLibrary(?:A|W)?|dlopen)\s*\(\s*[\"'][^\"']*"
    r"\b(ws2_32|wininet|winhttp|urlmon)(?:\.dll)?\b",
    re.IGNORECASE,
)
COMMENT = re.compile(r"/\*.*?\*/|//[^\n]*", re.DOTALL)


def source_files(root: Path):
    for directory in (root / "src", root / "apps"):
        if not directory.exists():
            continue
        for path in directory.rglob("*"):
            if path.is_file() and path.suffix in SOURCE_SUFFIXES:
                yield path


def inspect(path: Path):
    text = path.read_text(encoding="utf-8", errors="replace")
    code = COMMENT.sub(lambda match: "\n" * match.group(0).count("\n"), text)
    findings = []
    for pattern in (NETWORK_CALL, NETWORK_LIBRARY_LINK, DYNAMIC_NETWORK_LOAD):
        for match in pattern.finditer(code):
            line = code.count("\n", 0, match.start()) + 1
            token = match.group(0).strip()
            findings.append((line, token))
    return findings


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT, help="LoOS checkout root")
    args = parser.parse_args()
    root = args.root.resolve()
    files = sorted(source_files(root))
    findings = []
    for path in files:
        for line, token in inspect(path):
            findings.append((path.relative_to(root), line, token))

    if findings:
        for path, line, token in findings:
            print(f"{path}:{line}: forbidden network API or library: {token}", file=sys.stderr)
        print(f"privacy lint failed: {len(findings)} network references in {len(files)} source files", file=sys.stderr)
        return 1

    print(f"privacy lint passed: scanned {len(files)} source files; no direct network APIs or libraries found")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
