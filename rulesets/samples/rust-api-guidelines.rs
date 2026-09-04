// Synthetic sample for rulesets/rust-api-guidelines.toml — never compiled,
// only needs to parse. Each rule gets a violation, and a compliant form next
// to it where that's cheap to show.

#![deny(warnings)]
#![deny(unsafe_code)]

// C-STRUCT-PRIVATE — a public field is API surface forever.
#[derive(Debug)]
pub struct Config {
    pub timeout_ms: u32,
}

#[derive(Debug)]
pub struct ConfigPrivate {
    timeout_ms: u32,
}

// C-CUSTOM-TYPE — bool/Option arguments read as noise at the call site.
pub fn connect(retry: bool) {}

pub fn set_timeout(timeout: Option<u32>) {}

pub fn connect_mode(mode: ConnectMode) {}

#[derive(Debug)]
pub struct ConnectMode;

// C-NO-OUT — an out-parameter instead of a return value.
pub fn compute_into(out: &mut i32) {
    *out = 42;
}

pub fn compute() -> i32 {
    42
}

// C-CTOR — `new` is conventionally infallible, takes no `self`, returns `Self`.
#[derive(Debug)]
pub struct Widget;

impl Widget {
    pub fn new() -> Self {
        Widget
    }

    pub fn with_capacity(_n: usize) -> Self {
        Widget
    }
}

// C-DTOR-FAIL — a panicking `Drop::drop` aborts if it runs during unwinding.
pub struct FileHandle;

impl Drop for FileHandle {
    fn drop(&mut self) {
        self.flush().unwrap();
    }
}

impl FileHandle {
    fn flush(&self) -> Result<(), ()> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct FileHandleSafe;

impl Drop for FileHandleSafe {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

impl FileHandleSafe {
    fn flush(&self) -> Result<(), ()> {
        Ok(())
    }
}

// anti-pattern — Deref polymorphism fakes inheritance Rust doesn't have.
pub struct Wrapper(Inner);

impl std::ops::Deref for Wrapper {
    type Target = Inner;

    fn deref(&self) -> &Inner {
        &self.0
    }
}

#[derive(Debug)]
pub struct WrapperExplicit(Inner);

impl WrapperExplicit {
    pub fn inner(&self) -> &Inner {
        &self.0
    }
}

#[derive(Debug)]
pub struct Inner;

// C-DEBUG — a public type without Debug can't be inspected with `{:?}`.
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug)]
pub struct PointDebug {
    x: i32,
    y: i32,
}
