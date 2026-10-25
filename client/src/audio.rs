use client_api::gta::matrix::{CVector, RwMatrix};
use client_api::gta::menu_manager::CMenuManager;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use crossbeam_channel::{Receiver, Sender};
use crossbeam_queue::ArrayQueue;
use hrtf::{HrirSphere, HrtfContext, HrtfProcessor, Vec3};
use nalgebra::{Point3, Rotation3, Vector3};
use parking_lot::{Mutex, RwLock};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;
use winapi::um::winuser::{GetForegroundWindow, IsIconic};

pub const MAX_DISTANCE: f32 = 50.0;
pub const REFRENCE_DISTANCE: f32 = 15.0;

const HRTF_INTERPOLATION_STEPS: usize = 2;
const HRTF_BLOCK_LEN: usize = 128;
const MIX_FRAMES: usize = HRTF_INTERPOLATION_STEPS * HRTF_BLOCK_LEN;
const OUTPUT_QUEUE_BLOCKS: usize = 8;
const OUTPUT_TARGET_BLOCKS: usize = 4;
const COMMAND_QUEUE_CAPACITY: usize = 256;
const INPUT_PREBUFFER_MS: usize = 40;
const MAX_INPUT_BUFFER_MS: usize = 250;

const HRIR_SPHERE: &[u8] = include_bytes!("../assets/IRC_1002_C.bin");

#[derive(Copy, Clone)]
pub struct BrowserAudioSettings {
    pub max_distance: f32,
    pub reference_distance: f32,
}

struct Listener {
    position: Point3<f32>,
    rotation: Rotation3<f32>,
}

#[derive(Clone, Copy)]
struct StreamFormat {
    channels: usize,
    spatial: bool,
}

type StereoFrame = (f32, f32);

enum Command {
    Stream {
        browser: u32,
        stream_id: i32,
        sample_rate: i32,
        spatial: bool,
    },
    Source {
        browser: u32,
        object_id: i32,
    },
    Pcm {
        browser: u32,
        stream_id: i32,
        data: Vec<StereoFrame>,
    },
    RemoveStream {
        browser: u32,
        stream_id: i32,
    },
    RemoveAllStreams {
        browser: u32,
    },
    RemoveSource {
        browser: u32,
        object_id: i32,
    },
    ObjectSettings {
        object_id: i32,
        position: Point3<f32>,
        velocity: Point3<f32>,
        settings: BrowserAudioSettings,
    },
    Gain(f32),
    MuteObject(i32),
    TogglePause(bool),
    Terminate,
}

pub struct Audio {
    command_tx: Sender<Command>,
    stream_formats: RwLock<HashMap<(u32, i32), StreamFormat>>,
    listener: Mutex<Listener>,
    muted_objects: Mutex<HashSet<i32>>,
    pcm_seen: AtomicBool,
    paused: AtomicBool,
    gain_bits: AtomicU32,
}

impl Audio {
    pub fn new() -> Arc<Self> {
        let (command_tx, command_rx) = crossbeam_channel::bounded(COMMAND_QUEUE_CAPACITY);

        std::thread::Builder::new()
            .name("cef-audio-mixer".to_owned())
            .spawn(move || audio_thread(command_rx))
            .expect("cannot spawn audio mixer thread");

        Arc::new(Self {
            command_tx,
            stream_formats: RwLock::new(HashMap::new()),
            listener: Mutex::new(Listener {
                position: Point3::origin(),
                rotation: Rotation3::identity(),
            }),
            muted_objects: Mutex::new(HashSet::new()),
            pcm_seen: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            gain_bits: AtomicU32::new(1.0f32.to_bits()),
        })
    }

    fn send_control(&self, command: Command) {
        if let Err(error) = self.command_tx.send(command) {
            tracing::error!(%error, "audio mixer is unavailable");
        }
    }

    pub fn create_stream(
        &self, browser: u32, stream_id: i32, channels: i32, sample_rate: i32, _max_frames: i32,
        spatial: bool,
    ) {
        self.stream_formats.write().insert(
            (browser, stream_id),
            StreamFormat {
                channels: channels.max(1) as usize,
                spatial,
            },
        );
        tracing::info!(
            browser,
            stream_id,
            sample_rate,
            channels,
            spatial,
            "audio stream started"
        );
        self.send_control(Command::Stream {
            browser,
            stream_id,
            sample_rate: sample_rate.max(1),
            spatial,
        });
    }

    /// # Safety
    /// `data` must point to `channels` valid planar PCM buffers containing `frames` samples each.
    pub unsafe fn append_pcm(
        &self, browser: u32, stream_id: i32, data: *mut *const f32, frames: i32, pts: u64,
    ) {
        if frames <= 0 || data.is_null() {
            return;
        }

        if crate::utils::current_time() - pts as i128 >= 1000 {
            return;
        }

        if !self.pcm_seen.swap(true, Ordering::Relaxed) {
            tracing::info!(browser, stream_id, frames, "first audio PCM received");
        }

        let format = self
            .stream_formats
            .read()
            .get(&(browser, stream_id))
            .copied()
            .unwrap_or(StreamFormat {
                channels: 1,
                spatial: false,
            });
        let frames = frames as usize;
        let mut stereo = Vec::with_capacity(frames);

        unsafe {
            let channels = std::slice::from_raw_parts(data, format.channels);
            if format.spatial {
                for frame in 0..frames {
                    let mut mono = 0.0;
                    let mut valid_channels = 0;
                    for &channel in channels {
                        if !channel.is_null() {
                            mono += *channel.add(frame);
                            valid_channels += 1;
                        }
                    }
                    if valid_channels > 0 {
                        mono /= valid_channels as f32;
                    }
                    stereo.push((mono, mono));
                }
            } else {
                let left = channels[0];
                let right = channels.get(1).copied().unwrap_or(left);
                for frame in 0..frames {
                    let left = if left.is_null() {
                        0.0
                    } else {
                        *left.add(frame)
                    };
                    let right = if right.is_null() {
                        left
                    } else {
                        *right.add(frame)
                    };
                    stereo.push((left, right));
                }
            }
        }

        // PCM is intentionally lossy under overload: keeping old audio would increase A/V latency.
        let _ = self.command_tx.try_send(Command::Pcm {
            browser,
            stream_id,
            data: stereo,
        });
    }

    pub fn remove_stream(&self, browser: u32, stream_id: i32) {
        self.stream_formats.write().remove(&(browser, stream_id));
        self.send_control(Command::RemoveStream { browser, stream_id });
    }

    pub fn remove_all_streams(&self, browser: u32) {
        self.stream_formats
            .write()
            .retain(|(stream_browser, _), _| *stream_browser != browser);
        self.send_control(Command::RemoveAllStreams { browser });
    }

    pub fn add_source(&self, browser: u32, object_id: i32) {
        self.muted_objects.lock().remove(&object_id);
        self.send_control(Command::Source { browser, object_id });
    }

    pub fn remove_source(&self, browser: u32, object_id: i32) {
        self.send_control(Command::RemoveSource { browser, object_id });
    }

    pub fn set_gain(&self, gain: f32) {
        let gain = gain.max(0.0);
        let gain_bits = gain.to_bits();
        if self.gain_bits.swap(gain_bits, Ordering::Relaxed) != gain_bits {
            self.send_control(Command::Gain(gain));
        }
    }

    pub fn set_velocity(&self, _velocity: CVector) {}

    pub fn set_position(&self, position: CVector) {
        self.listener.lock().position = Point3::new(position.x, position.y, position.z);
    }

    pub fn set_orientation(&self, matrix: RwMatrix) {
        let at = Vector3::new(-matrix.at.x, -matrix.at.y, -matrix.at.z);
        let up = Vector3::new(matrix.up.x, matrix.up.y, matrix.up.z);
        self.listener.lock().rotation = Rotation3::face_towards(&at, &up);
    }

    pub fn set_object_settings(
        &self, object_id: i32, position: CVector, velocity: CVector, _direction: CVector,
        settings: BrowserAudioSettings,
    ) {
        self.muted_objects.lock().remove(&object_id);
        let relative_position = {
            let listener = self.listener.lock();
            let relative = Point3::new(
                position.x - listener.position.x,
                position.y - listener.position.y,
                position.z - listener.position.z,
            );
            listener.rotation.transform_point(&relative)
        };

        self.send_control(Command::ObjectSettings {
            object_id,
            position: relative_position,
            velocity: Point3::new(velocity.x, velocity.y, velocity.z),
            settings,
        });
    }

    pub fn object_mute(&self, object_id: i32) {
        if self.muted_objects.lock().insert(object_id) {
            self.send_control(Command::MuteObject(object_id));
        }
    }

    pub fn set_paused(&self, paused: bool) {
        if self.paused.swap(paused, Ordering::AcqRel) != paused {
            self.send_control(Command::TogglePause(paused));
        }
    }

    pub fn terminate(&self) {
        self.send_control(Command::Terminate);
    }
}

struct SpatialSource {
    object_id: i32,
    position: Point3<f32>,
    #[allow(dead_code)]
    velocity: Point3<f32>,
    settings: BrowserAudioSettings,
    muted: bool,
    previous_direction: Vec3,
    previous_gain: f32,
    previous_left: Vec<f32>,
    previous_right: Vec<f32>,
    rendered_once: bool,
}

impl SpatialSource {
    fn new(object_id: i32) -> Self {
        Self {
            object_id,
            position: Point3::new(0.0, 0.0, 1.0),
            velocity: Point3::origin(),
            settings: BrowserAudioSettings {
                max_distance: MAX_DISTANCE,
                reference_distance: REFRENCE_DISTANCE,
            },
            muted: true,
            previous_direction: Vec3::new(0.0, 0.0, 1.0),
            previous_gain: 0.0,
            previous_left: Vec::new(),
            previous_right: Vec::new(),
            rendered_once: false,
        }
    }

    fn reset_history(&mut self) {
        self.previous_gain = 0.0;
        self.previous_left.clear();
        self.previous_right.clear();
    }
}

struct AudioStream {
    stream_id: i32,
    sample_rate: u32,
    spatial: bool,
    input: VecDeque<StereoFrame>,
    resample_phase: f64,
    playing: bool,
    sources: HashMap<i32, SpatialSource>,
}

impl AudioStream {
    fn new(stream_id: i32, sample_rate: i32, spatial: bool) -> Self {
        Self {
            stream_id,
            sample_rate: sample_rate as u32,
            spatial,
            input: VecDeque::new(),
            resample_phase: 0.0,
            playing: false,
            sources: HashMap::new(),
        }
    }

    fn append(&mut self, samples: Vec<StereoFrame>) {
        self.input.extend(samples);
        let max_len = self.sample_rate as usize * MAX_INPUT_BUFFER_MS / 1000;
        if self.input.len() > max_len {
            let drop_count = self.input.len() - max_len;
            self.input.drain(..drop_count);
            self.resample_phase = 0.0;
        }
    }

    fn render_stereo(&mut self, output_rate: u32, output: &mut [StereoFrame; MIX_FRAMES]) -> bool {
        output.fill((0.0, 0.0));
        let prebuffer = self.sample_rate as usize * INPUT_PREBUFFER_MS / 1000;
        if !self.playing {
            if self.input.len() < prebuffer.max(2) {
                return false;
            }
            self.playing = true;
        }

        let step = self.sample_rate as f64 / output_rate as f64;
        let required = (self.resample_phase + step * (MIX_FRAMES - 1) as f64).floor() as usize + 2;
        if self.input.len() < required {
            self.input.clear();
            self.resample_phase = 0.0;
            self.playing = false;
            return false;
        }

        for sample in output.iter_mut() {
            let index = self.resample_phase.floor() as usize;
            let fraction = (self.resample_phase - index as f64) as f32;
            let a = self.input[index];
            let b = self.input[index + 1];
            *sample = (a.0 + (b.0 - a.0) * fraction, a.1 + (b.1 - a.1) * fraction);
            self.resample_phase += step;
        }

        let consumed = self.resample_phase.floor() as usize;
        self.input.drain(..consumed);
        self.resample_phase -= consumed as f64;
        true
    }

    fn render_mono(&mut self, output_rate: u32, output: &mut [f32; MIX_FRAMES]) -> bool {
        let mut stereo = [(0.0, 0.0); MIX_FRAMES];
        if !self.render_stereo(output_rate, &mut stereo) {
            output.fill(0.0);
            return false;
        }

        for (output, (left, right)) in output.iter_mut().zip(stereo) {
            *output = (left + right) * 0.5;
        }
        true
    }
}

struct Mixer {
    output_rate: u32,
    gain: f32,
    paused: bool,
    streams: HashMap<u32, Vec<AudioStream>>,
    hrtf: Option<HrtfProcessor>,
}

impl Mixer {
    fn new(output_rate: u32) -> Self {
        let hrtf = match HrirSphere::new(Cursor::new(HRIR_SPHERE), output_rate) {
            Ok(sphere) => Some(HrtfProcessor::new(
                sphere,
                HRTF_INTERPOLATION_STEPS,
                HRTF_BLOCK_LEN,
            )),
            Err(error) => {
                tracing::warn!(?error, "cannot initialize HRTF; using stereo panning");
                None
            }
        };

        Self {
            output_rate,
            gain: 1.0,
            paused: false,
            streams: HashMap::new(),
            hrtf,
        }
    }

    fn handle(&mut self, command: Command) -> bool {
        match command {
            Command::Stream {
                browser,
                stream_id,
                sample_rate,
                spatial,
            } => self
                .streams
                .entry(browser)
                .or_default()
                .push(AudioStream::new(stream_id, sample_rate, spatial)),
            Command::Source { browser, object_id } => {
                if let Some(streams) = self.streams.get_mut(&browser) {
                    for stream in streams {
                        stream
                            .sources
                            .entry(object_id)
                            .or_insert_with(|| SpatialSource::new(object_id));
                    }
                }
            }
            Command::Pcm {
                browser,
                stream_id,
                data,
            } => {
                if !self.paused
                    && let Some(stream) = self
                        .streams
                        .get_mut(&browser)
                        .and_then(|streams| streams.iter_mut().find(|s| s.stream_id == stream_id))
                {
                    stream.append(data);
                }
            }
            Command::RemoveStream { browser, stream_id } => {
                if let Some(streams) = self.streams.get_mut(&browser) {
                    streams.retain(|stream| stream.stream_id != stream_id);
                    if streams.is_empty() {
                        self.streams.remove(&browser);
                    }
                }
            }
            Command::RemoveAllStreams { browser } => {
                self.streams.remove(&browser);
            }
            Command::RemoveSource { browser, object_id } => {
                if let Some(streams) = self.streams.get_mut(&browser) {
                    for stream in streams {
                        stream.sources.remove(&object_id);
                    }
                }
            }
            Command::ObjectSettings {
                object_id,
                position,
                velocity,
                settings,
            } => self.for_object(object_id, |source| {
                source.position = position;
                source.velocity = velocity;
                source.settings = settings;
                source.muted = false;
            }),
            Command::Gain(gain) => self.gain = gain,
            Command::MuteObject(object_id) => self.for_object(object_id, |source| {
                source.muted = true;
                source.reset_history();
            }),
            Command::TogglePause(paused) => self.set_paused(paused),
            Command::Terminate => return false,
        }
        true
    }

    fn set_paused(&mut self, paused: bool) {
        if self.paused == paused {
            return;
        }

        self.paused = paused;
        tracing::debug!(paused, "browser audio pause state changed");

        if paused {
            for streams in self.streams.values_mut() {
                for stream in streams {
                    stream.input.clear();
                    stream.playing = false;
                    stream.resample_phase = 0.0;
                }
            }
        }
    }

    fn for_object(&mut self, object_id: i32, mut callback: impl FnMut(&mut SpatialSource)) {
        for streams in self.streams.values_mut() {
            for stream in streams {
                if let Some(source) = stream.sources.get_mut(&object_id) {
                    callback(source);
                }
            }
        }
    }

    fn render(&mut self) -> [(f32, f32); MIX_FRAMES] {
        let mut output = [(0.0, 0.0); MIX_FRAMES];
        if self.paused {
            return output;
        }

        let mut mono = [0.0; MIX_FRAMES];
        let mut stereo = [(0.0, 0.0); MIX_FRAMES];
        for streams in self.streams.values_mut() {
            for stream in streams {
                if stream.spatial {
                    if !stream.render_mono(self.output_rate, &mut mono) {
                        continue;
                    }

                    for source in stream.sources.values_mut().filter(|source| !source.muted) {
                        render_spatial_source(
                            self.hrtf.as_mut(),
                            source,
                            &mono,
                            &mut output,
                            self.gain,
                        );
                    }
                } else if stream.render_stereo(self.output_rate, &mut stereo) {
                    for (output, (left, right)) in output.iter_mut().zip(stereo) {
                        output.0 += left * self.gain;
                        output.1 += right * self.gain;
                    }
                }
            }
        }
        output
    }
}

fn distance_gain(position: Point3<f32>, settings: BrowserAudioSettings) -> f32 {
    let distance = position.coords.norm();
    let reference = settings.reference_distance.max(0.0);
    let maximum = settings.max_distance.max(reference + f32::EPSILON);
    if distance <= reference {
        1.0
    } else if distance >= maximum {
        0.0
    } else {
        (maximum - distance) / (maximum - reference)
    }
}

fn source_direction(position: Point3<f32>) -> Vec3 {
    let length_squared = position.coords.norm_squared();
    if length_squared <= f32::EPSILON {
        return Vec3::new(0.0, 0.0, 1.0);
    }
    let direction = -position.coords / length_squared.sqrt();
    Vec3::new(direction.x, direction.y, direction.z)
}

fn render_spatial_source(
    processor: Option<&mut HrtfProcessor>, source: &mut SpatialSource, mono: &[f32; MIX_FRAMES],
    output: &mut [(f32, f32); MIX_FRAMES], master_gain: f32,
) {
    let direction = source_direction(source.position);
    let gain = distance_gain(source.position, source.settings) * master_gain;
    let hrtf_enabled = processor.is_some();

    if !source.rendered_once {
        tracing::info!(
            object_id = source.object_id,
            gain,
            hrtf_enabled,
            "first spatial block rendered"
        );
        source.rendered_once = true;
    }

    if let Some(processor) = processor {
        processor.process_samples(HrtfContext {
            source: mono,
            output,
            new_sample_vector: direction,
            prev_sample_vector: source.previous_direction,
            prev_left_samples: &mut source.previous_left,
            prev_right_samples: &mut source.previous_right,
            new_distance_gain: gain,
            prev_distance_gain: source.previous_gain,
        });
    } else {
        // Equal-power panning is kept as a graceful fallback for a corrupt HRIR asset.
        let pan = direction.x.clamp(-1.0, 1.0);
        let left_gain = ((1.0 - pan) * 0.5).sqrt() * gain;
        let right_gain = ((1.0 + pan) * 0.5).sqrt() * gain;
        for ((left, right), sample) in output.iter_mut().zip(mono) {
            *left += *sample * left_gain;
            *right += *sample * right_gain;
        }
    }

    source.previous_direction = direction;
    source.previous_gain = gain;
}

fn write_output<T>(data: &mut [T], channels: usize, output: &ArrayQueue<f32>)
where
    T: SizedSample + FromSample<f32>,
{
    for frame in data.chunks_mut(channels) {
        let left = output.pop().unwrap_or(0.0).clamp(-1.0, 1.0);
        let right = output.pop().unwrap_or(0.0).clamp(-1.0, 1.0);
        if channels == 1 {
            frame[0] = T::from_sample((left + right) * 0.5);
        } else {
            frame[0] = T::from_sample(left);
            frame[1] = T::from_sample(right);
            for sample in &mut frame[2..] {
                *sample = T::from_sample(0.0);
            }
        }
    }
}

fn build_output_stream(
    device: &cpal::Device, config: cpal::StreamConfig, format: SampleFormat,
    output: Arc<ArrayQueue<f32>>,
) -> Result<cpal::Stream, cpal::Error> {
    let channels = config.channels as usize;
    let error_callback = |error| tracing::error!(%error, "audio output stream failed");

    macro_rules! stream {
        ($sample:ty) => {{
            let output = Arc::clone(&output);
            device.build_output_stream::<$sample, _, _>(
                config,
                move |data, _| write_output(data, channels, &output),
                error_callback,
                Some(Duration::from_secs(2)),
            )
        }};
    }

    match format {
        SampleFormat::I8 => stream!(i8),
        SampleFormat::I16 => stream!(i16),
        SampleFormat::I32 => stream!(i32),
        SampleFormat::I64 => stream!(i64),
        SampleFormat::U8 => stream!(u8),
        SampleFormat::U16 => stream!(u16),
        SampleFormat::U32 => stream!(u32),
        SampleFormat::U64 => stream!(u64),
        SampleFormat::F32 => stream!(f32),
        SampleFormat::F64 => stream!(f64),
        _ => Err(cpal::Error::with_message(
            cpal::ErrorKind::InvalidInput,
            format!("unsupported output sample format: {format:?}"),
        )),
    }
}

fn audio_thread(command_rx: Receiver<Command>) {
    let host = cpal::default_host();
    let Some(device) = host.default_output_device() else {
        tracing::error!("no default audio output device");
        return;
    };
    let supported = match device.default_output_config() {
        Ok(config) => config,
        Err(error) => {
            tracing::error!(%error, "cannot query default audio output format");
            return;
        }
    };
    let config = supported.config();
    if config.channels == 0 {
        tracing::error!("default audio output has no channels");
        return;
    }

    let output = Arc::new(ArrayQueue::new(OUTPUT_QUEUE_BLOCKS * MIX_FRAMES * 2));
    let stream = match build_output_stream(
        &device,
        config,
        supported.sample_format(),
        Arc::clone(&output),
    ) {
        Ok(stream) => stream,
        Err(error) => {
            tracing::error!(%error, "cannot open default audio output");
            return;
        }
    };

    let mut mixer = Mixer::new(config.sample_rate);
    tracing::info!(
        sample_rate = config.sample_rate,
        channels = config.channels,
        sample_format = ?supported.sample_format(),
        hrtf_enabled = mixer.hrtf.is_some(),
        "audio output initialized"
    );

    while output.len() < OUTPUT_TARGET_BLOCKS * MIX_FRAMES * 2 {
        push_block(&output, mixer.render());
    }
    if let Err(error) = stream.play() {
        tracing::error!(%error, "cannot start audio output");
        return;
    }

    loop {
        while let Ok(command) = command_rx.try_recv() {
            if !mixer.handle(command) {
                return;
            }
        }

        mixer.set_paused(game_audio_should_pause());

        while output.len() < OUTPUT_TARGET_BLOCKS * MIX_FRAMES * 2 {
            push_block(&output, mixer.render());
        }

        match command_rx.recv_timeout(Duration::from_millis(2)) {
            Ok(command) => {
                if !mixer.handle(command) {
                    return;
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn game_audio_should_pause() -> bool {
    let hwnd = client_api::gta::hwnd();
    if hwnd.is_null() {
        return true;
    }

    let window_inactive = unsafe { IsIconic(hwnd) != 0 || GetForegroundWindow() != hwnd };
    window_inactive || CMenuManager::is_menu_active()
}

fn push_block(output: &ArrayQueue<f32>, block: [(f32, f32); MIX_FRAMES]) {
    for (left, right) in block {
        if output.push(left).is_err() || output.push(right).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attenuation_respects_reference_and_max_distance() {
        let settings = BrowserAudioSettings {
            reference_distance: 10.0,
            max_distance: 30.0,
        };
        assert_eq!(distance_gain(Point3::new(0.0, 0.0, 5.0), settings), 1.0);
        assert_eq!(distance_gain(Point3::new(0.0, 0.0, 30.0), settings), 0.0);
        assert!((distance_gain(Point3::new(0.0, 0.0, 20.0), settings) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn streaming_resampler_produces_a_complete_block() {
        let mut stream = AudioStream::new(1, 44_100, true);
        stream.append(vec![(0.25, 0.25); 4_410]);
        let mut output = [0.0; MIX_FRAMES];
        assert!(stream.render_mono(48_000, &mut output));
        assert!(output.iter().all(|sample| (*sample - 0.25).abs() < 1e-6));
    }

    #[test]
    fn embedded_hrir_sphere_is_valid_at_common_device_rates() {
        for sample_rate in [44_100, 48_000] {
            let sphere = HrirSphere::new(Cursor::new(HRIR_SPHERE), sample_rate).unwrap();
            let _processor = HrtfProcessor::new(sphere, HRTF_INTERPOLATION_STEPS, HRTF_BLOCK_LEN);
        }
    }

    #[test]
    fn pausing_mixer_discards_buffered_audio_and_renders_silence() {
        let mut mixer = Mixer::new(48_000);
        assert!(mixer.handle(Command::Stream {
            browser: 1,
            stream_id: 2,
            sample_rate: 48_000,
            spatial: false,
        }));
        assert!(mixer.handle(Command::Pcm {
            browser: 1,
            stream_id: 2,
            data: vec![(0.5, 0.5); 4_800],
        }));

        mixer.set_paused(true);

        let stream = &mixer.streams[&1][0];
        assert!(stream.input.is_empty());
        assert!(!stream.playing);
        assert!(
            mixer
                .render()
                .iter()
                .all(|&(left, right)| left == 0.0 && right == 0.0)
        );
    }

    #[test]
    fn overlay_stream_preserves_stereo_channels() {
        let mut mixer = Mixer::new(48_000);
        assert!(mixer.handle(Command::Stream {
            browser: 1,
            stream_id: 2,
            sample_rate: 48_000,
            spatial: false,
        }));
        assert!(mixer.handle(Command::Pcm {
            browser: 1,
            stream_id: 2,
            data: vec![(0.25, -0.5); 4_800],
        }));

        let output = mixer.render();
        assert!(
            output
                .iter()
                .all(|&(left, right)| { (left - 0.25).abs() < 1e-6 && (right + 0.5).abs() < 1e-6 })
        );
    }
}
