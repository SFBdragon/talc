#![feature(iter_intersperse)]

use std::{
    alloc::GlobalAlloc,
    fmt::Write,
    path::PathBuf,
    sync::{Arc, Barrier},
    time::{Duration, Instant},
};

use benches::{
    AllocationWrapper, NAMED_ALLOCATORS, NamedAllocator, generate_align, generate_size,
    touch_the_whole_heap,
};

const TRIALS_AMOUNT: usize = 7;
const WARMUP: Duration = Duration::from_millis(2);
const DURATION: Duration = Duration::from_millis(200);
const RA_MAX_ALLOC_SIZES: &[usize] = &[1000, 3000, 10000, 30000];
const RA_MAX_REALLOC_SIZE_MULTI: usize = 3;
const RA_TARGET_MIN_ALLOCATIONS: usize = 300;

fn main() {
    let mut realloc = true;
    let mut thread_count = 1;
    let mut output_name = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--no-realloc" => realloc = false,
            "--thread-count" => {
                thread_count = args
                    .next()
                    .and_then(|arg| arg.parse::<usize>().ok())
                    .expect("expected number after --thread-count")
            }
            "--name" => output_name = Some(args.next().expect("expected string after --name")),
            "--help" => {
                println!(
                    r#"Random actions benchmark

Usage: cargo run -p benches --bin random-actions --release

Options:
  --name            The name of the output file [required].
  --no-realloc      Disables reallocation operations in the benchmark.
  --thread-count    Sets the number of threads the benchmark executes in parallel. [default = 1]."#
                );
                return;
            }
            argument => panic!("unrecognized argument '{}'", argument),
        }
    }

    let Some(output_name) = output_name else {
        panic!("--name is required");
    };

    let cargo_manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let benchmark_results_dir = cargo_manifest_dir.join("../results");
    std::fs::create_dir_all(&benchmark_results_dir).unwrap();

    let mut csv = String::new();

    write!(csv, ",").unwrap();
    csv.extend(RA_MAX_ALLOC_SIZES.iter().map(|i| i.to_string()).intersperse(",".to_owned()));
    writeln!(csv).unwrap();

    touch_the_whole_heap();

    for &NamedAllocator { name, init_fn } in NAMED_ALLOCATORS {
        write!(csv, "{}", name).unwrap();

        for &max_alloc_size in RA_MAX_ALLOC_SIZES.iter() {
            eprintln!("benchmarking {} - max alloc size {}B ...", name, max_alloc_size);

            let score = (0..TRIALS_AMOUNT)
                .map(|_| {
                    let allocator = unsafe { (init_fn)() };
                    let allocator_ref = allocator.as_ref();

                    std::thread::scope(|scope| {
                        let barrier = Arc::new(Barrier::new(thread_count));
                        let mut handles = vec![];

                        for _ in 0..thread_count {
                            let barrier = barrier.clone();
                            handles.push(scope.spawn(move || {
                                let run_immediately = Arc::new(Barrier::new(1));
                                random_actions(
                                    allocator_ref,
                                    max_alloc_size,
                                    run_immediately,
                                    WARMUP,
                                    realloc,
                                );
                                random_actions(
                                    allocator_ref,
                                    max_alloc_size,
                                    barrier,
                                    DURATION,
                                    realloc,
                                )
                            }));
                        }

                        handles.into_iter().map(|h| h.join().unwrap()).sum::<usize>()
                    })
                })
                .sum::<usize>()
                / TRIALS_AMOUNT;

            write!(csv, ",{}", score).unwrap();
        }

        writeln!(csv).unwrap();
    }
    // remove the last newline.
    csv.pop();

    let csv_file_path = benchmark_results_dir.join(format!("{}.csv", output_name));
    std::fs::write(csv_file_path, csv).unwrap();
}

pub fn random_actions(
    allocator: &dyn GlobalAlloc,
    max_alloc_size: usize,
    barrier: Arc<Barrier>,
    duration: Duration,
    realloc: bool,
) -> usize {
    let mut score = 0;
    let mut v: Vec<AllocationWrapper<'_>> = Vec::with_capacity(100000);
    let rng = fastrand::Rng::new();

    let mut allocation_failure_count = 0usize;
    let mut reallocation_failure_count = 0usize;

    barrier.wait();
    let start = Instant::now();
    while start.elapsed() < duration {
        for _ in 0..100 {
            let action = rng.usize(0..=(5 + realloc as usize));

            // 1/7 - reallocate
            // 3/7 - if there are enough allocations, deallocate
            // 3/7 - if enough allocations else 6/7, allocate

            // this avoids staying close to zero allocations
            // while also avoiding growing the heap unboundedly
            // as benchmarking high heap contention isn't usually relevant
            // but having a very low number of allocations isn't realistic either

            if action == 6 {
                if !v.is_empty() {
                    let index = rng.usize(0..v.len());
                    let allocation = v.get_mut(index).unwrap();
                    let new_size = rng.usize(1..(max_alloc_size * RA_MAX_REALLOC_SIZE_MULTI));
                    if allocation.realloc(new_size).is_ok() {
                        score += 1;
                    } else {
                        reallocation_failure_count += 1;
                    }
                }
            } else if action < 3 || v.len() < RA_TARGET_MIN_ALLOCATIONS {
                // bias towards smaller values over larger ones
                // I'm hoping this makes this a little more representative
                let size = generate_size(max_alloc_size);
                let align = generate_align();
                if let Some(allocation) = AllocationWrapper::new(size, align, allocator) {
                    v.push(allocation);
                    score += 1;
                } else {
                    allocation_failure_count += 1;
                }
            } else {
                let index = rng.usize(0..v.len());
                v.swap_remove(index);
                score += 1;
            }
        }
    }

    if allocation_failure_count != 0 {
        eprintln!("Allocation failure count: {}", allocation_failure_count);
    }
    if reallocation_failure_count != 0 {
        eprintln!("Reallocation failure count: {}", reallocation_failure_count);
    }

    score
}
