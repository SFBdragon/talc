use core::ptr::addr_of_mut;

use windows_sys::Win32::System::Threading::*;

static mut STATIC_SRWLOCK: SRWLOCK = SRWLOCK_INIT;

pub struct StaticSrwMutex;

unsafe impl lock_api::RawMutex for StaticSrwMutex {
    const INIT: Self = Self;

    type GuardMarker = lock_api::GuardSend;

    #[inline]
    fn lock(&self) {
        unsafe {
            AcquireSRWLockExclusive(addr_of_mut!(STATIC_SRWLOCK));
        }
    }

    #[inline]
    fn try_lock(&self) -> bool {
        unsafe {
            // https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-tryacquiresrwlockexclusive
            TryAcquireSRWLockExclusive(addr_of_mut!(STATIC_SRWLOCK)) != 0
        }
    }

    #[inline]
    unsafe fn unlock(&self) {
        unsafe {
            ReleaseSRWLockExclusive(addr_of_mut!(STATIC_SRWLOCK));
        }
    }
}
