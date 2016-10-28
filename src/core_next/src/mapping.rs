// Copyright 2015 The Gfx-rs Developers.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#![deny(missing_docs, missing_copy_implementations)]

//! Memory mapping

use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, MutexGuard};
use core::memory;
use {Resources, Factory};
use {handle};

/// Unsafe, backend-provided operations for a buffer mapping
#[doc(hidden)]
pub trait Gate<R: Resources> {
    /// Set the element at `index` to `val`. Not bounds-checked.
    unsafe fn set<T>(&self, index: usize, val: T);
    /// Returns a slice of the specified length.
    unsafe fn slice<'a, 'b, T>(&'a self, len: usize) -> &'b [T];
    /// Returns a mutable slice of the specified length.
    unsafe fn mut_slice<'a, 'b, T>(&'a self, len: usize) -> &'b mut [T];

    /// Hook before user read access
    fn before_read(&mut RawInner<R>) {}
    /// Hook before user write access
    fn before_write(&mut RawInner<R>) {}
}

fn valid_access(access: memory::Access, usage: memory::Usage) -> Result<(), Error> {
    use memory::Usage::*;
    match usage {
        Persistent(a) if a.contains(access) => Ok(()),
        _ => Err(Error::InvalidAccess(access, usage)),
    }
}

/// Would mapping this buffer with this memory access be an error ?
fn is_ok<R: Resources>(access: memory::Access, buffer: &handle::RawBuffer<R>) -> Result<(), Error> {
    try!(valid_access(access, buffer.get_info().usage));
    if buffer.mapping().is_some() { Err(Error::AlreadyMapped) }
    else { Ok(()) }
}

#[derive(Debug)]
#[doc(hidden)]
pub struct Status<R: Resources> {
    pub cpu_write: bool,
    pub gpu_access: Option<handle::Fence<R>>,
}

impl<R: Resources> Status<R> {
    fn clean() -> Self {
        Status {
            cpu_write: false,
            gpu_access: None,
        }
    }

    fn access(&mut self) {
        self.gpu_access.take().map(|fence| fence.wait());
    }

    fn write_access(&mut self) {
        self.access();
        self.cpu_write = true;
    }
}

/// Error mapping a buffer.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum Error {
    /// The requested mapping access did not match the expected usage.
    InvalidAccess(memory::Access, memory::Usage),
    /// The memory was already mapped
    AlreadyMapped,
}

#[derive(Debug)]
#[doc(hidden)]
pub struct RawInner<R: Resources> {
    pub resource: R::Mapping,
    pub buffer: handle::RawBuffer<R>,
    pub access: memory::Access,
    pub status: Status<R>,
}

impl<R: Resources> Drop for RawInner<R> {
    fn drop(&mut self) {
        self.buffer.was_unmapped();
    }
}

/// Raw mapping providing status tracking
#[derive(Debug)]
pub struct Raw<R: Resources>(Mutex<RawInner<R>>);

impl<R: Resources> Raw<R> {
    #[doc(hidden)]
    pub fn new<F>(access: memory::Access, buffer: &handle::RawBuffer<R>, f: F) -> Result<Self, Error>
        where F: FnOnce() -> R::Mapping
    {
        try!(is_ok(access, buffer));
        Ok(Raw(Mutex::new(RawInner {
            resource: f(),
            buffer: buffer.clone(),
            access: access,
            status: Status::clean(),
        })))
    }

    #[doc(hidden)]
    pub fn access(&self) -> Option<MutexGuard<RawInner<R>>> {
        self.0.try_lock().ok()
    }

    unsafe fn read<T: Copy>(&self, len: usize) -> Reader<R, T> {
        let mut inner = self.access().unwrap();
        R::Mapping::before_read(&mut inner);
        inner.status.access();

        Reader {
            slice: inner.resource.slice(len),
            inner: inner,
        }
    }

    unsafe fn write<T: Copy>(&self, len: usize) -> Writer<R, T> {
        let mut inner = self.access().unwrap();
        R::Mapping::before_write(&mut inner);
        inner.status.write_access();

        Writer {
            len: len,
            inner: inner,
            phantom: PhantomData,
        }
    }

    unsafe fn read_write<T: Copy>(&self, len: usize) -> RWer<R, T> {
        let mut inner = self.access().unwrap();
        R::Mapping::before_read(&mut inner);
        R::Mapping::before_write(&mut inner);
        inner.status.write_access();

        RWer {
            slice: inner.resource.mut_slice(len),
            inner: inner,
        }
    }
}

/// Mapping reader
pub struct Reader<'a, R: Resources, T: 'a + Copy> {
    slice: &'a [T],
    #[allow(dead_code)] inner: MutexGuard<'a, RawInner<R>>,
}

impl<'a, R: Resources, T: 'a + Copy> Deref for Reader<'a, R, T> {
    type Target = [T];

    fn deref(&self) -> &[T] { self.slice }
}

/// Mapping writer
pub struct Writer<'a, R: Resources, T: 'a + Copy> {
    len: usize,
    inner: MutexGuard<'a, RawInner<R>>,
    phantom: PhantomData<T>,
}

impl<'a, R: Resources, T: 'a + Copy> Writer<'a, R, T> {
    /// Set a value in the buffer
    pub fn set(&mut self, index: usize, value: T) {
        if index >= self.len {
            panic!("tried to write out of bounds of a mapped buffer");
        }
        unsafe { self.inner.resource.set(index, value); }
    }
}

/// Mapping reader & writer
pub struct RWer<'a, R: Resources, T: 'a + Copy> {
    slice: &'a mut [T],
    #[allow(dead_code)] inner: MutexGuard<'a, RawInner<R>>,
}

impl<'a, R: Resources, T: 'a + Copy> Deref for RWer<'a, R, T> {
    type Target = [T];

    fn deref(&self) -> &[T] { &*self.slice }
}

impl<'a, R: Resources, T: Copy> DerefMut for RWer<'a, R, T> {
    fn deref_mut(&mut self) -> &mut [T] { self.slice }
}

/// Readable mapping.
pub struct Readable<R: Resources, T: Copy> {
    raw: handle::RawMapping<R>,
    len: usize,
    phantom: PhantomData<T>,
}

impl<R: Resources, T: Copy> Readable<R, T> {
    /// Acquire a mapping Reader
    pub fn read(&mut self) -> Reader<R, T> {
        unsafe { self.raw.read::<T>(self.len) }
    }
}

/// Writable mapping.
pub struct Writable<R: Resources, T: Copy> {
    raw: handle::RawMapping<R>,
    len: usize,
    phantom: PhantomData<T>,
}

impl<R: Resources, T: Copy> Writable<R, T> {
    /// Acquire a mapping Writer
    pub fn write(&mut self) -> Writer<R, T> {
        unsafe { self.raw.write::<T>(self.len) }
    }
}

/// Readable & writable mapping.
pub struct RWable<R: Resources, T: Copy> {
    raw: handle::RawMapping<R>,
    len: usize,
    phantom: PhantomData<T>
}

impl<R: Resources, T: Copy> RWable<R, T> {
    /// Acquire a mapping Reader
    pub fn read(&mut self) -> Reader<R, T> {
        unsafe { self.raw.read::<T>(self.len) }
    }

    /// Acquire a mapping Writer
    pub fn write(&mut self) -> Writer<R, T> {
        unsafe { self.raw.write::<T>(self.len) }
    }

    /// Acquire a mapping reader & writer
    pub fn read_write(&mut self) -> RWer<R, T> {
        unsafe { self.raw.read_write::<T>(self.len) }
    }
}

/// A service trait with methods for mapping already implemented.
/// To be used by device back ends.
#[doc(hidden)]
pub trait Builder<R: Resources>: Factory<R> {
    fn map_readable<T: Copy>(&mut self, handle::RawMapping<R>, usize) -> Readable<R, T>;
    fn map_writable<T: Copy>(&mut self, handle::RawMapping<R>, usize) -> Writable<R, T>;
    fn map_read_write<T: Copy>(&mut self, handle::RawMapping<R>, usize) -> RWable<R, T>;
}

impl<R: Resources, F: Factory<R>> Builder<R> for F {
    fn map_readable<T: Copy>(&mut self, raw: handle::RawMapping<R>, len: usize) -> Readable<R, T> {
        Readable {
            raw: raw,
            len: len,
            phantom: PhantomData,
        }
    }

    fn map_writable<T: Copy>(&mut self, raw: handle::RawMapping<R>, len: usize) -> Writable<R, T> {
        Writable {
            raw: raw,
            len: len,
            phantom: PhantomData,
        }
    }

    fn map_read_write<T: Copy>(&mut self, raw: handle::RawMapping<R>, len: usize) -> RWable<R, T> {
        RWable {
            raw: raw,
            len: len,
            phantom: PhantomData,
        }
    }
}
