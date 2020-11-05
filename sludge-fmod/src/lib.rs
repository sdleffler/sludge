use ::{sludge::prelude::*, sludge_fmod_sys as sys, std::ptr};

bitflags::bitflags! {
    pub struct FmodStudioInitFlags: u32 {
        const NORMAL                = sys::FMOD_STUDIO_INIT_NORMAL;
        const LIVEUPDATE            = sys::FMOD_STUDIO_INIT_LIVEUPDATE;
        const ALLOW_MISSING_PLUGINS = sys::FMOD_STUDIO_INIT_ALLOW_MISSING_PLUGINS;
        const SYNCHRONOUS_UPDATE    = sys::FMOD_STUDIO_INIT_SYNCHRONOUS_UPDATE;
        const DEFERRED_CALLBACKS    = sys::FMOD_STUDIO_INIT_DEFERRED_CALLBACKS;
        const LOAD_FROM_UPDATE      = sys::FMOD_STUDIO_INIT_LOAD_FROM_UPDATE;
        const MEMORY_TRACKING       = sys::FMOD_STUDIO_INIT_MEMORY_TRACKING;    }
}

bitflags::bitflags! {
    pub struct FmodCoreInitFlags: u32 {
        const NORMAL                 = sys::FMOD_INIT_NORMAL;
        const STREAM_FROM_UPDATE     = sys::FMOD_INIT_STREAM_FROM_UPDATE;
        const MIX_FROM_UPDATE        = sys::FMOD_INIT_MIX_FROM_UPDATE;
        const _3D_RIGHTHANDED        = sys::FMOD_INIT_3D_RIGHTHANDED;
        const CHANNEL_LOWPASS        = sys::FMOD_INIT_CHANNEL_LOWPASS;
        const CHANNEL_DISTANCEFILTER = sys::FMOD_INIT_CHANNEL_DISTANCEFILTER;
        const PROFILE_ENABLE         = sys::FMOD_INIT_PROFILE_ENABLE;
        const VOL0_BECOMES_VIRTUAL   = sys::FMOD_INIT_VOL0_BECOMES_VIRTUAL;
        const GEOMETRY_USECLOSEST    = sys::FMOD_INIT_GEOMETRY_USECLOSEST;
        const PREFER_DOLBY_DOWNMIX   = sys::FMOD_INIT_PREFER_DOLBY_DOWNMIX;
        const THREAD_UNSAFE          = sys::FMOD_INIT_THREAD_UNSAFE;
        const PROFILE_METER_ALL      = sys::FMOD_INIT_PROFILE_METER_ALL;
        const MEMORY_TRACKING        = sys::FMOD_INIT_MEMORY_TRACKING;
    }
}

pub struct Fmod {
    system: *mut sys::FMOD_STUDIO_SYSTEM,
}

impl Fmod {
    pub fn new(
        max_channels: u32,
        studio_flags: FmodStudioInitFlags,
        core_flags: FmodCoreInitFlags,
    ) -> Result<Self> {
        let mut system = ptr::null_mut();

        unsafe {
            sys::FMOD_Studio_System_Create(&mut system, sys::FMOD_VERSION);
        }

        Ok(Self { system })
    }
}
