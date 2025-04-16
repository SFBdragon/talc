#![feature(iter_intersperse)]

use benches::{
    ARENA_ALLOCATORS, NamedAllocator, generate_align, generate_size, touch_the_whole_heap,
};

use std::alloc::{GlobalAlloc, Layout};
use std::fs::File;
use std::path::PathBuf;
use std::time::Instant;

const BENCH_DURATION: f64 = 1.0;
const MAX_ALLOCATIONS: usize = 600;

fn main() {
    let cargo_manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let benchmark_results_dir = cargo_manifest_dir.join("../results");
    std::fs::create_dir_all(&benchmark_results_dir).unwrap();

    let benchmark_file_path = benchmark_results_dir.join("microbench.csv");
    let mut csv = File::create(benchmark_file_path).unwrap();

    touch_the_whole_heap();

    for &NamedAllocator { name, init_fn } in ARENA_ALLOCATORS {
        // The following run far too slowly under this benchmark to be worth testing.
        // Thing is; these aren't slow allocators, either. Not sure what's wrong.
        // if matches!(name, "System" | "FRuSA") { continue; }

        let allocator = unsafe { init_fn() };
        benchmark_allocator(allocator.as_ref(), name, &mut csv);
    }
}

fn now() -> u64 {
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);

    #[cfg(target_arch = "x86_64")]
    let ret = {
        let mut x = 0u32;
        let ret = unsafe { std::arch::x86_64::__rdtscp(&mut x) };
        ret
    };

    #[cfg(target_arch = "aarch64")]
    let ret = {
        let mut timer: u64;
        unsafe {
            std::arch::asm!("mrs {0}, cntvct_el0", out(reg) timer, options(nomem, nostack));
        }
        return timer;
    };

    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);

    ret
    // If a compiler error crops up here, that's because a hardware-based counter
    // is not implemented for this architecture.
}

fn benchmark_allocator(allocator: &dyn GlobalAlloc, name: &str, csv_file: &mut File) {
    eprintln!("Benchmarking: {name}...");

    if name == "FRuSA" {
        eprintln!("  If FRuSA locks up, restart the micro-benchmark, it should work eventually.")
    }

    let mut active_allocations = Vec::new();

    let mut alloc_ticks_vec = Vec::new();
    let mut dealloc_ticks_vec = Vec::new();

    // warm up
    for i in 1..10000 {
        let layout = Layout::from_size_align(i * 8, 8).unwrap();
        let ptr = unsafe { allocator.alloc(layout) };
        assert!(!ptr.is_null());
        unsafe {
            let _ = ptr.read_volatile();
        }
        unsafe {
            allocator.dealloc(ptr, layout);
        }
    }

    let bench_timer = Instant::now();
    for i in 0.. {
        if i % 0x100 == 0 && (Instant::now() - bench_timer).as_secs_f64() > BENCH_DURATION {
            break;
        }

        // bias towards smaller values over larger ones
        let size = generate_size(0x10000);
        let align = generate_align();
        let layout = Layout::from_size_align(size, align).unwrap();

        let alloc_begin = now();
        let alloc = unsafe { allocator.alloc(layout) };
        let alloc_ticks = now().wrapping_sub(alloc_begin);

        if std::ptr::null_mut() != alloc {
            alloc_ticks_vec.push(alloc_ticks);
            active_allocations.push((alloc, layout));
        } else {
            for (ptr, layout) in active_allocations.drain(50..) {
                unsafe {
                    allocator.dealloc(ptr, layout);
                }
            }
            continue;
        }

        if (active_allocations.len() > 50 && fastrand::usize(..10) == 0)
            || active_allocations.len() > MAX_ALLOCATIONS
        {
            for _ in 0..8 {
                let index = fastrand::usize(..active_allocations.len());
                let allocation = active_allocations.swap_remove(index);

                let dealloc_begin = now();
                unsafe {
                    allocator.dealloc(allocation.0, allocation.1);
                }
                let dealloc_ticks = now().wrapping_sub(dealloc_begin);
                dealloc_ticks_vec.push(dealloc_ticks);
            }
        }
    }

    alloc_ticks_vec.sort_unstable();
    dealloc_ticks_vec.sort_unstable();
    let alloc_ticks = alloc_ticks_vec.into_iter().map(|x| x as f64).collect::<Vec<_>>();
    let dealloc_ticks = dealloc_ticks_vec.into_iter().map(|x| x as f64).collect::<Vec<_>>();

    let alloc_quartiles = quartiles(&alloc_ticks);
    let dealloc_quartiles = quartiles(&dealloc_ticks);
    let mut sum_quartiles = [0.0; 5];
    for i in 0..sum_quartiles.len() {
        sum_quartiles[i] = alloc_quartiles[i] + dealloc_quartiles[i]
    }

    let data_to_string = |data: &[f64]| {
        String::from_iter(data.into_iter().map(|x| x.to_string()).intersperse(",".to_owned()))
    };

    use std::io::Write;
    writeln!(csv_file, "{name},{}", data_to_string(&sum_quartiles[..])).unwrap();
}

fn quartiles(data: &[f64]) -> [f64; 5] {
    let len = data.len();
    [data[len / 100], data[len / 4], data[len / 2], data[3 * len / 4], data[99 * len / 100]]
}
