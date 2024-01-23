//! Note this only contains [`AssumeUnlockable`] which is not generally recommended.
//! Use of the `spin` crate's mutex with [`Talck`](crate::Talc) is a good default.

/// #### WARNING: [`AssumeUnlockable`] may cause undefined behaviour without `unsafe` code!
/// 
/// A dummy [`RawMutex`](lock_api::RawMutex) implementation to skip synchronization on single threaded systems.
///
/// # Safety
/// [`AssumeUnlockable`] is highly unsafe and may cause undefined behaviour if multiple 
/// threads enter a critical section it guards, even without explicit unsafe code.
/// 
/// Note that uncontended spin locks are cheap. Usage is only recommended on 
/// platforms that don't have atomics or are exclusively single threaded.
/// 
/// Through no fault of its own, `lock_api`'s API does not allow for safe 
/// encapsulation of this functionality. This is a hack for backwards compatibility.
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
