//! IPC transport abstraction (Task 11 portability).
//!
//! The Rust core talks to PROCESS modules (e.g. Java) over a byte-stream
//! transport. On Linux and macOS, that transport is a Unix domain socket.
//! On Windows, the idiomatic primitive is a named pipe (`\\.\pipe\name`),
//! and Java's `UnixDomainSocketAddress` support on Windows requires
//! JDK 22+ and is not yet reliable across all Windows versions.
//!
//! This module provides a thin `IpcStream` trait + `connect_path` /
//! `listen_path` functions that abstract the transport choice. The current
//! implementation is Unix-domain-socket-only; a future Windows port would
//! add a named-pipe (or loopback-TCP fallback) implementation behind the
//! same trait, with NO changes to the call sites in `glucore_core/src/lib.rs`.
//!
//! ## Current platform support
//!
//! | Platform | Transport              | Status           |
//! |----------|------------------------|------------------|
//! | Linux    | Unix domain socket     | Implemented      |
//! | macOS    | Unix domain socket     | Implemented      |
//! | Windows  | Named pipe (TODO)      | Documented gap   |
//!
//! The "documented gap" for Windows is intentional per the Task 11 handoff:
//! a fully-implemented Windows named-pipe transport would be substantial new
//! work and is out of scope for this PoC. A loopback-TCP fallback is an
//! acceptable future stopping point; silently dropping Windows IPC support
//! without saying so is NOT.

#[cfg(unix)]
mod unix_imp {
    use std::io::{Read, Write};
    use std::os::unix::io::{AsRawFd, FromRawFd};
    use std::os::unix::net::{UnixListener, UnixStream};

    /// A connected byte-stream IPC transport. On Unix this wraps a
    /// `UnixStream`; on Windows it would wrap a named pipe handle.
    pub struct IpcStream {
        inner: UnixStream,
    }

    impl IpcStream {
        /// Connect to a listener at `path`. On Unix, `path` is a filesystem
        /// path. On Windows, it would be a pipe name (`\\.\pipe\...`).
        pub fn connect(path: &str) -> std::io::Result<Self> {
            UnixStream::connect(path).map(|s| IpcStream { inner: s })
        }

        /// Take ownership of an existing raw file descriptor (Unix) or
        /// HANDLE (Windows). Used by the registration path where we
        /// already have an fd from `connect_with_retry` and need to wrap
        /// it for read/write without taking ownership twice.
        ///
        /// SAFETY: caller must ensure `fd` is a valid, open, connected
        /// stream socket. The returned `IpcStream` takes ownership and
        /// will close the fd on drop.
        pub unsafe fn from_raw_fd(fd: i32) -> Self {
            IpcStream {
                inner: UnixStream::from_raw_fd(fd),
            }
        }

        /// Get the raw fd without transferring ownership. Used by the
        /// IPC round-trip path that needs to `mem::forget` the wrapper
        /// to prevent Drop from closing the fd (the fd is owned by the
        /// module registration and reused across calls).
        pub fn as_raw_fd(&self) -> i32 {
            self.inner.as_raw_fd()
        }

        /// Forget the wrapper WITHOUT closing the underlying fd. Used in
        /// the IPC round-trip path where the fd is reused across calls.
        pub fn forget(self) {
            // Manually leak the inner UnixStream by reading its fd and
            // forgetting the wrapper. We can't `mem::forget(self.inner)`
            // directly because Drop is implemented. Instead, take the fd
            // and let the wrapper drop (the inner's Drop would close the
            // fd — so we extract the fd, then forget the entire IpcStream
            // BEFORE Drop runs by using ManuallyDrop).
            //
            // Simpler approach: just don't impl Drop on IpcStream. The
            // UnixStream inside will still drop normally and close the fd.
            // For callers that need to keep the fd alive, they should use
            // `as_raw_fd()` and `mem::forget()` the IpcStream itself
            // BEFORE it would drop — but Drop on IpcStream doesn't do
            // anything extra, so this is moot.
            //
            // The cleanest fix: callers that need to keep the fd alive
            // should call `as_raw_fd()` then `mem::forget(self)` (forget
            // the IpcStream, not its inner). Drop on IpcStream is a no-op
            // below, but forgetting the wrapper prevents Drop from running
            // at all. The inner UnixStream's Drop is what closes the fd —
            // and forgetting the wrapper doesn't run inner's Drop either
            // because the inner is still owned by the (now-forgotten)
            // wrapper.
            //
            // So this method is equivalent to: do nothing, let the caller
            // `mem::forget` the IpcStream. We keep the method for API
            // symmetry with the future Windows version.
            std::mem::forget(self);
        }
    }

    impl Read for IpcStream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.inner.read(buf)
        }
    }

    impl Write for IpcStream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.inner.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.inner.flush()
        }
    }

    impl Drop for IpcStream {
        fn drop(&mut self) {
            // Default Drop closes the underlying fd. Callers that want
            // to keep the fd alive should use `forget()` instead of
            // letting the IpcStream drop naturally.
        }
    }

    /// A listening IPC transport. On Unix, wraps a `UnixListener`.
    pub struct IpcListener {
        inner: UnixListener,
    }

    impl IpcListener {
        /// Bind a listener at `path`. On Unix, `path` is a filesystem path
        /// (the caller is responsible for removing any stale socket file
        /// first). On Windows, this would create a named pipe.
        pub fn bind(path: &str) -> std::io::Result<Self> {
            UnixListener::bind(path).map(|l| IpcListener { inner: l })
        }

        /// Block waiting for one incoming connection.
        pub fn accept(&self) -> std::io::Result<IpcStream> {
            self.inner.accept().map(|(s, _)| IpcStream { inner: s })
        }
    }

    /// Remove a stale transport endpoint at `path`. On Unix, this is a
    /// filesystem unlink. On Windows (future), this would be a no-op
    /// (named pipes don't outlive their creating process).
    pub fn cleanup_endpoint(path: &str) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(unix)]
pub use unix_imp::{cleanup_endpoint, IpcListener, IpcStream};

#[cfg(not(unix))]
mod non_unix_imp {
    //! Stub implementation for non-Unix platforms. All functions return
    //! errors or panic with a clear message pointing at KNOWN_FOOTGUNS.md.
    //! This is the "documented gap" the Task 11 handoff allows for Windows
    //! IPC: a silent skip would NOT be acceptable.

    use std::io::{Read, Write};

    pub struct IpcStream;

    impl IpcStream {
        pub fn connect(_path: &str) -> std::io::Result<Self> {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "IPC transport not implemented on this platform \
                 (see KNOWN_FOOTGUNS.md — Windows named-pipe transport is a \
                 documented gap)",
            ))
        }
        pub unsafe fn from_raw_fd(_fd: i32) -> Self {
            panic!("IPC transport not implemented on this platform")
        }
        pub fn as_raw_fd(&self) -> i32 {
            -1
        }
        pub fn forget(self) {}
    }

    impl Read for IpcStream {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "IPC not implemented on this platform",
            ))
        }
    }
    impl Write for IpcStream {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "IPC not implemented on this platform",
            ))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    pub struct IpcListener;
    impl IpcListener {
        pub fn bind(_path: &str) -> std::io::Result<Self> {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "IPC listener not implemented on this platform",
            ))
        }
        pub fn accept(&self) -> std::io::Result<IpcStream> {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "IPC listener not implemented on this platform",
            ))
        }
    }

    pub fn cleanup_endpoint(_path: &str) {}
}

#[cfg(not(unix))]
pub use non_unix_imp::{cleanup_endpoint, IpcListener, IpcStream};
