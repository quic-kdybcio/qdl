// SPDX-License-Identifier: BSD-3-Clause
/*
 * Copyright (c) Qualcomm Technologies, Inc. and/or its subsidiaries.
 */
//! Fixed-size path buffer, ported from src/pathbuf.c. The C prototypes are
//! declared in src/qdl_rs.h; the buffer never allocates.

use std::{
    ffi::CStr,
    path::{Component, Path, Prefix},
};

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
        // SAFETY: c_char and u8 have the same size and alignment, and buf
        // holds exactly QDL_PATH_MAX elements.
        unsafe { std::slice::from_raw_parts(self.buf.as_ptr().cast(), QDL_PATH_MAX) }
    }

    fn as_bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: c_char and u8 have the same size and alignment, and buf
        // holds exactly QDL_PATH_MAX elements.
        unsafe { std::slice::from_raw_parts_mut(self.buf.as_mut_ptr().cast(), QDL_PATH_MAX) }
    }
}

fn is_sep(ch: u8) -> bool {
    std::path::is_separator(char::from(ch))
}

/// Length of the NUL-terminated string in @s, or the whole slice if there
/// is no NUL.
fn strlen(s: &[u8]) -> usize {
    s.iter().position(|&ch| ch == 0).unwrap_or(s.len())
}

/// View @path as a std Path. Non-UTF-8 input has no Path on Windows.
#[cfg(unix)]
fn as_path(path: &[u8]) -> Option<&Path> {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

    Some(Path::new(OsStr::from_bytes(path)))
}

#[cfg(not(unix))]
fn as_path(path: &[u8]) -> Option<&Path> {
    std::str::from_utf8(path).ok().map(Path::new)
}

fn path_is_absolute(path: &[u8]) -> bool {
    as_path(path).is_some_and(Path::is_absolute)
}

/// Length of the root prefix of @path: the drive or UNC prefix, plus the
/// root separator if there is one. Zero for relative paths.
fn root_len(path: &[u8]) -> usize {
    let Some(p) = as_path(path) else {
        return 0;
    };

    let mut len = 0;
    for component in p.components() {
        match component {
            Component::Prefix(_) => len += component.as_os_str().len(),
            // A bare UNC prefix such as \\server\share also yields a RootDir
            // that is not in the bytes, so only count a separator that is.
            Component::RootDir if path.get(len).is_some_and(|&ch| is_sep(ch)) => len += 1,
            _ => break,
        }
    }
    len
}

/// Reset @path to the empty path.
///
/// # Safety
///
/// @path must be null or point to a valid struct pathbuf.
#[export]
pub unsafe fn qdl_pathbuf_reset(path: *mut Pathbuf) {
    // SAFETY: the caller guarantees path is null or valid.
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
    if dst.is_null() || orig.is_null() {
        return;
    }

    // SAFETY: the caller guarantees both pointers are valid. ptr::copy is
    // used rather than references because dst and orig may alias.
    unsafe { std::ptr::copy(orig, dst, 1) };
}

/// Append @component to @path, inserting a '/' separator when needed.
/// Leading "./" and duplicate separators in @component are dropped, and an
/// absolute @component replaces the current path.
///
/// Returns 0 on success, -EINVAL for null arguments or a component that is not
/// valid UTF-8 on Windows, and -ENAMETOOLONG if the result does not fit.
///
/// # Safety
///
/// @path must be null or point to a valid struct pathbuf, and @component
/// must be null or a NUL-terminated string.
#[export]
pub unsafe fn qdl_pathbuf_push(path: *mut Pathbuf, component: *const c_char) -> c_int {
    // SAFETY: the caller guarantees path is null or valid.
    let (Some(path), false) = (unsafe { path.as_mut() }, component.is_null()) else {
        return -EINVAL;
    };

    // SAFETY: the caller guarantees component is a NUL-terminated string.
    let component = unsafe { CStr::from_ptr(component) }.to_bytes();
    if component.is_empty() {
        return 0;
    }
    // Non-UTF-8 has no Path on Windows, so the absolute check below would
    // silently treat an absolute component as relative.
    if as_path(component).is_none() {
        return -EINVAL;
    }

    let mut path_len = path.len;
    if path_is_absolute(component) {
        path_len = 0;
    }

    let mut skip = 0;
    if component[0] == b'.' && component.get(1).is_some_and(|&ch| is_sep(ch)) {
        skip += 2;
        // Drop separators after "./" too, so ".//x" stays relative.
        while component.get(skip).is_some_and(|&ch| is_sep(ch)) {
            skip += 1;
        }
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
    // SAFETY: the caller guarantees path is null or valid.
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
    // SAFETY: the caller guarantees path is null or valid.
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
    let root_len = root_len(&buf[..strlen(buf)]);

    while len > root_len && is_sep(buf[len - 1]) {
        len -= 1;
        buf[len] = 0;
    }

    let Some(sep) = buf[..strlen(buf)].iter().rposition(|&ch| is_sep(ch)) else {
        path.len = len;
        return;
    };

    // A UNC share is its own root without a trailing separator, so cut it
    // at the end of the prefix: \\server\share\dir -> \\server\share.
    let unc_prefix_len = as_path(&buf[..strlen(buf)])
        .and_then(|p| p.components().next())
        .and_then(|c| match c {
            Component::Prefix(p)
                if matches!(p.kind(), Prefix::UNC(..) | Prefix::VerbatimUNC(..)) =>
            {
                Some(c.as_os_str().len())
            }
            _ => None,
        });

    let cut = if sep >= root_len {
        sep
    } else {
        unc_prefix_len.unwrap_or(root_len)
    };
    buf[cut] = 0;
    path.len = cut;
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;

    use super::*;

    fn make(s: &str, set_len: bool) -> Pathbuf {
        let mut path = Pathbuf {
            buf: [0; QDL_PATH_MAX],
            len: 0,
        };
        path.as_bytes_mut()[..s.len()].copy_from_slice(s.as_bytes());
        path.len = if set_len { s.len() } else { 0 };
        path
    }

    fn text(path: &Pathbuf) -> &str {
        CStr::from_bytes_until_nul(path.as_bytes())
            .unwrap()
            .to_str()
            .unwrap()
    }

    fn push(path: &mut Pathbuf, component: &str) -> c_int {
        let component = CString::new(component).unwrap();
        // SAFETY: path is a valid Pathbuf and component is NUL-terminated.
        unsafe { qdl_pathbuf_push(path, component.as_ptr()) }
    }

    fn assert_dirname(input: &str, expected: &str, set_len: bool) {
        let mut path = make(input, set_len);
        // SAFETY: path is a valid Pathbuf.
        unsafe { qdl_pathbuf_dirname(&mut path) };

        assert_eq!(text(&path), expected);
        assert_eq!(path.len, expected.len());
    }

    #[test]
    fn dirname_posix_root() {
        assert_dirname("/", "/", true);
        assert_dirname("/", "/", false);
    }

    #[test]
    fn dirname_posix_absolute() {
        assert_dirname("/foo", "/", true);
        assert_dirname("/foo/bar", "/foo", true);
        assert_dirname("/foo/bar/", "/foo", true);
    }

    #[test]
    fn dirname_relative() {
        assert_dirname("foo", "foo", true);
        assert_dirname("foo/bar", "foo", true);
        assert_dirname("foo/bar/", "foo", true);
    }

    #[test]
    fn reset_and_dup() {
        let src = make("/tmp/example", true);
        let mut dst = make("", false);

        // SAFETY: both arguments are valid, distinct Pathbufs.
        unsafe { qdl_pathbuf_dup(&mut dst, &src) };
        assert_eq!(text(&dst), text(&src));
        assert_eq!(dst.len, src.len);

        // SAFETY: dst is a valid Pathbuf.
        unsafe { qdl_pathbuf_reset(&mut dst) };
        assert_eq!(text(&dst), "");
        assert_eq!(dst.len, 0);

        // SAFETY: a null pointer is explicitly allowed.
        unsafe { qdl_pathbuf_reset(std::ptr::null_mut()) };
    }

    #[test]
    fn push_basic_behavior() {
        let mut path = make("", false);

        assert_eq!(push(&mut path, "foo"), 0);
        assert_eq!(text(&path), "foo");
        assert_eq!(path.len, "foo".len());

        assert_eq!(push(&mut path, "bar"), 0);
        assert_eq!(text(&path), "foo/bar");
        assert_eq!(path.len, "foo/bar".len());

        assert_eq!(push(&mut path, ""), 0);
        assert_eq!(text(&path), "foo/bar");
        assert_eq!(path.len, "foo/bar".len());

        assert_eq!(push(&mut path, "./baz"), 0);
        assert_eq!(text(&path), "foo/bar/baz");
        assert_eq!(path.len, "foo/bar/baz".len());

        #[cfg(windows)]
        {
            assert_eq!(push(&mut path, ".\\qux"), 0);
            assert_eq!(text(&path), "foo/bar/baz/qux");

            assert_eq!(push(&mut path, "nested\\leaf"), 0);
            assert_eq!(text(&path), "foo/bar/baz/qux/nested/leaf");

            assert_eq!(push(&mut path, "C:/abs"), 0);
            assert_eq!(text(&path), "C:/abs");
            assert_eq!(path.len, "C:/abs".len());
        }
        #[cfg(not(windows))]
        {
            assert_eq!(push(&mut path, "/abs"), 0);
            assert_eq!(text(&path), "/abs");
            assert_eq!(path.len, "/abs".len());
        }
    }

    #[test]
    fn push_dot_slash_separators() {
        let mut path = make("", false);

        assert_eq!(push(&mut path, ".//etc"), 0);
        assert_eq!(text(&path), "etc");

        let mut path = make("a", true);
        assert_eq!(push(&mut path, ".//b"), 0);
        assert_eq!(text(&path), "a/b");
    }

    #[test]
    fn push_invalid_args() {
        let mut path = make("", false);
        let component = c"foo";

        // SAFETY: a null pointer is explicitly allowed for path.
        let ret = unsafe { qdl_pathbuf_push(std::ptr::null_mut(), component.as_ptr()) };
        assert_eq!(ret, -EINVAL);

        // SAFETY: a null pointer is explicitly allowed for component.
        let ret = unsafe { qdl_pathbuf_push(&mut path, std::ptr::null()) };
        assert_eq!(ret, -EINVAL);
    }

    #[test]
    fn push_overflow_keeps_state() {
        let mut path = make("", false);
        path.as_bytes_mut()[..QDL_PATH_MAX - 1].fill(b'a');
        path.len = QDL_PATH_MAX - 1;
        let before = path.buf;

        assert_eq!(push(&mut path, "b"), -ENAMETOOLONG);
        assert_eq!(path.buf, before);
        assert_eq!(path.len, QDL_PATH_MAX - 1);
    }

    #[cfg(windows)]
    #[test]
    fn dirname_windows_drive() {
        assert_dirname("C:\\", "C:\\", true);
        assert_dirname("C:\\foo", "C:\\", true);
        assert_dirname("C:\\foo\\bar", "C:\\foo", true);
        assert_dirname("C:/foo/bar", "C:/foo", true);
    }

    #[cfg(windows)]
    #[test]
    fn dirname_windows_unc() {
        assert_dirname("\\\\server\\share", "\\\\server\\share", true);
        assert_dirname("\\\\server\\share\\dir", "\\\\server\\share", true);
        assert_dirname(
            "\\\\server\\share\\dir\\sub",
            "\\\\server\\share\\dir",
            true,
        );
    }
}
