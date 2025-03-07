//! Completion queue and Work completion.

mod exp;
mod wc;

use std::fmt;
use std::io::{self, Error as IoError};
use std::mem::MaybeUninit;
use std::ptr::{self, NonNull};
use std::sync::Arc;

use thiserror::Error;

#[cfg(mlnx4)]
pub use self::exp::*;
pub use self::wc::*;
use super::context::Context;
use crate::bindings::*;
use crate::utils::interop::from_c_ret;

/// Wrapper for `*mut ibv_cq`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub(crate) struct IbvCq(Option<NonNull<ibv_cq>>);

impl IbvCq {
    /// Destroy the CQ.
    ///
    /// # Safety
    ///
    /// - A CQ must not be destroyed more than once.
    /// - Destroyed CQs must not be used anymore.
    pub unsafe fn destroy(self) -> io::Result<()> {
        // SAFETY: FFI.
        let ret = ibv_destroy_cq(self.as_ptr());
        from_c_ret(ret)
    }
}

impl_ibv_wrapper_traits!(ibv_cq, IbvCq);

/// Ownership holder of completion queue.
struct CqInner {
    ctx: Context,
    cq: IbvCq,
}

impl Drop for CqInner {
    fn drop(&mut self) {
        // SAFETY: call only once, and no UAF since I will be dropped.
        unsafe { self.cq.destroy() }.expect("cannot destroy CQ on drop");
    }
}

/// Completion queue.
#[derive(Clone)]
pub struct Cq {
    /// Cached CQ pointer.
    cq: IbvCq,

    /// CQ body.
    inner: Arc<CqInner>,
}

impl fmt::Debug for Cq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("Cq<{:p}>", self.as_raw()))
    }
}

impl Cq {
    /// The default CQ depth.
    pub const DEFAULT_CQ_DEPTH: u32 = 128;

    /// Create a new completion queue.
    pub fn new(ctx: &Context, capacity: u32) -> Result<Cq, CqCreationError> {
        let max_capacity = ctx.attr().max_cqe as u32;
        if capacity > max_capacity {
            return Err(CqCreationError::TooManyCqes(max_capacity));
        }

        // SAFETY: FFI.
        let cq = unsafe {
            ibv_create_cq(
                ctx.as_raw(),
                capacity as i32,
                ptr::null_mut(),
                ptr::null_mut(),
                0,
            )
        };
        let cq = NonNull::new(cq).ok_or_else(IoError::last_os_error)?;
        let cq = IbvCq::from(cq);

        Ok(Self {
            inner: Arc::new(CqInner {
                ctx: ctx.clone(),
                cq,
            }),
            cq,
        })
    }

    /// Create a new completion queue with experimental features.
    #[cfg(mlnx4)]
    pub fn new_exp(ctx: &Context, capacity: u32) -> Result<Cq, CqCreationError> {
        let max_capacity = ctx.attr().max_cqe as u32;
        if capacity > max_capacity {
            return Err(CqCreationError::TooManyCqes(max_capacity));
        }

        // SAFETY: FFI.
        let cq = unsafe {
            let mut init_attr = ibv_exp_cq_init_attr {
                comp_mask: IBV_EXP_CQ_INIT_ATTR_FLAGS,
                flags: IBV_EXP_CQ_TIMESTAMP,
                ..Default::default()
            };

            ibv_exp_create_cq(
                ctx.as_raw(),
                capacity as i32,
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                &mut init_attr,
            )
        };
        let cq = NonNull::new(cq).ok_or_else(IoError::last_os_error)?;
        let cq = IbvCq::from(cq);

        Ok(Self {
            inner: Arc::new(CqInner {
                ctx: ctx.clone(),
                cq,
            }),
            cq,
        })
    }

    /// Get the underlying [`ibv_cq`] pointer.
    pub fn as_raw(&self) -> *mut ibv_cq {
        self.cq.as_ptr()
    }

    /// Get the underlying [`Context`].
    pub fn context(&self) -> &Context {
        &self.inner.ctx
    }

    /// Get the capacity of the completion queue.
    #[inline]
    pub fn capacity(&self) -> u32 {
        // SAFETY: the pointer is valid as long as the `Cq` is alive.
        (unsafe { (*self.cq.as_ptr()).cqe }) as u32
    }

    /// Non-blockingly poll. Return the work completions polled.
    ///
    /// It is the caller's responsibility to check the status codes of the
    /// returned work completion entries.
    ///
    /// **NOTE:** This method will try to poll as many completions as possible
    /// from the completion queue, incurring allocation overheads. For a more
    /// efficient poll with a smaller pre-allocated buffer, use `poll_some` or
    /// `poll_into`.
    #[inline]
    pub fn poll(&self) -> io::Result<Vec<Wc>> {
        self.poll_some(self.capacity())
    }

    /// Non-blockingly poll with a limited number of expected work completions.
    /// Return the work completions polled.
    /// This method should be preferred over `poll` when possible to avoid
    /// unnecessary allocation overheads.
    ///
    /// It is the caller's responsibility to check the status codes of the
    /// returned work completion entries.
    #[inline]
    pub fn poll_some(&self, num: u32) -> io::Result<Vec<Wc>> {
        let mut wc = <Vec<Wc>>::with_capacity(num as usize);

        // SAFETY: FFI, and that `Wc` is transparent over `ibv_wc`.
        let num = unsafe { ibv_poll_cq(self.as_raw(), num as i32, wc.as_mut_ptr().cast()) };
        if num >= 0 {
            unsafe { wc.set_len(num as usize) };
            Ok(wc)
        } else {
            Err(io::Error::from_raw_os_error(num))
        }
    }

    /// Non-blockingly poll one work completion. Return the work completion
    /// polled.
    /// This method should be preferred over `poll` and `poll_some` when you
    /// only have one work completion to poll to avoid all unnecessary
    /// overheads.
    ///
    /// It is the caller's responsibility to check the status codes of the
    /// returned work completion entry.
    #[inline(always)]
    pub fn poll_one(&self) -> io::Result<Option<Wc>> {
        let mut wc = <MaybeUninit<Wc>>::uninit();
        // SAFETY: FFI
        let num = unsafe { ibv_poll_cq(self.as_raw(), 1, wc.as_mut_ptr().cast()) };
        if num >= 0 {
            Ok(if num == 0 {
                None
            } else {
                // SAFETY: `ibv_poll_cq` returning 1 means `wc` is initialized.
                Some(unsafe { wc.assume_init() })
            })
        } else {
            Err(io::Error::from_raw_os_error(num))
        }
    }

    /// Non-blockingly poll into the given buffer. Return the number of work
    /// completions polled.
    ///
    /// It is the caller's responsibility to check the status codes of the
    /// returned work completion entries.
    ///
    /// **NOTE:** It is possible that the number of polled work completions is
    /// less than `wc.len()` or even zero. The validity of work completions
    /// beyond the number of polled work completions is not guaranteed.
    #[inline]
    pub fn poll_into(&self, wc: &mut [Wc]) -> io::Result<u32> {
        if wc.is_empty() {
            return Ok(0);
        }

        // SAFETY: FFI, and that `Wc` is transparent over `ibv_wc`.
        let num = unsafe { ibv_poll_cq(self.as_raw(), wc.len() as i32, wc.as_mut_ptr().cast()) };
        if num >= 0 {
            Ok(num as u32)
        } else {
            Err(io::Error::from_raw_os_error(num))
        }
    }

    /// Non-blockingly poll one work completion into the given work completion.
    /// Return the number of work completions polled.
    /// This method should be preferred over `poll_into` when you only have one
    /// work completion to poll to avoid all unnecessary overheads.
    ///
    /// It is the caller's responsibility to check the status codes of the
    /// returned work completion entry.
    ///
    /// **NOTE:** If this poll is not successful whether because some error
    /// occurred or simply no completion has arrived yet, the validity of the
    /// work completion is not guaranteed.
    #[inline(always)]
    pub fn poll_one_into(&self, wc: &mut Wc) -> io::Result<u32> {
        // SAFETY: FFI
        let num = unsafe { ibv_poll_cq(self.as_raw(), 1, (wc as *mut Wc).cast()) };
        if num >= 0 {
            Ok(num as u32)
        } else {
            Err(io::Error::from_raw_os_error(num))
        }
    }

    /// Blockingly poll until a given number of work completion are polled.
    ///
    /// It is the caller's responsibility to check the status codes of the
    /// returned work completion entries.
    ///
    /// When feature `warned_spin` is set, this function will print warning
    /// messages every second to the standard error.
    pub fn poll_blocking(&self, num: u32) -> io::Result<Vec<Wc>> {
        #[cfg(not(feature = "warned_spin"))]
        fn do_poll(cq: &Cq, num: u32, wc: &mut Vec<Wc>) -> io::Result<()> {
            while wc.len() < (num as usize) {
                let extra = cq.poll_some(num - wc.len() as u32)?;
                wc.extend(extra);
            }
            Ok(())
        }

        #[cfg(feature = "warned_spin")]
        fn do_poll(cq: &Cq, num: u32, wc: &mut Vec<Wc>) -> io::Result<()> {
            use quanta::Instant;

            let mut last_warn = Instant::now();
            while wc.len() < (num as usize) {
                let extra = cq.poll_some(num - wc.len() as u32)?;
                wc.extend(extra);

                if last_warn.elapsed().as_secs() >= 1 {
                    eprintln!("warning: spinning on CQ poll ...");
                    let bt = std::backtrace::Backtrace::capture();
                    if bt.status() == std::backtrace::BacktraceStatus::Captured {
                        eprintln!("backtrace: {:#?}", bt);
                    }

                    last_warn = Instant::now();
                }
            }
            Ok(())
        }

        let mut wc = <Vec<Wc>>::with_capacity(num as usize);
        do_poll(self, num, &mut wc)?;
        Ok(wc)
    }

    /// Blockingly poll one work completion. Return the work completion polled.
    /// This method should be preferred over `poll_blocking` when you only have
    /// one work completion to poll to avoid all unnecessary overheads.
    ///
    /// It is the caller's responsibility to check the status codes of the
    /// returned work completion entry.
    pub fn poll_one_blocking(&self) -> io::Result<Wc> {
        let mut wc = <MaybeUninit<Wc>>::uninit();
        // SAFETY: this call never reads `wc` and thus never touches
        // uninitialized data.
        self.poll_one_blocking_into(unsafe { wc.assume_init_mut() })?;
        // SAFETY: `wc` is initialized by `poll_one_blocking_into`.
        Ok(unsafe { wc.assume_init() })
    }

    /// Blockingly wait until a work completion occurs and consume that
    /// work request.
    ///
    /// ## Panics
    ///
    /// Panic if the work completion status is not success.
    pub fn poll_one_blocking_consumed(&self) {
        #[cfg(not(feature = "warned_spin"))]
        fn do_poll(cq: *mut ibv_cq, wc: &mut MaybeUninit<Wc>) {
            while unsafe { ibv_poll_cq(cq, 1, wc as *mut _ as _) } == 0 {}
        }

        #[cfg(feature = "warned_spin")]
        fn do_poll(cq: *mut ibv_cq, wc: &mut MaybeUninit<Wc>) {
            use quanta::Instant;

            let mut last_warn = Instant::now();
            while unsafe { ibv_poll_cq(cq, 1, wc as *mut _ as _) } == 0 {
                if last_warn.elapsed().as_secs() >= 1 {
                    eprintln!("warning: spinning on CQ poll ...");
                    let bt = std::backtrace::Backtrace::capture();
                    if bt.status() == std::backtrace::BacktraceStatus::Captured {
                        eprintln!("backtrace: {:#?}", bt);
                    }

                    last_warn = Instant::now();
                }
            }
        }

        // SAFETY: `Wc` is transparent over `ibv_wc`.
        let mut wc = <MaybeUninit<Wc>>::uninit();
        do_poll(self.as_raw(), &mut wc);

        // SAFETY: `wc` is initialized by `ibv_poll_cq`.
        assert_eq!(unsafe { wc.assume_init() }.status(), WcStatus::Success);
    }

    /// Blockingly poll until the given work completion buffer is filled.
    ///
    /// It is the caller's responsibility to check the status codes of the
    /// returned work completion entries.
    ///
    /// When feature `warned_spin` is set, this function will print warning
    /// messages every second to the standard error.
    ///
    /// **NOTE:** It is possible that the number of polled work completions is
    /// less than `wc.len()` or even zero. The validity of work completions
    /// beyond the number of polled work completions is not guaranteed.
    pub fn poll_blocking_into(&self, wc: &mut [Wc]) -> io::Result<()> {
        #[cfg(not(feature = "warned_spin"))]
        fn do_poll(cq: &Cq, wc: &mut [Wc]) -> io::Result<()> {
            let mut polled = 0;
            while polled < wc.len() {
                let n = cq.poll_into(&mut wc[polled..])?;
                polled += n as usize;
            }
            Ok(())
        }

        #[cfg(feature = "warned_spin")]
        fn do_poll(cq: &Cq, wc: &mut [Wc]) -> io::Result<()> {
            use quanta::Instant;

            let mut polled = 0;
            let mut last_warn = Instant::now();
            while polled < wc.len() {
                let n = cq.poll_into(&mut wc[polled..])?;
                polled += n as usize;

                if last_warn.elapsed().as_secs() >= 1 {
                    eprintln!("warning: spinning on CQ poll ...");
                    let bt = std::backtrace::Backtrace::capture();
                    if bt.status() == std::backtrace::BacktraceStatus::Captured {
                        eprintln!("backtrace: {:#?}", bt);
                    }

                    last_warn = Instant::now();
                }
            }
            Ok(())
        }
        do_poll(self, wc)?;
        Ok(())
    }

    /// Blockingly poll one work completion into the given work completion.
    ///
    /// It is the caller's responsibility to check the status codes of the
    /// returned work completion entry.
    pub fn poll_one_blocking_into(&self, wc: &mut Wc) -> io::Result<()> {
        #[cfg(not(feature = "warned_spin"))]
        fn do_poll(cq: &Cq, wc: &mut Wc) -> io::Result<()> {
            let mut polled = 0;
            while polled == 0 {
                polled = cq.poll_one_into(wc)?;
            }
            Ok(())
        }

        #[cfg(feature = "warned_spin")]
        fn do_poll(cq: &Cq, wc: &mut Wc) -> io::Result<()> {
            use quanta::Instant;

            let mut polled = 0;
            let mut last_warn = Instant::now();
            while polled == 0 {
                polled = cq.poll_one_into(wc)?;
                if last_warn.elapsed().as_secs() >= 1 {
                    eprintln!("warning: spinning on CQ poll ...");
                    let bt = std::backtrace::Backtrace::capture();
                    if bt.status() == std::backtrace::BacktraceStatus::Captured {
                        eprintln!("backtrace: {:#?}", bt);
                    }

                    last_warn = Instant::now();
                }
            }
            Ok(())
        }

        do_poll(self, wc)
    }
}

/// CQ creation error type.
#[derive(Debug, Error)]
pub enum CqCreationError {
    ///`libibverbs` interfaces returned an error.
    #[error("I/O error from ibverbs")]
    IoError(#[from] IoError),

    /// The capacity of the CQ is larger than the device's maximum allowed
    /// capacity, which is contained in the error.
    #[error("CQ capacity too large (maximum: {0})")]
    TooManyCqes(u32),
}
