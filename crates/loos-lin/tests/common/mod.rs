#![allow(dead_code)]
use loos_lin::fs::MemFs;
use loos_lin::sys::*;
use loos_lin::*;

/// Build a static ET_EXEC ELF with one RWX PT_LOAD at 0x400000 containing `code` at entry.
pub fn elf_exec(code: &[u8], bss: u64) -> Vec<u8> {
    let mut b = vec![0u8; 64 + 56];
    b[..4].copy_from_slice(b"\x7fELF");
    b[4] = 2;
    b[5] = 1;
    b[6] = 1;
    b[16..18].copy_from_slice(&2u16.to_le_bytes());
    b[18..20].copy_from_slice(&62u16.to_le_bytes());
    b[20..24].copy_from_slice(&1u32.to_le_bytes());
    b[24..32].copy_from_slice(&0x400078u64.to_le_bytes());
    b[32..40].copy_from_slice(&64u64.to_le_bytes());
    b[52..54].copy_from_slice(&64u16.to_le_bytes());
    b[54..56].copy_from_slice(&56u16.to_le_bytes());
    b[56..58].copy_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(code);
    let total = b.len() as u64;
    let ph = 64;
    b[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes());
    b[ph + 4..ph + 8].copy_from_slice(&7u32.to_le_bytes());
    b[ph + 16..ph + 24].copy_from_slice(&0x400000u64.to_le_bytes());
    b[ph + 24..ph + 32].copy_from_slice(&0x400000u64.to_le_bytes());
    b[ph + 32..ph + 40].copy_from_slice(&total.to_le_bytes());
    b[ph + 40..ph + 48].copy_from_slice(&(total + bss).to_le_bytes());
    b[ph + 48..ph + 56].copy_from_slice(&0x1000u64.to_le_bytes());
    b
}

pub fn mov32(reg: u8, v: u32) -> Vec<u8> {
    let mut o = vec![0xB8 + reg];
    o.extend(v.to_le_bytes());
    o
}
pub fn lea_rip(reg_modrm: u8, target_off: i32, insn_end_off: i32) -> Vec<u8> {
    let mut o = vec![0x48, 0x8D, reg_modrm];
    o.extend((target_off - insn_end_off).to_le_bytes());
    o
}
pub const SYSCALL: [u8; 2] = [0x0F, 0x05];

/// Assemble: write(1,msg,len); exit_group(code).
pub fn echo_program(msg: &str, code: u32) -> Vec<u8> {
    let mut c = Vec::new();
    c.extend(mov32(0, 1)); // eax
    c.extend(mov32(7, 1)); // edi
    let lea_at = c.len();
    c.extend(lea_rip(0x35, 0, 0)); // placeholder
    c.extend(mov32(2, msg.len() as u32)); // edx
    c.extend(SYSCALL);
    c.extend(mov32(0, 231));
    c.extend(mov32(7, code));
    c.extend(SYSCALL);
    let msg_off = c.len() as i32;
    let fix = lea_rip(0x35, msg_off, (lea_at + 7) as i32);
    c[lea_at..lea_at + 7].copy_from_slice(&fix);
    c.extend(msg.as_bytes());
    c
}

/// Execute machine code of the supported subset until the process exits. Returns the exit status.
pub fn run<F: loos_lin::fs::LinFs>(sys: &mut LinSystem<F>, tid: u32) -> Sys {
    for _ in 0..10_000 {
        let (rip, regs) = {
            let t = sys.thread(tid).unwrap();
            (t.regs.rip, t.regs)
        };
        let pid = sys.thread(tid).unwrap().pid;
        let mut op = [0u8; 8];
        sys.process_mut(pid).unwrap().mem.read(rip, &mut op).unwrap();
        let mut r = regs;
        match op[0] {
            b @ 0xB8..=0xBF => {
                let v = u32::from_le_bytes(op[1..5].try_into().unwrap()) as u64;
                match b - 0xB8 {
                    0 => r.rax = v,
                    1 => r.rcx = v,
                    2 => r.rdx = v,
                    6 => r.rsi = v,
                    7 => r.rdi = v,
                    x => panic!("reg {x}"),
                }
                r.rip += 5;
            }
            0x48 if op[1] == 0x8D => {
                let d = i32::from_le_bytes(op[3..7].try_into().unwrap()) as i64;
                let v = (rip as i64 + 7 + d) as u64;
                match op[2] {
                    0x35 => r.rsi = v,
                    0x3D => r.rdi = v,
                    0x15 => r.rdx = v,
                    x => panic!("lea {x:x}"),
                }
                r.rip += 7;
            }
            0x31 if op[1] == 0xFF => {
                r.rdi = 0;
                r.rip += 2;
            }
            0x0F if op[1] == 0x05 => {
                sys.thread_mut(tid).unwrap().regs.rip += 2;
                let a = [r.rdi, r.rsi, r.rdx, r.r10, r.r8, r.r9];
                match sys.syscall(tid, r.rax, a) {
                    Sys::Ret(_) | Sys::Exec | Sys::Restored => continue,
                    other => return other,
                }
            }
            x => panic!("unsupported opcode {x:#x} at {rip:#x}"),
        }
        sys.thread_mut(tid).unwrap().regs = r;
    }
    panic!("runaway program");
}

pub fn sh(s: &str) -> String {
    s.to_string()
}

pub fn new_sys() -> LinSystem<MemFs> {
    let mut fs = MemFs::new();
    for d in ["/bin", "/usr/bin", "/lib", "/lib64", "/etc", "/tmp", "/dev", "/proc", "/home"] {
        fs.add_dir(d);
    }
    LinSystem::new(fs)
}
