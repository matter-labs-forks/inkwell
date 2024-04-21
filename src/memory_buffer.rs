use llvm_sys::core::{
    LLVMCreateMemoryBufferWithContentsOfFile, LLVMCreateMemoryBufferWithMemoryRange,
    LLVMCreateMemoryBufferWithMemoryRangeCopy, LLVMCreateMemoryBufferWithSTDIN, LLVMDisposeMemoryBuffer,
    LLVMGetBufferSize, LLVMGetBufferStart,
};
use llvm_sys::linker::LLVMLinkMemoryBuffers;
use llvm_sys::object::LLVMCreateObjectFile;
use llvm_sys::prelude::LLVMMemoryBufferRef;

use crate::object_file::ObjectFile;
use crate::support::{to_c_str, LLVMString};

use std::mem::{forget, MaybeUninit};
use std::path::Path;
use std::ptr;
use std::slice;

#[derive(Debug)]
pub struct MemoryBuffer {
    pub(crate) memory_buffer: LLVMMemoryBufferRef,
}

impl MemoryBuffer {
    pub unsafe fn new(memory_buffer: LLVMMemoryBufferRef) -> Self {
        assert!(!memory_buffer.is_null());

        MemoryBuffer { memory_buffer }
    }

    pub fn as_mut_ptr(&self) -> LLVMMemoryBufferRef {
        self.memory_buffer
    }

    pub fn create_from_file(path: &Path) -> Result<Self, LLVMString> {
        let path = to_c_str(path.to_str().expect("Did not find a valid Unicode path string"));
        let mut memory_buffer = ptr::null_mut();
        let mut err_string = MaybeUninit::uninit();

        let return_code = unsafe {
            LLVMCreateMemoryBufferWithContentsOfFile(
                path.as_ptr() as *const ::libc::c_char,
                &mut memory_buffer,
                err_string.as_mut_ptr(),
            )
        };

        // TODO: Verify 1 is error code (LLVM can be inconsistent)
        if return_code == 1 {
            unsafe {
                return Err(LLVMString::new(err_string.assume_init()));
            }
        }

        unsafe { Ok(MemoryBuffer::new(memory_buffer)) }
    }

    pub fn create_from_stdin() -> Result<Self, LLVMString> {
        let mut memory_buffer = ptr::null_mut();
        let mut err_string = MaybeUninit::uninit();

        let return_code = unsafe { LLVMCreateMemoryBufferWithSTDIN(&mut memory_buffer, err_string.as_mut_ptr()) };

        // TODO: Verify 1 is error code (LLVM can be inconsistent)
        if return_code == 1 {
            unsafe {
                return Err(LLVMString::new(err_string.assume_init()));
            }
        }

        unsafe { Ok(MemoryBuffer::new(memory_buffer)) }
    }

    /// This function is likely slightly cheaper than `create_from_memory_range_copy` since it intentionally
    /// leaks data to LLVM so that it doesn't have to reallocate. `create_from_memory_range_copy` may be removed
    /// in the future
    pub fn create_from_memory_range(input: &[u8], name: &str) -> Self {
        let name_c_string = to_c_str(name);

        let memory_buffer = unsafe {
            LLVMCreateMemoryBufferWithMemoryRange(
                input.as_ptr() as *const ::libc::c_char,
                input.len(),
                name_c_string.as_ptr(),
                false as i32,
            )
        };

        unsafe { MemoryBuffer::new(memory_buffer) }
    }

    /// This will create a new `MemoryBuffer` from the given input.
    ///
    /// This function is likely slightly more expensive than `create_from_memory_range` since it does not leak
    /// data to LLVM, forcing LLVM to make a copy. This function may be removed in the future in favor of
    /// `create_from_memory_range`
    pub fn create_from_memory_range_copy(input: &[u8], name: &str) -> Self {
        let name_c_string = to_c_str(name);

        let memory_buffer = unsafe {
            LLVMCreateMemoryBufferWithMemoryRangeCopy(
                input.as_ptr() as *const ::libc::c_char,
                input.len(),
                name_c_string.as_ptr(),
            )
        };

        unsafe { MemoryBuffer::new(memory_buffer) }
    }

    /// Gets a byte slice of this `MemoryBuffer`.
    pub fn as_slice(&self) -> &[u8] {
        unsafe {
            let start = LLVMGetBufferStart(self.memory_buffer);

            slice::from_raw_parts(start as *const _, self.get_size())
        }
    }

    /// Gets the byte size of this `MemoryBuffer`.
    pub fn get_size(&self) -> usize {
        unsafe { LLVMGetBufferSize(self.memory_buffer) }
    }

    /// Convert this `MemoryBuffer` into an `ObjectFile`. LLVM does not currently
    /// provide any way to determine the cause of error if conversion fails.
    pub fn create_object_file(self) -> Result<ObjectFile, ()> {
        let object_file = unsafe { LLVMCreateObjectFile(self.memory_buffer) };

        forget(self);

        if object_file.is_null() {
            return Err(());
        }

        unsafe { Ok(ObjectFile::new(object_file)) }
    }

    /// Links multiple memory buffers.
    /// Used for the EVM target only.
    #[cfg(all(feature = "target-evm", feature = "llvm17-0"))]
    pub fn link_memory_buffers(buffers: &[&Self], lld_args: &[&str]) -> Result<Self, ()> {
        let buffer_ptrs: Vec<LLVMMemoryBufferRef> = buffers.iter().map(|buffer| buffer.memory_buffer).collect();
        let buffer_ptrs_ptr = buffer_ptrs.as_ptr();
        let buffer_ptrs_len = buffer_ptrs.len() as u32;

        let lld_args_length = lld_args.len() as u32;
        let lld_args: Vec<String> = lld_args
            .into_iter()
            .map(|arg| crate::support::to_null_terminated_owned(*arg))
            .collect();
        let lld_args: Vec<*const ::libc::c_char> = lld_args
            .iter()
            .map(|arg| {
                to_c_str(arg.as_str()).as_ptr()
            })
            .collect();

        let mut output_buffer = ptr::null_mut();

        let status = unsafe {
            LLVMLinkMemoryBuffers(
                buffer_ptrs_ptr as *const LLVMMemoryBufferRef,
                buffer_ptrs_len,
                &mut output_buffer,
                lld_args.as_ptr(),
                lld_args_length,
            )
        };

        if status == 0 {
            return Err(());
        }

        Ok(unsafe { Self::new(output_buffer) })
    }
}

impl Drop for MemoryBuffer {
    fn drop(&mut self) {
        unsafe {
            LLVMDisposeMemoryBuffer(self.memory_buffer);
        }
    }
}
