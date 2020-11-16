use crate::{CheckError, Fmod};
use {
    enum_primitive_derive::*,
    libc::c_void,
    num_traits::FromPrimitive,
    serde::*,
    sludge::{api::Module, prelude::*},
    sludge_fmod_sys::*,
    std::{
        ffi::{CStr, CString},
        ptr, str,
        sync::Arc,
    },
};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Primitive)]
#[repr(i32)]
pub enum PlaybackState {
    Playing = FMOD_STUDIO_PLAYBACK_STATE_FMOD_STUDIO_PLAYBACK_PLAYING as i32,
    Sustaining = FMOD_STUDIO_PLAYBACK_STATE_FMOD_STUDIO_PLAYBACK_SUSTAINING as i32,
    Stopped = FMOD_STUDIO_PLAYBACK_STATE_FMOD_STUDIO_PLAYBACK_STOPPED as i32,
    Starting = FMOD_STUDIO_PLAYBACK_STATE_FMOD_STUDIO_PLAYBACK_STARTING as i32,
    Stopping = FMOD_STUDIO_PLAYBACK_STATE_FMOD_STUDIO_PLAYBACK_STOPPING as i32,
}

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

impl<'lua> ToLua<'lua> for EventCallbackMask {
    fn to_lua(self, lua: LuaContext<'lua>) -> LuaResult<LuaValue<'lua>> {
        self.bits().to_lua(lua)
    }
}

impl<'lua> FromLua<'lua> for EventCallbackMask {
    fn from_lua(lua_value: LuaValue<'lua>, lua: LuaContext<'lua>) -> LuaResult<Self> {
        Self::from_bits(u32::from_lua(lua_value, lua)?)
            .ok_or_else(|| anyhow!("invalid callback mask"))
            .to_lua_err()
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
        FMOD_STUDIO_EVENT_CALLBACK_CREATED => {
            let fmod_result = cb(ev, EventCallbackInfo::Created);
            if let Ok(Some(ud)) = ev.get_userdata() {
                Arc::incr_strong_count(ud);
            }
            fmod_result
        }
        FMOD_STUDIO_EVENT_CALLBACK_DESTROYED => {
            let fmod_result = cb(ev, EventCallbackInfo::Destroyed);
            if let Ok(Some(ud)) = ev.get_userdata() {
                Arc::decr_strong_count(ud);
            }
            fmod_result
        }
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

/// Combination of the "raw" value and "final"/modulated value produced by fetching
/// the value of an event parameter.
#[derive(Debug, Copy, Clone)]
pub struct ParameterValue {
    /// The parameter's value as set by the public API.
    pub value: f32,

    /// The parameter's value as calculated after modulation, automation, seek speed,
    /// and parameter velocity.
    pub final_value: f32,
}

/// An identifier for an event parameter.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(C)]
pub struct ParameterId {
    /// Opaque first half of the identifier.
    pub data1: u32,

    /// Opaque second half of the identifier.
    pub data2: u32,
}

impl From<FMOD_STUDIO_PARAMETER_ID> for ParameterId {
    fn from(id: FMOD_STUDIO_PARAMETER_ID) -> Self {
        Self {
            data1: id.data1,
            data2: id.data2,
        }
    }
}

impl From<ParameterId> for FMOD_STUDIO_PARAMETER_ID {
    fn from(id: ParameterId) -> Self {
        Self {
            data1: id.data1,
            data2: id.data2,
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct EventInstance {
    pub(crate) ptr: *mut FMOD_STUDIO_EVENTINSTANCE,
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

    pub fn get_playback_state(&self) -> Result<PlaybackState> {
        let mut state = 0;
        unsafe {
            FMOD_Studio_EventInstance_GetPlaybackState(self.ptr, &mut state).check_err()?;
        }
        PlaybackState::from_i32(state as i32).ok_or_else(|| anyhow!("bad playback state {}", state))
    }

    pub fn is_paused(&self) -> Result<bool> {
        let mut is_paused = 0i32;
        unsafe {
            FMOD_Studio_EventInstance_GetPaused(self.ptr, &mut is_paused as *mut _).check_err()?;
        }
        Ok(is_paused != 0)
    }

    pub fn set_paused(&self, paused: bool) -> Result<()> {
        unsafe {
            FMOD_Studio_EventInstance_SetPaused(self.ptr, paused as i32).check_err()?;
        }
        Ok(())
    }

    pub fn trigger_cue(&self) -> Result<()> {
        unsafe {
            FMOD_Studio_EventInstance_TriggerCue(self.ptr).check_err()?;
        }
        Ok(())
    }

    pub fn set_pitch(&self, pitch_multiplier: f32) -> Result<()> {
        unsafe {
            FMOD_Studio_EventInstance_SetPitch(self.ptr, pitch_multiplier).check_err()?;
        }
        Ok(())
    }

    pub fn get_pitch(&self) -> Result<ParameterValue> {
        let mut pitch = ParameterValue {
            value: 0.,
            final_value: 0.,
        };
        unsafe {
            FMOD_Studio_EventInstance_GetPitch(self.ptr, &mut pitch.value, &mut pitch.final_value)
                .check_err()?;
        }
        Ok(pitch)
    }

    // TODO(sleffy)
    // pub fn set_property(&self, index: EventProperty, value: f32) -> Result<()>;
    // pub fn get_property(&self, index: EventProperty) -> Result<f32>;

    /// Set the timeline cursor position in milliseconds.
    // FIXME(sleffy): protect against overflow
    pub fn set_timeline_position(&self, position: u32) -> Result<()> {
        unsafe {
            FMOD_Studio_EventInstance_SetTimelinePosition(self.ptr, position as i32).check_err()?;
        }
        Ok(())
    }

    /// Get the timeline cursor position in milliseconds.
    // FIXME(sleffy): protect against overflow
    pub fn get_timeline_position(&self) -> Result<u32> {
        let mut out = 0;
        unsafe {
            FMOD_Studio_EventInstance_GetTimelinePosition(self.ptr, &mut out).check_err()?;
        }
        Ok(out as u32)
    }

    /// Set a unitless scaling factor for the event volume. This does not override any
    /// FMOD Studio volume level or internal volume automation/modulation; it only
    /// scales it.
    pub fn set_volume(&self, volume: f32) -> Result<()> {
        unsafe {
            FMOD_Studio_EventInstance_SetVolume(self.ptr, volume).check_err()?;
        }
        Ok(())
    }

    /// The `value` field is the unitless scaling factor if set by `set_volume`, and
    /// the `final_value` field is the final volume value as modified by automation/
    /// modulation.
    pub fn get_volume(&self) -> Result<ParameterValue> {
        let mut out = ParameterValue {
            value: 0.,
            final_value: 0.,
        };
        unsafe {
            FMOD_Studio_EventInstance_GetVolume(self.ptr, &mut out.value, &mut out.final_value)
                .check_err()?;
        }
        Ok(out)
    }

    /// Check whether this instance has been "virtualized" due to exceeding the polyphony
    /// limit.
    pub fn is_virtual(&self) -> Result<bool> {
        let mut out = 0;
        unsafe {
            FMOD_Studio_EventInstance_IsVirtual(self.ptr, &mut out).check_err()?;
        }
        Ok(out != 0)
    }

    pub fn get_description(&self) -> Result<EventDescription> {
        let mut ptr = ptr::null_mut();
        unsafe {
            FMOD_Studio_EventInstance_GetDescription(self.ptr, &mut ptr).check_err()?;
            EventDescription::from_ptr(ptr)
        }
    }

    pub fn set_parameter_by_name<T: AsRef<[u8]> + ?Sized>(
        &self,
        name: &T,
        value: f32,
        ignore_seek_speed: bool,
    ) -> Result<()> {
        let c_string = CString::new(name.as_ref())?;
        unsafe {
            FMOD_Studio_EventInstance_SetParameterByName(
                self.ptr,
                c_string.as_ptr(),
                value,
                ignore_seek_speed as i32,
            )
            .check_err()?;
        }

        Ok(())
    }

    pub fn get_parameter_by_name<T: AsRef<[u8]> + ?Sized>(
        &self,
        name: &T,
    ) -> Result<ParameterValue> {
        let c_string = CString::new(name.as_ref())?;
        let mut parameter_value = ParameterValue {
            value: 0.,
            final_value: 0.,
        };
        unsafe {
            FMOD_Studio_EventInstance_GetParameterByName(
                self.ptr,
                c_string.as_ptr(),
                &mut parameter_value.value,
                &mut parameter_value.final_value,
            )
            .check_err()?;
        }

        Ok(parameter_value)
    }

    pub fn set_parameter_by_id(
        &self,
        id: ParameterId,
        value: f32,
        ignore_seek_speed: bool,
    ) -> Result<()> {
        unsafe {
            FMOD_Studio_EventInstance_SetParameterByID(
                self.ptr,
                id.into(),
                value,
                ignore_seek_speed as i32,
            )
            .check_err()?;
        }

        Ok(())
    }

    pub fn get_parameter_by_id(&self, id: ParameterId) -> Result<ParameterValue> {
        let mut parameter_value = ParameterValue {
            value: 0.,
            final_value: 0.,
        };
        unsafe {
            FMOD_Studio_EventInstance_GetParameterByID(
                self.ptr,
                id.into(),
                &mut parameter_value.value,
                &mut parameter_value.final_value,
            )
            .check_err()?;
        }

        Ok(parameter_value)
    }

    pub fn set_parameters_by_ids(
        &self,
        ids: &[ParameterId],
        values: &[f32],
        ignore_seek_speed: bool,
    ) -> Result<()> {
        ensure!(
            ids.len() == values.len(),
            "length of ids slice and values slice do not match!"
        );
        let count = ids.len();
        unsafe {
            FMOD_Studio_EventInstance_SetParametersByIDs(
                self.ptr,
                ids.as_ptr() as *mut _,
                values.as_ptr() as *mut _,
                count as i32,
                ignore_seek_speed as i32,
            )
            .check_err()?;
        }

        Ok(())
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

    pub fn unset_callback(&self) -> Result<()> {
        unsafe {
            if let Some(ud_ptr) = self.get_userdata().unwrap() {
                Arc::decr_strong_count(ud_ptr);
            }

            FMOD_Studio_EventInstance_SetUserData(self.ptr, ptr::null_mut()).check_err()?;
            FMOD_Studio_EventInstance_SetCallback(
                self.ptr,
                None,
                EventCallbackMask::empty().bits(),
            )
            .check_err()?;
        }
        Ok(())
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
            |lua, this, (maybe_cb, mask): (Option<LuaFunction>, Option<EventCallbackMask>)| {
                if let Some(cb) = maybe_cb {
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
                        mask.unwrap_or(EventCallbackMask::ALL),
                    )
                    .to_lua_err()?;
                } else {
                    this.unset_callback().to_lua_err()?;
                }

                Ok(())
            },
        );
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct EventDescription {
    pub(crate) ptr: *mut FMOD_STUDIO_EVENTDESCRIPTION,
}

unsafe impl Send for EventDescription {}
unsafe impl Sync for EventDescription {}

impl EventDescription {
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

    pub(crate) unsafe fn from_ptr(ptr: *mut FMOD_STUDIO_EVENTDESCRIPTION) -> Result<Self> {
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

    pub fn unset_callback(&self) -> Result<()> {
        unsafe {
            if let Some(ud_ptr) = self.get_userdata().unwrap() {
                Arc::decr_strong_count(ud_ptr);
            }

            FMOD_Studio_EventDescription_SetUserData(self.ptr, ptr::null_mut()).check_err()?;
            FMOD_Studio_EventDescription_SetCallback(
                self.ptr,
                None,
                EventCallbackMask::empty().bits(),
            )
            .check_err()?;
        }
        Ok(())
    }
}

impl LuaUserData for EventDescription {
    fn add_methods<'lua, T: LuaUserDataMethods<'lua, Self>>(methods: &mut T) {
        methods.add_method("create_instance", |_lua, this, ()| {
            this.create_instance().to_lua_err()
        });

        methods.add_method(
            "set_callback",
            |lua, this, (maybe_cb, mask): (Option<LuaFunction>, Option<EventCallbackMask>)| {
                if let Some(cb) = maybe_cb {
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
                        mask.unwrap_or(EventCallbackMask::ALL),
                    )
                    .to_lua_err()?;
                } else {
                    this.unset_callback().to_lua_err()?;
                }

                Ok(())
            },
        );
    }
}

fn load<'lua>(lua: LuaContext<'lua>) -> Result<LuaValue<'lua>> {
    let table = lua.create_table_from(vec![
        ("CREATED", EventCallbackMask::CREATED),
        ("DESTROYED", EventCallbackMask::DESTROYED),
        ("STARTING", EventCallbackMask::STARTING),
        ("STARTED", EventCallbackMask::STARTED),
        ("RESTARTED", EventCallbackMask::RESTARTED),
        ("STOPPED", EventCallbackMask::STOPPED),
        ("START_FAILED", EventCallbackMask::START_FAILED),
        (
            "CREATE_PROGRAMMER_SOUND",
            EventCallbackMask::CREATE_PROGRAMMER_SOUND,
        ),
        (
            "DESTROY_PROGRAMMER_SOUND",
            EventCallbackMask::DESTROY_PROGRAMMER_SOUND,
        ),
        ("PLUGIN_CREATED", EventCallbackMask::PLUGIN_CREATED),
        ("PLUGIN_DESTROYED", EventCallbackMask::PLUGIN_DESTROYED),
        ("TIMELINE_MARKER", EventCallbackMask::TIMELINE_MARKER),
        ("TIMELINE_BEAT", EventCallbackMask::TIMELINE_BEAT),
        ("SOUND_PLAYED", EventCallbackMask::SOUND_PLAYED),
        ("SOUND_STOPPED", EventCallbackMask::SOUND_STOPPED),
        ("REAL_TO_VIRTUAL", EventCallbackMask::REAL_TO_VIRTUAL),
        ("VIRTUAL_TO_REAL", EventCallbackMask::VIRTUAL_TO_REAL),
        (
            "START_EVENT_COMMAND",
            EventCallbackMask::START_EVENT_COMMAND,
        ),
        ("ALL", EventCallbackMask::ALL),
    ])?;

    Ok(LuaValue::Table(table))
}

inventory::submit! {
    Module::parse("fmod.EventCallbackMask", load)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem;

    #[test]
    fn parameter_id_size_and_layout() {
        assert_eq!(
            mem::size_of::<FMOD_STUDIO_PARAMETER_ID>(),
            mem::size_of::<ParameterId>()
        );

        let c_param: FMOD_STUDIO_PARAMETER_ID;
        let rust_param = ParameterId {
            data1: 1234,
            data2: 5678,
        };

        unsafe {
            c_param = *(&rust_param as *const ParameterId as *const FMOD_STUDIO_PARAMETER_ID);
        };

        assert_eq!(rust_param.data1, c_param.data1);
        assert_eq!(rust_param.data2, c_param.data2);
    }
}
