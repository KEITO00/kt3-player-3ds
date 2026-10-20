use crate::draw::{Canvas, Font, Image, Rgb};

pub const BG: Rgb = [16, 18, 22];
const PANEL: Rgb = [28, 31, 38];
const TEXT: Rgb = [236, 239, 243];
const SUB: Rgb = [148, 156, 168];
const ACCENT: Rgb = [92, 232, 216];
const SELECT: Rgb = [30, 74, 72];
const TRACK: Rgb = [52, 57, 66];
const LOOP: Rgb = [70, 120, 116];
const ERROR: Rgb = [244, 112, 112];

pub const TOP_W: usize = 400;
pub const TOP_H: usize = 240;
pub const BOTTOM_W: usize = 320;
pub const BOTTOM_H: usize = 240;
pub const STATUS_Y: i32 = 196;

pub const THUMB: usize = 168;
const ROW_H: i32 = 33;
const ROWS: usize = 6;
const HEAD_H: i32 = 24;

pub struct Entry {
    pub path: std::path::PathBuf,
    pub title: String,
    pub artist: String,
    pub thumb_len: usize,
}

pub enum Status {
    Hint(String),
    Message(String),
    Error(String),
    Playing { pos: usize, len: usize, looped: Option<(usize, usize)>, rate: u32, stats: String },
}

pub struct Ui {
    pub top: Canvas,
    pub bottom: Canvas,
    pub font: Font,
}

fn time(n: usize, rate: u32) -> String {
    let s = n / rate as usize;
    format!("{}:{:02}", s / 60, s % 60)
}

impl Ui {
    pub fn new() -> Result<Ui, String> {
        Ok(Ui { top: Canvas::new(TOP_W, TOP_H), bottom: Canvas::new(BOTTOM_W, BOTTOM_H), font: Font::system()? })
    }

    pub fn list(&mut self, entries: &[Entry], sel: usize) {
        let (cv, f) = (&mut self.bottom, &mut self.font);
        cv.fill(0, 0, BOTTOM_W as i32, BOTTOM_H as i32, BG);
        cv.fill(0, 0, BOTTOM_W as i32, HEAD_H, PANEL);
        f.text(cv, 10, 5, "KT3 Player", 15, ACCENT, 200);
        let count = format!("{} songs", entries.len());
        let cw = f.width(&count, 12);
        f.text(cv, BOTTOM_W as i32 - 10 - cw, 7, &count, 12, SUB, 120);
        if entries.is_empty() {
            f.text(cv, 12, 40, "No songs", 15, TEXT, 296);
            f.text(cv, 12, 64, "Put .kt3 files in sdmc:/kt3/", 13, SUB, 296);
        }
        let first = sel.saturating_sub(ROWS - 1).min(entries.len().saturating_sub(ROWS));
        for (row, i) in (first..entries.len()).take(ROWS).enumerate() {
            let y = HEAD_H + row as i32 * ROW_H;
            if i == sel {
                cv.fill(0, y, BOTTOM_W as i32, ROW_H, SELECT);
                cv.fill(0, y, 3, ROW_H, ACCENT);
            }
            let e = &entries[i];
            f.text(cv, 12, y + 3, &e.title, 15, TEXT, 296);
            f.text(cv, 12, y + 19, &e.artist, 12, SUB, 296);
        }
        if entries.len() > ROWS {
            let track = ROWS as i32 * ROW_H;
            let bar = (track * ROWS as i32 / entries.len() as i32).max(12);
            let at = (track - bar) * first as i32 / (entries.len() - ROWS) as i32;
            cv.fill(BOTTOM_W as i32 - 3, HEAD_H + at, 3, bar, TRACK);
        }
        cv.fill(0, BOTTOM_H as i32 - 20, BOTTOM_W as i32, 20, PANEL);
        f.text(cv, 10, BOTTOM_H as i32 - 17, "A Play   Y Speed test   START Exit", 12, SUB, 300);
    }

    pub fn now(&mut self, title: &str, artist: &str, lines: &[String], thumb: Option<&Image>) {
        let (cv, f) = (&mut self.top, &mut self.font);
        cv.fill(0, 0, TOP_W as i32, STATUS_Y, BG);
        let (tx, ty) = (16, 16);
        match thumb {
            Some(img) => cv.image(tx, ty, img),
            None => {
                cv.fill(tx, ty, THUMB as i32, THUMB as i32, PANEL);
                let w = f.width("KT3", 30);
                f.text(cv, tx + (THUMB as i32 - w) / 2, ty + THUMB as i32 / 2 - 16, "KT3", 30, TRACK, THUMB as i32);
            }
        }
        let x = tx + THUMB as i32 + 14;
        let max_w = TOP_W as i32 - x - 12;
        let mut y = ty + 2;
        for line in wrap(f, title, 18, max_w, 3) {
            f.text(cv, x, y, &line, 18, TEXT, max_w);
            y += 22;
        }
        y += 4;
        if !artist.is_empty() {
            f.text(cv, x, y, artist, 14, SUB, max_w);
            y += 24;
        }
        for l in lines {
            f.text(cv, x, y, l, 12, SUB, max_w);
            y += 16;
        }
    }

    pub fn status(&mut self, st: &Status) {
        let (cv, f) = (&mut self.top, &mut self.font);
        cv.fill(0, STATUS_Y, TOP_W as i32, TOP_H as i32 - STATUS_Y, BG);
        let w = TOP_W as i32 - 32;
        match st {
            Status::Hint(s) => {
                f.text(cv, 16, STATUS_Y + 14, s, 13, SUB, w);
            }
            Status::Message(s) => {
                f.text(cv, 16, STATUS_Y + 14, s, 13, TEXT, w);
            }
            Status::Error(s) => {
                f.text(cv, 16, STATUS_Y + 14, s, 13, ERROR, w);
            }
            Status::Playing { pos, len, looped, rate, stats } => {
                let (bx, by, bh) = (16, STATUS_Y + 8, 6);
                let at = |n: usize| bx + (w as i64 * n as i64 / (*len).max(1) as i64) as i32;
                cv.fill(bx, by, w, bh, TRACK);
                if let Some((a, b)) = looped {
                    cv.fill(at(*a), by, (at(*b) - at(*a)).max(1), bh, LOOP);
                }
                cv.fill(bx, by, at(*pos) - bx, bh, ACCENT);
                f.text(cv, bx, by + 12, &time(*pos, *rate), 12, TEXT, 80);
                let total = time(*len, *rate);
                let tw = f.width(&total, 12);
                f.text(cv, bx + w - tw, by + 12, &total, 12, SUB, 80);
                let sw = f.width(stats, 12).min(w - 120);
                f.text(cv, bx + (w - sw) / 2, by + 12, stats, 12, SUB, w - 120);
            }
        }
    }
}

fn wrap(f: &mut Font, s: &str, px: u8, max_w: i32, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        let mut t = cur.clone();
        t.push(c);
        if f.width(&t, px) > max_w && !cur.is_empty() {
            if lines.len() + 1 == max_lines {
                cur.push_str(&s[s.char_indices().nth(lines.iter().map(|l| l.chars().count()).sum::<usize>() + cur.chars().count()).map_or(s.len(), |(i, _)| i)..]);
                lines.push(cur);
                return lines;
            }
            lines.push(std::mem::take(&mut cur));
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}
