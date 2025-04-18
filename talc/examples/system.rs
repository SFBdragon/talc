use talc::prelude::*;

// Run with:
// `cargo run --example system`

#[global_allocator]
static TALC: TalcLock<SysMutex, Os> = TalcLock::new(Os::new());
talc::static_system_mutex!(SysMutex);

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
