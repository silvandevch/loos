//! COM objects implemented by the runtime and handed to the guest (DirectWrite today, WinRT/combase next).
//!
//! A guest object is 16 bytes on the process heap: its vtable pointer, then the handle (index + 1) into
//! [`Process::com`]. Every vtable slot is a runtime stub whose syscall number encodes `(interface, slot)`
//! ([`COM_BASE`] + interface * 64 + slot), so a method call lands in [`Process::com_call`] with `this` in the first
//! argument register. `IUnknown` (slots 0-2) is handled here; the rest goes to the interface's implementation.

use crate::{Process, Ret};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// First syscall number of COM methods (up to [`crate::api::UNRESOLVED_BASE`]).
pub const COM_BASE: u32 = 0x4000;
const SLOTS: u32 = 64;

pub(crate) const S_OK: u64 = 0;
pub(crate) const E_NOTIMPL: u64 = 0x8000_4001;
pub(crate) const E_NOINTERFACE: u64 = 0x8000_4002;
pub(crate) const E_POINTER: u64 = 0x8000_4003;
pub(crate) const E_INVALIDARG: u64 = 0x8007_0057;
pub(crate) const E_NOT_SUFFICIENT_BUFFER: u64 = 0x8007_007A;

/// The `IUnknown` IID.
pub(crate) const IID_IUNKNOWN: [u8; 16] = guid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);

/// GUID bytes as they sit in memory.
pub(crate) const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> [u8; 16] {
    let a = d1.to_le_bytes();
    let b = d2.to_le_bytes();
    let c = d3.to_le_bytes();
    [a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d4[0], d4[1], d4[2], d4[3], d4[4], d4[5], d4[6], d4[7]]
}

/// One live object.
pub(crate) struct ComObj<K> {
    pub refs: u32,
    /// Guest address (`this`).
    pub ptr: u64,
    pub kind: K,
}

/// Object table and vtables of one object family.
pub(crate) struct ComTable<K> {
    pub objs: Vec<Option<ComObj<K>>>,
    vtbls: BTreeMap<u16, u64>,
}

impl<K> Default for ComTable<K> {
    fn default() -> Self {
        ComTable { objs: Vec::new(), vtbls: BTreeMap::new() }
    }
}

/// Syscall number of `slot` of interface `iface`.
pub(crate) fn method_nr(iface: u16, slot: u32) -> u32 {
    COM_BASE + u32::from(iface) * SLOTS + slot
}

/// `(interface, slot)` of a COM method syscall.
pub(crate) fn decode(nr: u32) -> Option<(u16, u32)> {
    (COM_BASE..crate::api::UNRESOLVED_BASE).contains(&nr).then(|| (((nr - COM_BASE) / SLOTS) as u16, (nr - COM_BASE) % SLOTS))
}

impl Process {
    /// Guest vtable for `iface` with `slots` entries (built once).
    fn com_vtable<K>(&mut self, t: fn(&mut Process) -> &mut ComTable<K>, iface: u16, slots: u32) -> u64 {
        if let Some(&v) = t(self).vtbls.get(&iface) {
            return v;
        }
        let v = self.heap_alloc(u64::from(slots) * 8);
        for s in 0..slots {
            let stub = self.dyn_stub(method_nr(iface, s));
            let _ = self.mem.write(v + u64::from(s) * 8, &stub.to_le_bytes());
        }
        t(self).vtbls.insert(iface, v);
        v
    }

    /// Creates an object with one reference and returns `this`.
    pub(crate) fn com_new<K>(&mut self, t: fn(&mut Process) -> &mut ComTable<K>, iface: u16, slots: u32, kind: K) -> u64 {
        let vt = self.com_vtable(t, iface, slots);
        let ptr = self.heap_alloc(16);
        let tab = t(self);
        let idx = tab.objs.iter().position(Option::is_none).unwrap_or(tab.objs.len());
        let obj = ComObj { refs: 1, ptr, kind };
        if idx == tab.objs.len() {
            tab.objs.push(Some(obj));
        } else {
            tab.objs[idx] = Some(obj);
        }
        let _ = self.mem.write(ptr, &vt.to_le_bytes());
        let _ = self.mem.write(ptr + 8, &(idx as u64 + 1).to_le_bytes());
        ptr
    }

    /// Index of the object at guest address `this` (checked against the table).
    pub(crate) fn com_index<K>(&mut self, t: fn(&mut Process) -> &mut ComTable<K>, this: u64) -> Option<usize> {
        let i = self.mem.read_u64(this.checked_add(8)?).ok()?.checked_sub(1)? as usize;
        t(self).objs.get(i)?.as_ref().is_some_and(|o| o.ptr == this).then_some(i)
    }

    /// `IUnknown` for an object of the table: `accepts` lists the IIDs `QueryInterface` answers with itself.
    pub(crate) fn com_unknown<K>(&mut self, t: fn(&mut Process) -> &mut ComTable<K>, i: usize, slot: u32, a: [u64; 4], accepts: &[[u8; 16]]) -> Ret {
        fn obj<K>(p: &mut Process, t: fn(&mut Process) -> &mut ComTable<K>, i: usize) -> &mut ComObj<K> {
            t(p).objs[i].as_mut().expect("checked by com_index")
        }
        match slot {
            0 => {
                if a[2] == 0 {
                    return Ret::Value(E_POINTER);
                }
                let iid: [u8; 16] = self.mem.read_vec(a[1], 16).ok().and_then(|v| v.try_into().ok()).unwrap_or([0; 16]);
                if iid == IID_IUNKNOWN || accepts.contains(&iid) {
                    obj(self, t, i).refs += 1;
                    let _ = self.mem.write(a[2], &a[0].to_le_bytes());
                    Ret::Value(S_OK)
                } else {
                    let _ = self.mem.write(a[2], &0u64.to_le_bytes());
                    Ret::Value(E_NOINTERFACE)
                }
            }
            1 => {
                let o = obj(self, t, i);
                o.refs += 1;
                Ret::Value(u64::from(o.refs))
            }
            _ => {
                let o = obj(self, t, i);
                o.refs = o.refs.saturating_sub(1);
                let left = o.refs;
                if left == 0 {
                    t(self).objs[i] = None; // the 16 guest bytes stay on the heap arena
                }
                Ret::Value(u64::from(left))
            }
        }
    }

    /// Writes an interface pointer to an out-parameter (`IFoo**`).
    pub(crate) fn put_ptr(&mut self, out: u64, v: u64) -> bool {
        out != 0 && self.mem.write(out, &v.to_le_bytes()).is_ok()
    }

    /// `FLOAT` argument `i` (0-based) of a COM/Win32 call: XMM registers for the first four, else the stack slot.
    pub(crate) fn f32_arg(&mut self, rsp: u64, i: usize) -> f32 {
        if i < 4 {
            f32::from_bits(u32::from_le_bytes(self.xmm[i][0..4].try_into().unwrap()))
        } else {
            f32::from_bits(self.stack_arg(rsp, i as u64 + 1) as u32)
        }
    }

    /// Dispatches a COM method syscall to the family that owns the interface.
    pub(crate) fn com_call(&mut self, nr: u32, a: [u64; 4], rsp: u64) -> Option<Ret> {
        let (iface, slot) = decode(nr)?;
        if iface < crate::dwrite::IFACES {
            return Some(self.dwrite_call(iface, slot, a, rsp));
        }
        None
    }
}
