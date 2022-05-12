//!
//! # clone-replace
//!
//! A [CloneReplace] is a synchronisation primitive that provides
//! owned handles for shared data.
//!
//! Example:
//! ```rust
//! use clone_replace::CloneReplace;
//!
//! let data = CloneReplace::new(1);
//!
//! let v1 = data.access();
//! assert_eq!(*v1, 1);
//! {
//!    let mut m = data.mutate();
//!    *m = 2;
//!    let v2 = data.access();
//!    assert_eq!(*v1, 1);
//!    assert_eq!(*v2, 1);
//! }
//! let v3 = data.access();
//! assert_eq!(*v3, 2);
//! assert_eq!(*v1, 1);
//! ```
//!
//! This is a primitive in a similar format to
//! [Mutex](std::sync::Mutex), in that it wraps data for
//! thread-safety, and provides access via guards.  A shared,
//! reference copy of the data is stored. When reading, a handle to an
//! immutable snapshot state is obtained, as an
//! [Arc](std::sync::Arc). All readers who access this version of the
//! data will receive handles to the same snapshot.
//!
//! To mutate the data, the reference copy is cloned into a mutable
//! guard object.  The writer is free to make whatever changes they
//! wish to this copy, and the new data will become the reference copy
//! for all subsequent reads and writes, whenever the guard is
//! dropped. No writes are visible whilst the guard is in scope.
//!
//! This is a somewhat niche primitive that has the following
//! properties:
//! - Readers can work with a coherent view for an extended period of
//!   time, without preventing writers from making updates, or other
//!   readers from seeing those updates.
//! - There are no lifetimes to plumb through for the guards: the data
//!   is owned. This is most significant before generic associated
//!   types stabilise, but it will remain an advantage for the
//!   simplicity of some use cases, compared to
//!   [Mutex](std::sync::Mutex) or [RwLock](std::sync::RwLock).
//! - Mutation is expensive. A full copy is made every time you create
//!   a mutation guard by calling [mutate](CloneReplace::mutate) on
//!   [CloneReplace].
//! - The memory overhead can be large. For scenarios with very long
//!   running readers, you may end up with many copies of your data
//!   being stored simultaneously.
//! - In the presence of multiple writers, it's entirely possible to
//!   lose updates, because multiple writers are not prevented from
//!   existing at the same time. Whatever state is set will always be
//!   internally consistent, but you give up guaranteed external
//!   consistency.

use arc_swap::ArcSwap;
use core::ops::{Deref, DerefMut, Drop};
use std::fmt::{Display, Formatter, Result};
use std::sync::Arc;

/// A shareable store for data which provides owned references.
///
/// A `CloneReplace` stores a reference version of an enclosed data
/// structure.  An owned snapshot of the current reference version can
/// be retrieved by calling [access](CloneReplace::access) on
/// [CloneReplace], which will preserve the reference version at that
/// moment until it is dropped. A mutatable snapshot of the current
/// reference version can be retrieved by calling
/// [mutate](CloneReplace::mutate) on [CloneReplace], and when this
/// goes out of scope, the reference version at that moment will be
/// replaced by the mutated one.
#[derive(Debug)]
pub struct CloneReplace<T> {
    data: Arc<ArcSwap<T>>,
}

impl<T> Clone for CloneReplace<T> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
        }
    }
}

impl<T: Default> Default for CloneReplace<T> {
    fn default() -> Self {
        Self::new(Default::default())
    }
}

impl<T> CloneReplace<T> {
    /// Create a new [CloneReplace].
    ///
    /// Example:
    /// ```rust
    /// use clone_replace::CloneReplace;
    ///
    /// struct Foo {
    ///    a: i32
    /// }
    ///
    /// let cr = CloneReplace::new(Foo { a: 0 });
    /// ```
    pub fn new(data: T) -> Self {
        Self {
            data: Arc::new(ArcSwap::new(Arc::new(data))),
        }
    }

    /// Retrieve a snapshot of the current reference version of the data.
    ///
    /// The return value is owned, and the snapshot taken will remain
    /// unchanging until it goes out of scope. The existence of the
    /// snapshot will not prevent the reference version from evolving,
    /// so holding snapshots must be carefully considered, as it can
    /// lead to memory pressure.
    ///
    /// Example:
    /// ```rust
    /// use clone_replace::CloneReplace;
    ///
    /// let c = CloneReplace::new(1);
    /// let v = c.access();
    /// assert_eq!(*v, 1);
    /// ```
    pub fn access(&self) -> Arc<T> {
        self.data.load_full()
    }

    fn set_value(&self, value: T) {
        self.data.swap(Arc::new(value));
    }
}

impl<T: Clone> CloneReplace<T> {
    /// Create a mutable replacement for the reference data.
    ///
    /// A copy of the current reference version of the data is
    /// created. The [MutateGuard] provides mutable references to that
    /// data. When the guard goes out of scope the reference version
    /// will be overwritten with the updated version.
    ///
    /// Multiple guards can exist simultaneously, and there is no
    /// attempt to prevent loss of data from stale updates.  An
    /// internally consistent version of the data, as produced by a
    /// single mutate call, will always exist, but not every mutate
    /// call will end up being reflected in a reference version of the
    /// data. This is a significantly weaker consistency guarantee
    /// than a [Mutex](std::sync::Mutex) provides, for example.
    ///
    /// Example:
    /// ```rust
    /// use clone_replace::CloneReplace;
    ///
    /// let c = CloneReplace::new(1);
    /// let mut v = c.mutate();
    /// *v = 2;
    /// drop(v);
    /// assert_eq!(*c.access(), 2);
    /// ```
    pub fn mutate(&self) -> MutateGuard<T> {
        let inner = &*self.data.load_full();
        MutateGuard {
            origin: self.clone(),
            data: Some(inner.clone()),
        }
    }
}

/// A handle to a writeable version of the data.
///
/// This structure is created by the [mutate](CloneReplace::mutate)
/// method on [CloneReplace]. The data held by the guard can be
/// accessed via its [Deref] and [DerefMut] implementations.
///
/// When the guard is dropped, the contents will be written back to
/// become the new reference version of the data. Any intermediate
/// writes that occurred between the mutate guard being constructed
/// and the writeback will be discarded.
pub struct MutateGuard<T> {
    origin: CloneReplace<T>,
    data: Option<T>,
}

impl<T> MutateGuard<T> {
    /// Discard the changes made in this mutation session.
    ///
    /// The changed data will not be written back to its origin.  If
    /// you do not call discard, the changes will always be committed
    /// when the guard goes out of scope.
    ///
    /// Example:
    /// ```rust
    /// use clone_replace::CloneReplace;
    ///
    /// let c = CloneReplace::new(1);
    /// let mut v = c.mutate();
    /// *v = 2;
    /// v.discard();
    /// assert_eq!(*c.access(), 1);
    /// ```
    pub fn discard(mut self) {
        self.data = None;
    }
}

impl<T> Deref for MutateGuard<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        // Does not panic: the Option is only None after drop()
        // returns, or if discard() has been called, which also drops
        // the value immediately. There's no way to get here so long
        // as we don't call deref() from those two methods.
        self.data.as_ref().unwrap()
    }
}

impl<T> DerefMut for MutateGuard<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // Does not panic: the Option is only None after drop()
        // returns, or if discard() has been called, which also drops
        // the value immediately. There's no way to get here so long
        // as we don't call deref_mut() from those two methods.
        self.data.as_mut().unwrap()
    }
}

impl<T: Display> Display for MutateGuard<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        // Does not panic: the Option is only None after drop()
        // returns, or if discard() has been called, which also drops
        // the value immediately. There's no way to get here so long
        // as we don't call fmt() from those two methods.
        self.data.as_ref().unwrap().fmt(f)
    }
}

impl<T> Drop for MutateGuard<T> {
    fn drop(&mut self) {
        if let Some(data) = self.data.take() {
            self.origin.set_value(data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CloneReplace;
    use std::fmt::{Display, Formatter};

    #[derive(Clone, Debug)]
    struct Foo {
        pub a: i32,
    }

    impl Display for Foo {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            self.a.fmt(f)
        }
    }

    #[test]
    fn test_basic() {
        let cr = CloneReplace::new(Foo { a: 0 });

        let v1 = cr.access();
        assert_eq!(v1.a, 0);
        {
            let mut m = cr.mutate();
            assert_eq!(m.a, 0);
            m.a = 2;
            assert_eq!(m.a, 2);
            let v2 = cr.access();
            assert_eq!(v1.a, 0);
            assert_eq!(v2.a, 0);
        }
        let v3 = cr.access();
        assert_eq!(v3.a, 2);
        assert_eq!(v1.a, 0);
    }

    #[test]
    fn test_discard() {
        let cr = CloneReplace::new(Foo { a: 5 });

        let v1 = cr.access();
        assert_eq!(v1.a, 5);
        {
            let mut m = cr.mutate();
            assert_eq!(m.a, 5);
            m.a = 1;
            assert_eq!(m.a, 1);
            let v2 = cr.access();
            assert_eq!(v1.a, 5);
            assert_eq!(v2.a, 5);
            m.discard();
        }
        let v3 = cr.access();
        assert_eq!(v3.a, 5);
        assert_eq!(v1.a, 5);
    }

    #[test]
    fn test_display() {
        let cr = CloneReplace::new(Foo { a: 3 });

        let v1 = cr.access();
        assert_eq!(v1.to_string(), "3");
        {
            let mut m = cr.mutate();
            assert_eq!(m.to_string(), "3");
            m.a = 2;
            assert_eq!(m.to_string(), "2");
            let v2 = cr.access();
            assert_eq!(v1.to_string(), "3");
            assert_eq!(v2.to_string(), "3");
        }
        let v3 = cr.access();
        assert_eq!(v3.to_string(), "2");
        assert_eq!(v1.to_string(), "3");
    }

    #[test]
    fn test_multiple_writers() {
        let cr = CloneReplace::new(Foo { a: 4 });

        let v1 = cr.access();
        assert_eq!(v1.a, 4);
        {
            let mut m1 = cr.mutate();
            let mut m2 = cr.mutate();

            assert_eq!(m1.a, 4);
            m1.a = 1;
            assert_eq!(m1.a, 1);

            let v2 = cr.access();
            assert_eq!(v1.a, 4);
            assert_eq!(v2.a, 4);

            assert_eq!(m2.a, 4);
            m2.a = 5;
            assert_eq!(m2.a, 5);
            let v3 = cr.access();
            assert_eq!(v1.a, 4);
            assert_eq!(v2.a, 4);
            assert_eq!(v3.a, 4);
            assert_eq!(m1.a, 1);
        }
        let v4 = cr.access();
        assert_eq!(v4.a, 1);
        assert_eq!(v1.a, 4);
    }
}
