pub use lock_api;
pub use windows_sys;

#[macro_export]
macro_rules! static_system_mutex {
    ($name:ident) => {
        static mut STATIC_SRWLOCK: ::talc::sync::win::windows_sys::Win32::System::Threading::SRWLOCK
            = ::talc::sync::win::windows_sys::Win32::System::Threading::SRWLOCK_INIT;

        // TODO
        pub struct $name;

        unsafe impl ::talc::sync::win::lock_api::RawMutex for $name {
            const INIT: Self = Self;

            type GuardMarker = ::talc::sync::win::lock_api::GuardSend;

            #[inline]
            fn lock(&self) {
                unsafe {
                    ::talc::sync::win::windows_sys::Win32::System::Threading::AcquireSRWLockExclusive(
                        core::mem::addr_of_mut!(STATIC_SRWLOCK),
                    );
                }
            }

            #[inline]
            fn try_lock(&self) -> bool {
                unsafe {
                    // https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-tryacquiresrwlockexclusive
                    ::talc::sync::win::windows_sys::Win32::System::Threading::TryAcquireSRWLockExclusive(
                        core::mem::addr_of_mut!(STATIC_SRWLOCK),
                    ) != 0
                }
            }

            #[inline]
            unsafe fn unlock(&self) {
                unsafe {
                    ::talc::sync::win::windows_sys::Win32::System::Threading::ReleaseSRWLockExclusive(
                        core::mem::addr_of_mut!(STATIC_SRWLOCK),
                    );
                }
            }
        }
    };
}
