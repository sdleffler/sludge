use crate::{fmod::Fmod, CheckError};
use {
    libc::c_void,
    serde::*,
    sludge::prelude::*,
    sludge_fmod_sys::*,
    std::{
        ffi::{CStr, CString},
        ptr, str,
        sync::Arc,
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

#[derive(Debug, Clone, Serialize)]
pub struct TimelineMarkerProperties {
    pub name: String,
    pub position: i32,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct TimelineBeatProperties {
    pub bar: i32,
    pub beat: i32,
    pub position: i32,
    pub tempo: f32,
    pub time_signature_numerator: i32,
    pub time_signature_denominator: i32,
}

#[derive(Debug)]
pub enum EventCallbackInfo {
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
    TimelineMarker(TimelineMarkerProperties),
    TimelineBeat(TimelineBeatProperties),
    //SoundPlayed(&'a Sound),
    //SoundStopped(&'a Sound),
    RealToVirtual,
    VirtualToReal,
    StartEventCommand(EventInstance),
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

type BoxedEventCallback = Box<dyn Fn(EventInstance, EventCallbackInfo) -> Result<()>>;

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

impl<'lua> FromLua<'lua> for StopMode {
    fn from_lua(lua_value: LuaValue<'lua>, lua: LuaContext<'lua>) -> LuaResult<Self> {
        let lua_str = <LuaString>::from_lua(lua_value, lua)
            .with_context(|| anyhow!("error while parsing StopMode from Lua"))
            .to_lua_err()?;
        match lua_str.to_str()? {
            "immediate" => Ok(StopMode::Immediate),
            "allow_fadeout" => Ok(StopMode::AllowFadeout),
            s => Err(anyhow!("bad stop mode {}", s)).to_lua_err(),
        }
    }
}

unsafe extern "C" fn event_instance_callback_shim(
    type_: FMOD_STUDIO_EVENT_CALLBACK_TYPE,
    event: *mut FMOD_STUDIO_EVENTINSTANCE,
    parameters: *mut c_void,
) -> FMOD_RESULT {
    let mut callback_ptr = ptr::null_mut();
    FMOD_Studio_EventInstance_GetUserData(event, &mut callback_ptr)
        .check_err()
        .expect("todo: error handling from within callback_shim");
    assert!(!callback_ptr.is_null());

    let ev = EventInstance::from_ptr(event).unwrap();
    callback_shim(type_, parameters, ev, &*(callback_ptr as *const _))
}

unsafe extern "C" fn event_description_callback_shim(
    type_: FMOD_STUDIO_EVENT_CALLBACK_TYPE,
    event: *mut FMOD_STUDIO_EVENTINSTANCE,
    parameters: *mut c_void,
) -> FMOD_RESULT {
    let mut event_desc = ptr::null_mut();
    FMOD_Studio_EventInstance_GetDescription(event, &mut event_desc)
        .check_err()
        .unwrap();
    let mut callback_ptr = ptr::null_mut();
    FMOD_Studio_EventDescription_GetUserData(event_desc, &mut callback_ptr)
        .check_err()
        .expect("todo: error handling from within callback_shim");
    assert!(!callback_ptr.is_null());

    let ev = EventInstance::from_ptr(event).unwrap();
    callback_shim(type_, parameters, ev, &*(callback_ptr as *const _))
}

#[inline]
unsafe fn callback_shim(
    type_: FMOD_STUDIO_EVENT_CALLBACK_TYPE,
    parameters: *mut c_void,
    ev: EventInstance,
    cb: &BoxedEventCallback,
) -> FMOD_RESULT {
    let parameters = parameters as *mut EventCallbackParameters;
    let result = match type_ {
        FMOD_STUDIO_EVENT_CALLBACK_CREATED => cb(ev, EventCallbackInfo::Created),
        FMOD_STUDIO_EVENT_CALLBACK_DESTROYED => cb(ev, EventCallbackInfo::Destroyed),
        FMOD_STUDIO_EVENT_CALLBACK_STARTING => cb(ev, EventCallbackInfo::Starting),
        FMOD_STUDIO_EVENT_CALLBACK_STARTED => cb(ev, EventCallbackInfo::Started),
        FMOD_STUDIO_EVENT_CALLBACK_RESTARTED => cb(ev, EventCallbackInfo::Restarted),
        FMOD_STUDIO_EVENT_CALLBACK_STOPPED => cb(ev, EventCallbackInfo::Stopped),
        FMOD_STUDIO_EVENT_CALLBACK_START_FAILED => cb(ev, EventCallbackInfo::StartFailed),

        // TODO(sleffy):
        FMOD_STUDIO_EVENT_CALLBACK_CREATE_PROGRAMMER_SOUND
        | FMOD_STUDIO_EVENT_CALLBACK_DESTROY_PROGRAMMER_SOUND
        | FMOD_STUDIO_EVENT_CALLBACK_PLUGIN_CREATED
        | FMOD_STUDIO_EVENT_CALLBACK_PLUGIN_DESTROYED => Ok(()),

        FMOD_STUDIO_EVENT_CALLBACK_TIMELINE_MARKER => {
            let props = &(*parameters).timeline_marker_properties;
            let bytes = CStr::from_ptr(props.name as *const _).to_bytes();
            let timeline_marker_properties = TimelineMarkerProperties {
                name: str::from_utf8_unchecked(bytes).to_owned(),
                position: props.position,
            };

            cb(
                ev,
                EventCallbackInfo::TimelineMarker(timeline_marker_properties),
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
                ev,
                EventCallbackInfo::TimelineBeat(timeline_beat_properties),
            )
        }

        // TODO(sleffy):
        FMOD_STUDIO_EVENT_CALLBACK_SOUND_PLAYED | FMOD_STUDIO_EVENT_CALLBACK_SOUND_STOPPED => {
            Ok(())
        }

        FMOD_STUDIO_EVENT_CALLBACK_REAL_TO_VIRTUAL => cb(ev, EventCallbackInfo::RealToVirtual),
        FMOD_STUDIO_EVENT_CALLBACK_VIRTUAL_TO_REAL => cb(ev, EventCallbackInfo::VirtualToReal),

        FMOD_STUDIO_EVENT_CALLBACK_START_EVENT_COMMAND => {
            let instance = &mut (*parameters).event_instance;
            let secondary = EventInstance { ptr: instance };
            cb(ev, EventCallbackInfo::StartEventCommand(secondary))
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
    pub(crate) ptr: *mut FMOD_STUDIO_EVENTINSTANCE,
}

impl Clone for EventInstance {
    fn clone(&self) -> Self {
        unsafe {
            if let Some(ud_ptr) = self.get_userdata().unwrap() {
                Arc::incr_strong_count(ud_ptr);
            }
        }

        Self { ptr: self.ptr }
    }
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

    pub fn get_description(&self) -> Result<EventDescription> {
        let mut ptr = ptr::null_mut();
        unsafe {
            FMOD_Studio_EventInstance_GetDescription(self.ptr, &mut ptr).check_err()?;
            EventDescription::from_ptr(ptr)
        }
    }

    unsafe fn from_ptr(ptr: *mut FMOD_STUDIO_EVENTINSTANCE) -> Result<Self> {
        let this = EventInstance { ptr };

        if let Some(ud_ptr) = this.get_userdata()? {
            Arc::incr_strong_count(ud_ptr);
        }

        Ok(this)
    }

    unsafe fn get_userdata(&self) -> Result<Option<*const BoxedEventCallback>> {
        let mut ud_ptr = ptr::null_mut();
        FMOD_Studio_EventInstance_GetUserData(self.ptr, &mut ud_ptr).check_err()?;

        if ud_ptr.is_null() {
            Ok(None)
        } else {
            Ok(Some(ud_ptr as *const _))
        }
    }

    unsafe fn set_userdata(&self, ud: Arc<BoxedEventCallback>) -> Result<()> {
        if let Some(ud_ptr) = self.get_userdata()? {
            Arc::decr_strong_count(ud_ptr);
        }

        FMOD_Studio_EventInstance_SetUserData(self.ptr, Arc::into_raw(ud) as *mut _).check_err()?;

        Ok(())
    }

    pub fn set_callback<F>(&self, callback: F, mask: EventCallbackMask) -> Result<()>
    where
        F: Fn(EventInstance, EventCallbackInfo) -> Result<()> + 'static + Send + Sync,
    {
        let boxed = Box::new(callback) as BoxedEventCallback;
        unsafe {
            self.set_userdata(Arc::new(boxed))?;
            FMOD_Studio_EventInstance_SetCallback(
                self.ptr,
                Some(event_instance_callback_shim),
                mask.bits,
            )
            .check_err()?;
        }
        Ok(())
    }
}

impl Drop for EventInstance {
    fn drop(&mut self) {
        unsafe {
            if let Some(ud_ptr) = self.get_userdata().unwrap() {
                Arc::decr_strong_count(ud_ptr);
            }

            FMOD_Studio_EventInstance_Release(self.ptr)
                .check_err()
                .unwrap();
        }
    }
}

impl LuaUserData for EventInstance {
    fn add_methods<'lua, T: LuaUserDataMethods<'lua, Self>>(methods: &mut T) {
        methods.add_method("start", |_lua, this, ()| this.start().to_lua_err());
        methods.add_method("stop", |_lua, this, stop_mode: StopMode| {
            this.stop(stop_mode).to_lua_err()
        });
        methods.add_method("trigger_cue", |_lua, this, ()| {
            this.trigger_cue().to_lua_err()
        });

        methods.add_method(
            "set_callback",
            |lua, this, (cb, _mask): (LuaFunction, ())| {
                let resources = lua.resources();
                let fmod = resources.fetch::<Fmod>();
                let cq_send = fmod.cq_send.clone();
                let key = Arc::new(lua.create_registry_value(cb)?);
                this.set_callback(
                    move |event_instance, event_info| {
                        cq_send
                            .send((key.clone(), event_instance, event_info))
                            .map_err(|_| anyhow!("error while sending callback info"))
                    },
                    EventCallbackMask::ALL,
                )
                .to_lua_err()?;

                Ok(())
            },
        );
    }
}

#[derive(Debug)]
pub struct EventDescription {
    pub(crate) ptr: *mut FMOD_STUDIO_EVENTDESCRIPTION,
}

impl Clone for EventDescription {
    fn clone(&self) -> Self {
        unsafe {
            if let Some(ud_ptr) = self.get_userdata().unwrap() {
                Arc::incr_strong_count(ud_ptr);
            }
        }

        Self { ptr: self.ptr }
    }
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

        unsafe {
            if let Some(ud) = self.get_userdata()? {
                Arc::incr_strong_count(ud);
            }
        }

        Ok(EventInstance { ptr })
    }

    unsafe fn from_ptr(ptr: *mut FMOD_STUDIO_EVENTDESCRIPTION) -> Result<Self> {
        let this = EventDescription { ptr };

        if let Some(ud_ptr) = this.get_userdata()? {
            Arc::incr_strong_count(ud_ptr);
        }

        Ok(this)
    }

    unsafe fn get_userdata(&self) -> Result<Option<*const BoxedEventCallback>> {
        let mut ud_ptr = ptr::null_mut();
        FMOD_Studio_EventDescription_GetUserData(self.ptr, &mut ud_ptr).check_err()?;

        if ud_ptr.is_null() {
            Ok(None)
        } else {
            Ok(Some(ud_ptr as *const _))
        }
    }

    unsafe fn set_userdata(&self, ud: Arc<BoxedEventCallback>) -> Result<()> {
        if let Some(ud_ptr) = self.get_userdata()? {
            Arc::decr_strong_count(ud_ptr);
        }

        FMOD_Studio_EventDescription_SetUserData(self.ptr, Arc::into_raw(ud) as *mut _)
            .check_err()?;

        Ok(())
    }

    pub fn set_callback<F>(&self, callback: F, mask: EventCallbackMask) -> Result<()>
    where
        F: Fn(EventInstance, EventCallbackInfo) -> Result<()> + 'static + Send + Sync,
    {
        let boxed = Box::new(callback) as BoxedEventCallback;
        unsafe {
            self.set_userdata(Arc::new(boxed))?;
            FMOD_Studio_EventDescription_SetCallback(
                self.ptr,
                Some(event_description_callback_shim),
                mask.bits,
            )
            .check_err()?;
        }
        Ok(())
    }
}

impl Drop for EventDescription {
    fn drop(&mut self) {
        unsafe {
            if let Some(ud_ptr) = self.get_userdata().unwrap() {
                Arc::decr_strong_count(ud_ptr);
            }

            FMOD_Studio_EventDescription_ReleaseAllInstances(self.ptr)
                .check_err()
                .unwrap();
        }
    }
}

impl LuaUserData for EventDescription {
    fn add_methods<'lua, T: LuaUserDataMethods<'lua, Self>>(methods: &mut T) {
        methods.add_method("create_instance", |_lua, this, ()| {
            this.create_instance().to_lua_err()
        });

        methods.add_method(
            "set_callback",
            |lua, this, (cb, _mask): (LuaFunction, ())| {
                let resources = lua.resources();
                let fmod = resources.fetch::<Fmod>();
                let cq_send = fmod.cq_send.clone();
                let key = Arc::new(lua.create_registry_value(cb)?);
                this.set_callback(
                    move |event_instance, event_info| {
                        cq_send
                            .send((key.clone(), event_instance, event_info))
                            .map_err(|_| anyhow!("error while sending callback info"))
                    },
                    EventCallbackMask::ALL,
                )
                .to_lua_err()?;

                Ok(())
            },
        );
    }
}
