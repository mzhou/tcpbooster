use std::os::unix::io::RawFd;

use libc::{c_void, getsockopt, setsockopt, socklen_t, IPPROTO_TCP, TCP_MAXSEG};
use nix::sys::socket::{GetSockOpt, SetSockOpt};

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TcpMaxSeg;

impl GetSockOpt for TcpMaxSeg {
    type Val = i32;

    fn get(&self, fd: RawFd) -> nix::Result<Self::Val> {
        unsafe {
            let mut val = Self::Val::default();
            let mut len = core::mem::size_of::<Self::Val>() as socklen_t;
            let res = getsockopt(
                fd,
                IPPROTO_TCP,
                TCP_MAXSEG,
                &mut val as *mut _ as *mut c_void,
                &mut len as *mut socklen_t,
            );
            nix::errno::Errno::result(res)?;
            Ok(val)
        }
    }
}

impl SetSockOpt for TcpMaxSeg {
    type Val = i32;

    fn set(&self, fd: RawFd, val: &Self::Val) -> nix::Result<()> {
        unsafe {
            let mut ffi_val = val;
            let len = core::mem::size_of::<Self::Val>() as socklen_t;
            let res = setsockopt(
                fd,
                IPPROTO_TCP,
                TCP_MAXSEG,
                &mut ffi_val as *mut _ as *mut c_void,
                len,
            );
            nix::errno::Errno::result(res)?;
            Ok(())
        }
    }
}
