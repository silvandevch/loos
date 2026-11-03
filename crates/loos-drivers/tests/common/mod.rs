#![allow(dead_code)]
use loos_drivers::hw::DmaArena;
use std::cell::RefCell;
use std::rc::Rc;

/// Bus memory shared between the driver under test and the device model.
#[derive(Clone)]
pub struct Mem {
    pub base: u64,
    pub data: Rc<RefCell<Vec<u8>>>,
}

impl Mem {
    pub fn new(base: u64, len: usize) -> Mem {
        Mem { base, data: Rc::new(RefCell::new(vec![0; len])) }
    }
    pub fn rd(&self, phys: u64, n: usize) -> Vec<u8> {
        let o = (phys - self.base) as usize;
        self.data.borrow()[o..o + n].to_vec()
    }
    pub fn wr(&self, phys: u64, d: &[u8]) {
        let o = (phys - self.base) as usize;
        self.data.borrow_mut()[o..o + d.len()].copy_from_slice(d);
    }
    pub fn r32(&self, phys: u64) -> u32 {
        u32::from_le_bytes(self.rd(phys, 4).try_into().unwrap())
    }
    pub fn r64(&self, phys: u64) -> u64 {
        u64::from_le_bytes(self.rd(phys, 8).try_into().unwrap())
    }
    pub fn w32(&self, phys: u64, v: u32) {
        self.wr(phys, &v.to_le_bytes())
    }
    pub fn w64(&self, phys: u64, v: u64) {
        self.wr(phys, &v.to_le_bytes())
    }
}

impl DmaArena for Mem {
    fn len(&self) -> usize {
        self.data.borrow().len()
    }
    fn phys(&self, off: usize) -> u64 {
        self.base + off as u64
    }
    fn read(&self, off: usize, buf: &mut [u8]) {
        buf.copy_from_slice(&self.data.borrow()[off..off + buf.len()]);
    }
    fn write(&mut self, off: usize, d: &[u8]) {
        self.data.borrow_mut()[off..off + d.len()].copy_from_slice(d);
    }
}

pub fn pattern(n: usize, seed: u8) -> Vec<u8> {
    (0..n).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed).wrapping_add((i >> 8) as u8)).collect()
}
