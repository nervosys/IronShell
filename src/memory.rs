//! A process-wide memory budget.
//!
//! The `ish` binary installs [`CountingAlloc`] as its global allocator. It
//! wraps the system allocator and keeps a count of live heap bytes, so the
//! evaluator can ask, cheaply, whether the program has outgrown its budget
//! ([`check`]) and answer `E_BUDGET_EXCEEDED` instead of the process being
//! killed by a failed allocation.
//!
//! A library user (or a test binary) that does not install the allocator sees
//! a count of zero, so [`check`] never fires there.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Live heap bytes, as counted by [`CountingAlloc`].
static LIVE: AtomicUsize = AtomicUsize::new(0);

/// The budget in bytes. `usize::MAX` means no limit has been set.
static LIMIT: AtomicUsize = AtomicUsize::new(usize::MAX);

/// The system allocator, counting live bytes.
pub struct CountingAlloc;

// SAFETY: every call is forwarded unchanged to `System`; the counter only
// observes sizes and never touches the memory.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = System.realloc(ptr, layout, new_size);
        if !p.is_null() {
            if new_size >= layout.size() {
                LIVE.fetch_add(new_size - layout.size(), Ordering::Relaxed);
            } else {
                LIVE.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        p
    }
}

/// Live heap bytes (zero when [`CountingAlloc`] is not installed).
pub fn live_bytes() -> usize {
    LIVE.load(Ordering::Relaxed)
}

/// Set the budget, in megabytes. 0 removes the limit.
pub fn set_limit_mb(mb: usize) {
    let bytes = if mb == 0 {
        usize::MAX
    } else {
        mb.saturating_mul(1024 * 1024)
    };
    LIMIT.store(bytes, Ordering::Relaxed);
}

/// The budget in megabytes, or `None` when there is no limit.
pub fn limit_mb() -> Option<usize> {
    match LIMIT.load(Ordering::Relaxed) {
        usize::MAX => None,
        bytes => Some(bytes / (1024 * 1024)),
    }
}

/// The default budget in megabytes: the smaller of 2048 and half the physical
/// memory (2048 when that cannot be read).
pub fn default_limit_mb() -> usize {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let total_mb = (sys.total_memory() / (1024 * 1024)) as usize;
    if total_mb == 0 {
        2048
    } else {
        (total_mb / 2).clamp(64, 2048)
    }
}

/// Set the budget from `IRONSHELL_MEMORY_LIMIT_MB` if it is a number (0 means
/// no limit), else to the default. Returns the megabytes set, 0 for none.
pub fn init_from_env() -> usize {
    let mb = std::env::var("IRONSHELL_MEMORY_LIMIT_MB")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or_else(default_limit_mb);
    set_limit_mb(mb);
    mb
}

/// Fail with `E_BUDGET_EXCEEDED` when live bytes are over the budget.
///
/// Two relaxed atomic loads and a compare; called at every evaluator step.
#[inline]
pub fn check() -> anyhow::Result<()> {
    let limit = LIMIT.load(Ordering::Relaxed);
    if LIVE.load(Ordering::Relaxed) > limit {
        return Err(over(limit));
    }
    Ok(())
}

#[cold]
fn over(limit: usize) -> anyhow::Error {
    crate::safety::budget_exceeded(
        "eval",
        format!("memory budget of {} MB exceeded", limit / (1024 * 1024)),
        "work on the data in smaller pieces, or raise the budget with --memory-limit MB or IRONSHELL_MEMORY_LIMIT_MB",
    )
}
