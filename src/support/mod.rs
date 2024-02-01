#[deny(missing_docs)]
pub mod error_handling;

use libc::c_char;
use llvm_sys::core::{LLVMCreateMessage, LLVMDisposeMessage};
use llvm_sys::error_handling::LLVMEnablePrettyStackTrace;
use llvm_sys::support::LLVMLoadLibraryPermanently;
use llvm_sys::support::LLVMParseCommandLineOptions;
use llvm_sys::support::LLVMPrintCommitIDTo;

use std::borrow::Cow;
use std::error::Error;
use std::ffi::{CStr, CString};
use std::fmt::{self, Debug, Display, Formatter};
use std::ops::Deref;

/// An owned LLVM String. Also known as a LLVM Message
#[derive(Eq)]
pub struct LLVMString {
    pub(crate) ptr: *const c_char,
}

impl LLVMString {
    pub(crate) unsafe fn new(ptr: *const c_char) -> Self {
        LLVMString { ptr }
    }

    /// This is a convenience method for creating a Rust `String`,
    /// however; it *will* reallocate. `LLVMString` should be used
    /// as much as possible to save memory since it is allocated by
    /// LLVM. It's essentially a `CString` with a custom LLVM
    /// deallocator
    pub fn to_string(&self) -> String {
        (*self).to_string_lossy().into_owned()
    }

    /// This method will allocate a c string through LLVM
    pub(crate) fn create_from_c_str(string: &CStr) -> LLVMString {
        unsafe { LLVMString::new(LLVMCreateMessage(string.as_ptr() as *const _)) }
    }

    /// This method will allocate a c string through LLVM
    pub(crate) fn create_from_str(string: &str) -> LLVMString {
        debug_assert_eq!(string.as_bytes()[string.as_bytes().len() - 1], 0);

        unsafe { LLVMString::new(LLVMCreateMessage(string.as_ptr() as *const _)) }
    }
}

impl Deref for LLVMString {
    type Target = CStr;

    fn deref(&self) -> &Self::Target {
        unsafe { CStr::from_ptr(self.ptr) }
    }
}

impl Debug for LLVMString {
    fn fmt(&self, f: &mut Formatter) -> Result<(), fmt::Error> {
        write!(f, "{:?}", self.deref())
    }
}

impl Display for LLVMString {
    fn fmt(&self, f: &mut Formatter) -> Result<(), fmt::Error> {
        write!(f, "{:?}", self.deref())
    }
}

impl PartialEq for LLVMString {
    fn eq(&self, other: &LLVMString) -> bool {
        **self == **other
    }
}

impl Error for LLVMString {
    fn description(&self) -> &str {
        self.to_str()
            .expect("Could not convert LLVMString to str (likely invalid unicode)")
    }

    fn cause(&self) -> Option<&dyn Error> {
        None
    }
}

impl Drop for LLVMString {
    fn drop(&mut self) {
        unsafe {
            LLVMDisposeMessage(self.ptr as *mut _);
        }
    }
}

// Similar to Cow; however does not provide ability to clone
// since memory is allocated by LLVM. Could use a better name
// too. This is meant to be an internal wrapper only. Maybe
// belongs in a private utils module.
#[derive(Eq)]
pub(crate) enum LLVMStringOrRaw {
    Owned(LLVMString),
    Borrowed(*const c_char),
}

impl LLVMStringOrRaw {
    pub fn as_str(&self) -> &CStr {
        match self {
            LLVMStringOrRaw::Owned(llvm_string) => llvm_string.deref(),
            LLVMStringOrRaw::Borrowed(ptr) => unsafe { CStr::from_ptr(*ptr) },
        }
    }
}

impl PartialEq for LLVMStringOrRaw {
    fn eq(&self, other: &LLVMStringOrRaw) -> bool {
        self.as_str() == other.as_str()
    }
}

/// This function is very unsafe. Any reference to LLVM data after this function is called will likey segfault.
/// Probably only ever useful to call before your program ends. Might not even be absolutely necessary.
pub unsafe fn shutdown_llvm() {
    use llvm_sys::core::LLVMShutdown;

    LLVMShutdown()
}

pub fn parse_command_line_options(argc: i32, argv: &[&str], overview: &str) {
    let argv: Vec<String> = argv
        .into_iter()
        .map(|arg| to_null_terminated_owned(*arg))
        .collect();
    let argv: Vec<*const ::libc::c_char> = argv
        .iter()
        .map(|arg| {
            to_c_str(arg.as_str()).as_ptr()
        })
        .collect();

    let overview = to_null_terminated_owned(overview);
    let overview = to_c_str(overview.as_str());

    unsafe {
        LLVMParseCommandLineOptions(argc, argv.as_ptr(), overview.as_ptr());
    }
}

pub fn load_library_permanently(filename: &str) -> bool {
    let filename = to_null_terminated_owned(filename);
    let filename = to_c_str(filename.as_str());

    unsafe { LLVMLoadLibraryPermanently(filename.as_ptr()) == 1 }
}

pub fn get_commit_id() -> LLVMString {
    const COMMIT_HASH_LENGTH: usize = 40;

    let mut buffer = vec![0u8; COMMIT_HASH_LENGTH + 1];

    unsafe { LLVMPrintCommitIDTo(buffer.as_mut_ptr() as *mut ::libc::c_char) };
    LLVMString::create_from_str(String::from_utf8_lossy(&buffer[..COMMIT_HASH_LENGTH + 1]).as_ref())
}

/// Determines whether or not LLVM has been configured to run in multithreaded mode. (Inkwell currently does
/// not officially support multithreaded mode)
pub fn is_multithreaded() -> bool {
    use llvm_sys::core::LLVMIsMultithreaded;

    unsafe { LLVMIsMultithreaded() == 1 }
}

pub fn enable_llvm_pretty_stack_trace() {
    unsafe { LLVMEnablePrettyStackTrace() }
}

/// This function takes in a Rust string and either:
///
/// A) Finds a terminating null byte in the Rust string and can reference it directly like a C string.
///
/// B) Finds no null byte and allocates a new C string based on the input Rust string.
pub(crate) fn to_c_str<'s>(mut s: &'s str) -> Cow<'s, CStr> {
    if s.is_empty() {
        s = "\0";
    }

    // Start from the end of the string as it's the most likely place to find a null byte
    if !s.chars().rev().any(|ch| ch == '\0') {
        return Cow::from(CString::new(s).expect("unreachable since null bytes are checked"));
    }

    unsafe { Cow::from(CStr::from_ptr(s.as_ptr() as *const _)) }
}

/// Adds a null byte to the end of a Rust string if it doesn't already have one.
pub(crate) fn to_null_terminated_owned(s: &str) -> String {
    if let Some(p) = s.rfind('\0') {
        s[..=p].to_string()
    } else {
        format!("{s}\0")
    }
}

#[test]
fn test_to_c_str() {
    assert!(matches!(to_c_str("my string"), Cow::Owned(_)));
    assert!(matches!(to_c_str("my string\0"), Cow::Borrowed(_)));
}
