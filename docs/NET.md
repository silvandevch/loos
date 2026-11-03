# Network stack (`crates/loos-net`)

Safe `no_std` Rust TCP/IP stack, written from the RFCs. No I/O of its own: NIC drivers
(`loos-drivers` e1000 / virtio-net) hand frames to `Stack::rx` and drain `Stack::pop_tx`; time is
fed to `Stack::poll(ms)`.

| Layer | Support |
|---|---|
| Link | Ethernet II, ARP (RFC 826), IPv6 NDP NS/NA (RFC 4861) |
| IP | IPv4 + IPv6, header/checksum validation, source selection, longest-prefix routing with gateways and metrics, loopback (127/8, ::1) |
| ICMP | v4/v6 echo, v4 port-unreachable |
| UDP | bind / send / recv, checksums (mandatory on v6) |
| TCP | full state machine, passive/active open, backlog, RST on closed port, go-back-N retransmit with exponential RTO, flow control + zero-window recovery, TIME-WAIT, fixed MSS 1400 |

Not implemented (by design, v1): IP forwarding, fragmentation/reassembly, IPv6 extension headers,
TCP options (SACK, window scaling, timestamps), congestion control beyond the retransmit backoff,
DHCPv6/SLAAC. DHCPv4/DNS/firewall live in `src/net/` (C) and sit on top of the UDP API.

Tests: `cargo test -p loos-net` — loopback TCP echo (v4+v6, 300 kB), two hosts on a simulated wire
with ARP/NDP, 1-in-7 frame loss transfer, routing, ICMP/UDP.
