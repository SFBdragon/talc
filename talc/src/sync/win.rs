#[macro_export]
macro_rules! static_system_mutex_ {
    ($name:ident) => {
        static mut STATIC_SRWLOCK: SRWLOCK = SRWLOCK_INIT;

        // TODO
        pub struct $name;

        unsafe impl lock_api::RawMutex for $name {
            const INIT: Self = Self;

            type GuardMarker = lock_api::GuardSend;

            #[inline]
            fn lock(&self) {
                unsafe {
                    windows_sys::Win32::System::Threading::AcquireSRWLockExclusive(
                        core::mem::addr_of_mut!(STATIC_SRWLOCK),
                    );
                }
            }

            #[inline]
            fn try_lock(&self) -> bool {
                unsafe {
                    // https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-tryacquiresrwlockexclusive
                    windows_sys::Win32::System::Threading::TryAcquireSRWLockExclusive(
                        core::mem::addr_of_mut!(STATIC_SRWLOCK),
                    ) != 0
                }
            }

            #[inline]
            unsafe fn unlock(&self) {
                unsafe {
                    windows_sys::Win32::System::Threading::ReleaseSRWLockExclusive(
                        core::mem::addr_of_mut!(STATIC_SRWLOCK),
                    );
                }
            }
        }
    };
}
