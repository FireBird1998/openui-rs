//! Private dev dependency: unsafe allocator plumbing is allowed ONLY here in
//! test-only code, never in the published library crates.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::io::Write;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

// Process-wide tracked payloads/headers, including cross-thread frees. Account
// metadata and stacker's mapped segments remain outside this measurement.
static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);

/// Current tracked heap bytes across all threads. Use an isolated test binary
/// when comparing samples, since other tests would contribute to this count.
pub fn live_bytes() -> usize {
    LIVE_BYTES.load(Relaxed)
}

pub const DEFAULT: usize = 256 * 1024 * 1024;
pub const LIMITS: usize = 64 * 1024 * 1024;

struct Account {
    cap: usize,
    live: AtomicUsize,
    peak: AtomicUsize,
    measured_peak: AtomicUsize,
    allocation_calls: AtomicUsize,
    allocated_bytes: AtomicUsize,
}

struct ThreadAccount {
    owner: Cell<*const Account>,
    bypass: Cell<bool>,
    recycle_size: Cell<usize>,
    recycled: Cell<*mut u8>,
}

thread_local! {
    // Constant initialization does not invoke the allocator. The stable owner
    // record is allocated directly through System on first use, never through
    // GlobalAlloc. It survives thread exit for cross-thread frees.
    static CURRENT: ThreadAccount = const { ThreadAccount {
        owner: Cell::new(ptr::null()), bypass: Cell::new(false),
        recycle_size: Cell::new(0), recycled: Cell::new(ptr::null_mut()),
    } };
}

impl Drop for ThreadAccount {
    fn drop(&mut self) {
        self.bypass.set(true);
        let owner = self.owner.get();
        if !owner.is_null() && std::env::var_os("OPENUI_ALLOCATION_PEAKS").is_some() {
            // SAFETY: owner records are never freed, including after thread exit.
            let a = unsafe { &*owner };
            let thread = std::thread::current();
            let _ = writeln!(
                std::io::stderr().lock(),
                "allocation peak: {}: {} bytes (cap {} bytes)",
                thread.name().unwrap_or("unnamed thread"),
                a.peak.load(Relaxed),
                a.cap
            );
        }
    }
}

/// Peak since reset_peak, for shape assertions only. Reporting and enforcement
/// always retain the full thread's high-water mark, even across shape resets.
pub fn peak() -> usize {
    CURRENT.with(|c| {
        let p = c.owner.get();
        if p.is_null() {
            0
        } else {
            // SAFETY: owner records survive until process exit.
            unsafe { (*p).measured_peak.load(Relaxed) }
        }
    })
}

pub fn reset_peak() {
    CURRENT.with(|c| {
        let p = c.owner.get();
        if !p.is_null() {
            // SAFETY: owner records survive until process exit.
            unsafe {
                (*p).measured_peak.store((*p).live.load(Relaxed), Relaxed);
            }
        }
    });
}

/// Cumulative allocation/reallocation requests charged to this thread's account.
/// Unlike peak live bytes, this catches temporary allocations freed in a loop.
pub fn allocation_calls() -> usize {
    CURRENT.with(|c| {
        let p = c.owner.get();
        if p.is_null() {
            0
        } else {
            // SAFETY: owner records survive until process exit.
            unsafe { (*p).allocation_calls.load(Relaxed) }
        }
    })
}

/// Cumulative bytes reserved by allocation/reallocation requests on this thread.
/// Shrinking/frees do not subtract. Reallocation counts positive growth, so
/// this is allocation traffic rather than a sum of every requested capacity.
pub fn allocated_bytes() -> usize {
    CURRENT.with(|c| {
        let p = c.owner.get();
        if p.is_null() {
            0
        } else {
            // SAFETY: owner records survive until process exit.
            unsafe { (*p).allocated_bytes.load(Relaxed) }
        }
    })
}

/// Deterministically reuse one freed byte allocation of `size` on this thread.
/// The allocator keeps the cached block charged until reused or this guard drops.
/// This is test-only allocator machinery for checking address-based caches.
pub fn recycle_bytes(size: usize) -> RecycleBytes {
    assert!(size > 0);
    CURRENT.with(|c| {
        assert_eq!(c.recycle_size.get(), 0);
        c.recycle_size.set(size);
    });
    RecycleBytes(std::marker::PhantomData)
}
/// Thread-bound scope returned by [`recycle_bytes`].
pub struct RecycleBytes(std::marker::PhantomData<std::rc::Rc<()>>);
impl Drop for RecycleBytes {
    fn drop(&mut self) {
        CURRENT.with(|c| {
            let size = c.recycle_size.replace(0);
            let base = c.recycled.replace(ptr::null_mut());
            if base.is_null() {
                return;
            }
            let requested = Layout::array::<u8>(size).expect("previously allocated byte layout");
            let (layout, _) = layout(requested).expect("previously allocated header layout");
            // SAFETY: only a matching dealloc stores this block. It has no
            // live payload owner; its prefix and original layout remain valid.
            unsafe {
                let owner = base.cast::<*const Account>().read();
                if !owner.is_null() {
                    (*owner).live.fetch_sub(layout.size(), Relaxed);
                    LIVE_BYTES.fetch_sub(layout.size(), Relaxed);
                }
                System.dealloc(base, layout);
            }
        });
    }
}

fn reserve(owner: *const Account, bytes: usize) {
    if owner.is_null() {
        return;
    }
    // SAFETY: every header points to a permanent owner record.
    let a = unsafe { &*owner };
    let mut previous = a.allocation_calls.load(Relaxed);
    while let Err(actual) = a.allocation_calls.compare_exchange_weak(
        previous,
        previous.saturating_add(1),
        Relaxed,
        Relaxed,
    ) {
        previous = actual;
    }
    let mut total = a.allocated_bytes.load(Relaxed);
    while let Err(actual) = a.allocated_bytes.compare_exchange_weak(
        total,
        total.saturating_add(bytes),
        Relaxed,
        Relaxed,
    ) {
        total = actual;
    }
    LIVE_BYTES.fetch_add(bytes, Relaxed);
    let live = a.live.fetch_add(bytes, Relaxed) + bytes;
    a.peak.fetch_max(live, Relaxed);
    a.measured_peak.fetch_max(live, Relaxed);
    if live > a.cap && !CURRENT.try_with(|c| c.bypass.get()).unwrap_or(true) {
        let peak = a.peak.load(Relaxed);
        // GlobalAlloc must never unwind. Bypass accounting while acquiring the
        // thread name and formatting, and bypass libtest capture with OS stderr.
        let _ = CURRENT.try_with(|c| c.bypass.set(true));
        let thread = std::thread::current();
        let _ = writeln!(
            std::io::stderr().lock(),
            "allocation cap exceeded: test {}; cap {} bytes; peak {} bytes",
            thread.name().unwrap_or("unnamed thread"),
            a.cap,
            peak
        );
        std::process::exit(101);
    }
}

pub struct Allocator {
    cap: usize,
}

impl Allocator {
    pub const fn with_cap(cap: usize) -> Self {
        Self { cap }
    }

    fn owner(&self) -> *const Account {
        CURRENT
            .try_with(|c| {
                if c.bypass.replace(true) {
                    return ptr::null();
                }
                let mut owner = c.owner.get();
                if owner.is_null() {
                    // SAFETY: System bypasses this global allocator and provides
                    // aligned storage for Account. Leaking one small record per
                    // thread keeps allocation headers valid after the thread exits.
                    unsafe {
                        let layout = Layout::new::<Account>();
                        let allocated = System.alloc(layout).cast::<Account>();
                        if allocated.is_null() {
                            std::alloc::handle_alloc_error(layout);
                        }
                        allocated.write(Account {
                            cap: self.cap,
                            live: AtomicUsize::new(layout.size()),
                            peak: AtomicUsize::new(layout.size()),
                            measured_peak: AtomicUsize::new(layout.size()),
                            allocation_calls: AtomicUsize::new(0),
                            allocated_bytes: AtomicUsize::new(0),
                        });
                        owner = allocated;
                    }
                    c.owner.set(owner);
                }
                c.bypass.set(false);
                owner
            })
            .unwrap_or(ptr::null()) // runtime teardown after TLS destruction
    }
}

fn layout(layout: Layout) -> Option<(Layout, usize)> {
    let (combined, offset) = Layout::new::<*const Account>().extend(layout).ok()?;
    Some((combined.pad_to_align(), offset))
}

// SAFETY: System owns each allocation. A suitably aligned prefix holds its
// account; deallocation reconstructs the identical layout. No allocator path
// allocates recursively, except the fatal diagnostic with accounting disabled.
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, requested: Layout) -> *mut u8 {
        let Some((layout, offset)) = layout(requested) else {
            return ptr::null_mut();
        };
        let recycled = CURRENT
            .try_with(|c| {
                if requested.align() == 1 && requested.size() == c.recycle_size.get() {
                    c.recycled.replace(ptr::null_mut())
                } else {
                    ptr::null_mut()
                }
            })
            .unwrap_or(ptr::null_mut());
        if !recycled.is_null() {
            // SAFETY: this block came from a matching dealloc, with the same
            // owner/header/layout. Its live bytes remained charged in the cache.
            unsafe {
                reserve(recycled.cast::<*const Account>().read(), 0);
                return recycled.add(offset);
            }
        }
        let owner = self.owner();
        reserve(owner, layout.size());
        // SAFETY: layout is valid, including space for the prefix.
        let base = unsafe { System.alloc(layout) };
        if base.is_null() {
            if !owner.is_null() {
                unsafe {
                    (*owner).live.fetch_sub(layout.size(), Relaxed);
                    LIVE_BYTES.fetch_sub(layout.size(), Relaxed);
                }
            }
            return base;
        }
        // SAFETY: prefix is aligned and in bounds; payload matches requested alignment.
        unsafe {
            base.cast::<*const Account>().write(owner);
            base.add(offset)
        }
    }

    unsafe fn realloc(&self, payload: *mut u8, requested: Layout, new_size: usize) -> *mut u8 {
        let (old, offset) = layout(requested).expect("a previously allocated layout");
        let Ok(new_requested) = Layout::from_size_align(new_size, requested.align()) else {
            return ptr::null_mut();
        };
        let Some((new, _)) = layout(new_requested) else {
            return ptr::null_mut();
        };
        // SAFETY: the prefix and layout belong to this allocation. System's
        // realloc preserves alignment and the prefix, including its owner.
        unsafe {
            let base = payload.sub(offset);
            let owner = base.cast::<*const Account>().read();
            let growth = new.size().saturating_sub(old.size());
            reserve(owner, growth);
            let resized = System.realloc(base, old, new.size());
            if resized.is_null() {
                if !owner.is_null() {
                    (*owner).live.fetch_sub(growth, Relaxed);
                    LIVE_BYTES.fetch_sub(growth, Relaxed);
                }
                return resized;
            }
            if !owner.is_null() {
                (*owner)
                    .live
                    .fetch_sub(old.size().saturating_sub(new.size()), Relaxed);
                LIVE_BYTES.fetch_sub(old.size().saturating_sub(new.size()), Relaxed);
            }
            resized.add(offset)
        }
    }

    unsafe fn dealloc(&self, payload: *mut u8, requested: Layout) {
        let (layout, offset) = layout(requested).expect("a previously allocated layout");
        // SAFETY: payload came from alloc with this requested layout.
        unsafe {
            let base = payload.sub(offset);
            let owner = base.cast::<*const Account>().read();
            let cached = CURRENT
                .try_with(|c| {
                    if requested.align() == 1
                        && requested.size() == c.recycle_size.get()
                        && owner == c.owner.get()
                        && c.recycled.get().is_null()
                    {
                        c.recycled.set(base);
                        true
                    } else {
                        false
                    }
                })
                .unwrap_or(false);
            if cached {
                return;
            }
            if !owner.is_null() {
                (*owner).live.fetch_sub(layout.size(), Relaxed);
                LIVE_BYTES.fetch_sub(layout.size(), Relaxed);
            }
            System.dealloc(base, layout);
        }
    }
}
