//! kdump, tasksched, timesvc and kms.

use crate::{bytes, bytes_mut, cstr, put_str};
use std::ffi::{c_char, c_void};

// ------------------------------------------------------------------ kdump
#[repr(C)]
#[derive(Clone, Copy)]
pub struct KdumpContext {
    gpr: [u64; 16],
    rip: u64,
    rflags: u64,
    cr: [u64; 4],
    cs: u16,
    ss: u16,
}

/// Serialize a crash dump into `out`. Returns the dump size, or a negative value (-1 bad args, -2 buffer too small).
#[no_mangle]
pub unsafe extern "C" fn loos_kdump_write(
    out: *mut u8, cap: usize, code: u32, params: *const u64, time_ns: u64, cpu: u32, ncpus: u32, build_id: u64,
    ctx: *const KdumpContext, frames: *const u64, nframes: usize, log: *const u8, log_len: usize,
) -> i64 {
    use loos_kdump::format::{Context, DumpWriter, Header, SliceSink};
    if out.is_null() || params.is_null() {
        return -1;
    }
    // SAFETY: pointers valid per header contract.
    unsafe {
        let p = std::slice::from_raw_parts(params, 4);
        let h = Header { bugcheck: loos_kdump::BugCheck::new(code, [p[0], p[1], p[2], p[3]]), time_ns, cpu, ncpus, build_id };
        let mut w = match DumpWriter::begin(SliceSink::new(bytes_mut(out, cap)), &h) {
            Ok(w) => w,
            Err(_) => return -2,
        };
        let mut ok = true;
        if let Some(c) = ctx.as_ref() {
            ok &= w.context(&Context { gpr: c.gpr, rip: c.rip, rflags: c.rflags, cr: c.cr, cs: c.cs, ss: c.ss }).is_ok();
        }
        if !frames.is_null() && nframes > 0 {
            ok &= w.stack(std::slice::from_raw_parts(frames, nframes)).is_ok();
        }
        if log_len > 0 {
            ok &= w.log(bytes(log, log_len)).is_ok();
        }
        match (ok, w.finish()) {
            (true, Ok((_, total))) => total as i64,
            _ => -2,
        }
    }
}

/// Verify and render a dump as text. Returns the text length (may exceed `cap`), or -1 if the dump is invalid.
#[no_mangle]
pub unsafe extern "C" fn loos_kdump_render(dump: *const u8, len: usize, out: *mut u8, cap: usize) -> i64 {
    // SAFETY: see crate contract.
    unsafe {
        match loos_kdump::DumpReader::parse(bytes(dump, len)) {
            Ok(d) => put_str(&loos_kdump::report::render(&d), out, cap) as i64,
            Err(_) => -1,
        }
    }
}

/// Frame-pointer walk. `read_u64(opaque, addr, &value)` returns nonzero on success. Returns frames written.
#[no_mangle]
pub unsafe extern "C" fn loos_kdump_walk(
    rip: u64, rbp: u64, stack_lo: u64, stack_hi: u64, out: *mut u64, max: usize,
    read_u64: unsafe extern "C" fn(*mut c_void, u64, *mut u64) -> i32, opaque: *mut c_void,
) -> usize {
    let v = loos_kdump::stack::walk(rip, rbp, stack_lo, stack_hi, max, |a| {
        let mut x = 0u64;
        // SAFETY: the C hook is trusted to validate `a` and write `x`.
        (unsafe { read_u64(opaque, a, &mut x) } != 0).then_some(x)
    });
    // SAFETY: `out` has room for `max` entries.
    unsafe {
        for (i, f) in v.iter().enumerate().take(max) {
            *out.add(i) = *f;
        }
    }
    v.len().min(max)
}

// ------------------------------------------------------------------ tasksched
use loos_tasksched::{MissedPolicy, Scheduler, Trigger};

#[no_mangle]
pub extern "C" fn loos_sched_new() -> *mut Scheduler {
    Box::into_raw(Box::new(Scheduler::new()))
}
#[no_mangle]
pub unsafe extern "C" fn loos_sched_free(s: *mut Scheduler) {
    if !s.is_null() {
        // SAFETY: allocated by loos_sched_new.
        drop(unsafe { Box::from_raw(s) });
    }
}
/// kind: 0 once(a=at) 1 interval(a=anchor,b=secs) 2 daily(a=start_day,b=every_days,c=secs_of_day) 3 weekly(a=mask,c=secs)
/// 4 monthly(a=day,c=secs) 5 boot 6 logon. Returns the task id, or 0 on error (duplicate name / bad kind).
#[no_mangle]
pub unsafe extern "C" fn loos_sched_add(s: *mut Scheduler, name: *const c_char, command: *const c_char, kind: u32, a: i64, b: i64, c: i64, run_missed: bool, now: i64) -> u32 {
    // SAFETY: see crate contract.
    unsafe {
        let (Some(s), Some(name), Some(cmd)) = (s.as_mut(), cstr(name), cstr(command)) else { return 0 };
        let t = match kind {
            0 => Trigger::Once(a),
            1 => Trigger::Interval { anchor: a, secs: b as u32 },
            2 => Trigger::Daily { start_day: a, every_days: b as u32, secs_of_day: c as u32 },
            3 => Trigger::Weekly { mask: a as u8, secs_of_day: c as u32 },
            4 => Trigger::Monthly { day: a as u8, secs_of_day: c as u32 },
            5 => Trigger::Boot,
            6 => Trigger::Logon,
            _ => return 0,
        };
        s.add(name, cmd, vec![t], if run_missed { MissedPolicy::RunOnce } else { MissedPolicy::Skip }, now).unwrap_or(0)
    }
}
#[no_mangle]
pub unsafe extern "C" fn loos_sched_poll(s: *mut Scheduler, now: i64, out: *mut u32, cap: usize) -> usize {
    // SAFETY: see crate contract.
    unsafe {
        let Some(s) = s.as_mut() else { return 0 };
        let ids = s.poll(now);
        for (i, id) in ids.iter().enumerate().take(cap) {
            *out.add(i) = *id;
        }
        ids.len().min(cap)
    }
}
#[no_mangle]
pub unsafe extern "C" fn loos_sched_finished(s: *mut Scheduler, id: u32) {
    // SAFETY: see crate contract.
    if let Some(s) = unsafe { s.as_mut() } {
        s.finished(id);
    }
}
#[no_mangle]
pub unsafe extern "C" fn loos_sched_event(s: *mut Scheduler, logon: bool) {
    // SAFETY: see crate contract.
    if let Some(s) = unsafe { s.as_mut() } {
        if logon { s.notify_logon() } else { s.notify_boot() }
    }
}
#[no_mangle]
pub unsafe extern "C" fn loos_sched_remove(s: *mut Scheduler, id: u32) -> i32 {
    // SAFETY: see crate contract.
    unsafe { s.as_mut().map_or(-1, |s| if s.remove(id).is_ok() { 0 } else { -1 }) }
}
/// Next run time of a task, or -1.
#[no_mangle]
pub unsafe extern "C" fn loos_sched_next_run(s: *const Scheduler, id: u32) -> i64 {
    // SAFETY: see crate contract.
    unsafe { s.as_ref().and_then(|s| s.get(id)).and_then(|t| t.next_run).unwrap_or(-1) }
}
/// Copies the task's command line; returns its length or -1.
#[no_mangle]
pub unsafe extern "C" fn loos_sched_command(s: *const Scheduler, id: u32, out: *mut u8, cap: usize) -> i64 {
    // SAFETY: see crate contract.
    unsafe { s.as_ref().and_then(|s| s.get(id)).map_or(-1, |t| put_str(&t.command, out, cap) as i64) }
}

// ------------------------------------------------------------------ timesvc
use loos_timesvc::ntp;

/// Builds a 48-byte SNTP request. `t1_ns` = local unix time in ns. Writes the transmit timestamp to `*t1_ntp`.
#[no_mangle]
pub unsafe extern "C" fn loos_ntp_request(t1_ns: i64, out48: *mut u8, t1_ntp: *mut u64) {
    // SAFETY: `out48` has 48 bytes.
    unsafe {
        let ts = ntp::Ts::from_unix_ns(t1_ns as i128);
        bytes_mut(out48, 48).copy_from_slice(&ntp::request(ts));
        if !t1_ntp.is_null() {
            *t1_ntp = ts.0;
        }
    }
}
/// Validates a reply and computes offset/delay (ns). Returns 0, or a positive reject code:
/// 1 length 2 mode 3 version 4 unsynchronized 5 origin mismatch 6 no transmit timestamp.
#[no_mangle]
pub unsafe extern "C" fn loos_ntp_measure(reply: *const u8, len: usize, t1_ntp: u64, t1_ns: i64, t4_ns: i64, offset_ns: *mut i64, delay_ns: *mut i64, stratum: *mut u8) -> i32 {
    // SAFETY: see crate contract.
    unsafe {
        match ntp::parse_reply(bytes(reply, len), ntp::Ts(t1_ntp)) {
            Ok(r) => {
                let s = ntp::measure(&r, t1_ns as i128, t4_ns as i128);
                *offset_ns = s.offset_ns as i64;
                *delay_ns = s.delay_ns as i64;
                *stratum = s.stratum;
                0
            }
            Err(e) => 1 + e as i32,
        }
    }
}
/// 0 = leave the clock, 1 = slew (`*value` ppm), 2 = step (`*value` ns).
#[no_mangle]
pub unsafe extern "C" fn loos_ntp_discipline(offset_ns: i64, over_secs: i64, value: *mut i64) -> i32 {
    // SAFETY: `value` is writable.
    unsafe {
        match ntp::discipline(offset_ns as i128, over_secs as i128) {
            ntp::Action::None => 0,
            ntp::Action::Slew(p) => {
                *value = p as i64;
                1
            }
            ntp::Action::Step(n) => {
                *value = n as i64;
                2
            }
        }
    }
}
/// Local offset (seconds east of UTC) and DST flag for a POSIX TZ string at `unix`. 0 ok, -1 bad TZ.
#[no_mangle]
pub unsafe extern "C" fn loos_tz_offset(tz: *const c_char, unix: i64, offset: *mut i32, is_dst: *mut bool) -> i32 {
    // SAFETY: see crate contract.
    unsafe {
        match cstr(tz).and_then(loos_timesvc::tz::Zone::parse) {
            Some(z) => {
                let (o, d) = z.offset_at(unix);
                *offset = o;
                *is_dst = d;
                0
            }
            None => -1,
        }
    }
}
/// UTC broken-down time: out[0..7] = year, month, day, hour, min, sec, weekday.
#[no_mangle]
pub unsafe extern "C" fn loos_time_from_unix(unix: i64, out: *mut i64) {
    let d = loos_timesvc::calendar::from_unix(unix);
    // SAFETY: `out` has 7 entries.
    unsafe {
        for (i, v) in [d.year, d.month as i64, d.day as i64, d.hour as i64, d.min as i64, d.sec as i64, d.weekday as i64].iter().enumerate() {
            *out.add(i) = *v;
        }
    }
}

// ------------------------------------------------------------------ kms
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KmsMode {
    clock: u32,
    hdisplay: u16,
    hsync_start: u16,
    hsync_end: u16,
    htotal: u16,
    vdisplay: u16,
    vsync_start: u16,
    vsync_end: u16,
    vtotal: u16,
    flags: u32,
    preferred: u32,
}

fn to_c(m: &loos_kms::mode::Mode) -> KmsMode {
    KmsMode { clock: m.clock, hdisplay: m.hdisplay, hsync_start: m.hsync_start, hsync_end: m.hsync_end, htotal: m.htotal, vdisplay: m.vdisplay, vsync_start: m.vsync_start, vsync_end: m.vsync_end, vtotal: m.vtotal, flags: m.flags, preferred: m.preferred as u32 }
}
fn from_c(m: &KmsMode) -> loos_kms::mode::Mode {
    loos_kms::mode::Mode { clock: m.clock, hdisplay: m.hdisplay, hsync_start: m.hsync_start, hsync_end: m.hsync_end, htotal: m.htotal, vdisplay: m.vdisplay, vsync_start: m.vsync_start, vsync_end: m.vsync_end, vtotal: m.vtotal, flags: m.flags, preferred: m.preferred != 0 }
}

#[repr(C)]
pub struct KmsEdid {
    manufacturer: [u8; 4],
    product: u16,
    digital: u8,
    name: [u8; 16],
    width_mm: u32,
    height_mm: u32,
    nmodes: u32,
}

/// Parses an EDID block; returns 0 or a negative code (-1 short, -2 bad header, -3 bad checksum).
#[no_mangle]
pub unsafe extern "C" fn loos_edid_parse(data: *const u8, len: usize, out: *mut KmsEdid) -> i32 {
    use loos_kms::edid::{Edid, Error};
    // SAFETY: see crate contract.
    unsafe {
        match Edid::parse(bytes(data, len)) {
            Ok(e) => {
                let o = &mut *out;
                o.manufacturer = [e.manufacturer[0], e.manufacturer[1], e.manufacturer[2], 0];
                o.product = e.product;
                o.digital = e.digital as u8;
                o.name = [0; 16];
                let n = e.name.len().min(15);
                o.name[..n].copy_from_slice(&e.name.as_bytes()[..n]);
                o.width_mm = e.size_mm.0;
                o.height_mm = e.size_mm.1;
                o.nmodes = e.modes().len() as u32;
                0
            }
            Err(Error::Short) => -1,
            Err(Error::BadHeader) => -2,
            Err(Error::BadChecksum) => -3,
        }
    }
}
/// Mode `idx` of an EDID (preferred first). 0 ok, -1 invalid EDID or index out of range.
#[no_mangle]
pub unsafe extern "C" fn loos_edid_mode(data: *const u8, len: usize, idx: u32, out: *mut KmsMode) -> i32 {
    // SAFETY: see crate contract.
    unsafe {
        match loos_kms::edid::Edid::parse(bytes(data, len)).ok().and_then(|e| e.modes().get(idx as usize).copied()) {
            Some(m) => {
                *out = to_c(&m);
                0
            }
            None => -1,
        }
    }
}
#[no_mangle]
pub unsafe extern "C" fn loos_cvt_rb(w: u32, h: u32, hz: u32, out: *mut KmsMode) -> i32 {
    // SAFETY: `out` writable.
    unsafe {
        match loos_kms::mode::cvt_rb(w, h, hz) {
            Some(m) => {
                *out = to_c(&m);
                0
            }
            None => -1,
        }
    }
}
/// Picks a mode: `w == 0` means preferred. `max_hz` 0 = no cap. 0 ok, -1 none.
#[no_mangle]
pub unsafe extern "C" fn loos_kms_pick(modes: *const KmsMode, n: usize, w: u16, h: u16, max_hz: u32, out: *mut KmsMode) -> i32 {
    use loos_kms::mode::{pick, Request};
    // SAFETY: `modes` has n entries.
    unsafe {
        let v: Vec<_> = if modes.is_null() { vec![] } else { std::slice::from_raw_parts(modes, n).iter().map(from_c).collect() };
        let req = if w == 0 { Request::Preferred } else { Request::Size { w, h, max_hz } };
        match pick(&v, req) {
            Some(m) => {
                *out = to_c(&m);
                0
            }
            None => -1,
        }
    }
}
#[no_mangle]
pub extern "C" fn loos_kms_refresh_mhz(m: *const KmsMode) -> u32 {
    // SAFETY: caller passes a valid mode.
    unsafe { m.as_ref().map_or(0, |m| from_c(m).refresh_mhz()) }
}
/// Suggested UI scale percent for a physical DPI.
#[no_mangle]
pub extern "C" fn loos_ui_scale_for_dpi(dpi: u32) -> u32 {
    loos_kms::layout::suggest_scale(dpi)
}
