use talc::talc::{alignment::{alloc_unit, ChunkAlign, DefaultAlign, SetAlign}, bitfield::{TwoUsizeBitField, U64BitField}, bucket_config::auto_size_to_bucket_with_soft_max};

fn main() {
    const BUCKET_COUNT: usize = 128;
    type A = SetAlign<32>;
    
    let mut size = alloc_unit::<A>();
    let mut increment = size / 4;
    let mut b = usize::MAX;

    // let size_to_bucket = |size: usize| unsafe { auto_size_to_bucket_with_soft_max::<A, TwoUsizeBitField, {1<<20}>(size) };
    let size_to_bucket = |size: usize| unsafe { talc::talc::bucket_config::bucket_of_size_l1_l2_pexp::<talc::talc::bucket_config::TwoUsizeBinCfg, A>(size) };

    loop {
        let b_calc = size_to_bucket(size);

        if b_calc != b {
            if b_calc != 0 {
                loop {
                    if b_calc == size_to_bucket(size - 1) {
                        size -= 1;
                    } else {
                        break;
                    }
                }
            }

            eprintln!("{:>3}: {1:>10}  {1:>8X}", b_calc, size);
            increment = size / alloc_unit::<A>();

            if b != usize::MAX {
                assert_eq!(b + 1, b_calc);
            }

            b = b_calc;

            assert!(b < BUCKET_COUNT);
            // if b == BUCKET_COUNT - 1 { break; }
        }
        size += increment;
    }
}

