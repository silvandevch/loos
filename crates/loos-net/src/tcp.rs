//! TCP segment codec and connection state machine (RFC 793 with RFC 6298-style retransmit backoff).
//!
//! In-order delivery only: out-of-order segments are dropped and re-ACKed, the sender go-back-N
//! retransmits from `snd_una`. Fixed MSS, no options.

use crate::wire::{l4_checksum, P_TCP};
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::net::IpAddr;

/// FIN flag.
pub const FIN: u8 = 0x01;
/// SYN flag.
pub const SYN: u8 = 0x02;
/// RST flag.
pub const RST: u8 = 0x04;
/// PSH flag.
pub const PSH: u8 = 0x08;
/// ACK flag.
pub const ACK: u8 = 0x10;

/// Maximum segment payload (fits a 1500-byte MTU for both families).
pub const MSS: usize = 1400;
const RX_CAP: usize = 65535;
const TX_CAP: usize = 65536;
const RTO_INIT: u64 = 300;
const RTO_MAX: u64 = 8000;
const MAX_RETRIES: u32 = 8;
const TIME_WAIT_MS: u64 = 2000;

/// A TCP segment (header fields + payload).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seg {
    /// Source port.
    pub sport: u16,
    /// Destination port.
    pub dport: u16,
    /// Sequence number.
    pub seq: u32,
    /// Acknowledgement number.
    pub ack: u32,
    /// Flag bits.
    pub flags: u8,
    /// Advertised window.
    pub win: u16,
    /// Payload.
    pub data: Vec<u8>,
}

impl Seg {
    /// Sequence space consumed (payload + SYN + FIN).
    pub fn seq_len(&self) -> u32 {
        self.data.len() as u32 + u32::from(self.flags & SYN != 0) + u32::from(self.flags & FIN != 0)
    }
}

/// Serialises `s` with a valid checksum.
pub fn encode(src: IpAddr, dst: IpAddr, s: &Seg) -> Vec<u8> {
    let mut b = Vec::with_capacity(20 + s.data.len());
    b.extend_from_slice(&s.sport.to_be_bytes());
    b.extend_from_slice(&s.dport.to_be_bytes());
    b.extend_from_slice(&s.seq.to_be_bytes());
    b.extend_from_slice(&s.ack.to_be_bytes());
    b.extend_from_slice(&[0x50, s.flags]);
    b.extend_from_slice(&s.win.to_be_bytes());
    b.extend_from_slice(&[0, 0, 0, 0]);
    b.extend_from_slice(&s.data);
    let c = l4_checksum(src, dst, P_TCP, &b);
    b[16..18].copy_from_slice(&c.to_be_bytes());
    b
}

/// Parses and checksum-verifies a segment.
pub fn decode(src: IpAddr, dst: IpAddr, b: &[u8]) -> Option<Seg> {
    if b.len() < 20 {
        return None;
    }
    let off = usize::from(b[12] >> 4) * 4;
    if off < 20 || off > b.len() || l4_checksum(src, dst, P_TCP, b) != 0 {
        return None;
    }
    Some(Seg {
        sport: u16::from_be_bytes([b[0], b[1]]),
        dport: u16::from_be_bytes([b[2], b[3]]),
        seq: u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
        ack: u32::from_be_bytes([b[8], b[9], b[10], b[11]]),
        flags: b[13],
        win: u16::from_be_bytes([b[14], b[15]]),
        data: b[off..].to_vec(),
    })
}

fn lt(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) < 0
}
fn le(a: u32, b: u32) -> bool {
    a == b || lt(a, b)
}

/// Connection state (RFC 793 §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// No connection.
    Closed,
    /// Waiting for a SYN (listener).
    Listen,
    /// SYN sent.
    SynSent,
    /// SYN received, SYN+ACK sent.
    SynRcvd,
    /// Open.
    Established,
    /// We sent FIN.
    FinWait1,
    /// Our FIN acked.
    FinWait2,
    /// Peer sent FIN.
    CloseWait,
    /// Both FINs in flight.
    Closing,
    /// Waiting for the ACK of our FIN after CloseWait.
    LastAck,
    /// Waiting out stray segments.
    TimeWait,
}

/// Why a connection ended abnormally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fail {
    /// Peer sent RST.
    Reset,
    /// Retransmit budget exhausted.
    Timeout,
}

/// Transmission control block.
#[derive(Debug)]
pub struct Tcb {
    /// Current state.
    pub state: State,
    /// Local address and port.
    pub local: (IpAddr, u16),
    /// Remote address and port.
    pub remote: (IpAddr, u16),
    /// Set when the connection died abnormally.
    pub failed: Option<Fail>,
    iss: u32,
    snd_una: u32,
    snd_nxt: u32,
    snd_wnd: u32,
    rcv_nxt: u32,
    tx: VecDeque<u8>,
    rx: VecDeque<u8>,
    fin_queued: bool,
    fin_sent: bool,
    rto: u64,
    deadline: Option<u64>,
    retries: u32,
    tw_until: u64,
    out: Vec<Seg>,
}

impl Tcb {
    fn new(local: (IpAddr, u16), remote: (IpAddr, u16), iss: u32) -> Self {
        Tcb {
            state: State::Closed,
            local,
            remote,
            failed: None,
            iss,
            snd_una: iss,
            snd_nxt: iss,
            snd_wnd: 1,
            rcv_nxt: 0,
            tx: VecDeque::new(),
            rx: VecDeque::new(),
            fin_queued: false,
            fin_sent: false,
            rto: RTO_INIT,
            deadline: None,
            retries: 0,
            tw_until: 0,
            out: Vec::new(),
        }
    }

    /// A listening placeholder (never sends).
    pub fn listener(local: (IpAddr, u16)) -> Self {
        let mut t = Self::new(local, (local.0, 0), 0);
        t.state = State::Listen;
        t
    }

    /// Active open: emits a SYN.
    pub fn connect(local: (IpAddr, u16), remote: (IpAddr, u16), iss: u32, now: u64) -> Self {
        let mut t = Self::new(local, remote, iss);
        t.state = State::SynSent;
        t.snd_nxt = iss.wrapping_add(1);
        t.emit(SYN, iss, 0, Vec::new());
        t.deadline = Some(now + t.rto);
        t
    }

    /// Passive open from a received SYN: emits SYN+ACK.
    pub fn accept(local: (IpAddr, u16), remote: (IpAddr, u16), syn: &Seg, iss: u32, now: u64) -> Self {
        let mut t = Self::new(local, remote, iss);
        t.state = State::SynRcvd;
        t.rcv_nxt = syn.seq.wrapping_add(1);
        t.snd_wnd = u32::from(syn.win);
        t.snd_nxt = iss.wrapping_add(1);
        t.emit(SYN | ACK, iss, t.rcv_nxt, Vec::new());
        t.deadline = Some(now + t.rto);
        t
    }

    fn window(&self) -> u16 {
        (RX_CAP - self.rx.len()).min(0xffff) as u16
    }

    fn emit(&mut self, flags: u8, seq: u32, ack: u32, data: Vec<u8>) {
        let win = self.window();
        self.out.push(Seg { sport: self.local.1, dport: self.remote.1, seq, ack, flags, win, data });
    }

    fn emit_ack(&mut self) {
        let (s, a) = (self.snd_nxt, self.rcv_nxt);
        self.emit(ACK, s, a, Vec::new());
    }

    fn fail(&mut self, why: Fail) {
        self.state = State::Closed;
        self.failed = Some(why);
        self.deadline = None;
    }

    /// Queues application data; returns how many bytes were accepted.
    pub fn send(&mut self, data: &[u8]) -> usize {
        if self.fin_queued || !matches!(self.state, State::Established | State::CloseWait | State::SynSent | State::SynRcvd) {
            return 0;
        }
        let n = data.len().min(TX_CAP - self.tx.len());
        self.tx.extend(&data[..n]);
        n
    }

    /// Reads received data into `buf`.
    pub fn recv(&mut self, buf: &mut [u8]) -> usize {
        let before = self.window();
        let n = buf.len().min(self.rx.len());
        for (i, b) in self.rx.drain(..n).enumerate() {
            buf[i] = b;
        }
        if n > 0 && before < MSS as u16 && self.state != State::Closed {
            self.emit_ack(); // window update
        }
        n
    }

    /// Bytes waiting to be read.
    pub fn readable(&self) -> usize {
        self.rx.len()
    }

    /// True once the peer's FIN has been received and all data drained.
    pub fn eof(&self) -> bool {
        self.rx.is_empty() && matches!(self.state, State::CloseWait | State::LastAck | State::Closing | State::TimeWait | State::Closed)
    }

    /// Graceful close: FIN goes out after the queued data.
    pub fn close(&mut self) {
        match self.state {
            State::Established | State::CloseWait => self.fin_queued = true,
            State::SynSent | State::Listen => self.state = State::Closed,
            _ => {}
        }
    }

    /// Takes the segments produced since the last call.
    pub fn take_out(&mut self) -> Vec<Seg> {
        core::mem::take(&mut self.out)
    }

    fn data_inflight(&self) -> usize {
        let n = self.snd_nxt.wrapping_sub(self.snd_una) as usize;
        let n = if matches!(self.state, State::SynSent | State::SynRcvd) { 0 } else { n };
        if self.fin_sent { n.saturating_sub(1) } else { n }
    }

    /// Processes a segment addressed to this connection.
    pub fn on_segment(&mut self, s: &Seg, now: u64) {
        match self.state {
            State::Closed | State::Listen => return,
            State::SynSent => return self.on_syn_sent(s, now),
            _ => {}
        }
        // RFC 793 §3.9: trim to the receive window start.
        let mut data = s.data.as_slice();
        let mut seq = s.seq;
        let mut fin = s.flags & FIN != 0;
        if lt(seq, self.rcv_nxt) {
            let skip = self.rcv_nxt.wrapping_sub(seq) as usize;
            let syn = usize::from(s.flags & SYN != 0);
            if skip >= data.len() + syn + usize::from(fin) {
                if s.flags & RST == 0 {
                    self.emit_ack();
                }
                if s.flags & ACK != 0 {
                    self.on_ack(s, now);
                }
                return;
            }
            let skip_data = skip.saturating_sub(syn).min(data.len());
            data = &data[skip_data..];
            seq = self.rcv_nxt;
        } else if seq != self.rcv_nxt {
            if s.flags & RST == 0 {
                self.emit_ack();
            }
            return;
        }
        if s.flags & RST != 0 {
            self.fail(Fail::Reset);
            return;
        }
        if s.flags & SYN != 0 && self.state == State::SynRcvd {
            // Retransmitted SYN: our SYN+ACK was lost.
            let (i, a) = (self.iss, self.rcv_nxt);
            self.emit(SYN | ACK, i, a, Vec::new());
            return;
        }
        if s.flags & ACK == 0 {
            return;
        }
        self.on_ack(s, now);
        if self.state == State::Closed {
            return;
        }
        let mut need_ack = false;
        if !data.is_empty() && matches!(self.state, State::Established | State::FinWait1 | State::FinWait2) {
            let room = RX_CAP - self.rx.len();
            let take = data.len().min(room);
            self.rx.extend(&data[..take]);
            self.rcv_nxt = self.rcv_nxt.wrapping_add(take as u32);
            if take < data.len() {
                fin = false; // window overflow: peer must retransmit
            }
            need_ack = true;
        }
        let _ = seq;
        if fin && matches!(self.state, State::Established | State::FinWait1 | State::FinWait2) {
            self.rcv_nxt = self.rcv_nxt.wrapping_add(1);
            need_ack = true;
            self.state = match self.state {
                State::Established => State::CloseWait,
                State::FinWait1 => State::Closing,
                _ => {
                    self.tw_until = now + TIME_WAIT_MS;
                    State::TimeWait
                }
            };
        }
        if need_ack {
            self.emit_ack();
        }
    }

    fn on_syn_sent(&mut self, s: &Seg, now: u64) {
        if s.flags & ACK != 0 && s.ack != self.iss.wrapping_add(1) {
            return;
        }
        if s.flags & RST != 0 {
            if s.flags & ACK != 0 {
                self.fail(Fail::Reset);
            }
            return;
        }
        if s.flags & (SYN | ACK) == (SYN | ACK) {
            self.rcv_nxt = s.seq.wrapping_add(1);
            self.snd_una = s.ack;
            self.snd_wnd = u32::from(s.win);
            self.state = State::Established;
            self.retries = 0;
            self.rto = RTO_INIT;
            self.deadline = None;
            let _ = now;
            self.emit_ack();
        }
    }

    fn on_ack(&mut self, s: &Seg, now: u64) {
        if self.state == State::SynRcvd {
            if s.ack == self.iss.wrapping_add(1) {
                self.snd_una = s.ack;
                self.state = State::Established;
                self.retries = 0;
                self.rto = RTO_INIT;
                self.deadline = None;
            } else {
                return;
            }
        }
        let reopened = self.snd_wnd == 0 && s.win > 0;
        self.snd_wnd = u32::from(s.win);
        if reopened && s.ack == self.snd_una && self.snd_una != self.snd_nxt {
            self.resend_head(); // the zero-window probe was dropped by the receiver
        }
        if lt(self.snd_una, s.ack) && le(s.ack, self.snd_nxt) {
            let mut acked = s.ack.wrapping_sub(self.snd_una) as usize;
            let fin_acked = self.fin_sent && s.ack == self.snd_nxt;
            if fin_acked {
                acked -= 1;
            }
            self.tx.drain(..acked.min(self.tx.len()));
            self.snd_una = s.ack;
            self.retries = 0;
            self.rto = RTO_INIT;
            self.deadline = if self.snd_una == self.snd_nxt { None } else { Some(now + self.rto) };
            if fin_acked {
                self.state = match self.state {
                    State::FinWait1 => State::FinWait2,
                    State::Closing => {
                        self.tw_until = now + TIME_WAIT_MS;
                        State::TimeWait
                    }
                    State::LastAck => State::Closed,
                    st => st,
                };
            }
        }
    }

    /// Timer and transmit pump: sends new data/FIN, retransmits on timeout, expires TIME-WAIT.
    pub fn poll(&mut self, now: u64) {
        match self.state {
            State::TimeWait if now >= self.tw_until => self.state = State::Closed,
            State::Established | State::CloseWait | State::FinWait1 | State::LastAck | State::Closing => self.pump(now),
            _ => {}
        }
        if let Some(d) = self.deadline {
            if now >= d && self.snd_una != self.snd_nxt {
                self.retransmit(now);
            }
        }
    }

    fn pump(&mut self, now: u64) {
        if !self.fin_sent {
            loop {
                let sent = self.data_inflight();
                let unsent = self.tx.len().saturating_sub(sent);
                let wnd = (self.snd_wnd as usize).max(1);
                if unsent == 0 || sent >= wnd {
                    break;
                }
                let n = unsent.min(MSS).min(wnd - sent);
                let chunk: Vec<u8> = self.tx.iter().skip(sent).take(n).copied().collect();
                let (seq, ack) = (self.snd_nxt, self.rcv_nxt);
                self.emit(ACK | PSH, seq, ack, chunk);
                self.snd_nxt = self.snd_nxt.wrapping_add(n as u32);
                if self.deadline.is_none() {
                    self.deadline = Some(now + self.rto);
                }
            }
            if self.fin_queued && self.tx.len() <= self.data_inflight() {
                let (seq, ack) = (self.snd_nxt, self.rcv_nxt);
                self.emit(ACK | FIN, seq, ack, Vec::new());
                self.snd_nxt = self.snd_nxt.wrapping_add(1);
                self.fin_sent = true;
                self.state = if self.state == State::CloseWait { State::LastAck } else { State::FinWait1 };
                if self.deadline.is_none() {
                    self.deadline = Some(now + self.rto);
                }
            }
        }
    }

    fn retransmit(&mut self, now: u64) {
        self.retries += 1;
        if self.retries > MAX_RETRIES {
            self.fail(Fail::Timeout);
            return;
        }
        self.rto = (self.rto * 2).min(RTO_MAX);
        self.deadline = Some(now + self.rto);
        self.resend_head();
    }

    /// Retransmits the oldest unacknowledged segment (SYN, data or FIN).
    fn resend_head(&mut self) {
        let ack = self.rcv_nxt;
        match self.state {
            State::SynSent => self.emit(SYN, self.iss, 0, Vec::new()),
            State::SynRcvd => self.emit(SYN | ACK, self.iss, ack, Vec::new()),
            _ => {
                let inflight = self.data_inflight();
                if inflight > 0 {
                    let n = inflight.min(MSS);
                    let chunk: Vec<u8> = self.tx.iter().take(n).copied().collect();
                    let seq = self.snd_una;
                    self.emit(ACK | PSH, seq, ack, chunk);
                } else if self.fin_sent {
                    let seq = self.snd_nxt.wrapping_sub(1);
                    self.emit(ACK | FIN, seq, ack, Vec::new());
                }
            }
        }
    }
}
