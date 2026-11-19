//! Diagnostics for the ordinary CPU and translated span execution paths.
//! Each CPU construction or reset starts a fresh counter epoch.
use crate::cpu::{CpuFault, Step};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct CacheCounts {
    /// Includes cached unsupported results.
    pub hits: u64,
    /// Every miss calls the native compiler once.
    pub misses: u64,
    /// Successful compilations.
    pub compiled: u64,
    /// Entries dropped on capacity eviction, including negative entries.
    pub evicted_entries: u64,
    /// Elapsed host time in compilation, including failed attempts.
    pub compile_ns: u64,
}

impl CacheCounts {
    fn since(self, old: Self) -> Option<Self> {
        Some(Self {
            hits: self.hits.checked_sub(old.hits)?,
            misses: self.misses.checked_sub(old.misses)?,
            compiled: self.compiled.checked_sub(old.compiled)?,
            evicted_entries: self.evicted_entries.checked_sub(old.evicted_entries)?,
            compile_ns: self.compile_ns.checked_sub(old.compile_ns)?,
        })
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct PhaseCounts {
    /// Index zero counts one-instruction native calls.
    pub calls_by_length: [u64; helios_jit_native::MAX_BLOCK_INSNS],
    /// Instructions retired by the interpreter in either execution mode.
    pub interpreted: u64,
    /// Interpreted retirements with native mode enabled.
    pub fallbacks: u64,
    /// Subsets of fallbacks: branches/CALL/JMPL/RETT, and format-3 memory.
    pub control_fallbacks: u64,
    pub memory_fallbacks: u64,
    pub traps: u64,
    pub interrupts: u64,
    /// CPU error returns, including repeated attempts in error state.
    pub cpu_errors: u64,
    pub words: CacheCounts,
    pub spans: CacheCounts,
}

impl PhaseCounts {
    pub fn native_instructions(&self) -> u64 {
        self.calls_by_length
            .iter()
            .enumerate()
            .map(|(i, n)| (i as u64 + 1) * n)
            .sum()
    }

    pub(crate) fn retirement(&mut self, word: u32, native_enabled: bool) {
        self.interpreted += 1;
        if native_enabled {
            self.fallbacks += 1;
            match word >> 30 {
                0 if matches!(word >> 22 & 7, 2 | 6 | 7) => self.control_fallbacks += 1,
                1 => self.control_fallbacks += 1,
                2 if matches!(word >> 19 & 63, 0x38 | 0x39) => self.control_fallbacks += 1,
                3 => self.memory_fallbacks += 1,
                _ => {}
            }
        }
    }

    pub(crate) fn boundary(&mut self, result: Result<Step, CpuFault>) {
        match result {
            Ok(Step::TrapTaken { .. }) => self.traps += 1,
            Ok(Step::InterruptTaken { .. }) => self.interrupts += 1,
            Err(_) => self.cpu_errors += 1,
            _ => {}
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhaseSnapshot {
    pub epoch: u64,
    pub counts: PhaseCounts,
}
impl PhaseSnapshot {
    pub(crate) fn fresh() -> Self {
        Self {
            epoch: NEXT_EPOCH.fetch_add(1, Ordering::Relaxed),
            counts: PhaseCounts::default(),
        }
    }

    /// Rejects different CPU lifetimes and counter decreases.
    pub fn since(&self, earlier: &Self) -> Option<PhaseCounts> {
        if self.epoch != earlier.epoch {
            return None;
        }
        let new = self.counts;
        let old = earlier.counts;
        let mut calls_by_length = new.calls_by_length;
        for (calls, prev) in calls_by_length.iter_mut().zip(old.calls_by_length) {
            *calls = calls.checked_sub(prev)?;
        }
        Some(PhaseCounts {
            calls_by_length,
            interpreted: new.interpreted.checked_sub(old.interpreted)?,
            fallbacks: new.fallbacks.checked_sub(old.fallbacks)?,
            control_fallbacks: new.control_fallbacks.checked_sub(old.control_fallbacks)?,
            memory_fallbacks: new.memory_fallbacks.checked_sub(old.memory_fallbacks)?,
            traps: new.traps.checked_sub(old.traps)?,
            interrupts: new.interrupts.checked_sub(old.interrupts)?,
            cpu_errors: new.cpu_errors.checked_sub(old.cpu_errors)?,
            words: new.words.since(old.words)?,
            spans: new.spans.since(old.spans)?,
        })
    }
}
