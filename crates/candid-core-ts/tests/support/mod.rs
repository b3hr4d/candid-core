//! Small-stack harness shared by the generator's integration tests
//! (issue #218).
//!
//! A stack overflow aborts the whole process and cannot be caught, so a test
//! that could overflow re-runs itself in a child process — the pattern of the
//! root crate's `tests/deep_nesting.rs` — and asserts the child's exit status:
//! a regression shows as one failed test, with the child's signal in the
//! message, not as a killed test binary.

use std::process::Command;
use std::time::{Duration, Instant};

/// The thread stack generation must fit in, the same value as the root
/// crate's `tests/deep_nesting.rs`. The Windows test runtime itself needs
/// more than 64 KiB; 512 KiB is still far below its default stack.
#[cfg(not(windows))]
pub const SMALL_STACK_BYTES: usize = 64 * 1024;
#[cfg(windows)]
pub const SMALL_STACK_BYTES: usize = 512 * 1024;

/// Set in the child process to the name of the test it runs.
const CHILD: &str = "CANDID_CORE_TS_SMALL_STACK_CHILD";

/// Set in the child process to a file it creates once `body` has returned.
const MARKER: &str = "CANDID_CORE_TS_SMALL_STACK_MARKER";

/// How long a child may run before it counts as not terminating. A hang
/// guard, far above what any child needs, not a timing measurement.
const HANG_GUARD: Duration = Duration::from_secs(600);

/// Run `body` in a child process: this test binary re-run with `--exact
/// test`. In the child, `body` runs on the test thread; it puts whatever
/// must fit a small stack on [`on_small_stack`].
///
/// `test` must be the calling test's name. The child proves it ran `body`
/// by creating a marker file once `body` returns, and the parent requires
/// it: a name that matches no test makes the child run zero tests and exit
/// successfully, and without the marker that would pass silently.
pub fn in_child_process(test: &str, body: fn()) {
    if std::env::var(CHILD).is_ok_and(|running| running == test) {
        body();
        let marker = std::env::var(MARKER).expect("the parent names the marker file");
        std::fs::write(&marker, test).expect("the child must write its marker");
        return;
    }
    let marker = std::env::temp_dir().join(format!(
        "candid-core-ts-small-stack-{}-{test}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&marker);
    let mut child = Command::new(std::env::current_exe().expect("the test binary has a path"))
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(CHILD, test)
        .env(MARKER, &marker)
        .spawn()
        .expect("the child test process must start");
    let deadline = Instant::now() + HANG_GUARD;
    let status = loop {
        if let Some(status) = child.try_wait().expect("the child must be waitable") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("`{test}` did not finish within {HANG_GUARD:?}: it does not terminate");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        status.success(),
        "`{test}` failed in its child process (an abort is a stack overflow): {status}"
    );
    let ran = std::fs::read_to_string(&marker).is_ok_and(|written| written == test);
    let _ = std::fs::remove_file(&marker);
    assert!(
        ran,
        "`{test}` ran no test body in its child process: the name passed to \
         `in_child_process` must be the calling test's"
    );
}

/// Run `work` on a fresh thread of [`SMALL_STACK_BYTES`] and return its
/// result; a panic on that thread fails the calling test.
pub fn on_small_stack<T, F>(work: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    std::thread::Builder::new()
        .stack_size(SMALL_STACK_BYTES)
        .spawn(work)
        .expect("the small-stack thread must start")
        .join()
        .expect("the small-stack thread must not panic")
}
