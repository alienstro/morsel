//! System audio: the "Others" stream, through a Core Audio process tap (macOS 14.2 or later).
//!
//! Steps: a global stereo tap, a private aggregate device that holds the tap and the default
//! output device (for the clock), and a C IO proc that resamples to 16 kHz mono.

use super::mic::SampleSink;
use super::resample::Resampler;
use super::source::AudioSource;
use crate::types::{MorselError, Result};
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_audio::{
    AudioDeviceCreateIOProcID, AudioDeviceDestroyIOProcID, AudioDeviceIOProcID, AudioDeviceStart, AudioDeviceStop,
    AudioHardwareCreateAggregateDevice, AudioHardwareCreateProcessTap, AudioHardwareDestroyAggregateDevice,
    AudioHardwareDestroyProcessTap, AudioObjectGetPropertyData, AudioObjectID, AudioObjectPropertyAddress,
    CATapDescription, CATapMuteBehavior, kAudioAggregateDeviceIsPrivateKey, kAudioAggregateDeviceIsStackedKey,
    kAudioAggregateDeviceMainSubDeviceKey, kAudioAggregateDeviceNameKey, kAudioAggregateDeviceSubDeviceListKey,
    kAudioAggregateDeviceTapAutoStartKey, kAudioAggregateDeviceTapListKey, kAudioAggregateDeviceUIDKey,
    kAudioDevicePropertyDeviceUID, kAudioHardwarePropertyDefaultSystemOutputDevice, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, kAudioSubDeviceUIDKey,
    kAudioSubTapDriftCompensationKey, kAudioSubTapUIDKey, kAudioTapPropertyFormat,
};
use objc2_core_audio_types::{AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp};
use objc2_core_foundation::{CFDictionary, CFRetained, CFString};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSProcessInfo, NSString};
use std::ffi::{CStr, c_void};
use std::mem::MaybeUninit;
use std::ptr::NonNull;
use tokio::sync::mpsc::Sender;

/// True when the macOS version has process taps (14.2 or later).
pub fn tap_supported(major: u32, minor: u32) -> bool {
    major > 14 || (major == 14 && minor >= 2)
}

/// Makes one interleaved buffer from one buffer for each channel.
pub fn interleave(planes: &[&[f32]]) -> Vec<f32> {
    let frames = planes.iter().map(|p| p.len()).min().unwrap_or(0);
    let mut out = Vec::with_capacity(frames * planes.len());
    for i in 0..frames {
        for plane in planes {
            out.push(plane[i]);
        }
    }
    out
}

struct CallbackState {
    resampler: Resampler,
    sink: SampleSink,
}

struct Running {
    tap: AudioObjectID,
    aggregate: AudioObjectID,
    proc_id: AudioDeviceIOProcID,
    state: *mut CallbackState,
}

pub struct SystemTapSource {
    running: Option<Running>,
}

// The raw ids and the state pointer are used only from `start` and `stop`,
// and the IO proc is destroyed before the state is freed.
unsafe impl Send for SystemTapSource {}

impl SystemTapSource {
    pub fn new() -> Self {
        Self { running: None }
    }
}

impl Default for SystemTapSource {
    fn default() -> Self {
        Self::new()
    }
}

fn check(status: i32, what: &str) -> Result<()> {
    if status == 0 { Ok(()) } else { Err(MorselError::Audio(format!("{what} failed (OSStatus {status})"))) }
}

fn get_property<T: Copy>(object: AudioObjectID, selector: u32) -> Result<T> {
    let address = AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    };
    let mut size = std::mem::size_of::<T>() as u32;
    let mut value = MaybeUninit::<T>::zeroed();
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new_unchecked(value.as_mut_ptr().cast()),
        )
    };
    check(status, "reading an audio property")?;
    Ok(unsafe { value.assume_init() })
}

fn default_output_uid() -> Result<String> {
    let device: AudioObjectID = get_property(kAudioObjectSystemObject as AudioObjectID, kAudioHardwarePropertyDefaultSystemOutputDevice)?;
    let raw: *const CFString = get_property(device, kAudioDevicePropertyDeviceUID)?;
    let uid = NonNull::new(raw as *mut CFString)
        .map(|ptr| unsafe { CFRetained::from_raw(ptr) })
        .ok_or_else(|| MorselError::Audio("the output device has no id".into()))?;
    Ok(uid.to_string())
}

fn key(name: &CStr) -> Retained<NSString> {
    NSString::from_str(name.to_str().unwrap_or_default())
}

fn dictionary(entries: &[(&CStr, &AnyObject)]) -> Retained<NSDictionary<NSString, AnyObject>> {
    let keys: Vec<Retained<NSString>> = entries.iter().map(|(k, _)| key(k)).collect();
    let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    let values: Vec<&AnyObject> = entries.iter().map(|(_, v)| *v).collect();
    NSDictionary::from_slices(&key_refs, &values)
}

fn macos_version() -> (u32, u32) {
    let v = NSProcessInfo::processInfo().operatingSystemVersion();
    (v.majorVersion as u32, v.minorVersion as u32)
}

unsafe extern "C-unwind" fn io_proc(
    _device: AudioObjectID,
    _now: NonNull<AudioTimeStamp>,
    input: NonNull<AudioBufferList>,
    _input_time: NonNull<AudioTimeStamp>,
    _output: NonNull<AudioBufferList>,
    _output_time: NonNull<AudioTimeStamp>,
    client: *mut c_void,
) -> i32 {
    let state = unsafe { &mut *(client as *mut CallbackState) };
    let list = unsafe { input.as_ref() };
    let count = list.mNumberBuffers as usize;
    let buffers = unsafe { std::slice::from_raw_parts(list.mBuffers.as_ptr(), count) };
    let planes: Vec<&[f32]> = buffers
        .iter()
        .filter(|b| !b.mData.is_null())
        .map(|b| unsafe { std::slice::from_raw_parts(b.mData as *const f32, b.mDataByteSize as usize / 4) })
        .collect();
    let samples = match planes.len() {
        0 => return 0,
        1 => planes[0].to_vec(),
        _ => interleave(&planes),
    };
    let out = state.resampler.process(&samples);
    if !out.is_empty() {
        (state.sink)(out);
    }
    0
}

impl Running {
    fn tear_down(self) {
        unsafe {
            if self.proc_id.is_some() {
                AudioDeviceStop(self.aggregate, self.proc_id);
                AudioDeviceDestroyIOProcID(self.aggregate, self.proc_id);
            }
            if self.aggregate != 0 {
                AudioHardwareDestroyAggregateDevice(self.aggregate);
            }
            if self.tap != 0 {
                AudioHardwareDestroyProcessTap(self.tap);
            }
            if !self.state.is_null() {
                drop(Box::from_raw(self.state));
            }
        }
    }
}

impl SystemTapSource {
    fn start_inner(&mut self, tx: Sender<Vec<f32>>, run: &mut Running) -> Result<()> {
        let description = unsafe {
            let d = CATapDescription::initStereoGlobalTapButExcludeProcesses(CATapDescription::alloc(), &NSArray::new());
            d.setPrivate(true);
            d.setMuteBehavior(CATapMuteBehavior::Unmuted);
            d.setName(&NSString::from_str("Morsel"));
            d
        };
        let status = unsafe { AudioHardwareCreateProcessTap(Some(&description), &mut run.tap) };
        if status != 0 {
            return Err(MorselError::SystemAudioPermissionDenied);
        }
        let tap_uid = unsafe { description.UUID().UUIDString() };
        let format: AudioStreamBasicDescription = get_property(run.tap, kAudioTapPropertyFormat)?;
        let output_uid = NSString::from_str(&default_output_uid()?);

        let yes = NSNumber::new_bool(true);
        let no = NSNumber::new_bool(false);
        let sub_tap = dictionary(&[(kAudioSubTapUIDKey, &tap_uid), (kAudioSubTapDriftCompensationKey, &yes)]);
        let sub_device = dictionary(&[(kAudioSubDeviceUIDKey, &output_uid)]);
        let taps = NSArray::from_retained_slice(&[sub_tap]);
        let devices = NSArray::from_retained_slice(&[sub_device]);
        let name = NSString::from_str("Morsel system audio");
        let uid = NSString::from_str(&format!("app.morsel.tap.{}", std::process::id()));
        let aggregate = dictionary(&[
            (kAudioAggregateDeviceNameKey, &name),
            (kAudioAggregateDeviceUIDKey, &uid),
            (kAudioAggregateDeviceMainSubDeviceKey, &output_uid),
            (kAudioAggregateDeviceIsPrivateKey, &yes),
            (kAudioAggregateDeviceIsStackedKey, &no),
            (kAudioAggregateDeviceTapAutoStartKey, &yes),
            (kAudioAggregateDeviceSubDeviceListKey, &devices),
            (kAudioAggregateDeviceTapListKey, &taps),
        ]);
        let cf: &CFDictionary = unsafe { &*(Retained::as_ptr(&aggregate).cast::<CFDictionary>()) };
        check(
            unsafe { AudioHardwareCreateAggregateDevice(cf, NonNull::from(&mut run.aggregate)) },
            "creating the aggregate device",
        )?;

        let resampler = Resampler::new(format.mSampleRate as u32, format.mChannelsPerFrame.max(1) as u16)?;
        let sink: SampleSink = Box::new(move |samples| {
            let _ = tx.try_send(samples);
        });
        run.state = Box::into_raw(Box::new(CallbackState { resampler, sink }));
        check(
            unsafe {
                AudioDeviceCreateIOProcID(run.aggregate, Some(io_proc), run.state.cast(), NonNull::from(&mut run.proc_id))
            },
            "creating the IO proc",
        )?;
        check(unsafe { AudioDeviceStart(run.aggregate, run.proc_id) }, "starting system audio")?;
        Ok(())
    }
}

impl AudioSource for SystemTapSource {
    fn start(&mut self, tx: Sender<Vec<f32>>) -> Result<()> {
        if self.running.is_some() {
            return Err(MorselError::Audio("system audio is already running".into()));
        }
        let (major, minor) = macos_version();
        if !tap_supported(major, minor) {
            return Err(MorselError::SystemAudioUnsupported);
        }
        let mut run = Running { tap: 0, aggregate: 0, proc_id: None, state: std::ptr::null_mut() };
        match self.start_inner(tx, &mut run) {
            Ok(()) => {
                self.running = Some(run);
                Ok(())
            }
            Err(err) => {
                run.tear_down();
                Err(err)
            }
        }
    }

    fn stop(&mut self) {
        if let Some(run) = self.running.take() {
            run.tear_down();
        }
    }
}

impl Drop for SystemTapSource {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_os_version_returns_unsupported() {
        assert!(!tap_supported(13, 6));
        assert!(!tap_supported(14, 1));
        assert!(tap_supported(14, 2));
        assert!(tap_supported(15, 0));
        assert!(tap_supported(27, 2));
    }

    #[test]
    fn interleave_merges_planar_buffers() {
        let left = [1.0, 2.0];
        let right = [3.0, 4.0];
        assert_eq!(interleave(&[&left, &right]), vec![1.0, 3.0, 2.0, 4.0]);
    }
}
