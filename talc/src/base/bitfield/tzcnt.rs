macro_rules! opt_tzcnt_for {
    ($num:ty, $name:ident, $reg:expr) => {

        #[inline(always)]
        pub fn $name(n: $num) -> u32 {
            // This implements this accepted optimization: https://github.com/llvm/llvm-project/issues/122004
            // See the issue for rationale and details
            #[cfg(all(
                not(miri), // MIRI doesn't support inline assembly
                not(target_feature="bmi1"), // else the compiler will just generate a `tzcnt` without branching, let it be
                target_arch = "x86_64",
            ))]
            {
                let x = n;

                // `x` is zero, and thus `bsf` doesn't overwrite `tz`, then `tz` will have the same
                // value as if we ran `tzcnt` instead of `bsf`.
                // If `tzcnt` runs (i.e. `rep bsf` on a CPU that supports `tzcnt`) then this `mov`
                // will be useless, but it's a lot cheaper than the branching that LLVM normally does.
                // In fact, due to complicated CPU implementation details (dependency tracking, register renaming), it's practically free:
                // https://uica.uops.info/?code=mov%20eax%2C%2064%0D%0Arep%20bsf%20rax%2C%20rcx&syntax=asIntel&uArchs=SNB&uArchs=SKL&uArchs=RKL&tools=uiCA&alignment=0
                let mut tz: $num = <$num>::BITS as $num;

                unsafe {
                    core::arch::asm!(
                        // Using the `rep` prefix doesn't effect the behavior of the instruction,
                        // but it does cause this instruction to be interpreted as `tzcnt` instead
                        // if it's available. LLVM does this for `trailing_zeros` as well; it's a sound optimization.
                        // `tzcnt` is a lot faster than `bsf` almost always: https://www.agner.org/optimize/instruction_tables.pdf
                        concat!("rep bsf {tz:", $reg, "}, {x:", $reg, "}"),
                        x = in(reg) x,
                        tz = inlateout(reg) tz,
                        options(pure, nomem, nostack)
                    );
                }
                return tz as u32;
            }

            #[allow(unreachable_code)]
            n.trailing_zeros()
        }
    };
}

opt_tzcnt_for!(u32, tzcnt_u32, "e");

#[cfg(target_pointer_width = "64")]
opt_tzcnt_for!(u64, tzcnt_u64, "r");
#[cfg(not(target_pointer_width = "64"))]
pub fn tzcnt_u64(n: u64) -> u32 {
    n.trailing_zeros()
}

#[cfg(target_pointer_width = "64")]
opt_tzcnt_for!(usize, tzcnt_usize, "r");
#[cfg(target_pointer_width = "32")]
opt_tzcnt_for!(usize, tzcnt_usize, "e");
