//! Bridges the GTK main loop and the tokio runtime that does the network
//! work.
//!
//! GTK widgets live on the main thread; HTTP requests and disk IO
//! run on tokio's worker threads. [`background`] runs a future on tokio and
//! hands its result back to whoever awaits it on the main thread.

use std::future::Future;
use std::sync::OnceLock;

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// The tokio runtime, started on first use.
pub fn handle() -> tokio::runtime::Handle {
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(3)
                .thread_name("yt-rs-io")
                .enable_all()
                .build()
                .expect("failed to start the tokio runtime")
        })
        .handle()
        .clone()
}

/// Runs `fut` on the tokio runtime and resolves to its output. Await it from
/// the GTK main loop (e.g. inside [`spawn_local`]).
pub async fn background<T: Send + 'static>(fut: impl Future<Output = T> + Send + 'static) -> T {
    match handle().spawn(fut).await {
        Ok(value) => value,
        // A panic in a background task is a bug; surface it here.
        Err(e) => std::panic::resume_unwind(e.into_panic()),
    }
}

/// Runs a future to completion on the calling thread. Only for quick work
/// at startup; it blocks the UI.
pub fn block_on<T>(fut: impl Future<Output = T>) -> T {
    handle().block_on(fut)
}

/// Runs a future on the GTK main loop.
pub fn spawn_local(fut: impl Future<Output = ()> + 'static) {
    gtk::glib::spawn_future_local(fut);
}

thread_local! {
    static RELEASE: std::cell::RefCell<Option<gtk::glib::SourceId>> = const { std::cell::RefCell::new(None) };
}

/// Hands memory the app has freed back to the system a few seconds from
/// now (again later if called again). glibc keeps freed memory for reuse,
/// often tens of megabytes after leaving a page; other platforms'
/// allocators return it on their own.
pub fn release_memory_soon() {
    RELEASE.with(|r| {
        if let Some(pending) = r.borrow_mut().take() {
            pending.remove();
        }
        let id = gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(3), || {
            RELEASE.with(|r| r.borrow_mut().take());
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            // SAFETY: malloc_trim only works on the allocator's free lists.
            unsafe {
                libc::malloc_trim(0);
            }
        });
        *r.borrow_mut() = Some(id);
    });
}
