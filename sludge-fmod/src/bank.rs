use crate::{fmod::Fmod, CheckError};
use {
    sludge::prelude::*,
    sludge_fmod_sys::*,
    std::{ffi::CString, ptr},
};

bitflags::bitflags! {
    pub struct LoadBankFlags: u32 {
        const NORMAL = FMOD_STUDIO_LOAD_BANK_NORMAL;
        const NONBLOCKING = FMOD_STUDIO_LOAD_BANK_NONBLOCKING;
        const DECOMPRESS_SAMPLES = FMOD_STUDIO_LOAD_BANK_DECOMPRESS_SAMPLES;
        const UNENCRYPTED = FMOD_STUDIO_LOAD_BANK_UNENCRYPTED;
    }
}

#[derive(Debug)]
pub struct Bank {
    pub(crate) ptr: *mut FMOD_STUDIO_BANK,
}

unsafe impl Send for Bank {}
unsafe impl Sync for Bank {}

impl Bank {
    pub(crate) fn load_bank_file<T: AsRef<[u8]>>(
        fmod: &Fmod,
        filename: T,
        flags: LoadBankFlags,
    ) -> Result<Self> {
        let c_string = CString::new(filename.as_ref())?;
        let mut ptr = ptr::null_mut();
        unsafe {
            FMOD_Studio_System_LoadBankFile(fmod.ptr, c_string.as_ptr(), flags.bits, &mut ptr)
                .check_err()?;
        }

        Ok(Self { ptr })
    }

    pub fn is_valid(&self) -> bool {
        unsafe { FMOD_Studio_Bank_IsValid(self.ptr) != 0 }
    }

    pub fn load_sample_data(&self) -> Result<()> {
        unsafe {
            FMOD_Studio_Bank_LoadSampleData(self.ptr).check_err()?;
        }
        Ok(())
    }

    pub fn unload_sample_data(&self) -> Result<()> {
        unsafe {
            FMOD_Studio_Bank_UnloadSampleData(self.ptr).check_err()?;
        }
        Ok(())
    }
}

impl Drop for Bank {
    fn drop(&mut self) {
        unsafe {
            FMOD_Studio_Bank_Unload(self.ptr)
                .check_err()
                .expect("error while dropping FMOD bank");
        }
    }
}
