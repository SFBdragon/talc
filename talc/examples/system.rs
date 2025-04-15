use talc::{Talck, oom::WithSysMem};

// Run with:
// `cargo run --example system`

talc::static_system_mutex!(SysMutex);

#[cfg(all(not(miri), any(unix, windows)))]
#[global_allocator]
static TALC: Talck<SysMutex, WithSysMem> = Talck::new(WithSysMem::new());

fn main() {
    eprint!("Doing some small allocations... ");

    let mut vec = Vec::with_capacity(100);
    vec.extend(0..300usize);
    vec.truncate(100);
    vec.shrink_to_fit();

    eprintln!("Done!");

    eprint!("Using MORE memory... ");

    vec.extend(0..10000000usize);
    drop(vec);

    eprintln!("Done! Ending...");
}
