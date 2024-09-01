use llvm_sys::core::{
    LLVMCreateMemoryBufferWithContentsOfFile, LLVMCreateMemoryBufferWithMemoryRange,
    LLVMCreateMemoryBufferWithMemoryRangeCopy, LLVMCreateMemoryBufferWithSTDIN, LLVMDisposeMemoryBuffer,
    LLVMGetBufferSize, LLVMGetBufferStart,
};
use llvm_sys::linker::{LLVMAssembleEraVM, LLVMDisassembleEraVM, LLVMExceedsSizeLimitEraVM, LLVMLinkEraVM};
use llvm_sys::object::LLVMCreateObjectFile;
use llvm_sys::prelude::LLVMMemoryBufferRef;

use crate::object_file::ObjectFile;
use crate::support::{to_c_str, LLVMString};
#[llvm_versions(13.0..=latest)]
use crate::targets::TargetMachine;

use std::mem::{forget, MaybeUninit};
use std::path::Path;
use std::ptr;
use std::slice;

#[derive(Debug)]
pub struct MemoryBuffer {
    pub(crate) memory_buffer: LLVMMemoryBufferRef,
}

impl MemoryBuffer {
    pub const ETHEREUM_ADDRESS_SIZE: usize = 20;

    pub const ERAVM_WORD_SIZE: usize = 32;

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
    pub fn create_from_memory_range(input: &[u8], name: &str, requires_null_terminator: bool) -> Self {
        let name_c_string = to_c_str(name);

        let memory_buffer = unsafe {
            LLVMCreateMemoryBufferWithMemoryRange(
                input.as_ptr() as *const ::libc::c_char,
                input.len(),
                name_c_string.as_ptr(),
                requires_null_terminator as i32,
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

    /// Translates textual assembly to the object code.
    #[cfg(all(feature = "target-eravm", feature = "llvm17-0"))]
    pub fn assemble_eravm(&self, machine: &TargetMachine) -> Result<Self, LLVMString> {
        let mut output_buffer = ptr::null_mut();
        let mut err_string = MaybeUninit::uninit();

        let return_code = unsafe {
            LLVMAssembleEraVM(
                machine.target_machine,
                self.memory_buffer,
                &mut output_buffer,
                err_string.as_mut_ptr(),
            )
        };

        if return_code == 1 {
            unsafe {
                return Err(LLVMString::new(err_string.assume_init()));
            }
        }

        Ok(unsafe { Self::new(output_buffer) })
    }

    /// Disassembles the bytecode in the buffer.
    #[cfg(all(feature = "target-eravm", feature = "llvm17-0"))]
    pub fn disassemble_eravm(&self, machine: &TargetMachine, pc: u32, options: u32) -> Result<Self, LLVMString> {
        let mut output_buffer = ptr::null_mut();
        let mut err_string = MaybeUninit::uninit();

        let return_code = unsafe {
            LLVMDisassembleEraVM(
                machine.target_machine,
                self.memory_buffer,
                pc,
                options,
                &mut output_buffer,
                err_string.as_mut_ptr(),
            )
        };

        if return_code == 1 {
            unsafe {
                return Err(LLVMString::new(err_string.assume_init()));
            }
        }

        Ok(unsafe { Self::new(output_buffer) })
    }

    #[cfg(all(feature = "target-eravm", feature = "llvm17-0"))]
    /// Checks if the bytecode exceeds the EraVM size limit.
    pub fn exceeds_size_limit_eravm(&self, metadata_size: usize) -> bool {
        let return_code = unsafe { LLVMExceedsSizeLimitEraVM(self.memory_buffer, metadata_size as libc::c_uint) };

        return_code != 0
    }

    /// Links an EraVM module.
    #[cfg(all(feature = "target-eravm", feature = "llvm17-0"))]
    pub fn link_module_eravm(
        &self,
        linker_symbols: &[([u8; Self::ERAVM_WORD_SIZE], [u8; Self::ETHEREUM_ADDRESS_SIZE])],
        metadata: Option<&[u8]>,
    ) -> Result<Self, LLVMString> {
        let mut output_buffer = ptr::null_mut();
        let mut err_string = MaybeUninit::uninit();

        let metadata_ptr = match metadata {
            Some(metadata) => metadata.as_ptr(),
            None => ptr::null(),
        } as *const ::libc::c_char;
        let metadata_size = metadata.map_or(0, |metadata| metadata.len() as libc::c_uint);

        let linker_symbol_keys = linker_symbols
            .iter()
            .map(|(key, _)| key.as_ptr() as *const ::libc::c_char)
            .collect::<Vec<_>>();
        let linker_symbol_values = linker_symbols
            .iter()
            .map(|(_, value)| value.as_ptr() as *const ::libc::c_char)
            .collect::<Vec<_>>();
        let linker_symbols_size = linker_symbols.len() as libc::c_uint;

        let return_code = unsafe {
            LLVMLinkEraVM(
                self.memory_buffer,
                &mut output_buffer,
                metadata_ptr,
                metadata_size,
                linker_symbol_keys.as_ptr(),
                linker_symbol_values.as_ptr(),
                linker_symbols_size,
                err_string.as_mut_ptr(),
            )
        };

        if return_code == 1 {
            unsafe {
                return Err(LLVMString::new(err_string.assume_init()));
            }
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
