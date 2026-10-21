#![allow(missing_docs)]
use loos_khal::{apic::*, gdt::*, idt::*, msr};

#[test]
fn gdt_descriptors_match_architecture_values() {
    assert_eq!(code(0), 0x00af_9a00_0000_ffff);
    assert_eq!(data(0), 0x00cf_9200_0000_ffff);
    assert_eq!(code(3), 0x00af_fa00_0000_ffff);
    assert_eq!(data(3), 0x00cf_f200_0000_ffff);
    assert_eq!(sel::USER_CS, 0x23);
    assert_eq!(sel::USER_DS, 0x1b);
    assert_eq!(core::mem::size_of::<Tss>(), 104);
    let t = Tss::new();
    assert_eq!({ t.iomap_base }, 104);
}

#[test]
fn tss_descriptor_splits_base() {
    let t = tss_descriptor(0xffff_8000_1234_5678, 103);
    assert_eq!(t[0] & 0xffff, 103);
    assert_eq!((t[0] >> 16) & 0xffff, 0x5678);
    assert_eq!((t[0] >> 32) & 0xff, 0x34);
    assert_eq!((t[0] >> 40) & 0xff, 0x89);
    assert_eq!((t[0] >> 56) & 0xff, 0x12);
    assert_eq!(t[1], 0xffff_8000);
    let g = Gdt::new(0x1000);
    assert_eq!(g.entries[1], code(0));
    assert_eq!(g.entries[5] >> 40 & 0xff, 0x89);
}

#[test]
fn star_layout_supports_sysret() {
    // kernel CS 0x08, user data 0x1b -> sysret base 0x10 (CS = base+16 = 0x20|3, SS = base+8 = 0x18|3)
    let s = msr::star(sel::KERNEL_CS, 0x10);
    assert_eq!(s >> 32 & 0xffff, 0x08);
    assert_eq!(s >> 48 & 0xffff, 0x10);
    assert_eq!((s >> 48) as u16 + 16 | 3, sel::USER_CS);
    assert_eq!((s >> 48) as u16 + 8 | 3, sel::USER_DS);
}

#[test]
fn idt_gate_roundtrip() {
    let g = Gate::new(0xffff_8000_dead_beef, sel::KERNEL_CS, GateType::Interrupt, 3, 2);
    assert_eq!(g.handler(), 0xffff_8000_dead_beef);
    assert!(g.present() && g.dpl() == 3 && g.ist() == 2);
    assert_eq!(core::mem::size_of::<Gate>(), 16);
    assert!(!Gate::MISSING.present());
    let mut t = Idt::new();
    assert!(!t.get(33).present());
    t.set(33, g);
    assert_eq!(t.get(33), g);
    assert_eq!(Idt::limit(), 4095);
    assert!(has_error_code(14) && has_error_code(8) && !has_error_code(3) && !has_error_code(32));
    assert_eq!(core::mem::size_of::<TrapFrame>(), 22 * 8);
}

#[test]
fn apic_maths() {
    assert_eq!(x2apic_msr(reg::EOI), 0x80b);
    assert_eq!(x2apic_msr(reg::ICR_LO), 0x830);
    assert_eq!(svr_enable(0xff), 0x1ff);
    assert_eq!(lvt_timer(0xec, TimerMode::Periodic, false), 0xec | 1 << 17);
    assert_eq!(lvt_timer(0xec, TimerMode::OneShot, true), 0xec | 1 << 16);
    assert_eq!(divide_config(16), Some(3));
    assert_eq!(divide_config(3), None);
    assert_eq!(calibrate_bus_hz(1_000_000, 16, 10), 1_600_000_000);
    assert_eq!(timer_count(1_600_000_000, 16, 1000), Some(100_000));
    assert_eq!(timer_count(100, 16, 1000), None);
    assert_eq!(timer_count(1, 0, 1), None);
}

#[test]
fn ipi_encoding() {
    let v = icr(0xfd, Delivery::Fixed, Dest::Id(5), true);
    assert_eq!(v, 0xfd | 1 << 14 | 5 << 32);
    assert_eq!(icr_xapic(v), (5 << 24, 0xfd | 1 << 14));
    let init = icr(0, Delivery::Init, Dest::AllButSelf, true);
    assert_eq!(init >> 18 & 3, 3);
    assert_eq!(init >> 8 & 7, 5);
    let sipi = icr(0x08, Delivery::Startup, Dest::Id(1), true);
    assert_eq!(sipi >> 8 & 7, 6);
    assert_eq!(msr::apic_phys(0xfee0_0900), 0xfee0_0000);
}
