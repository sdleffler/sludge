use crate::{fmod::Fmod, CheckError};
use {
    libc::c_void,
    sludge::prelude::*,
    sludge_fmod_sys::*,
    std::{
        ffi::{CStr, CString},
        ptr, str,
    },
};

bitflags::bitflags! {
    pub struct EventCallbackMask: u32 {
        const CREATED                  = FMOD_STUDIO_EVENT_CALLBACK_CREATED                 ;
        const DESTROYED                = FMOD_STUDIO_EVENT_CALLBACK_DESTROYED               ;
        const STARTING                 = FMOD_STUDIO_EVENT_CALLBACK_STARTING                ;
        const STARTED                  = FMOD_STUDIO_EVENT_CALLBACK_STARTED                 ;
        const RESTARTED                = FMOD_STUDIO_EVENT_CALLBACK_RESTARTED               ;
        const STOPPED                  = FMOD_STUDIO_EVENT_CALLBACK_STOPPED                 ;
        const START_FAILED             = FMOD_STUDIO_EVENT_CALLBACK_START_FAILED            ;
        const CREATE_PROGRAMMER_SOUND  = FMOD_STUDIO_EVENT_CALLBACK_CREATE_PROGRAMMER_SOUND ;
        const DESTROY_PROGRAMMER_SOUND = FMOD_STUDIO_EVENT_CALLBACK_DESTROY_PROGRAMMER_SOUND;
        const PLUGIN_CREATED           = FMOD_STUDIO_EVENT_CALLBACK_PLUGIN_CREATED          ;
        const PLUGIN_DESTROYED         = FMOD_STUDIO_EVENT_CALLBACK_PLUGIN_DESTROYED        ;
        const TIMELINE_MARKER          = FMOD_STUDIO_EVENT_CALLBACK_TIMELINE_MARKER         ;
        const TIMELINE_BEAT            = FMOD_STUDIO_EVENT_CALLBACK_TIMELINE_BEAT           ;
        const SOUND_PLAYED             = FMOD_STUDIO_EVENT_CALLBACK_SOUND_PLAYED            ;
        const SOUND_STOPPED            = FMOD_STUDIO_EVENT_CALLBACK_SOUND_STOPPED           ;
        const REAL_TO_VIRTUAL          = FMOD_STUDIO_EVENT_CALLBACK_REAL_TO_VIRTUAL         ;
        const VIRTUAL_TO_REAL          = FMOD_STUDIO_EVENT_CALLBACK_VIRTUAL_TO_REAL         ;
        const START_EVENT_COMMAND      = FMOD_STUDIO_EVENT_CALLBACK_START_EVENT_COMMAND     ;
        const ALL                      = FMOD_STUDIO_EVENT_CALLBACK_ALL                     ;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TimelineMarkerProperties<'a> {
    pub name: &'a str,
    pub position: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct TimelineBeatProperties {
    pub bar: i32,
    pub beat: i32,
    pub position: i32,
    pub tempo: f32,
    pub time_signature_numerator: i32,
    pub time_signature_denominator: i32,
}

#[derive(Debug)]
pub enum EventCallbackInfo<'a> {
    Created,
    Destroyed,
    Starting,
    Started,
    Restarted,
    Stopped,
    StartFailed,
    //CreateProgrammerSound(&'a Sound),
    //DestroyProgrammerSound(&'a Sound),
    //PluginCreated(PluginInstanceProperties),
    //PluginDestroyed(PluginInstanceProperties),
    TimelineMarker(&'a TimelineMarkerProperties<'a>),
    TimelineBeat(&'a TimelineBeatProperties),
    //SoundPlayed(&'a Sound),
    //SoundStopped(&'a Sound),
    RealToVirtual,
    VirtualToReal,
    StartEventCommand(&'a EventInstance),
}

#[allow(dead_code)]
union EventCallbackParameters {
    programmer_sound_properties: FMOD_STUDIO_PROGRAMMER_SOUND_PROPERTIES,
    plugin_instance_properties: FMOD_STUDIO_PLUGIN_INSTANCE_PROPERTIES,
    timeline_marker_properties: FMOD_STUDIO_TIMELINE_MARKER_PROPERTIES,
    timeline_beat_properties: FMOD_STUDIO_TIMELINE_BEAT_PROPERTIES,
    sound: FMOD_SOUND,
    event_instance: FMOD_STUDIO_EVENTINSTANCE,
}

type BoxedEventCallback = Box<dyn Fn(&EventInstance, EventCallbackInfo) -> Result<()>>;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum StopMode {
    Immediate,
    AllowFadeout,
}

impl From<StopMode> for FMOD_STUDIO_STOP_MODE {
    fn from(stop_mode: StopMode) -> Self {
        match stop_mode {
            StopMode::Immediate => FMOD_STUDIO_STOP_MODE_FMOD_STUDIO_STOP_IMMEDIATE,
            StopMode::AllowFadeout => FMOD_STUDIO_STOP_MODE_FMOD_STUDIO_STOP_ALLOWFADEOUT,
        }
    }
}

unsafe extern "C" fn callback_shim(
    type_: FMOD_STUDIO_EVENT_CALLBACK_TYPE,
    event: *mut FMOD_STUDIO_EVENTINSTANCE,
    parameters: *mut c_void,
) -> FMOD_RESULT {
    let mut callback_ptr = ptr::null_mut();
    FMOD_Studio_EventInstance_GetUserData(event, &mut callback_ptr)
        .check_err()
        .expect("todo: error handling from within callback_shim");
    let cb = &*(callback_ptr as *mut BoxedEventCallback);
    let ev = EventInstance { ptr: event };
    let parameters = parameters as *mut EventCallbackParameters;
    let result = match type_ {
        FMOD_STUDIO_EVENT_CALLBACK_CREATED => cb(&ev, EventCallbackInfo::Created),
        FMOD_STUDIO_EVENT_CALLBACK_DESTROYED => cb(&ev, EventCallbackInfo::Destroyed),
        FMOD_STUDIO_EVENT_CALLBACK_STARTING => cb(&ev, EventCallbackInfo::Starting),
        FMOD_STUDIO_EVENT_CALLBACK_STARTED => cb(&ev, EventCallbackInfo::Started),
        FMOD_STUDIO_EVENT_CALLBACK_RESTARTED => cb(&ev, EventCallbackInfo::Restarted),
        FMOD_STUDIO_EVENT_CALLBACK_STOPPED => cb(&ev, EventCallbackInfo::Stopped),
        FMOD_STUDIO_EVENT_CALLBACK_START_FAILED => cb(&ev, EventCallbackInfo::StartFailed),

        // TODO(sleffy):
        FMOD_STUDIO_EVENT_CALLBACK_CREATE_PROGRAMMER_SOUND
        | FMOD_STUDIO_EVENT_CALLBACK_DESTROY_PROGRAMMER_SOUND
        | FMOD_STUDIO_EVENT_CALLBACK_PLUGIN_CREATED
        | FMOD_STUDIO_EVENT_CALLBACK_PLUGIN_DESTROYED => Ok(()),

        FMOD_STUDIO_EVENT_CALLBACK_TIMELINE_MARKER => {
            let props = &(*parameters).timeline_marker_properties;
            let bytes = CStr::from_ptr(props.name as *const _).to_bytes();
            let timeline_marker_properties = TimelineMarkerProperties {
                name: str::from_utf8_unchecked(bytes),
                position: props.position,
            };

            cb(
                &ev,
                EventCallbackInfo::TimelineMarker(&timeline_marker_properties),
            )
        }

        FMOD_STUDIO_EVENT_CALLBACK_TIMELINE_BEAT => {
            let props = &(*parameters).timeline_beat_properties;
            let timeline_beat_properties = TimelineBeatProperties {
                bar: props.bar,
                beat: props.beat,
                position: props.position,
                tempo: props.tempo,
                time_signature_numerator: props.timesignatureupper,
                time_signature_denominator: props.timesignaturelower,
            };

            cb(
                &ev,
                EventCallbackInfo::TimelineBeat(&timeline_beat_properties),
            )
        }

        // TODO(sleffy):
        FMOD_STUDIO_EVENT_CALLBACK_SOUND_PLAYED | FMOD_STUDIO_EVENT_CALLBACK_SOUND_STOPPED => {
            Ok(())
        }

        FMOD_STUDIO_EVENT_CALLBACK_REAL_TO_VIRTUAL => cb(&ev, EventCallbackInfo::RealToVirtual),
        FMOD_STUDIO_EVENT_CALLBACK_VIRTUAL_TO_REAL => cb(&ev, EventCallbackInfo::VirtualToReal),

        FMOD_STUDIO_EVENT_CALLBACK_START_EVENT_COMMAND => {
            let instance = &mut (*parameters).event_instance;
            let secondary = EventInstance { ptr: instance };
            cb(&ev, EventCallbackInfo::StartEventCommand(&secondary))
        }

        other => {
            log::error!("unknown FMOD_STUDIO_EVENT_CALLBACK_TYPE {:x}", other);
            Ok(())
        }
    };

    // Discard the error, but log it. We don't know what will happen if we panic in here.
    let _ = result.log_error_err("sludge_fmod::event::EventCallback");

    FMOD_RESULT_FMOD_OK
}

#[derive(Debug)]
pub struct EventInstance {
    ptr: *mut FMOD_STUDIO_EVENTINSTANCE,
}

unsafe impl Send for EventInstance {}
unsafe impl Sync for EventInstance {}

impl EventInstance {
    pub fn start(&self) -> Result<()> {
        unsafe {
            FMOD_Studio_EventInstance_Start(self.ptr).check_err()?;
        }
        Ok(())
    }

    pub fn stop(&self, stop_mode: StopMode) -> Result<()> {
        unsafe {
            FMOD_Studio_EventInstance_Stop(self.ptr, stop_mode.into()).check_err()?;
        }
        Ok(())
    }

    pub fn trigger_cue(&self) -> Result<()> {
        unsafe {
            FMOD_Studio_EventInstance_TriggerCue(self.ptr).check_err()?;
        }
        Ok(())
    }

    unsafe fn set_userdata(&self, ptr: *mut BoxedEventCallback) -> Result<()> {
        let mut old_ptr = ptr::null_mut();
        FMOD_Studio_EventInstance_GetUserData(self.ptr, &mut old_ptr).check_err()?;
        if !old_ptr.is_null() {
            drop(BoxedEventCallback::from_raw(
                old_ptr as *mut BoxedEventCallback,
            ));
        }
        FMOD_Studio_EventInstance_SetUserData(self.ptr, ptr as *mut _).check_err()?;

        Ok(())
    }

    pub fn set_callback<F>(&self, callback: F, mask: EventCallbackMask) -> Result<()>
    where
        F: Fn(&EventInstance, EventCallbackInfo) -> Result<()> + 'static + Send + Sync,
    {
        let boxed = Box::new(callback) as BoxedEventCallback;
        let raw_userdata = Box::into_raw(Box::new(boxed));
        unsafe {
            self.set_userdata(raw_userdata)?;
            FMOD_Studio_EventInstance_SetCallback(self.ptr, Some(callback_shim), mask.bits)
                .check_err()?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct EventDescription {
    ptr: *mut FMOD_STUDIO_EVENTDESCRIPTION,
}

unsafe impl Send for EventDescription {}
unsafe impl Sync for EventDescription {}

impl EventDescription {
    pub(crate) fn get_event<T: AsRef<[u8]>>(fmod: &Fmod, id: T) -> Result<Self> {
        let c_string = CString::new(id.as_ref())?;
        let mut ptr = ptr::null_mut();
        unsafe {
            FMOD_Studio_System_GetEvent(fmod.ptr, c_string.as_ptr(), &mut ptr).check_err()?;
        }

        Ok(Self { ptr })
    }

    pub fn is_valid(&self) -> bool {
        unsafe { FMOD_Studio_EventDescription_IsValid(self.ptr) != 0 }
    }

    pub fn release_all_instances(&self) -> Result<()> {
        unsafe {
            FMOD_Studio_EventDescription_ReleaseAllInstances(self.ptr).check_err()?;
        }

        Ok(())
    }

    pub fn create_instance(&self) -> Result<EventInstance> {
        let mut ptr = ptr::null_mut();
        unsafe {
            FMOD_Studio_EventDescription_CreateInstance(self.ptr, &mut ptr).check_err()?;
        }

        Ok(EventInstance { ptr })
    }

    unsafe fn set_userdata(&self, ptr: *mut BoxedEventCallback) -> Result<()> {
        let mut old_ptr = ptr::null_mut();
        FMOD_Studio_EventDescription_GetUserData(self.ptr, &mut old_ptr).check_err()?;
        if !old_ptr.is_null() {
            drop(BoxedEventCallback::from_raw(
                old_ptr as *mut BoxedEventCallback,
            ));
        }
        FMOD_Studio_EventDescription_SetUserData(self.ptr, ptr as *mut _).check_err()?;

        Ok(())
    }

    pub fn set_callback<F>(&self, callback: F, mask: EventCallbackMask) -> Result<()>
    where
        F: Fn(&EventInstance, EventCallbackInfo) -> Result<()> + 'static + Send + Sync,
    {
        let boxed = Box::new(callback) as BoxedEventCallback;
        let raw_userdata = Box::into_raw(Box::new(boxed));
        unsafe {
            self.set_userdata(raw_userdata)?;
            FMOD_Studio_EventDescription_SetCallback(self.ptr, Some(callback_shim), mask.bits)
                .check_err()?;
        }
        Ok(())
    }
}
