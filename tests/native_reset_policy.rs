#![cfg(feature = "jit-native")]
use helios::{
    bus::Bus,
    cpu::Step,
    machine::{Machine, MachineProfile},
};
use std::num::NonZeroUsize;

fn check(reset: fn(&mut Machine)) {
    for interpret in [false, true] {
        let p = MachineProfile::Ss20;
        let mut m = Machine::with_profile(p, 16 << 20, &[1, 0, 0, 0], p.default_nvram()).unwrap();
        m.bus.write32(0x4000, 0x82006001).unwrap();
        m.set_native_execution_enabled(true);
        m.set_interpret_span_fallback(interpret);
        for after in [false, true] {
            if after {
                reset(&mut m);
            }
            assert!(m.set_processor_entry(0, 0x4000, 0x4004));
            let calls = m.native_execution_calls();
            let out = m.step_native_span(NonZeroUsize::MIN).unwrap();
            assert!(matches!(
                out.get(0),
                Some(Step::Executed {
                    instruction: 0x82006001,
                    ..
                })
            ));
            assert_eq!(
                m.native_execution_calls() - calls,
                u64::from(!interpret),
                "after reset={after}, requested interpreted fallback={interpret}"
            );
        }
    }
}
#[test]
fn power_on_retains_fallback() {
    check(Machine::reset);
}
#[test]
fn module_reset_retains_fallback() {
    check(|m| assert!(m.reset_processor_module(0)));
}
#[test]
fn software_reset_retains_fallback() {
    check(|m| {
        m.bus.write32(m.bus.layout().reset, 1).unwrap();
        assert!(matches!(m.step().unwrap(), Step::SoftwareReset { .. }));
    });
}
