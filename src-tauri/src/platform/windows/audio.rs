//! System audio for the visualisers: WASAPI loopback capture of the default
//! render endpoint (what the speakers play), analysed by `crate::dsp`.
//!
//! One thread per [`AudioHandle`]: COM (MTA) → default render endpoint
//! (eRender/eConsole) → shared-mode IAudioClient in loopback mode on the
//! device mix format → IAudioCaptureClient polled every 10 ms → mono →
//! analyser → `PlatformEvent::Audio` at ≤ 60 Hz. An IMMNotificationClient
//! restarts capture on the new device when the default output changes.
//! Dropping the handle stops the thread within one poll interval.
//!
//! Loopback delivers no packets at all while nothing plays, so silence is
//! synthesised then: the analysis and the PCM window the renderer reads decay
//! to zero instead of freezing on the last sound, and once everything is
//! silent no more frames are sent until audio returns.

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use ::windows::core::{Error, Result, GUID, HRESULT, PCWSTR};
use ::windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use ::windows::Win32::Foundation::PROPERTYKEY;
use ::windows::Win32::Media::Audio::{
    eConsole, eRender, EDataFlow, ERole, IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator,
    IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT,
    AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, DEVICE_STATE, WAVEFORMATEX,
    WAVEFORMATEXTENSIBLE, WAVE_FORMAT_PCM,
};
use ::windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, STGM_READ};
use windows_core::implement;

use super::util::ComApartment;
use crate::dsp::{self, Analyzer, SampleFormat};
use crate::platform::{AudioHandle, EventSink, PlatformError, PlatformEvent, PlatformResult};

const POLL: Duration = Duration::from_millis(10);
/// Shared-mode buffer: 1 s (100 ns units), ample for a 10 ms poll.
const BUFFER_DURATION: i64 = 10_000_000;
/// No packet for this long means nothing is playing.
const SILENCE_AFTER: Duration = Duration::from_millis(60);
/// At most ~60 frames per second.
const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(16);
/// Consecutive silent frames (~2 s) before going idle.
const SILENT_FRAMES_BEFORE_IDLE: u32 = 120;
const RETRY: Duration = Duration::from_secs(2);

const WAVE_FORMAT_IEEE_FLOAT: u32 = 0x0003;
const WAVE_FORMAT_EXTENSIBLE: u32 = 0xFFFE;
const SUBTYPE_PCM: GUID = GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);
const SUBTYPE_IEEE_FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
/// HRESULT_FROM_WIN32(ERROR_NOT_FOUND): no default render endpoint.
const E_NOT_FOUND: HRESULT = HRESULT(0x8007_0490_u32 as i32);

pub fn start_audio(sink: EventSink) -> PlatformResult<AudioHandle> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    thread::Builder::new()
        .name("lunalyr-audio".into())
        .spawn(move || capture_thread(&sink, &flag))
        .map_err(|error| PlatformError::msg(format!("could not start audio capture: {error}")))?;
    // Not joined: dropping the handle must not block the caller; the thread
    // sees the flag within one poll interval and releases WASAPI itself.
    Ok(AudioHandle::new(move || stop.store(true, Ordering::SeqCst)))
}

/// Default-device notifications, forwarded to the capture thread. Called on
/// an MMDevice API thread: must not block or call back into the enumerator.
#[implement(IMMNotificationClient)]
struct DefaultDeviceWatcher {
    changed: Sender<()>,
}

impl IMMNotificationClient_Impl for DefaultDeviceWatcher_Impl {
    fn OnDeviceStateChanged(&self, _device: &PCWSTR, _state: DEVICE_STATE) -> Result<()> {
        Ok(())
    }

    fn OnDeviceAdded(&self, _device: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn OnDeviceRemoved(&self, _device: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _device: &PCWSTR) -> Result<()> {
        if flow == eRender && role == eConsole {
            let _ = self.changed.send(());
        }
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _device: &PCWSTR, _key: &PROPERTYKEY) -> Result<()> {
        Ok(())
    }
}

/// Sends `AudioStatus` only when the text changes.
struct Status<'a> {
    sink: &'a EventSink,
    last: Option<String>,
}

impl Status<'_> {
    fn set(&mut self, text: impl Into<String>) {
        let text = text.into();
        if self.last.as_deref() != Some(text.as_str()) {
            self.last = Some(text.clone());
            let _ = self.sink.send(PlatformEvent::AudioStatus(text));
        }
    }
}

fn capture_thread(sink: &EventSink, stop: &AtomicBool) {
    let mut status = Status { sink, last: None };
    let _com = match ComApartment::multithreaded() {
        Ok(com) => com,
        Err(error) => {
            status.set(format!("Audio capture error: {error}"));
            return;
        }
    };
    let enumerator: IMMDeviceEnumerator = match unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) } {
        Ok(enumerator) => enumerator,
        Err(error) => {
            status.set(format!("Audio capture error: {error}"));
            return;
        }
    };
    let (changed_tx, changed_rx) = mpsc::channel();
    let watcher: IMMNotificationClient = DefaultDeviceWatcher { changed: changed_tx }.into();
    let registered = match unsafe { enumerator.RegisterEndpointNotificationCallback(&watcher) } {
        Ok(()) => true,
        Err(error) => {
            log::warn!("audio: default-device notifications unavailable: {error}");
            false
        }
    };

    let mut analyzer = Analyzer::new(48_000);
    while !stop.load(Ordering::SeqCst) && !sink.is_closed() {
        drain(&changed_rx);
        match Capture::open(&enumerator) {
            Ok(capture) => {
                log::info!(
                    "audio: capturing \"{}\" ({} Hz, {} ch, {:?})",
                    capture.device_name,
                    capture.format.rate,
                    capture.format.channels,
                    capture.format.sample
                );
                status.set("Receiving audio");
                if analyzer.sample_rate() != capture.format.rate {
                    analyzer.reset(capture.format.rate);
                }
                match capture.run(sink, stop, &changed_rx, &mut analyzer) {
                    Ended::Stopped => break,
                    Ended::DeviceChanged => log::info!("audio: default output changed; restarting capture"),
                    Ended::Failed(error) if error.code() == AUDCLNT_E_DEVICE_INVALIDATED => {
                        log::info!("audio: the output device went away; restarting capture");
                        if !pause(stop, &changed_rx, Duration::from_millis(250)) {
                            break;
                        }
                    }
                    Ended::Failed(error) => {
                        log::warn!("audio: capture failed: {error}");
                        status.set(format!("Audio capture error: {error}"));
                        if !pause(stop, &changed_rx, RETRY) {
                            break;
                        }
                    }
                }
            }
            Err(OpenError::NoDevice) => {
                status.set("No audio device");
                if !pause(stop, &changed_rx, RETRY) {
                    break;
                }
            }
            Err(OpenError::Unsupported(description)) => {
                status.set(format!("Audio capture error: unsupported output format ({description})"));
                if !pause(stop, &changed_rx, RETRY * 5) {
                    break;
                }
            }
            Err(OpenError::Failed(error)) => {
                log::warn!("audio: could not open the output device: {error}");
                status.set(format!("Audio capture error: {error}"));
                if !pause(stop, &changed_rx, RETRY) {
                    break;
                }
            }
        }
    }
    if registered {
        unsafe {
            let _ = enumerator.UnregisterEndpointNotificationCallback(&watcher);
        }
    }
    log::info!("audio: capture stopped");
}

fn drain(changed: &Receiver<()>) {
    while changed.try_recv().is_ok() {}
}

/// Waits up to `timeout`, returning early when the default device changes.
/// False if capture should stop.
fn pause(stop: &AtomicBool, changed: &Receiver<()>, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if stop.load(Ordering::SeqCst) {
            return false;
        }
        match changed.recv_timeout(Duration::from_millis(50)) {
            Ok(()) => return true,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return !stop.load(Ordering::SeqCst),
        }
    }
    !stop.load(Ordering::SeqCst)
}

#[derive(Debug, Clone, Copy)]
struct Format {
    sample: SampleFormat,
    channels: usize,
    block_align: usize,
    rate: u32,
}

enum OpenError {
    NoDevice,
    Unsupported(String),
    Failed(Error),
}

impl From<Error> for OpenError {
    fn from(error: Error) -> Self {
        OpenError::Failed(error)
    }
}

enum Ended {
    Stopped,
    DeviceChanged,
    Failed(Error),
}

/// A started loopback stream; stopped on drop.
struct Capture {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    format: Format,
    device_name: String,
}

/// Frees the mix format returned by GetMixFormat.
struct MixFormat(*mut WAVEFORMATEX);

impl Drop for MixFormat {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CoTaskMemFree(Some(self.0 as *const c_void)) };
        }
    }
}

impl Capture {
    fn open(enumerator: &IMMDeviceEnumerator) -> std::result::Result<Capture, OpenError> {
        unsafe {
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).map_err(|error| {
                if error.code() == E_NOT_FOUND {
                    OpenError::NoDevice
                } else {
                    OpenError::Failed(error)
                }
            })?;
            let device_name = device_name(&device);
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
            let mix = MixFormat(client.GetMixFormat()?);
            let format = parse_format(mix.0).ok_or_else(|| OpenError::Unsupported(describe_format(mix.0)))?;
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                BUFFER_DURATION,
                0,
                mix.0,
                None,
            )?;
            drop(mix);
            let capture: IAudioCaptureClient = client.GetService()?;
            client.Start()?;
            Ok(Capture { client, capture, format, device_name })
        }
    }

    fn run(&self, sink: &EventSink, stop: &AtomicBool, changed: &Receiver<()>, analyzer: &mut Analyzer) -> Ended {
        let rate = f64::from(self.format.rate);
        let mut mono: Vec<f32> = Vec::with_capacity(self.format.rate as usize / 10);
        let mut last_packet = Instant::now();
        let mut silence_fed_until = Instant::now();
        let mut last_frame = Instant::now();
        let mut fresh = false;
        let mut silent_frames = 0u32;
        loop {
            if stop.load(Ordering::SeqCst) || sink.is_closed() {
                return Ended::Stopped;
            }
            match changed.try_recv() {
                Ok(()) => {
                    drain(changed);
                    return Ended::DeviceChanged;
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
            }

            mono.clear();
            let mut got_packets = false;
            loop {
                let pending = match unsafe { self.capture.GetNextPacketSize() } {
                    Ok(frames) => frames,
                    Err(error) => return Ended::Failed(error),
                };
                if pending == 0 {
                    break;
                }
                let mut data: *mut u8 = ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                if let Err(error) = unsafe { self.capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None) } {
                    return Ended::Failed(error);
                }
                if frames > 0 {
                    if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                        mono.resize(mono.len() + frames as usize, 0.0);
                    } else {
                        // SAFETY: WASAPI guarantees `frames` frames of
                        // `nBlockAlign` bytes at `data` until ReleaseBuffer.
                        let bytes =
                            unsafe { std::slice::from_raw_parts(data, frames as usize * self.format.block_align) };
                        dsp::downmix(
                            bytes,
                            self.format.sample,
                            self.format.channels,
                            self.format.block_align,
                            &mut mono,
                        );
                    }
                }
                if let Err(error) = unsafe { self.capture.ReleaseBuffer(frames) } {
                    return Ended::Failed(error);
                }
                got_packets = true;
                if mono.len() >= self.format.rate as usize {
                    break;
                }
            }

            let now = Instant::now();
            if got_packets {
                last_packet = now;
                silence_fed_until = now;
            } else if now.duration_since(last_packet) >= SILENCE_AFTER && silent_frames < SILENT_FRAMES_BEFORE_IDLE {
                let gap = now.duration_since(silence_fed_until).min(Duration::from_millis(500));
                mono.resize((gap.as_secs_f64() * rate) as usize, 0.0);
                silence_fed_until = now;
            }
            if !mono.is_empty() && analyzer.push(&mono) {
                fresh = true;
            }
            if fresh && now.duration_since(last_frame) >= MIN_FRAME_INTERVAL {
                fresh = false;
                last_frame = now;
                silent_frames = if analyzer.is_silent() { silent_frames.saturating_add(1) } else { 0 };
                if silent_frames <= SILENT_FRAMES_BEFORE_IDLE {
                    let _ = sink.send(PlatformEvent::Audio(analyzer.frame()));
                }
            }
            thread::sleep(POLL);
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}

unsafe fn device_name(device: &IMMDevice) -> String {
    device
        .OpenPropertyStore(STGM_READ)
        .and_then(|store| store.GetValue(&PKEY_Device_FriendlyName))
        .map(|value| value.to_string())
        .unwrap_or_default()
}

/// Reads a mix format: IEEE float or integer PCM, plain or
/// WAVEFORMATEXTENSIBLE. The struct is packed, so it is copied out with an
/// unaligned read and its fields are only read by value.
unsafe fn parse_format(format: *const WAVEFORMATEX) -> Option<Format> {
    if format.is_null() {
        return None;
    }
    let base = ptr::read_unaligned(format);
    let tag = u32::from(base.wFormatTag);
    let is_float = match tag {
        WAVE_FORMAT_IEEE_FLOAT => true,
        WAVE_FORMAT_PCM => false,
        WAVE_FORMAT_EXTENSIBLE if base.cbSize >= 22 => {
            let extensible = ptr::read_unaligned(format as *const WAVEFORMATEXTENSIBLE);
            let sub_format = extensible.SubFormat;
            if sub_format == SUBTYPE_IEEE_FLOAT {
                true
            } else if sub_format == SUBTYPE_PCM {
                false
            } else {
                return None;
            }
        }
        _ => return None,
    };
    let sample = SampleFormat::from_wave(is_float, base.wBitsPerSample)?;
    let channels = usize::from(base.nChannels);
    let block_align = usize::from(base.nBlockAlign);
    let rate = base.nSamplesPerSec;
    if !(1..=32).contains(&channels) || block_align < channels * sample.bytes() || !(8_000..=384_000).contains(&rate) {
        return None;
    }
    Some(Format { sample, channels, block_align, rate })
}

unsafe fn describe_format(format: *const WAVEFORMATEX) -> String {
    if format.is_null() {
        return "none".into();
    }
    let base = ptr::read_unaligned(format);
    let (tag, bits, channels, rate) = (base.wFormatTag, base.wBitsPerSample, base.nChannels, base.nSamplesPerSec);
    format!("tag {tag:#06x}, {bits} bit, {channels} ch, {rate} Hz")
}
