mod draw;
mod ui;

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ctru::linear::{LinearAllocation, LinearAllocator};
use ctru::prelude::*;
use ctru::services::gfx::{Flush, Screen, Swap};
use ctru::services::ndsp::wave::{Status as WaveStatus, Wave};
use ctru::services::ndsp::{AudioFormat, AudioMix, InterpolationType, Ndsp, OutputMode};
use draw::Image;
use ktc::transform::M;
use ui::{Entry, Status, Ui};

const DIR: &str = "/kt3";
const WAVE_FRAMES: usize = 4096;
const WAVES: usize = 4;
const LEAD_SECS: f64 = 3.0;
const START_SECS: f64 = 0.5;
const BUDGET: Duration = Duration::from_millis(30);
const UI_EVERY: Duration = Duration::from_millis(250);
const THUMB_DELAY: Duration = Duration::from_millis(180);
const CHUNK_SAMPLES: usize = ktc::format::CHUNK * M;

struct Buffer(Vec<i16, LinearAllocator>);

impl AsRef<[u8]> for Buffer {
    fn as_ref(&self) -> &[u8] {
        bytemuck::cast_slice(&self.0[..])
    }
}

impl AsMut<[u8]> for Buffer {
    fn as_mut(&mut self) -> &mut [u8] {
        bytemuck::cast_slice_mut(&mut self.0[..])
    }
}

unsafe impl LinearAllocation for Buffer {}

struct Song {
    title: String,
    artist: String,
    note: String,
    ktc: Vec<u8>,
    gain: f32,
    second: Option<(Vec<u8>, f32)>,
    looped: (usize, usize),
}

fn load(name: String, bytes: Vec<u8>) -> Result<Song, String> {
    if bytes.starts_with(b"KTC\0") {
        let info = ktc::info(&bytes)?;
        let len = info.len;
        return Ok(Song { title: name, artist: String::new(), note: "KTC file".into(), ktc: bytes, gain: 1.0, second: None, looped: (0, len) });
    }
    if bytes.len() < 48 || &bytes[0..3] != b"KT3" {
        return Err("Not a KT3 or KTC file".into());
    }
    let u32_at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as usize;
    let f64_at = |o: usize| f64::from_le_bytes(bytes[o..o + 8].try_into().unwrap());
    let (meta_len, lyrics_len, media_len) = (u32_at(24), u32_at(28), u32_at(32));
    let media_at = 48 + meta_len + lyrics_len;
    if bytes.len() < media_at + media_len {
        return Err("The KT3 file is truncated".into());
    }
    let meta: serde_json::Value = serde_json::from_slice(&bytes[48..48 + meta_len]).map_err(|e| format!("Invalid KT3 metadata: {e}"))?;
    let text = |k: &str| meta.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let num = |v: Option<&serde_json::Value>, d: f64| v.and_then(|v| v.as_f64()).unwrap_or(d);
    let base = num(meta.get("gain"), 1.0);
    let mut gain = base;
    let mut note = String::new();
    let mut second = None;
    let ktc = if media_len > 0 {
        bytes[media_at..media_at + media_len].to_vec()
    } else if let Some(st) = meta.get("stems") {
        let thumb = num(meta.get("thumbnailLen"), 0.0) as usize;
        let inst_len = num(st.get("instLen"), 0.0) as usize;
        let vocal_len = num(st.get("vocalLen"), 0.0) as usize;
        let bad = || "The KT3 file is truncated".to_string();
        let inst_end = bytes.len().checked_sub(thumb).ok_or_else(bad)?;
        let inst_start = inst_end.checked_sub(inst_len).ok_or_else(bad)?;
        if inst_len > 0 {
            gain *= num(st.get("instGain"), 1.0);
            if vocal_len > 0 {
                let vocal_start = inst_start.checked_sub(vocal_len).ok_or_else(bad)?;
                second = Some((bytes[vocal_start..inst_start].to_vec(), (base * num(st.get("vocalGain"), 1.0)) as f32));
                note = "Vocal + instrumental (2 CPU cores)".into();
            } else {
                note = "Instrumental only".into();
            }
            bytes[inst_start..inst_end].to_vec()
        } else {
            let vocal_start = inst_start.checked_sub(vocal_len).ok_or_else(bad)?;
            gain *= num(st.get("vocalGain"), 1.0);
            note = "Vocal only".into();
            bytes[vocal_start..inst_start].to_vec()
        }
    } else {
        return Err("The KT3 file has no audio".into());
    };
    if !ktc.starts_with(b"KTC\0") {
        return Err("The audio is not KTC (older KT3 files are not supported)".into());
    }
    let info = ktc::info(&ktc)?;
    let (sr, len) = (info.sr as f64, info.len);
    let (ls, le) = match meta.get("loops").and_then(|l| l.as_array()).and_then(|a| a.first()) {
        Some(l) => (num(l.get("start"), 0.0), num(l.get("end"), 0.0)),
        None => (f64_at(8), f64_at(16)),
    };
    let start = ((ls * sr).round().max(0.0) as usize).min(len);
    let end = if le > 0.0 { ((le * sr).round() as usize).min(len) } else { len };
    let looped = if end > start { (start, end) } else { (0, len) };
    if let Some((v, _)) = &second {
        let vi = ktc::info(v)?;
        if !v.starts_with(b"KTC\0") || vi.sr != info.sr || vi.len != info.len || vi.chans != info.chans {
            return Err("The vocal and instrumental do not match".into());
        }
    }
    Ok(Song { title: text("title"), artist: text("artist"), note, ktc, gain: gain as f32, second, looped })
}

struct Player<'a> {
    ktc: &'a [u8],
    info: &'a ktc::Info,
    gain: f32,
    looped: (usize, usize),
    stream: ktc::Stream<'a>,
    next_start: isize,
    block: Vec<Vec<f32>>,
    chans: usize,
    ring: VecDeque<i16>,
    pos: usize,
    cache: Vec<i16>,
    cache_end: usize,
    cache_full: bool,
    replay: Option<usize>,
    looping: bool,
    finished: bool,
    decoded: usize,
    decode_time: Duration,
}

impl<'a> Player<'a> {
    fn new(ktc: &'a [u8], info: &'a ktc::Info, gain: f32, looped: (usize, usize)) -> Self {
        let chans = info.chans;
        let (ls, le) = looped;
        Player {
            ktc,
            info,
            gain,
            looped,
            stream: ktc::Stream::new(ktc, info),
            next_start: -(M as isize),
            block: vec![vec![0.0; M]; chans],
            chans,
            ring: VecDeque::new(),
            pos: 0,
            cache: Vec::new(),
            cache_end: (ls.div_ceil(CHUNK_SAMPLES) * CHUNK_SAMPLES).min(le),
            cache_full: false,
            replay: None,
            looping: true,
            finished: false,
            decoded: 0,
            decode_time: Duration::ZERO,
        }
    }

    fn restart(&mut self, c: usize) {
        self.stream = ktc::Stream::from_chunk(self.ktc, self.info, c);
        self.next_start = (c * CHUNK_SAMPLES) as isize - M as isize;
    }

    fn ring_frames(&self) -> usize {
        self.ring.len() / self.chans
    }

    fn step(&mut self) -> Result<(), String> {
        let (ls, le) = self.looped;
        let end = if self.looping { le } else { self.info.len };
        let chans = self.chans;
        if let Some(at) = self.replay {
            let total = self.cache.len() / chans;
            let n = (total - at).min(WAVE_FRAMES);
            self.ring.extend(&self.cache[at * chans..(at + n) * chans]);
            if at + n < total {
                self.replay = Some(at + n);
                return Ok(());
            }
            self.replay = None;
            self.pos = self.cache_end;
            if self.cache_end < le {
                self.restart(self.cache_end / CHUNK_SAMPLES);
            }
            return Ok(());
        }
        if self.pos >= end {
            if self.looping {
                self.cache_full = true;
                self.pos = ls;
                if self.cache_end > ls {
                    self.replay = Some(0);
                } else {
                    self.restart(ls / CHUNK_SAMPLES);
                }
            } else {
                self.finished = true;
            }
            return Ok(());
        }
        if self.next_start > self.pos as isize {
            self.restart(self.pos / CHUNK_SAMPLES);
        }
        let t0 = Instant::now();
        let got = self.stream.next(&mut self.block)?;
        self.decode_time += t0.elapsed();
        let Some(start) = got else {
            self.pos = end;
            return Ok(());
        };
        self.decoded += M;
        self.next_start = start + M as isize;
        let a = start.max(self.pos as isize).max(0) as usize;
        let b = (start + M as isize).min(end as isize).max(0) as usize;
        let g = self.gain * 32767.0;
        for i in a..b.max(a) {
            for c in 0..chans {
                let v = (self.block[c][(i as isize - start) as usize] * g).round().clamp(-32768.0, 32767.0) as i16;
                self.ring.push_back(v);
                if !self.cache_full && i >= ls && i < self.cache_end {
                    self.cache.push(v);
                }
            }
        }
        self.pos = self.pos.max(b);
        Ok(())
    }
}

struct Second<'a> {
    ktc: &'a [u8],
    info: ktc::Info,
    gain: f32,
    looped: (usize, usize),
    ring: Mutex<VecDeque<i16>>,
    lead: usize,
    bench: bool,
    stop: AtomicBool,
    looping: AtomicBool,
    finished: AtomicBool,
    error: Mutex<Option<String>>,
    decoded: AtomicUsize,
    decode_us: AtomicUsize,
}

impl<'a> Second<'a> {
    fn new(ktc: &'a [u8], gain: f32, looped: (usize, usize), lead_frames: usize, bench: bool) -> Result<Self, String> {
        let info = ktc::info(ktc)?;
        let lead = lead_frames * info.chans;
        Ok(Second {
            ktc,
            info,
            gain,
            looped,
            ring: Mutex::new(VecDeque::new()),
            lead,
            bench,
            stop: AtomicBool::new(false),
            looping: AtomicBool::new(true),
            finished: AtomicBool::new(false),
            error: Mutex::new(None),
            decoded: AtomicUsize::new(0),
            decode_us: AtomicUsize::new(0),
        })
    }

    fn speed(&self) -> f64 {
        let us = self.decode_us.load(Ordering::Relaxed);
        if us == 0 { 0.0 } else { self.decoded.load(Ordering::Relaxed) as f64 / self.info.sr as f64 / (us as f64 / 1e6) }
    }

    fn run(&self) {
        if self.bench {
            let mut s = ktc::Stream::new(self.ktc, &self.info);
            let mut block = vec![vec![0.0f32; M]; self.info.chans];
            let t0 = Instant::now();
            loop {
                match s.next(&mut block) {
                    Ok(Some(_)) => self.decoded.fetch_add(M, Ordering::Relaxed),
                    Ok(None) => break,
                    Err(e) => {
                        *self.error.lock().unwrap() = Some(e);
                        break;
                    }
                };
                self.decode_us.store(t0.elapsed().as_micros() as usize, Ordering::Relaxed);
                if self.stop.load(Ordering::Relaxed) {
                    break;
                }
            }
            self.finished.store(true, Ordering::Release);
            return;
        }
        let mut p = Player::new(self.ktc, &self.info, self.gain, self.looped);
        while !self.stop.load(Ordering::Relaxed) {
            p.looping = self.looping.load(Ordering::Relaxed);
            let have = self.ring.lock().unwrap().len();
            if have >= self.lead || p.finished {
                self.finished.store(p.finished, Ordering::Release);
                unsafe { ctru_sys::svcSleepThread(2_000_000) };
                continue;
            }
            if let Err(e) = p.step() {
                *self.error.lock().unwrap() = Some(e);
                self.finished.store(true, Ordering::Release);
                break;
            }
            if !p.ring.is_empty() {
                self.ring.lock().unwrap().extend(p.ring.drain(..));
            }
            self.decoded.store(p.decoded, Ordering::Relaxed);
            self.decode_us.store(p.decode_time.as_micros() as usize, Ordering::Relaxed);
        }
    }
}

unsafe extern "C" fn second_main(arg: *mut std::ffi::c_void) {
    let second = unsafe { &*(arg as *const Second) };
    second.run();
}

const CORE1_PERCENT: u32 = 80;
static CORE1_WANTED: AtomicBool = AtomicBool::new(false);
static CORE1_IDLE: AtomicU32 = AtomicU32::new(5);
static LIMIT_ERROR: AtomicI32 = AtomicI32::new(0);

fn cpu_limit(percent: u32) -> Result<(), String> {
    let r = unsafe { ctru_sys::APT_SetAppCpuTimeLimit(percent) };
    if r < 0 {
        return Err(format!("Cannot set the CPU time limit to {percent}% ({r:#x})"));
    }
    Ok(())
}

fn limit_error() -> Result<(), String> {
    match LIMIT_ERROR.swap(0, Ordering::Relaxed) {
        0 => Ok(()),
        r => Err(format!("Cannot change the CPU time limit ({r:#x})")),
    }
}

unsafe extern "C" fn on_apt(hook: ctru_sys::APT_HookType, _: *mut std::ffi::c_void) {
    if !CORE1_WANTED.load(Ordering::Relaxed) {
        return;
    }
    let percent = match hook {
        ctru_sys::APTHOOK_ONSUSPEND => CORE1_IDLE.load(Ordering::Relaxed),
        ctru_sys::APTHOOK_ONRESTORE => CORE1_PERCENT,
        _ => return,
    };
    let r = unsafe { ctru_sys::APT_SetAppCpuTimeLimit(percent) };
    if r < 0 {
        LIMIT_ERROR.store(r, Ordering::Relaxed);
    }
}

struct AptHook(Box<ctru_sys::aptHookCookie>);

impl AptHook {
    fn new() -> Result<AptHook, String> {
        let mut idle = 0u32;
        let r = unsafe { ctru_sys::APT_GetAppCpuTimeLimit(&mut idle) };
        if r < 0 {
            return Err(format!("Cannot read the CPU time limit ({r:#x})"));
        }
        CORE1_IDLE.store(idle.max(5), Ordering::Relaxed);
        let mut cookie = Box::new(unsafe { std::mem::zeroed::<ctru_sys::aptHookCookie>() });
        unsafe { ctru_sys::aptHook(&mut *cookie, Some(on_apt), std::ptr::null_mut()) };
        Ok(AptHook(cookie))
    }
}

impl Drop for AptHook {
    fn drop(&mut self) {
        unsafe { ctru_sys::aptUnhook(&mut *self.0) };
    }
}

struct Core1;

impl Core1 {
    fn claim() -> Result<Core1, String> {
        cpu_limit(CORE1_PERCENT)?;
        CORE1_WANTED.store(true, Ordering::Relaxed);
        Ok(Core1)
    }
}

impl Drop for Core1 {
    fn drop(&mut self) {
        CORE1_WANTED.store(false, Ordering::Relaxed);
        let r = unsafe { ctru_sys::APT_SetAppCpuTimeLimit(CORE1_IDLE.load(Ordering::Relaxed)) };
        if r < 0 {
            LIMIT_ERROR.store(r, Ordering::Relaxed);
        }
    }
}

struct SecondThread<'s, 'a>(ctru_sys::Thread, &'s Second<'a>, Core1);

impl<'s, 'a> SecondThread<'s, 'a> {
    fn start(second: &'s Second<'a>) -> Result<Self, String> {
        let core = Core1::claim()?;
        let t = unsafe { ctru_sys::threadCreate(Some(second_main), second as *const Second as *mut std::ffi::c_void, 256 * 1024, 0x30, 1, false) };
        if t.is_null() {
            return Err("Cannot start a thread on the second CPU core".into());
        }
        Ok(SecondThread(t, second, core))
    }
}

impl Drop for SecondThread<'_, '_> {
    fn drop(&mut self) {
        self.1.stop.store(true, Ordering::Relaxed);
        unsafe {
            let _ = ctru_sys::threadJoin(self.0, u64::MAX);
            ctru_sys::threadFree(self.0);
        }
    }
}

const LID_EVERY: Duration = Duration::from_millis(100);

struct Lid {
    lit: bool,
    open: bool,
    at: Instant,
}

impl Lid {
    fn new() -> Result<Lid, String> {
        if unsafe { ctru_sys::ptmuInit() } < 0 {
            return Err("Cannot read the lid state (ptm:u)".into());
        }
        Ok(Lid { lit: true, open: true, at: Instant::now() - LID_EVERY })
    }

    fn backlight(&mut self, on: bool) -> Result<(), String> {
        let r = unsafe {
            let r = ctru_sys::gspLcdInit();
            if r < 0 {
                return Err(format!("Cannot control the backlight ({r:#x})"));
            }
            let r = if on {
                ctru_sys::GSPLCD_PowerOnBacklight(ctru_sys::GSPLCD_SCREEN_BOTH as u32)
            } else {
                ctru_sys::GSPLCD_PowerOffBacklight(ctru_sys::GSPLCD_SCREEN_BOTH as u32)
            };
            ctru_sys::gspLcdExit();
            r
        };
        if r < 0 {
            return Err(format!("Cannot switch the backlight ({r:#x})"));
        }
        self.lit = on;
        Ok(())
    }

    fn playing(&mut self, apt: &mut Apt) -> Result<bool, String> {
        if self.at.elapsed() < LID_EVERY {
            return Ok(self.open);
        }
        self.at = Instant::now();
        let (mut phones, mut shell) = (false, 0u8);
        unsafe {
            if ctru_sys::DSP_GetHeadphoneStatus(&mut phones) < 0 {
                return Err("Cannot read the headphone state".into());
            }
            if ctru_sys::PTMU_GetShellState(&mut shell) < 0 {
                return Err("Cannot read the lid state".into());
            }
        }
        let open = shell != 0;
        self.open = open;
        let lit = open || !phones;
        if lit && !self.lit {
            self.backlight(true)?;
        }
        apt.set_sleep_allowed(!phones);
        if !lit && self.lit {
            self.backlight(false)?;
        }
        Ok(open)
    }

    fn idle(&mut self, apt: &mut Apt) -> Result<(), String> {
        self.at = Instant::now() - LID_EVERY;
        if !self.lit {
            self.backlight(true)?;
        }
        apt.set_sleep_allowed(true);
        Ok(())
    }
}

impl Drop for Lid {
    fn drop(&mut self) {
        if !self.lit {
            let _ = self.backlight(true);
        }
        unsafe { ctru_sys::ptmuExit() };
    }
}

impl Player<'_> {
    fn produced(&self) -> usize {
        match self.replay {
            Some(at) => self.looped.0 + at,
            None => self.pos,
        }
    }
}

fn scan() -> Vec<Entry> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(DIR)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("kt3") || x.eq_ignore_ascii_case("ktc")))
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    paths.into_iter().map(|p| entry(&p)).collect()
}

fn entry(p: &Path) -> Entry {
    let name = p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut e = Entry { path: p.to_path_buf(), title: name, artist: String::new(), thumb_len: 0 };
    let Ok(mut f) = std::fs::File::open(p) else { return e };
    let mut h = [0u8; 48];
    if f.read_exact(&mut h).is_err() || &h[0..3] != b"KT3" {
        return e;
    }
    let meta_len = u32::from_le_bytes(h[24..28].try_into().unwrap()) as usize;
    let mut m = vec![0u8; meta_len.min(1 << 20)];
    if f.read_exact(&mut m).is_err() {
        return e;
    }
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&m) {
        if let Some(t) = v.get("title").and_then(|t| t.as_str()).filter(|t| !t.is_empty()) {
            e.title = t.to_string();
        }
        e.artist = v.get("artist").and_then(|a| a.as_str()).unwrap_or("").to_string();
        e.thumb_len = v.get("thumbnailLen").and_then(|n| n.as_u64()).unwrap_or(0) as usize;
    }
    e
}

fn thumbnail(e: &Entry) -> Option<Image> {
    if e.thumb_len == 0 {
        return None;
    }
    let mut f = std::fs::File::open(&e.path).ok()?;
    f.seek(SeekFrom::End(-(e.thumb_len as i64))).ok()?;
    let mut b = vec![0u8; e.thumb_len];
    f.read_exact(&mut b).ok()?;
    Image::webp(&b, ui::THUMB).ok()
}

fn show_top(gfx: &Gfx, ui: &Ui, x: i32, y: i32, w: i32, h: i32) {
    let mut s = gfx.top_screen.borrow_mut();
    {
        let fb = s.raw_framebuffer();
        ui.top.present(fb.ptr, fb.width, x, y, w, h);
    }
    s.flush_buffers();
    s.swap_buffers();
}

fn show_bottom(gfx: &Gfx, ui: &Ui) {
    let mut s = gfx.bottom_screen.borrow_mut();
    {
        let fb = s.raw_framebuffer();
        ui.bottom.present(fb.ptr, fb.width, 0, 0, ui::BOTTOM_W as i32, ui::BOTTOM_H as i32);
    }
    s.flush_buffers();
    s.swap_buffers();
}

fn clock(n: usize, rate: u32) -> String {
    let s = n as f64 / rate as f64;
    format!("{}:{:05.2}", (s / 60.0) as u32, s % 60.0)
}

fn show_song(gfx: &Gfx, ui: &mut Ui, song: &Song, thumb: Option<&Image>) -> Result<(), String> {
    let info = ktc::info(&song.ktc)?;
    let (ls, le) = song.looped;
    let mut lines = vec![
        format!("{} Hz  {}ch  {}", info.sr, info.chans, clock(info.len, info.sr)),
        format!("Loop  {} - {}", clock(ls, info.sr), clock(le, info.sr)),
    ];
    if !song.note.is_empty() {
        lines.push(song.note.clone());
    }
    ui.now(&song.title, &song.artist, &lines, thumb);
    show_top(gfx, ui, 0, 0, ui::TOP_W as i32, ui::STATUS_Y);
    Ok(())
}

enum After {
    Back,
    Quit,
}

#[allow(clippy::too_many_arguments)]
fn play(apt: &mut Apt, hid: &mut Hid, gfx: &Gfx, ui: &mut Ui, ndsp: &Ndsp, lid: &mut Lid, song: &Song, thumb: Option<&Image>) -> Result<After, String> {
    show_song(gfx, ui, song, thumb)?;
    let info = ktc::info(&song.ktc)?;
    let chans = info.chans;
    let sr = info.sr;
    let format = if chans == 2 { AudioFormat::PCM16Stereo } else { AudioFormat::PCM16Mono };
    let mut waves: Vec<Wave<Buffer>> = (0..WAVES)
        .map(|_| {
            let mut v = Vec::with_capacity_in(WAVE_FRAMES * chans, LinearAllocator);
            v.resize(WAVE_FRAMES * chans, 0i16);
            Wave::new(Buffer(v), format, false)
        })
        .collect();
    let mut ch = ndsp.channel(0).map_err(|e| format!("NDSP channel: {e:?}"))?;
    ch.reset();
    ch.set_interpolation(InterpolationType::Polyphase);
    ch.set_sample_rate(sr as f32);
    ch.set_format(format);
    ch.set_mix(&AudioMix::default());

    let lead = (LEAD_SECS * sr as f64) as usize;
    let mut p = Player::new(&song.ktc, &info, song.gain, song.looped);
    let second = match &song.second {
        Some((k, g)) => Some(Second::new(k, *g, song.looped, lead, false)?),
        None => None,
    };
    let _thread = match &second {
        Some(s) => Some(SecondThread::start(s)?),
        None => None,
    };
    let second_frames = || second.as_ref().map_or(usize::MAX, |s| s.ring.lock().unwrap().len() / chans);
    let start_frames = ((START_SECS * sr as f64) as usize).max(WAVE_FRAMES);
    let (mut started, mut underruns) = (false, 0u32);
    let mut ui_at = Instant::now() - UI_EVERY;
    loop {
        if !apt.main_loop() {
            return Ok(After::Quit);
        }
        limit_error()?;
        let open = lid.playing(apt)?;
        hid.scan_input();
        let keys = hid.keys_down();
        if keys.contains(KeyPad::START) {
            return Ok(After::Quit);
        }
        if keys.contains(KeyPad::B) {
            return Ok(After::Back);
        }
        if keys.contains(KeyPad::X) {
            p.looping = !p.looping;
            if let Some(s) = &second {
                s.looping.store(p.looping, Ordering::Relaxed);
            }
        }
        if let Some(e) = second.as_ref().and_then(|s| s.error.lock().unwrap().take()) {
            return Err(e);
        }
        let finished = p.finished && second.as_ref().is_none_or(|s| s.finished.load(Ordering::Acquire));

        let mut busy = 0;
        for w in waves.iter_mut() {
            match w.status() {
                WaveStatus::Queued | WaveStatus::Playing => busy += 1,
                _ => {
                    let have = p.ring_frames().min(second_frames());
                    let last = finished && have > 0;
                    if !(have >= WAVE_FRAMES || last) || (!started && have < start_frames && !finished) {
                        continue;
                    }
                    let n = have.min(WAVE_FRAMES);
                    let buf = w.get_raw_buffer_mut().map_err(|e| format!("NDSP wave: {e:?}"))?;
                    for (d, s) in buf.0.iter_mut().zip(p.ring.drain(..n * chans)) {
                        *d = s;
                    }
                    if let Some(s) = &second {
                        let mut r = s.ring.lock().unwrap();
                        for (d, v) in buf.0.iter_mut().zip(r.drain(..n * chans)) {
                            *d = d.saturating_add(v);
                        }
                    }
                    buf.0[n * chans..].fill(0);
                    ch.queue_wave(w).map_err(|e| format!("NDSP queue: {e:?}"))?;
                    started = true;
                    busy += 1;
                }
            }
        }
        if started && busy == 0 {
            if finished && p.ring.is_empty() {
                return Ok(After::Back);
            }
            underruns += 1;
            started = false;
        }

        let t0 = Instant::now();
        while p.ring_frames() < lead && !p.finished && t0.elapsed() < BUDGET {
            p.step()?;
        }

        if open && ui_at.elapsed() >= UI_EVERY {
            ui_at = Instant::now();
            let speed = if p.decode_time.is_zero() { 0.0 } else { p.decoded as f64 / sr as f64 / p.decode_time.as_secs_f64() };
            let (ls, le) = song.looped;
            let mut at = p.produced() as isize - (p.ring_frames() + busy * WAVE_FRAMES) as isize;
            if p.looping && le > ls {
                while at < ls as isize && p.cache_full {
                    at += (le - ls) as isize;
                }
            }
            let mut stats = match &second {
                None => format!("Decode {speed:.2}x"),
                Some(s) => format!("Decode {speed:.2}x / {:.2}x", s.speed()),
            };
            if underruns > 0 {
                stats.push_str(&format!("  Gaps {underruns}"));
            }
            let looped = p.looping.then_some(song.looped);
            ui.status(&Status::Playing { pos: at.max(0) as usize, len: info.len, looped, rate: sr, stats });
            show_top(gfx, ui, 0, ui::STATUS_Y, ui::TOP_W as i32, ui::TOP_H as i32 - ui::STATUS_Y);
        }
        if p.ring_frames() >= lead || p.finished {
            gfx.wait_for_vblank();
        }
    }
}

fn bench(apt: &Apt, hid: &mut Hid, gfx: &Gfx, ui: &mut Ui, song: &Song) -> Result<Option<String>, String> {
    let info = ktc::info(&song.ktc)?;
    let second = match &song.second {
        Some((k, g)) => Some(Second::new(k, *g, song.looped, 0, true)?),
        None => None,
    };
    let mut s = ktc::Stream::new(&song.ktc, &info);
    let mut block = vec![vec![0.0f32; M]; info.chans];
    let t0 = Instant::now();
    let thread = match &second {
        Some(s) => Some(SecondThread::start(s)?),
        None => None,
    };
    let audio_total = info.len as f64 / info.sr as f64;
    let mut frames = 0usize;
    let mut ui_at = Instant::now() - Duration::from_secs(1);
    let mut done = false;
    loop {
        if !done {
            done = s.next(&mut block)?.is_none();
            if !done {
                frames += 1;
            }
        }
        let second_done = second.as_ref().is_none_or(|s| s.finished.load(Ordering::Acquire));
        if done && second_done {
            break;
        }
        if done {
            unsafe { ctru_sys::svcSleepThread(1_000_000) };
        }
        if ui_at.elapsed() >= Duration::from_millis(500) {
            ui_at = Instant::now();
            if !apt.main_loop() {
                return Ok(None);
            }
            hid.scan_input();
            if hid.keys_down().contains(KeyPad::B) {
                return Ok(None);
            }
            let mut a = (frames * M).min(info.len);
            if let Some(s) = &second {
                a = a.min(s.decoded.load(Ordering::Relaxed).min(info.len));
            }
            ui.status(&Status::Message(format!("Speed test... {}%   (B: cancel)", a * 100 / info.len.max(1))));
            show_top(gfx, ui, 0, ui::STATUS_Y, ui::TOP_W as i32, ui::TOP_H as i32 - ui::STATUS_Y);
        }
    }
    let t = t0.elapsed().as_secs_f64();
    drop(thread);
    if let Some(e) = second.as_ref().and_then(|s| s.error.lock().unwrap().take()) {
        return Err(e);
    }
    Ok(Some(match &second {
        None => format!("{:.1} s of audio in {:.1} s: {:.2}x real time", audio_total, t, audio_total / t),
        Some(_) => format!("2 streams of {:.1} s in {:.1} s: {:.2}x real time", audio_total, t, audio_total / t),
    }))
}

fn main() {
    let mut apt = Apt::new().expect("Couldn't obtain APT controller");
    let mut hid = Hid::new().expect("Couldn't obtain HID controller");
    let gfx = Gfx::new().expect("Couldn't obtain GFX controller");
    gfx.top_screen.borrow_mut().set_double_buffering(false);
    gfx.bottom_screen.borrow_mut().set_double_buffering(false);

    let (mut ui, mut lid, _hook) = match Ui::new().and_then(|u| Ok((u, Lid::new()?, AptHook::new()?))) {
        Ok(v) => v,
        Err(e) => {
            let _console = Console::new(gfx.top_screen.borrow_mut());
            println!("{e}\n\nPress START to exit.");
            while apt.main_loop() {
                hid.scan_input();
                if hid.keys_down().contains(KeyPad::START) {
                    break;
                }
                gfx.wait_for_vblank();
            }
            return;
        }
    };
    let ndsp = Ndsp::new().ok().map(|mut n| {
        n.set_output_mode(OutputMode::Stereo);
        n
    });

    let entries = scan();
    let mut sel = 0usize;
    let mut thumbs: HashMap<usize, Option<Image>> = HashMap::new();
    let mut moved = Some(Instant::now() - THUMB_DELAY);
    let hint = || Status::Hint("A Play   Y Speed test   X Loop on/off   B Stop".into());
    let mut status = match &ndsp {
        Some(_) => hint(),
        None => Status::Error("Audio is not available: sdmc:/3ds/dspfirm.cdc may be missing (run DSP1)".into()),
    };
    let draw_now = |ui: &mut Ui, thumbs: &HashMap<usize, Option<Image>>, sel: usize| match entries.get(sel) {
        Some(e) => ui.now(&e.title, &e.artist, &[], thumbs.get(&sel).and_then(|t| t.as_ref())),
        None => ui.now("KT3 Player for 3DS", "", &["Put .kt3 files in sdmc:/kt3/".into()], None),
    };
    ui.list(&entries, sel);
    show_bottom(&gfx, &ui);
    draw_now(&mut ui, &thumbs, sel);
    ui.status(&status);
    show_top(&gfx, &ui, 0, 0, ui::TOP_W as i32, ui::TOP_H as i32);

    while apt.main_loop() {
        hid.scan_input();
        let keys = hid.keys_down();
        if keys.contains(KeyPad::START) {
            break;
        }
        let old = sel;
        if keys.contains(KeyPad::DPAD_DOWN) && sel + 1 < entries.len() {
            sel += 1;
        }
        if keys.contains(KeyPad::DPAD_UP) && sel > 0 {
            sel -= 1;
        }
        if keys.contains(KeyPad::DPAD_RIGHT) {
            sel = (sel + 6).min(entries.len().saturating_sub(1));
        }
        if keys.contains(KeyPad::DPAD_LEFT) {
            sel = sel.saturating_sub(6);
        }
        if sel != old {
            ui.list(&entries, sel);
            show_bottom(&gfx, &ui);
            draw_now(&mut ui, &thumbs, sel);
            show_top(&gfx, &ui, 0, 0, ui::TOP_W as i32, ui::STATUS_Y);
            moved = Some(Instant::now());
        }
        if moved.is_some_and(|t| t.elapsed() >= THUMB_DELAY) {
            moved = None;
            if let Some(e) = entries.get(sel) {
                if !thumbs.contains_key(&sel) {
                    thumbs.insert(sel, thumbnail(e));
                    draw_now(&mut ui, &thumbs, sel);
                    show_top(&gfx, &ui, 0, 0, ui::TOP_W as i32, ui::STATUS_Y);
                }
            }
        }
        if let Some(e) = entries.get(sel).filter(|_| keys.contains(KeyPad::A) || keys.contains(KeyPad::Y)) {
            let bench_mode = keys.contains(KeyPad::Y);
            if !thumbs.contains_key(&sel) {
                thumbs.insert(sel, thumbnail(e));
            }
            let thumb = thumbs.get(&sel).and_then(|t| t.as_ref());
            let name = e.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            ui.status(&Status::Message("Loading...".into()));
            show_top(&gfx, &ui, 0, ui::STATUS_Y, ui::TOP_W as i32, ui::TOP_H as i32 - ui::STATUS_Y);
            let song = std::fs::read(&e.path).map_err(|err| format!("Cannot read the file: {err}")).and_then(|b| load(name, b));
            status = match song {
                Err(err) => Status::Error(err),
                Ok(song) if bench_mode => {
                    let _ = show_song(&gfx, &mut ui, &song, thumb);
                    match bench(&apt, &mut hid, &gfx, &mut ui, &song) {
                        Ok(Some(r)) => Status::Message(r),
                        Ok(None) => hint(),
                        Err(err) => Status::Error(err),
                    }
                }
                Ok(song) => match &ndsp {
                    None => Status::Error("Audio is not available".into()),
                    Some(n) => {
                        let r = play(&mut apt, &mut hid, &gfx, &mut ui, n, &mut lid, &song, thumb);
                        match lid.idle(&mut apt).and(r) {
                            Ok(After::Quit) => break,
                            Ok(After::Back) => hint(),
                            Err(err) => Status::Error(err),
                        }
                    }
                },
            };
            if let Err(err) = limit_error() {
                status = Status::Error(err);
            }
            ui.status(&status);
            show_top(&gfx, &ui, 0, ui::STATUS_Y, ui::TOP_W as i32, ui::TOP_H as i32 - ui::STATUS_Y);
        }
        gfx.wait_for_vblank();
    }
}
