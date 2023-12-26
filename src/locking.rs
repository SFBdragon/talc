//! Lock implementations for use with `Talck`.
//!
//! Note, at the moment this only contains [`AssumeUnlockable`] which is not recommended in general.
//!
//! Use of the `spin` crate's mutex with `Talck` is a good default.

/// A dummy RawMutex implementation to skip synchronization on single threaded systems.
///
/// # Safety
/// This is very unsafe and may cause undefined behaviour if multiple threads enter
/// a critical section syncronized by this, even without explicit unsafe code.
pub struct AssumeUnlockable;

// SAFETY: nope
unsafe impl lock_api::RawMutex for AssumeUnlockable {
    const INIT: AssumeUnlockable = AssumeUnlockable;

    // A spinlock guard can be sent to another thread and unlocked there
    type GuardMarker = lock_api::GuardSend;

    fn lock(&self) {}

    fn try_lock(&self) -> bool {
        true
    }

    unsafe fn unlock(&self) {}
}
