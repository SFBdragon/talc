/* use core::ptr::addr_of_mut;
 */

#[macro_export]
macro_rules! static_system_mutex {
    ($name:ident) => {
        static mut STATIC_PTHREAD_MUTEX: libc::pthread_mutex_t = libc::PTHREAD_MUTEX_INITIALIZER;

        /// TODO
        pub struct $name;

        unsafe impl lock_api::RawMutex for $name {
            const INIT: Self = Self;

            type GuardMarker = lock_api::GuardSend;

            #[inline]
            fn lock(&self) {
                unsafe {
                    libc::pthread_mutex_lock(core::ptr::addr_of_mut!(STATIC_PTHREAD_MUTEX));
                }
            }

            #[inline]
            fn try_lock(&self) -> bool {
                unsafe {
                    libc::pthread_mutex_trylock(core::ptr::addr_of_mut!(STATIC_PTHREAD_MUTEX)) == 0
                }
            }

            #[inline]
            unsafe fn unlock(&self) {
                unsafe {
                    libc::pthread_mutex_unlock(core::ptr::addr_of_mut!(STATIC_PTHREAD_MUTEX));
                }
            }
        }

        impl $name {
            /// Allows [`StaticPThreadMutex`] ([`StaticGlobalLock`](super::StaticGlobalLock) on unix-like systems)
            /// to remain usable in the child process, after a call to `fork(2)`
            ///
            /// It's instead recommended to immediately call `exec*` after `fork`, in which case you shouldn't need this.
            pub fn enable_child_alloc_after_fork() {
                // atfork must only be called once, to avoid a deadlock,
                // where the handler attempts to acquire the global lock twice
                static FORK_PROTECTED: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

                unsafe extern fn _lock_mutex() {
                    libc::pthread_mutex_lock(core::ptr::addr_of_mut!(STATIC_PTHREAD_MUTEX));
                }

                unsafe extern fn _unlock_mutex() {
                    libc::pthread_mutex_unlock(core::ptr::addr_of_mut!(STATIC_PTHREAD_MUTEX));
                }

                let cmpxchg_result = FORK_PROTECTED.compare_exchange(
                    0,
                    1,
                    core::sync::atomic::Ordering::Acquire,
                    core::sync::atomic::Ordering::Relaxed,
                );

                if cmpxchg_result.is_ok() {
                    // acquires the mutex before forking.
                    // releases the mutex in parent and child after forking.
                    // this protects against deadlocks

                    let result = unsafe {
                        libc::pthread_atfork(
                            Some(_lock_mutex),
                            Some(_unlock_mutex),
                            Some(_unlock_mutex),
                        )
                    };

                    debug_assert_eq!(result, 0);
                }
            }
        }
    };
}

/* static mut STATIC_PTHREAD_MUTEX: libc::pthread_mutex_t = libc::PTHREAD_MUTEX_INITIALIZER;

pub struct StaticPThreadMutex;

unsafe impl lock_api::RawMutex for StaticPThreadMutex {
    const INIT: Self = Self;

    type GuardMarker = lock_api::GuardSend;

    #[inline]
    fn lock(&self) {
        unsafe {
            libc::pthread_mutex_lock(addr_of_mut!(STATIC_PTHREAD_MUTEX));
        }
    }

    #[inline]
    fn try_lock(&self) -> bool {
        unsafe {
            libc::pthread_mutex_trylock(addr_of_mut!(STATIC_PTHREAD_MUTEX)) == 0
        }
    }

    #[inline]
    unsafe fn unlock(&self) {
        unsafe {
            libc::pthread_mutex_unlock(addr_of_mut!(STATIC_PTHREAD_MUTEX));
        }
    }
} */
