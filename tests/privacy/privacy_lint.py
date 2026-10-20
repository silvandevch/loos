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
# Any literal web address with a host is a potential egress point. Placeholders in UI hints and loopback are allowed.
EXTERNAL_URL = re.compile(r"""[\"'](?:L)?[\"']?\s*(?:https?|ftp|wss?)://([A-Za-z0-9][A-Za-z0-9.\-]*)""", re.IGNORECASE)
URL_ALLOWED_HOSTS = ("mirror.example", "example.com", "example.org", "localhost", "127.0.0.1")


def source_files(root: Path):
    for directory in (root / "src", root / "apps"):
        if not directory.exists():
            continue
        for path in directory.rglob("*"):
            if path.is_file() and path.suffix in SOURCE_SUFFIXES:
                yield path


def split_code(text: str):
    """Return (code_with_strings, code_without_string_contents), comments blanked in both. A '//' inside a string
    (every URL) is not a comment, so this walks the text instead of using a regex."""
    keep, blank = [], []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if c == "/" and i + 1 < n and text[i + 1] == "*":
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            seg = "\n" * text.count("\n", i, j)
            keep.append(seg); blank.append(seg); i = j
        elif c == "/" and i + 1 < n and text[i + 1] == "/":
            j = text.find("\n", i)
            j = n if j < 0 else j
            i = j
        elif c in "\"'":
            j = i + 1
            while j < n and text[j] != c and text[j] != "\n":
                j += 2 if text[j] == "\\" else 1
            j = min(j + 1, n)
            keep.append(text[i:j])
            blank.append(c + " " * max(0, j - i - 2) + (c if j - i >= 2 else ""))
            i = j
        else:
            keep.append(c); blank.append(c); i += 1
    return "".join(keep), "".join(blank)


def inspect(path: Path):
    text = path.read_text(encoding="utf-8", errors="replace")
    with_strings, code = split_code(text)
    findings = []
    for match in re.finditer(r"(?:https?|ftp|wss?)://([A-Za-z0-9][A-Za-z0-9.\-]*)", with_strings, re.IGNORECASE):
        host = match.group(1).lower()
        if host.endswith(URL_ALLOWED_HOSTS):
            continue
        line = with_strings.count("\n", 0, match.start()) + 1
        findings.append((line, "external URL " + match.group(0)))
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
