//! Heap allocations per pulled block, through the C API, with a counting global allocator.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use truehd_atmos::types::*;
use truehd_atmos::*;

struct Counting;
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(layout.size() as u64, Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(new as u64, Relaxed);
        unsafe { System.realloc(ptr, layout, new) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn main() {
    let path = std::env::args().nth(1).expect("path to .thd");
    let presentation: i32 = std::env::args().nth(2).map_or(-1, |s| s.parse().unwrap());
    let data = std::fs::read(path).unwrap();
    let d = truehd_atmos_decoder_create(presentation);
    let mut block: Block = unsafe { std::mem::zeroed() };
    let (mut blocks, mut warm_allocs, mut warm_bytes) = (0u64, 0u64, 0u64);
    for chunk in data.chunks(4096) {
        unsafe { truehd_atmos_decoder_push(d, chunk.as_ptr(), chunk.len(), 0) };
        loop {
            let (a0, b0) = (ALLOCS.load(Relaxed), BYTES.load(Relaxed));
            if unsafe { truehd_atmos_decoder_pull(d, &mut block) } != OK {
                break;
            }
            blocks += 1;
            if blocks > 1000 {
                warm_allocs += ALLOCS.load(Relaxed) - a0;
                warm_bytes += BYTES.load(Relaxed) - b0;
            }
        }
    }
    let n = blocks.saturating_sub(1000).max(1);
    println!(
        "{blocks} blocks (p{}, {} ch); after warm-up: {:.2} allocations and {:.0} bytes allocated per pulled block",
        block.presentation,
        block.channel_count,
        warm_allocs as f64 / n as f64,
        warm_bytes as f64 / n as f64
    );
    unsafe { truehd_atmos_decoder_destroy(d) };
}
