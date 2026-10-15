//! Screen recording to an H.264 MP4 through Windows Media Foundation (no extra binaries or codecs to install).
//!
//! A worker thread grabs the chosen rectangle with GDI at the requested frame rate and hands the frames to a sink
//! writer, which converts them and encodes H.264. Frames carry their real capture time, so the video plays at the
//! right speed even when the machine cannot keep up with the nominal frame rate. There is no audio.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::channel;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::Win32::Media::MediaFoundation::{
    IMFMediaType, IMFSinkWriter, MF_MT_AVG_BITRATE, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
    MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
    MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SINK_WRITER_DISABLE_THROTTLING, MF_VERSION, MFCreateAttributes,
    MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample, MFCreateSinkWriterFromURL, MFMediaType_Video,
    MFSTARTUP_FULL, MFShutdown, MFStartup, MFVideoFormat_H264, MFVideoFormat_RGB32, MFVideoInterlace_Progressive,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::core::HSTRING;

use crate::capture::gdi;
use crate::geometry::Rect;

/// The H.264 encoder of Windows handles up to 4096 x 2304.
pub const MAX_W: i32 = 4096;
pub const MAX_H: i32 = 2304;

fn pack(hi: u32, lo: u32) -> u64 {
    (u64::from(hi) << 32) | u64::from(lo)
}

fn mf(what: &str, e: windows::core::Error) -> String {
    format!("{what}: {}", e.message())
}

/// H.264 wants even dimensions: the area is trimmed by at most one pixel on each axis.
pub fn even_size(rect: Rect) -> Result<Rect, String> {
    let (w, h) = (rect.w & !1, rect.h & !1);
    if w < 16 || h < 16 {
        return Err("the area is too small to record (at least 16 x 16 pixels)".into());
    }
    if w > MAX_W || h > MAX_H {
        return Err(format!(
            "the area is too large to record ({w} x {h}); the limit is {MAX_W} x {MAX_H}: choose a region or one monitor"
        ));
    }
    Ok(Rect::new(rect.x, rect.y, w, h))
}

/// Writes BGRA frames into an MP4 file.
pub struct Encoder {
    writer: IMFSinkWriter,
    stream: u32,
    width: u32,
    height: u32,
    finished: bool,
}

impl Encoder {
    /// Media Foundation and COM must already be initialised on the calling thread.
    pub fn new(path: &Path, width: u32, height: u32, fps: u32) -> Result<Encoder, String> {
        let fps = fps.clamp(1, 120);
        // SAFETY: plain Media Foundation calls with valid arguments; every returned interface is owned.
        unsafe {
            let mut attrs = None;
            MFCreateAttributes(&mut attrs, 2).map_err(|e| mf("MFCreateAttributes", e))?;
            let attrs = attrs.ok_or("no attribute store")?;
            let _ = attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1);
            let _ = attrs.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1);
            let url = HSTRING::from(path.as_os_str());
            let writer = MFCreateSinkWriterFromURL(&url, None, &attrs).map_err(|e| mf("cannot create the file", e))?;

            // about 0.15 bit per pixel: screen content compresses well
            let bitrate =
                (u64::from(width) * u64::from(height) * u64::from(fps) * 15 / 100).clamp(1_500_000, 40_000_000);
            let out: IMFMediaType = MFCreateMediaType().map_err(|e| mf("MFCreateMediaType", e))?;
            out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(|e| mf("major type", e))?;
            out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)
                .map_err(|e| mf("subtype", e))?;
            out.SetUINT32(&MF_MT_AVG_BITRATE, bitrate as u32)
                .map_err(|e| mf("bitrate", e))?;
            out.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
                .map_err(|e| mf("interlace", e))?;
            out.SetUINT64(&MF_MT_FRAME_SIZE, pack(width, height))
                .map_err(|e| mf("frame size", e))?;
            out.SetUINT64(&MF_MT_FRAME_RATE, pack(fps, 1))
                .map_err(|e| mf("frame rate", e))?;
            out.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))
                .map_err(|e| mf("aspect", e))?;
            let stream = writer
                .AddStream(&out)
                .map_err(|e| mf("the H.264 encoder is not available", e))?;

            let input: IMFMediaType = MFCreateMediaType().map_err(|e| mf("MFCreateMediaType", e))?;
            input
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(|e| mf("major type", e))?;
            input
                .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
                .map_err(|e| mf("subtype", e))?;
            input
                .SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
                .map_err(|e| mf("interlace", e))?;
            input
                .SetUINT64(&MF_MT_FRAME_SIZE, pack(width, height))
                .map_err(|e| mf("frame size", e))?;
            input
                .SetUINT64(&MF_MT_FRAME_RATE, pack(fps, 1))
                .map_err(|e| mf("frame rate", e))?;
            input
                .SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))
                .map_err(|e| mf("aspect", e))?;
            // a positive stride tells Media Foundation the rows go top to bottom
            input
                .SetUINT32(&MF_MT_DEFAULT_STRIDE, width * 4)
                .map_err(|e| mf("stride", e))?;
            writer
                .SetInputMediaType(stream, &input, None)
                .map_err(|e| mf("input format", e))?;
            writer.BeginWriting().map_err(|e| mf("BeginWriting", e))?;
            Ok(Encoder {
                writer,
                stream,
                width,
                height,
                finished: false,
            })
        }
    }

    /// `bgra` holds `width * height * 4` bytes, top row first. Times are in 100 ns units.
    pub fn write(&mut self, bgra: &[u8], time: i64, duration: i64) -> Result<(), String> {
        let len = self.width as usize * self.height as usize * 4;
        if bgra.len() != len {
            return Err("the frame has the wrong size".into());
        }
        // SAFETY: the buffer is locked, filled with exactly `len` bytes inside its capacity and unlocked.
        unsafe {
            let buffer = MFCreateMemoryBuffer(len as u32).map_err(|e| mf("MFCreateMemoryBuffer", e))?;
            let mut dst = std::ptr::null_mut();
            buffer.Lock(&mut dst, None, None).map_err(|e| mf("Lock", e))?;
            std::ptr::copy_nonoverlapping(bgra.as_ptr(), dst, len);
            buffer.Unlock().map_err(|e| mf("Unlock", e))?;
            buffer
                .SetCurrentLength(len as u32)
                .map_err(|e| mf("SetCurrentLength", e))?;
            let sample = MFCreateSample().map_err(|e| mf("MFCreateSample", e))?;
            sample.AddBuffer(&buffer).map_err(|e| mf("AddBuffer", e))?;
            sample.SetSampleTime(time).map_err(|e| mf("SetSampleTime", e))?;
            sample
                .SetSampleDuration(duration.max(1))
                .map_err(|e| mf("SetSampleDuration", e))?;
            self.writer
                .WriteSample(self.stream, &sample)
                .map_err(|e| mf("WriteSample", e))
        }
    }

    /// Writes the index and closes the file. The file is unusable without this.
    pub fn finish(mut self) -> Result<(), String> {
        self.finished = true;
        // SAFETY: the writer is valid; Finalize flushes the encoder and the MP4 headers.
        unsafe { self.writer.Finalize().map_err(|e| mf("Finalize", e)) }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        if !self.finished {
            // SAFETY: best effort, so an aborted recording still leaves a playable file
            let _ = unsafe { self.writer.Finalize() };
        }
    }
}

/// Media Foundation and COM for the lifetime of the value (one per thread that encodes).
struct MfSession;

impl MfSession {
    fn new() -> Result<MfSession, String> {
        // SAFETY: balanced by `Drop`; "already initialised" results are harmless.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(|e| mf("Media Foundation is not available", e))?;
        }
        Ok(MfSession)
    }
}

impl Drop for MfSession {
    fn drop(&mut self) {
        // SAFETY: matches the calls in `new`.
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
    }
}

#[derive(Clone, Debug)]
pub struct RecordOptions {
    /// The area in virtual-desktop pixels.
    pub rect: Rect,
    pub fps: u32,
    pub cursor: bool,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct Finished {
    pub path: PathBuf,
    pub frames: u64,
    pub seconds: f64,
}

/// A recording in progress.
pub struct Recorder {
    stop: Arc<AtomicBool>,
    frames: Arc<AtomicU64>,
    started: Instant,
    path: PathBuf,
    join: Option<JoinHandle<Result<Finished, String>>>,
}

impl Recorder {
    /// Starts recording; fails at once when the encoder or the file cannot be created.
    pub fn start(opts: RecordOptions) -> Result<Recorder, String> {
        let rect = even_size(opts.rect)?;
        if let Some(dir) = opts.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(AtomicU64::new(0));
        let (ready_tx, ready_rx) = channel::<Result<(), String>>();
        let (stop2, frames2, path) = (stop.clone(), frames.clone(), opts.path.clone());
        let (fps, cursor) = (opts.fps.clamp(1, 60), opts.cursor);
        let join = std::thread::Builder::new()
            .name("snapforge-record".into())
            .spawn(move || run(rect, fps, cursor, &path, &stop2, &frames2, ready_tx))
            .map_err(|e| e.to_string())?;
        match ready_rx.recv_timeout(Duration::from_secs(15)) {
            Ok(Ok(())) => Ok(Recorder {
                stop,
                frames,
                started: Instant::now(),
                path: opts.path,
                join: Some(join),
            }),
            Ok(Err(e)) => {
                let _ = join.join();
                let _ = std::fs::remove_file(&opts.path);
                Err(e)
            }
            Err(_) => {
                stop.store(true, Ordering::SeqCst);
                Err("the recorder did not start in time".into())
            }
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn frames(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// True when the worker ended on its own (an error); `stop` then returns the reason.
    pub fn has_ended(&self) -> bool {
        self.join.as_ref().is_none_or(|j| j.is_finished())
    }

    /// Stops, finalises the file and reports what was written.
    pub fn stop(mut self) -> Result<Finished, String> {
        self.stop.store(true, Ordering::SeqCst);
        match self.join.take() {
            Some(j) => j.join().unwrap_or_else(|_| Err("the recorder crashed".into())),
            None => Err("the recorder is not running".into()),
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn run(
    rect: Rect,
    fps: u32,
    cursor: bool,
    path: &Path,
    stop: &AtomicBool,
    frames: &AtomicU64,
    ready: std::sync::mpsc::Sender<Result<(), String>>,
) -> Result<Finished, String> {
    // declared in this order so the encoder is dropped before Media Foundation shuts down
    let _session = match MfSession::new() {
        Ok(s) => s,
        Err(e) => {
            let _ = ready.send(Err(e.clone()));
            return Err(e);
        }
    };
    let mut encoder = match Encoder::new(path, rect.w as u32, rect.h as u32, fps) {
        Ok(x) => x,
        Err(e) => {
            let _ = ready.send(Err(e.clone()));
            return Err(e);
        }
    };
    let _ = ready.send(Ok(()));
    let interval = Duration::from_secs_f64(1.0 / f64::from(fps));
    let t0 = Instant::now();
    let mut next = t0;
    let mut failures = 0u32;
    let mut count = 0u64;
    let mut last_time = -1i64;
    let mut bgra: Vec<u8> = Vec::new();
    while !stop.load(Ordering::SeqCst) {
        match gdi::grab(rect, cursor) {
            Ok(img) => {
                failures = 0;
                bgra.clear();
                bgra.extend_from_slice(img.as_raw());
                for px in bgra.chunks_exact_mut(4) {
                    px.swap(0, 2);
                }
                let mut time = (t0.elapsed().as_nanos() / 100) as i64;
                if time <= last_time {
                    time = last_time + 1;
                }
                last_time = time;
                encoder
                    .write(&bgra, time, (interval.as_nanos() / 100) as i64)
                    .map_err(|e| format!("encoding stopped: {e}"))?;
                count += 1;
                frames.store(count, Ordering::Relaxed);
            }
            Err(e) => {
                // a locked screen or a UAC prompt blocks the grab for a while: skip frames, give up after a few seconds
                failures += 1;
                if failures > fps * 5 {
                    return Err(format!("the screen cannot be captured: {e}"));
                }
            }
        }
        next += interval;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > interval * 4 {
            next = now; // far behind: do not try to catch up with a burst
        }
    }
    let seconds = t0.elapsed().as_secs_f64();
    encoder.finish()?;
    if count == 0 {
        let _ = std::fs::remove_file(path);
        return Err("nothing was recorded".into());
    }
    Ok(Finished {
        path: path.to_owned(),
        frames: count,
        seconds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_made_even_and_limited() {
        let r = even_size(Rect::new(10, 20, 801, 601)).unwrap();
        assert_eq!((r.x, r.y, r.w, r.h), (10, 20, 800, 600));
        assert!(even_size(Rect::new(0, 0, 10, 600)).is_err());
        assert!(even_size(Rect::new(0, 0, 5000, 600)).is_err());
    }

    /// Writes synthetic frames (red top half, blue bottom half) so a decoder can check the orientation and colours.
    /// Needs Media Foundation (missing on Windows Server runners), hence `ignore`; run with `-- --ignored`.
    #[test]
    #[ignore = "needs Media Foundation"]
    fn encoder_writes_a_playable_top_down_mp4() {
        let path = std::env::temp_dir().join(format!("sf-rec-{}.mp4", std::process::id()));
        let _s = MfSession::new().expect("media foundation");
        let (w, h) = (320u32, 240u32);
        let mut frame = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                // BGRA
                let (b, g, r) = if y < h / 2 { (0, 0, 255) } else { (255, 0, 0) };
                frame[i..i + 4].copy_from_slice(&[b, g, r, 255]);
            }
        }
        let mut enc = Encoder::new(&path, w, h, 30).expect("encoder");
        for i in 0..30i64 {
            enc.write(&frame, i * 333_333, 333_333).expect("frame");
        }
        enc.finish().expect("finish");
        let len = std::fs::metadata(&path).expect("file").len();
        assert!(len > 1000, "the file is not empty ({len} bytes)");
        println!("wrote {}", path.display());
    }

    /// The whole chain: GDI grab of a screen corner, encoding on the worker thread, stop and finalise.
    #[test]
    #[ignore = "needs Media Foundation and a desktop"]
    fn recorder_records_a_piece_of_the_screen() {
        let path = std::env::temp_dir().join(format!("sf-rec-live-{}.mp4", std::process::id()));
        let rec = Recorder::start(RecordOptions {
            rect: Rect::new(0, 0, 641, 361),
            fps: 30,
            cursor: true,
            path: path.clone(),
        })
        .expect("start");
        std::thread::sleep(Duration::from_millis(1500));
        assert!(rec.frames() > 5, "frames are being written");
        let done = rec.stop().expect("stop");
        assert!(done.frames > 10, "{} frames", done.frames);
        assert!(std::fs::metadata(&path).expect("file").len() > 1000);
        println!(
            "wrote {} ({} frames, {:.2} s)",
            path.display(),
            done.frames,
            done.seconds
        );
    }
}
