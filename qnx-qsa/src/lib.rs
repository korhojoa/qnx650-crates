// SPDX-License-Identifier: MIT OR Apache-2.0
//! The QNX Sound Architecture on QNX 6.5 through its client library
//! `libasound.so`: PCM playback through the plugin layer ([`PcmDevice`])
//! and the volume of a mixer group ([`Mixer`]).
use std::ffi::{c_char, c_int, c_void, CStr};
use std::fmt;
use std::mem::{offset_of, size_of};
const OPEN_PLAYBACK: c_int = 1;
const CHANNEL_PLAYBACK: i32 = 0;
const SAMPLE_FORMAT_S16_LE: i32 = 4;
const MODE_BLOCK: i32 = 1;
const START_FULL: i32 = 2;
const STOP_STOP: i32 = 1;
pub const CHANNEL_FRONT_LEFT: usize = 0;
pub const CHANNEL_FRONT_RIGHT: usize = 1;
pub const GROUP_MAX_VOICES: usize = 32;
pub const GROUP_CAP_VOLUME: u32 = 1;
#[repr(C)]
struct PcmFormat {
    interleave: u32,
    format: i32,
    rate: i32,
    voices: i32,
    special: i32,
    reserved: [u8; 124],
}
#[repr(C)]
struct PcmDigital([u8; 304]);
#[repr(C)]
struct ChannelParams {
    channel: i32,
    mode: i32,
    sync: [u64; 2],
    format: PcmFormat,
    digital: PcmDigital,
    start_mode: i32,
    stop_mode: i32,
    time_flags: i32,
    why_failed: u32,
    fragment_bytes: i32,
    fragments_min: i32,
    fragments_max: i32,
    buffer_reserved: [u8; 124],
    sw_mixer_subchannel_name: [u8; 32],
    reserved: [u8; 96],
}
#[repr(C)]
struct ChannelSetup {
    channel: i32,
    mode: i32,
    format: PcmFormat,
    digital: PcmDigital,
    fragment_bytes: i32,
    fragments: i32,
    fragments_min: i32,
    fragments_max: i32,
    max_fragment_bytes: u32,
    buffer_reserved: [u8; 124],
    msbits_per_sample: i16,
    pad1: i16,
    mixer_device: i32,
    mixer_element: u32,
    mixer_group: u32,
    mmap_flags: [u8; 4],
    mixer_card: i32,
    reserved: [u8; 104],
}
#[repr(C)]
struct ChannelStatus {
    channel: i32,
    mode: i32,
    status: i32,
    processed_bytes: u32,
    start_time: [u32; 2],
    start_time_ust: u64,
    fragment: i32,
    buffered_bytes: i32,
    free_bytes: i32,
    underruns: i32,
    overruns: i32,
    overrange: i32,
    plugin_held_bytes: u32,
    reserved: [u8; 124],
}
#[repr(C)]
#[derive(Clone, Copy)]
struct RawGroupId {
    kind: i32,
    name: [u8; 32],
    index: i32,
    reserved: [u8; 124],
    weight: i32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct RawGroup {
    gid: RawGroupId,
    caps: u32,
    channels: u32,
    min: i32,
    max: i32,
    volume: [u32; GROUP_MAX_VOICES],
    volume_rest: [u8; 24],
    mute: u32,
    capture: u32,
    capture_group: i32,
    elements_size: i32,
    elements: i32,
    elements_over: i32,
    elements_list: *mut c_void,
    change_duration: u16,
    spare: u16,
    min_db: i32,
    max_db: i32,
    reserved: [u8; 120],
}
#[repr(C)]
struct RawGroups {
    groups: i32,
    groups_size: i32,
    groups_over: i32,
    zero: [u8; 4],
    list: *mut RawGroupId,
    pzero: *mut c_void,
    reserved: [u8; 128],
}
const _: () = assert!(size_of::<PcmFormat>() == 144);
const _: () = assert!(offset_of!(PcmFormat, reserved) == 20);
const _: () = assert!(size_of::<PcmDigital>() == 304);
const _: () = assert!(size_of::<ChannelParams>() == 752);
const _: () = assert!(offset_of!(ChannelParams, mode) == 4);
const _: () = assert!(offset_of!(ChannelParams, sync) == 8);
const _: () = assert!(offset_of!(ChannelParams, format) == 24);
const _: () = assert!(offset_of!(ChannelParams, digital) == 168);
const _: () = assert!(offset_of!(ChannelParams, start_mode) == 472);
const _: () = assert!(offset_of!(ChannelParams, stop_mode) == 476);
const _: () = assert!(offset_of!(ChannelParams, why_failed) == 484);
const _: () = assert!(offset_of!(ChannelParams, fragment_bytes) == 488);
const _: () = assert!(offset_of!(ChannelParams, fragments_min) == 492);
const _: () = assert!(offset_of!(ChannelParams, fragments_max) == 496);
const _: () = assert!(offset_of!(ChannelParams, sw_mixer_subchannel_name) == 624);
const _: () = assert!(offset_of!(ChannelParams, reserved) == 656);
const _: () = assert!(size_of::<ChannelSetup>() == 728);
const _: () = assert!(offset_of!(ChannelSetup, format) == 8);
const _: () = assert!(offset_of!(ChannelSetup, digital) == 152);
const _: () = assert!(offset_of!(ChannelSetup, fragment_bytes) == 456);
const _: () = assert!(offset_of!(ChannelSetup, fragments) == 460);
const _: () = assert!(offset_of!(ChannelSetup, fragments_min) == 464);
const _: () = assert!(offset_of!(ChannelSetup, fragments_max) == 468);
const _: () = assert!(offset_of!(ChannelSetup, max_fragment_bytes) == 472);
const _: () = assert!(offset_of!(ChannelSetup, msbits_per_sample) == 600);
const _: () = assert!(offset_of!(ChannelSetup, mixer_device) == 604);
const _: () = assert!(offset_of!(ChannelSetup, mixer_element) == 608);
const _: () = assert!(offset_of!(ChannelSetup, mixer_group) == 612);
const _: () = assert!(offset_of!(ChannelSetup, mixer_card) == 620);
const _: () = assert!(offset_of!(ChannelSetup, reserved) == 624);
const _: () = assert!(size_of::<ChannelStatus>() == 184);
const _: () = assert!(offset_of!(ChannelStatus, status) == 8);
const _: () = assert!(offset_of!(ChannelStatus, processed_bytes) == 12);
const _: () = assert!(offset_of!(ChannelStatus, start_time) == 16);
const _: () = assert!(offset_of!(ChannelStatus, start_time_ust) == 24);
const _: () = assert!(offset_of!(ChannelStatus, fragment) == 32);
const _: () = assert!(offset_of!(ChannelStatus, buffered_bytes) == 36);
const _: () = assert!(offset_of!(ChannelStatus, underruns) == 44);
const _: () = assert!(offset_of!(ChannelStatus, plugin_held_bytes) == 56);
const _: () = assert!(offset_of!(ChannelStatus, reserved) == 60);
const _: () = assert!(size_of::<RawGroupId>() == 168);
const _: () = assert!(offset_of!(RawGroupId, name) == 4);
const _: () = assert!(offset_of!(RawGroupId, index) == 36);
const _: () = assert!(offset_of!(RawGroupId, weight) == 164);
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<RawGroup>() == 496);
    assert!(offset_of!(RawGroup, caps) == 168);
    assert!(offset_of!(RawGroup, channels) == 172);
    assert!(offset_of!(RawGroup, min) == 176);
    assert!(offset_of!(RawGroup, max) == 180);
    assert!(offset_of!(RawGroup, volume) == 184);
    assert!(offset_of!(RawGroup, mute) == 336);
    assert!(offset_of!(RawGroup, elements_size) == 348);
    assert!(offset_of!(RawGroup, elements_list) == 360);
    assert!(offset_of!(RawGroup, change_duration) == 364);
    assert!(offset_of!(RawGroup, min_db) == 368);
    assert!(offset_of!(RawGroup, reserved) == 376);
    assert!(size_of::<RawGroups>() == 152);
    assert!(offset_of!(RawGroups, groups_size) == 4);
    assert!(offset_of!(RawGroups, groups_over) == 8);
    assert!(offset_of!(RawGroups, list) == 16);
    assert!(offset_of!(RawGroups, pzero) == 20);
    assert!(offset_of!(RawGroups, reserved) == 24);
};
fn zeroed<T: Zeroable>() -> T {
    // SAFETY: `Zeroable` is implemented only for the `repr(C)` structures
    // above. Their fields are integers, byte arrays and raw pointers, for
    // which all bits zero is a valid value (a null pointer).
    unsafe { std::mem::zeroed() }
}
trait Zeroable {}
impl Zeroable for ChannelParams {}
impl Zeroable for ChannelSetup {}
impl Zeroable for ChannelStatus {}
impl Zeroable for RawGroupId {}
impl Zeroable for RawGroup {}
impl Zeroable for RawGroups {}
#[cfg(target_os = "nto")]
mod sys {
    use super::{
        c_char, c_int, c_void, ChannelParams, ChannelSetup, ChannelStatus, RawGroup, RawGroups,
    };
    #[link(name = "asound")]
    extern "C" {
        pub fn snd_pcm_open_preferred(
            handle: *mut *mut c_void,
            card: *mut c_int,
            device: *mut c_int,
            mode: c_int,
        ) -> c_int;
        pub fn snd_pcm_close(handle: *mut c_void) -> c_int;
        pub fn snd_pcm_plugin_params(handle: *mut c_void, params: *mut ChannelParams) -> c_int;
        pub fn snd_pcm_plugin_setup(handle: *mut c_void, setup: *mut ChannelSetup) -> c_int;
        pub fn snd_pcm_plugin_status(handle: *mut c_void, status: *mut ChannelStatus) -> c_int;
        pub fn snd_pcm_plugin_prepare(handle: *mut c_void, channel: c_int) -> c_int;
        pub fn snd_pcm_plugin_write(
            handle: *mut c_void,
            buffer: *const c_void,
            size: usize,
        ) -> isize;
        pub fn snd_pcm_plugin_flush(handle: *mut c_void, channel: c_int) -> c_int;
        pub fn snd_pcm_plugin_playback_drain(handle: *mut c_void) -> c_int;
        pub fn snd_strerror(code: c_int) -> *const c_char;
        pub fn snd_mixer_open(handle: *mut *mut c_void, card: c_int, device: c_int) -> c_int;
        pub fn snd_mixer_close(handle: *mut c_void) -> c_int;
        pub fn snd_mixer_groups(handle: *mut c_void, groups: *mut RawGroups) -> c_int;
        pub fn snd_mixer_group_read(handle: *mut c_void, group: *mut RawGroup) -> c_int;
        pub fn snd_mixer_group_write(handle: *mut c_void, group: *mut RawGroup) -> c_int;
    }
}
#[cfg(not(target_os = "nto"))]
#[allow(clippy::unnecessary_wraps, clippy::missing_safety_doc)]
mod sys {
    use super::{
        c_char, c_int, c_void, ChannelParams, ChannelSetup, ChannelStatus, RawGroup, RawGroups,
    };
    pub unsafe fn snd_pcm_open_preferred(
        _handle: *mut *mut c_void,
        _card: *mut c_int,
        _device: *mut c_int,
        _mode: c_int,
    ) -> c_int {
        -1
    }
    pub unsafe fn snd_pcm_close(_handle: *mut c_void) -> c_int {
        -1
    }
    pub unsafe fn snd_pcm_plugin_params(
        _handle: *mut c_void,
        _params: *mut ChannelParams,
    ) -> c_int {
        -1
    }
    pub unsafe fn snd_pcm_plugin_setup(_handle: *mut c_void, _setup: *mut ChannelSetup) -> c_int {
        -1
    }
    pub unsafe fn snd_pcm_plugin_status(
        _handle: *mut c_void,
        _status: *mut ChannelStatus,
    ) -> c_int {
        -1
    }
    pub unsafe fn snd_pcm_plugin_prepare(_handle: *mut c_void, _channel: c_int) -> c_int {
        -1
    }
    pub unsafe fn snd_pcm_plugin_write(
        _handle: *mut c_void,
        _buffer: *const c_void,
        _size: usize,
    ) -> isize {
        -1
    }
    pub unsafe fn snd_pcm_plugin_flush(_handle: *mut c_void, _channel: c_int) -> c_int {
        -1
    }
    pub unsafe fn snd_pcm_plugin_playback_drain(_handle: *mut c_void) -> c_int {
        -1
    }
    pub unsafe fn snd_strerror(_code: c_int) -> *const c_char {
        std::ptr::null()
    }
    pub unsafe fn snd_mixer_open(_handle: *mut *mut c_void, _card: c_int, _device: c_int) -> c_int {
        -1
    }
    pub unsafe fn snd_mixer_close(_handle: *mut c_void) -> c_int {
        -1
    }
    pub unsafe fn snd_mixer_groups(_handle: *mut c_void, _groups: *mut RawGroups) -> c_int {
        -1
    }
    pub unsafe fn snd_mixer_group_read(_handle: *mut c_void, _group: *mut RawGroup) -> c_int {
        -1
    }
    pub unsafe fn snd_mixer_group_write(_handle: *mut c_void, _group: *mut RawGroup) -> c_int {
        -1
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QsaError {
    pub call: &'static str,
    pub code: i32,
    pub text: String,
}
impl fmt::Display for QsaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} returned {} ({})", self.call, self.code, self.text)
    }
}
impl std::error::Error for QsaError {}
#[must_use]
pub fn strerror(code: i32) -> Option<String> {
    // SAFETY: `snd_strerror` returns a static NUL-terminated string or
    // null. The host stand-in returns null.
    unsafe {
        let text = sys::snd_strerror(code);
        if text.is_null() {
            None
        } else {
            Some(CStr::from_ptr(text).to_string_lossy().into_owned())
        }
    }
}
fn error(call: &'static str, code: c_int) -> QsaError {
    let text = strerror(code).unwrap_or_else(|| String::from("no text"));
    QsaError { call, code, text }
}
fn check(call: &'static str, code: c_int) -> Result<(), QsaError> {
    if code < 0 {
        Err(error(call, code))
    } else {
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceState {
    NotReady,
    Ready,
    Prepared,
    Running,
    Underrun,
    Overrun,
    Paused,
    Error,
    Change,
    Other(i32),
}
impl DeviceState {
    #[must_use]
    pub fn from_raw(raw: i32) -> Self {
        match raw {
            0 => Self::NotReady,
            1 => Self::Ready,
            2 => Self::Prepared,
            3 => Self::Running,
            4 => Self::Underrun,
            5 => Self::Overrun,
            10 => Self::Paused,
            10_000 => Self::Error,
            10_001 => Self::Change,
            other => Self::Other(other),
        }
    }
    #[must_use]
    pub fn raw(self) -> i32 {
        match self {
            Self::NotReady => 0,
            Self::Ready => 1,
            Self::Prepared => 2,
            Self::Running => 3,
            Self::Underrun => 4,
            Self::Overrun => 5,
            Self::Paused => 10,
            Self::Error => 10_000,
            Self::Change => 10_001,
            Self::Other(other) => other,
        }
    }
}
impl fmt::Display for DeviceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotReady => f.write_str("not ready"),
            Self::Ready => f.write_str("ready"),
            Self::Prepared => f.write_str("prepared"),
            Self::Running => f.write_str("running"),
            Self::Underrun => f.write_str("underrun"),
            Self::Overrun => f.write_str("overrun"),
            Self::Paused => f.write_str("paused"),
            Self::Error => f.write_str("error"),
            Self::Change => f.write_str("changed"),
            Self::Other(raw) => write!(f, "state {raw}"),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaybackConfig {
    pub rate: u32,
    pub voices: u32,
    pub fragment_bytes: usize,
    pub fragments_min: usize,
    pub fragments_max: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub rate: i32,
    pub voices: i32,
    pub sample_format: i32,
    pub fragment_bytes: i32,
    pub fragments: i32,
    pub fragments_min: i32,
    pub fragments_max: i32,
    pub max_fragment_bytes: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceStatus {
    pub state: DeviceState,
    pub processed_bytes: u32,
    pub buffered_bytes: i32,
    pub free_bytes: i32,
    pub underruns: i32,
    pub plugin_held_bytes: u32,
}
pub struct PcmDevice {
    handle: *mut c_void,
    pub card: i32,
    pub device: i32,
}
impl PcmDevice {
    /// # Errors
    /// The failed `snd_pcm_open_preferred` (always on a host).
    pub fn open_preferred() -> Result<Self, QsaError> {
        let mut handle: *mut c_void = std::ptr::null_mut();
        let (mut card, mut device) = (0, 0);
        // SAFETY: three live out-parameters.
        let code = unsafe {
            sys::snd_pcm_open_preferred(
                &raw mut handle,
                &raw mut card,
                &raw mut device,
                OPEN_PLAYBACK,
            )
        };
        check("snd_pcm_open_preferred", code)?;
        if handle.is_null() {
            return Err(error("snd_pcm_open_preferred", -1));
        }
        Ok(Self {
            handle,
            card,
            device,
        })
    }
    /// # Errors
    /// The failed call. A refusal of the parameters carries the library's
    /// `why_failed` value in its text when it set one.
    pub fn configure(&self, config: &PlaybackConfig) -> Result<Geometry, QsaError> {
        let int = |value: usize| i32::try_from(value).unwrap_or(i32::MAX);
        let mut params: ChannelParams = zeroed();
        params.channel = CHANNEL_PLAYBACK;
        params.mode = MODE_BLOCK;
        params.format.interleave = 1;
        params.format.format = SAMPLE_FORMAT_S16_LE;
        params.format.rate = i32::try_from(config.rate).unwrap_or(i32::MAX);
        params.format.voices = i32::try_from(config.voices).unwrap_or(i32::MAX);
        params.start_mode = START_FULL;
        params.stop_mode = STOP_STOP;
        params.fragment_bytes = int(config.fragment_bytes);
        params.fragments_min = int(config.fragments_min);
        params.fragments_max = int(config.fragments_max);
        // SAFETY: an open handle and a structure of the library's size.
        let code = unsafe { sys::snd_pcm_plugin_params(self.handle, &raw mut params) };
        if code < 0 {
            let mut failure = error("snd_pcm_plugin_params", code);
            if params.why_failed != 0 {
                use fmt::Write;
                let _ = write!(failure.text, "; why_failed {}", params.why_failed);
            }
            return Err(failure);
        }
        self.prepare()?;
        let mut setup: ChannelSetup = zeroed();
        setup.channel = CHANNEL_PLAYBACK;
        // SAFETY: an open handle and a structure of the library's size.
        let code = unsafe { sys::snd_pcm_plugin_setup(self.handle, &raw mut setup) };
        check("snd_pcm_plugin_setup", code)?;
        Ok(Geometry {
            rate: setup.format.rate,
            voices: setup.format.voices,
            sample_format: setup.format.format,
            fragment_bytes: setup.fragment_bytes,
            fragments: setup.fragments,
            fragments_min: setup.fragments_min,
            fragments_max: setup.fragments_max,
            max_fragment_bytes: setup.max_fragment_bytes,
        })
    }
    /// # Errors
    /// The failed `snd_pcm_plugin_prepare`.
    pub fn prepare(&self) -> Result<(), QsaError> {
        // SAFETY: an open handle.
        let code = unsafe { sys::snd_pcm_plugin_prepare(self.handle, CHANNEL_PLAYBACK) };
        check("snd_pcm_plugin_prepare", code)
    }
    #[must_use]
    pub fn write(&self, data: &[u8]) -> isize {
        // SAFETY: an open handle. `data` is live for the call.
        unsafe { sys::snd_pcm_plugin_write(self.handle, data.as_ptr().cast(), data.len()) }
    }
    /// # Errors
    /// The failed `snd_pcm_plugin_status`.
    pub fn status(&self) -> Result<DeviceStatus, QsaError> {
        let mut status: ChannelStatus = zeroed();
        status.channel = CHANNEL_PLAYBACK;
        // SAFETY: an open handle and a structure of the library's size.
        let code = unsafe { sys::snd_pcm_plugin_status(self.handle, &raw mut status) };
        check("snd_pcm_plugin_status", code)?;
        Ok(DeviceStatus {
            state: DeviceState::from_raw(status.status),
            processed_bytes: status.processed_bytes,
            buffered_bytes: status.buffered_bytes,
            free_bytes: status.free_bytes,
            underruns: status.underruns,
            plugin_held_bytes: status.plugin_held_bytes,
        })
    }
    /// # Errors
    /// The failed `snd_pcm_plugin_flush`.
    pub fn play_out(&self) -> Result<(), QsaError> {
        // SAFETY: an open handle.
        let code = unsafe { sys::snd_pcm_plugin_flush(self.handle, CHANNEL_PLAYBACK) };
        check("snd_pcm_plugin_flush", code)
    }
    /// # Errors
    /// The failed `snd_pcm_plugin_playback_drain`.
    pub fn discard(&self) -> Result<(), QsaError> {
        // SAFETY: an open handle.
        let code = unsafe { sys::snd_pcm_plugin_playback_drain(self.handle) };
        check("snd_pcm_plugin_playback_drain", code)
    }
}
impl Drop for PcmDevice {
    fn drop(&mut self) {
        // SAFETY: the handle this value opened, closed once.
        unsafe {
            sys::snd_pcm_close(self.handle);
        }
    }
}
#[derive(Clone, Copy)]
pub struct GroupId(RawGroupId);
impl GroupId {
    #[must_use]
    pub fn name(&self) -> String {
        let end = self.0.name.iter().position(|&b| b == 0).unwrap_or(32);
        String::from_utf8_lossy(&self.0.name[..end]).into_owned()
    }
    #[must_use]
    pub fn index(&self) -> i32 {
        self.0.index
    }
}
impl fmt::Debug for GroupId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GroupId")
            .field("name", &self.name())
            .field("index", &self.0.index)
            .finish()
    }
}
#[derive(Clone, Copy)]
pub struct Group(RawGroup);
impl Group {
    #[must_use]
    pub fn id(&self) -> GroupId {
        GroupId(self.0.gid)
    }
    #[must_use]
    pub fn caps(&self) -> u32 {
        self.0.caps
    }
    #[must_use]
    pub fn channels(&self) -> u32 {
        self.0.channels
    }
    #[must_use]
    pub fn range(&self) -> (i32, i32) {
        (self.0.min, self.0.max)
    }
    #[must_use]
    pub fn volume(&self, channel: usize) -> Option<u32> {
        self.0.volume.get(channel).copied()
    }
    pub fn set_volume(&mut self, channel: usize, value: u32) {
        if let Some(slot) = self.0.volume.get_mut(channel) {
            *slot = value;
        }
    }
    #[must_use]
    pub fn mute(&self) -> u32 {
        self.0.mute
    }
}
impl fmt::Debug for Group {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Group")
            .field("id", &self.id())
            .field("caps", &self.0.caps)
            .field("channels", &self.0.channels)
            .field("range", &self.range())
            .field("mute", &self.0.mute)
            .finish_non_exhaustive()
    }
}
pub struct Mixer {
    handle: *mut c_void,
}
impl Mixer {
    /// # Errors
    /// The failed `snd_mixer_open` (always on a host).
    pub fn open(card: i32, device: i32) -> Result<Self, QsaError> {
        let mut handle: *mut c_void = std::ptr::null_mut();
        // SAFETY: a live out-parameter.
        let code = unsafe { sys::snd_mixer_open(&raw mut handle, card, device) };
        check("snd_mixer_open", code)?;
        if handle.is_null() {
            return Err(error("snd_mixer_open", -1));
        }
        Ok(Self { handle })
    }
    /// # Errors
    /// The failed `snd_mixer_groups`.
    pub fn group_ids(&self) -> Result<Vec<GroupId>, QsaError> {
        let mut list: Vec<RawGroupId> = vec![zeroed(); 16];
        loop {
            let mut groups: RawGroups = zeroed();
            groups.groups_size = i32::try_from(list.len()).unwrap_or(i32::MAX);
            groups.list = list.as_mut_ptr();
            // SAFETY: an open handle. `list` holds `groups_size` entries
            // and lives for the call.
            let code = unsafe { sys::snd_mixer_groups(self.handle, &raw mut groups) };
            check("snd_mixer_groups", code)?;
            let filled = usize::try_from(groups.groups).unwrap_or(0).min(list.len());
            let over = usize::try_from(groups.groups_over).unwrap_or(0);
            if over == 0 || list.len() >= 1024 {
                list.truncate(filled);
                return Ok(list.into_iter().map(GroupId).collect());
            }
            list.resize(filled + over, zeroed());
        }
    }
    /// # Errors
    /// The failed `snd_mixer_group_read`.
    pub fn read_group(&self, id: &GroupId) -> Result<Group, QsaError> {
        let mut group: RawGroup = zeroed();
        group.gid = id.0;
        // SAFETY: an open handle and a structure of the library's size,
        // with a null element list so the library fills none.
        let code = unsafe { sys::snd_mixer_group_read(self.handle, &raw mut group) };
        check("snd_mixer_group_read", code)?;
        Ok(Group(group))
    }
    /// # Errors
    /// The failed `snd_mixer_group_write`.
    pub fn write_group(&self, group: &Group) -> Result<(), QsaError> {
        let mut raw = group.0;
        // SAFETY: an open handle and a structure of the library's size,
        // with a null element list.
        let code = unsafe { sys::snd_mixer_group_write(self.handle, &raw mut raw) };
        check("snd_mixer_group_write", code)
    }
}
impl Drop for Mixer {
    fn drop(&mut self) {
        // SAFETY: the handle this value opened, closed once.
        unsafe {
            sys::snd_mixer_close(self.handle);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn device_state_values_round_trip() {
        for raw in [0, 1, 2, 3, 4, 5, 10, 10_000, 10_001, 77] {
            assert_eq!(DeviceState::from_raw(raw).raw(), raw);
        }
        assert_eq!(DeviceState::from_raw(4), DeviceState::Underrun);
        assert_eq!(DeviceState::from_raw(3).to_string(), "running");
    }
    #[test]
    fn calls_fail_off_qnx() {
        if cfg!(target_os = "nto") {
            return;
        }
        let failure = PcmDevice::open_preferred()
            .err()
            .expect("no libasound here");
        assert_eq!(failure.call, "snd_pcm_open_preferred");
        assert_eq!(failure.code, -1);
        assert_eq!(failure.text, "no text");
        assert_eq!(strerror(-1), None);
        let failure = Mixer::open(0, 0).err().expect("no libasound here");
        assert_eq!(failure.call, "snd_mixer_open");
    }
    #[cfg(target_os = "nto")]
    #[test]
    fn links_every_call() {
        let calls: [*const (); 15] = [
            sys::snd_pcm_open_preferred as *const (),
            sys::snd_pcm_close as *const (),
            sys::snd_pcm_plugin_params as *const (),
            sys::snd_pcm_plugin_setup as *const (),
            sys::snd_pcm_plugin_status as *const (),
            sys::snd_pcm_plugin_prepare as *const (),
            sys::snd_pcm_plugin_write as *const (),
            sys::snd_pcm_plugin_flush as *const (),
            sys::snd_pcm_plugin_playback_drain as *const (),
            sys::snd_strerror as *const (),
            sys::snd_mixer_open as *const (),
            sys::snd_mixer_close as *const (),
            sys::snd_mixer_groups as *const (),
            sys::snd_mixer_group_read as *const (),
            sys::snd_mixer_group_write as *const (),
        ];
        assert!(calls.iter().all(|call| !call.is_null()));
    }
    #[test]
    fn group_volume_and_name() {
        let mut raw: RawGroup = zeroed();
        raw.gid.name[..3].copy_from_slice(b"PCM");
        raw.gid.index = 2;
        let mut group = Group(raw);
        group.set_volume(CHANNEL_FRONT_RIGHT, 70);
        group.set_volume(GROUP_MAX_VOICES, 1);
        assert_eq!(group.volume(CHANNEL_FRONT_RIGHT), Some(70));
        assert_eq!(group.volume(CHANNEL_FRONT_LEFT), Some(0));
        assert_eq!(group.volume(GROUP_MAX_VOICES), None);
        assert_eq!(group.id().name(), "PCM");
        assert_eq!(group.id().index(), 2);
    }
}
