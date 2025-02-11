use core::ptr::NonNull;

/// This represents a contiguous memory region that a [`Talc`](crate::base::Talc)
/// instance has claimed and will allocate into.
///
/// [`Arena`]s function as a handle that a [`Talc`](crate::base::Talc)
/// can use to resize an arena. If you don't ever want to do this,
/// you can safely drop the [`Arena`] - otherwise hold onto it.
///
/// # Layout
///
/// Here's an example of how an arena might be laid out:
///
/// ```not_rust
///     ├─────────────────────────Arena──────────────────────────┤
/// ────┬───────────────┬───────────┬─────┬───────────┬──────────┬────
/// ... | Talc metadata | Allocated | Gap | Allocated | Gap ...  | ...
/// ────┴───────────────┴───────────┴─────┴───────────┴──────────┴────
///     ├ Base                                               End ┤
///     ├───────Size─────────────────────────────────────────────┤
///     ├───────Allocated extent──────────────────────┤
/// ```
///
/// Notes on the metadata section:
/// - This is only placed in the first claimed arena.
/// - This is isomorphic to allocated memory.
///
/// # Resizing
///
/// Arenas have a fixed base, but can be resized using
/// - [`Talc::resize`](crate::base::Talc::resize)
/// - [`Talc::extend`](crate::base::Talc::extend)
/// - [`Talc::truncate`](crate::base::Talc::truncate)
///
/// [`Talc::truncate`](crate::base::Talc::truncate) can be used to delete
/// arenas entirely if no allocations are in them.
///
/// [`Arena`]s provided by a certain [`Talc`](crate::base::Talc) must only
/// be resized using that `Talc`, and no other instance.
#[derive(Hash)]
pub struct Arena {
    pub(crate) base: *mut u8,
    pub(crate) end: NonNull<u8>,
}

// Cloning the Arena is not possible, so we don't need to worry about
// concurrent use.
unsafe impl Send for Arena {}

impl core::fmt::Debug for Arena {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Arena({:p}..[{}]..{:p})", self.base, self.size(), self.end())
    }
}
impl core::fmt::Display for Arena {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:p}..[{}]..{:p}", self.base, self.size(), self.end())
    }
}

impl Arena {
    /// Internal function to create an [`Arena`].
    ///
    /// # Safety
    /// There's a handful of properties that need to be upheld here:
    /// - `base < end` and by extension, `end` must not be null
    /// - The resulting `Arena` must indicate a valid region of memory
    ///     that a particular instance of Talc controls. Only that instance
    ///     of Talc may return such an [`Arena`] to the user.
    #[inline]
    pub(crate) unsafe fn new(base: *mut u8, end: *mut u8) -> Self {
        debug_assert!(end > base); // ensures that end is non-null
        Self { base, end: NonNull::new_unchecked(end) }
    }

    /// Get the pointer to the bottom of the arena.
    #[inline]
    pub fn base(&self) -> *mut u8 {
        self.base
    }

    /// The number of bytes in the arena.
    #[inline]
    pub fn size(&self) -> usize {
        self.end.as_ptr() as usize - self.base as usize
    }

    /// Get the pointer to the top of the arena.
    #[inline]
    pub fn end(&self) -> *mut u8 {
        self.end.as_ptr()
    }
}
