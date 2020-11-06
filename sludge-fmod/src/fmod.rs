use crate::{
    bank::{Bank, LoadBankFlags},
    event::EventDescription,
    CheckError,
};
use {sludge::prelude::*, sludge_fmod_sys::*, std::ptr};

bitflags::bitflags! {
    pub struct FmodStudioInitFlags: u32 {
        const NORMAL                = FMOD_STUDIO_INIT_NORMAL;
        const LIVEUPDATE            = FMOD_STUDIO_INIT_LIVEUPDATE;
        const ALLOW_MISSING_PLUGINS = FMOD_STUDIO_INIT_ALLOW_MISSING_PLUGINS;
        const SYNCHRONOUS_UPDATE    = FMOD_STUDIO_INIT_SYNCHRONOUS_UPDATE;
        const DEFERRED_CALLBACKS    = FMOD_STUDIO_INIT_DEFERRED_CALLBACKS;
        const LOAD_FROM_UPDATE      = FMOD_STUDIO_INIT_LOAD_FROM_UPDATE;
        const MEMORY_TRACKING       = FMOD_STUDIO_INIT_MEMORY_TRACKING;    }
}

bitflags::bitflags! {
    pub struct FmodCoreInitFlags: u32 {
        const NORMAL                 = FMOD_INIT_NORMAL;
        const STREAM_FROM_UPDATE     = FMOD_INIT_STREAM_FROM_UPDATE;
        const MIX_FROM_UPDATE        = FMOD_INIT_MIX_FROM_UPDATE;
        const _3D_RIGHTHANDED        = FMOD_INIT_3D_RIGHTHANDED;
        const CHANNEL_LOWPASS        = FMOD_INIT_CHANNEL_LOWPASS;
        const CHANNEL_DISTANCEFILTER = FMOD_INIT_CHANNEL_DISTANCEFILTER;
        const PROFILE_ENABLE         = FMOD_INIT_PROFILE_ENABLE;
        const VOL0_BECOMES_VIRTUAL   = FMOD_INIT_VOL0_BECOMES_VIRTUAL;
        const GEOMETRY_USECLOSEST    = FMOD_INIT_GEOMETRY_USECLOSEST;
        const PREFER_DOLBY_DOWNMIX   = FMOD_INIT_PREFER_DOLBY_DOWNMIX;
        const THREAD_UNSAFE          = FMOD_INIT_THREAD_UNSAFE;
        const PROFILE_METER_ALL      = FMOD_INIT_PROFILE_METER_ALL;
        const MEMORY_TRACKING        = FMOD_INIT_MEMORY_TRACKING;
    }
}

pub struct FmodSystemBuilder {
    system: *mut FMOD_STUDIO_SYSTEM,
}

impl FmodSystemBuilder {
    pub fn create() -> Result<Self> {
        let mut system = ptr::null_mut();

        unsafe {
            FMOD_Studio_System_Create(&mut system, FMOD_VERSION).check_err()?;
        }

        Ok(Self { system })
    }

    /// Create and initialize the FMOD studio system object.
    pub fn initialize(
        self,
        max_channels: u32,
        studio_flags: FmodStudioInitFlags,
        core_flags: FmodCoreInitFlags,
    ) -> Result<Fmod> {
        ensure!(
            !studio_flags.contains(FmodStudioInitFlags::SYNCHRONOUS_UPDATE)
                && !core_flags.contains(FmodCoreInitFlags::THREAD_UNSAFE),
            "initialization flags contain options which disable thread safety \
             and are not currently supported!"
        );

        unsafe {
            FMOD_Studio_System_Initialize(
                self.system,
                max_channels as i32,
                studio_flags.bits,
                core_flags.bits,
                ptr::null_mut(),
            )
            .check_err()?;
        }

        Ok(Fmod { ptr: self.system })
    }
}

/// This is our main FMOD context type, representing the studio system object.
#[derive(Debug)]
pub struct Fmod {
    pub(crate) ptr: *mut FMOD_STUDIO_SYSTEM,
}

// FMOD Studio API is thread safe by default, and we panic if we see something which
// would cause otherwise in `Fmod::new()`. So this is okay.
unsafe impl Send for Fmod {}
unsafe impl Sync for Fmod {}

impl Fmod {
    pub fn update(&self) -> Result<()> {
        unsafe {
            FMOD_Studio_System_Update(self.ptr).check_err()?;
        }
        Ok(())
    }

    pub fn load_bank_file<T: AsRef<[u8]>>(
        &self,
        filename: T,
        flags: LoadBankFlags,
    ) -> Result<Bank> {
        Bank::load_bank_file(self, filename, flags)
    }

    pub fn get_event<T: AsRef<[u8]> + ?Sized>(&self, path: &T) -> Result<EventDescription> {
        EventDescription::get_event(self, path)
    }
}

impl Drop for Fmod {
    fn drop(&mut self) {
        unsafe {
            FMOD_Studio_System_Release(self.ptr)
                .check_err()
                .expect("error dropping FMOD system");
        }
    }
}
