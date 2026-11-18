// SPDX-License-Identifier: BSD-3-Clause
/*
 * Copyright (c) Qualcomm Technologies, Inc. and/or its subsidiaries.
 */
//! Fixed-size path buffer, ported from src/pathbuf.c. The C prototypes are
//! generated into qdl_rs.h; the buffer never allocates.

use std::{ffi::CStr, fs};

use core::ffi::{c_char, c_int};

use libc::{EINVAL, ENAMETOOLONG};
use qdl_macros::export;

/// Size of the buffer, matching PATH_MAX on each supported platform. The C
/// side checks this against PATH_MAX in src/pathbuf.h.
#[cfg(windows)]
pub const QDL_PATH_MAX: usize = 260;
#[cfg(target_os = "macos")]
pub const QDL_PATH_MAX: usize = 1024;
#[cfg(not(any(windows, target_os = "macos")))]
pub const QDL_PATH_MAX: usize = 4096;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Pathbuf {
    pub buf: [c_char; QDL_PATH_MAX],
    pub len: usize,
}

impl Pathbuf {
    fn as_bytes(&self) -> &[u8] {
        // SAFETY: c_char and u8 have the same size and alignment.
        unsafe { std::slice::from_raw_parts(self.buf.as_ptr().cast(), QDL_PATH_MAX) }
    }

    fn as_bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: c_char and u8 have the same size and alignment.
        unsafe { std::slice::from_raw_parts_mut(self.buf.as_mut_ptr().cast(), QDL_PATH_MAX) }
    }
}

fn is_sep(ch: u8) -> bool {
    ch == b'/' || (cfg!(windows) && ch == b'\\')
}

/// Byte at @i of a NUL-terminated string, or 0 past its end.
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// Length of the NUL-terminated string in @s, or the whole slice if there
/// is no NUL.
fn strlen(s: &[u8]) -> usize {
    s.iter().position(|&ch| ch == 0).unwrap_or(s.len())
}

#[cfg(windows)]
fn path_is_absolute(path: &[u8]) -> bool {
    (at(path, 0).is_ascii_alphabetic() && at(path, 1) == b':')
        || (at(path, 0) == b'\\' && at(path, 1) == b'\\')
}

#[cfg(not(windows))]
fn path_is_absolute(path: &[u8]) -> bool {
    at(path, 0) == b'/'
}

#[cfg(windows)]
fn unc_root_len(path: &[u8]) -> usize {
    if !is_sep(at(path, 0)) || !is_sep(at(path, 1)) {
        return 0;
    }

    let mut i = 2;
    while is_sep(at(path, i)) {
        i += 1;
    }
    if at(path, i) == 0 {
        return 2;
    }

    let start = i;
    while at(path, i) != 0 && !is_sep(at(path, i)) {
        i += 1;
    }
    if i == start {
        return 2;
    }

    while is_sep(at(path, i)) {
        i += 1;
    }
    if at(path, i) == 0 {
        return 2;
    }

    let start = i;
    while at(path, i) != 0 && !is_sep(at(path, i)) {
        i += 1;
    }
    if i == start {
        return 2;
    }

    i
}

/// Length of the root prefix of a NUL-terminated, non-empty @path.
#[cfg(windows)]
fn root_len(path: &[u8]) -> usize {
    if at(path, 0).is_ascii_alphabetic() && at(path, 1) == b':' && is_sep(at(path, 2)) {
        return 3;
    }

    match unc_root_len(path) {
        0 if is_sep(at(path, 0)) => 1,
        len => len,
    }
}

#[cfg(not(windows))]
fn root_len(path: &[u8]) -> usize {
    usize::from(at(path, 0) == b'/')
}

#[cfg(unix)]
fn path_exists(name: &[u8]) -> bool {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

    fs::metadata(OsStr::from_bytes(name)).is_ok()
}

#[cfg(not(unix))]
fn path_exists(name: &[u8]) -> bool {
    std::str::from_utf8(name).is_ok_and(|name| fs::metadata(name).is_ok())
}

/// Reset @path to the empty path.
///
/// # Safety
///
/// @path must be null or point to a valid struct pathbuf.
#[export]
pub unsafe fn qdl_pathbuf_reset(path: *mut Pathbuf) {
    let Some(path) = (unsafe { path.as_mut() }) else {
        return;
    };

    path.as_bytes_mut()[0] = 0;
    path.len = 0;
}

/// Copy the whole @orig buffer into @dst.
///
/// # Safety
///
/// Both pointers must be null or point to valid struct pathbufs.
#[export]
pub unsafe fn qdl_pathbuf_dup(dst: *mut Pathbuf, orig: *const Pathbuf) {
    if let (Some(dst), Some(orig)) = unsafe { (dst.as_mut(), orig.as_ref()) } {
        *dst = *orig;
    }
}

/// Append @component to @path, inserting a '/' separator when needed.
/// Leading "./" and duplicate separators in @component are dropped, and an
/// absolute @component replaces the current path.
///
/// Returns 0 on success, -EINVAL for null arguments and -ENAMETOOLONG if the
/// result does not fit.
///
/// # Safety
///
/// @path must be null or point to a valid struct pathbuf, and @component
/// must be null or a NUL-terminated string.
#[export]
pub unsafe fn qdl_pathbuf_push(path: *mut Pathbuf, component: *const c_char) -> c_int {
    let (Some(path), false) = (unsafe { path.as_mut() }, component.is_null()) else {
        return -EINVAL;
    };

    let component = unsafe { CStr::from_ptr(component) }.to_bytes();
    if component.is_empty() {
        return 0;
    }

    let mut path_len = path.len;
    if path_is_absolute(component) {
        path_len = 0;
    }

    let mut skip = 0;
    if component[0] == b'.' && component.get(1).is_some_and(|&ch| is_sep(ch)) {
        skip += 2;
    }

    let need_sep = path_len > 0 && !is_sep(path.as_bytes()[path_len - 1]);
    if path_len > 0 && !need_sep {
        while component.get(skip).is_some_and(|&ch| is_sep(ch)) {
            skip += 1;
        }
    }

    let rest = &component[skip..];
    if rest.is_empty() {
        return 0;
    }
    if path_len + usize::from(need_sep) + rest.len() + 1 > QDL_PATH_MAX {
        return -ENAMETOOLONG;
    }

    let buf = path.as_bytes_mut();
    let mut pos = path_len;
    if need_sep {
        buf[pos] = b'/';
        pos += 1;
    }
    for (i, &ch) in rest.iter().enumerate() {
        buf[pos + i] = if is_sep(ch) { b'/' } else { ch };
    }
    buf[pos + rest.len()] = 0;
    path.len = pos + rest.len();

    0
}

/// Return the NUL-terminated string held by @path, or NULL if @path is NULL.
///
/// # Safety
///
/// @path must be null or point to a valid struct pathbuf.
#[export]
pub unsafe fn qdl_pathbuf_str(path: *const Pathbuf) -> *const c_char {
    match unsafe { path.as_ref() } {
        Some(path) => path.buf.as_ptr(),
        None => std::ptr::null(),
    }
}

/// Strip the last component of @path, keeping the root if there is one.
///
/// # Safety
///
/// @path must be null or point to a valid struct pathbuf.
#[export]
pub unsafe fn qdl_pathbuf_dirname(path: *mut Pathbuf) {
    let Some(path) = (unsafe { path.as_mut() }) else {
        return;
    };
    if path.as_bytes()[0] == 0 {
        return;
    }
    if path.len == 0 {
        path.len = strlen(path.as_bytes());
    }

    let mut len = path.len;
    let buf = path.as_bytes_mut();
    let root_len = root_len(buf);

    while len > root_len && is_sep(buf[len - 1]) {
        len -= 1;
        buf[len] = 0;
    }

    let Some(sep) = buf[..strlen(buf)].iter().rposition(|&ch| is_sep(ch)) else {
        path.len = len;
        return;
    };

    if sep < root_len {
        buf[root_len] = 0;
        len = root_len;
    } else {
        buf[sep] = 0;
        len = sep;
    }
    path.len = len;
}

/// Return true if the path held by @path exists.
///
/// # Safety
///
/// @path must be null or point to a valid struct pathbuf.
#[export]
pub unsafe fn qdl_path_exists(path: *const Pathbuf) -> bool {
    let Some(path) = (unsafe { path.as_ref() }) else {
        return false;
    };

    let name = &path.as_bytes()[..strlen(path.as_bytes())];
    if name.is_empty() {
        return false;
    }

    path_exists(name)
}
