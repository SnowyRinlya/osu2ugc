use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

const TICKS_PER_BEAT: f64 = 480.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Warning(pub String);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SixKeyLayout {
    /// Widths 3, 2, 3, 3, 2, 3 across all 16 units.
    #[default]
    Full,
    /// Six width-2 lanes at units 2..14, leaving two units on each side.
    Centered,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ConvertOptions {
    /// Adds a DAMAGE note at the end position of every converted long note.
    pub damage_ln_end: bool,
    /// Applies only to 6K maps. Other key counts retain their existing layouts.
    pub six_key_layout: SixKeyLayout,
}

#[derive(Debug)]
pub enum ConvertError {
    Io(std::io::Error),
    Invalid(String),
}

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Invalid(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ConvertError {}

impl From<std::io::Error> for ConvertError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Debug, Clone)]
struct TimingPoint {
    time: f64,
    beat_length: f64,
    meter: u32,
    uninherited: bool,
    order: usize,
}

#[derive(Debug, Clone)]
enum HitObject {
    Tap { x: i32, time: f64 },
    Hold { x: i32, start: f64, end: f64 },
}

#[derive(Default)]
struct Beatmap {
    mode: Option<i32>,
    key_count: Option<f64>,
    audio: String,
    title: String,
    title_unicode: String,
    artist: String,
    artist_unicode: String,
    creator: String,
    version: String,
    background: String,
    timing: Vec<TimingPoint>,
    objects: Vec<HitObject>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    General,
    Metadata,
    Difficulty,
    Events,
    TimingPoints,
    HitObjects,
}

pub fn convert_file(input: &Path, output: &Path) -> Result<Vec<Warning>, ConvertError> {
    convert_file_with_options(input, output, ConvertOptions::default())
}

pub fn convert_file_with_options(
    input: &Path,
    output: &Path,
    options: ConvertOptions,
) -> Result<Vec<Warning>, ConvertError> {
    let bytes = fs::read(input)?;
    let source = String::from_utf8(bytes)
        .map_err(|_| ConvertError::Invalid("输入文件不是有效的 UTF-8 编码 .osu 文件".to_owned()))?;
    let (result, warnings) = convert_str_with_options(&source, options)?;
    fs::write(output, result.as_bytes())?;
    Ok(warnings)
}

pub fn convert_str(source: &str) -> Result<(String, Vec<Warning>), ConvertError> {
    convert_str_with_options(source, ConvertOptions::default())
}

pub fn convert_str_with_options(
    source: &str,
    options: ConvertOptions,
) -> Result<(String, Vec<Warning>), ConvertError> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let map = parse_beatmap(source)?;
    validate(&map)?;
    render_ugc(&map, source, options)
}

#[derive(Debug, Clone, Copy)]
struct MeterSegment {
    start_tick: i64,
    start_bar: i64,
    meter: u32,
}

fn render_ugc(
    map: &Beatmap,
    source: &str,
    options: ConvertOptions,
) -> Result<(String, Vec<Warning>), ConvertError> {
    let layout = LaneLayout::from_key_count(map.key_count, options.six_key_layout)?;
    let mut timing = map.timing.clone();
    timing.sort_by(|a, b| {
        a.time
            .partial_cmp(&b.time)
            .unwrap_or(Ordering::Equal)
            .then(a.order.cmp(&b.order))
    });
    extend_timing_for_early_notes(&mut timing, &map.objects);
    let red: Vec<_> = timing.iter().filter(|point| point.uninherited).collect();
    let origin = red[0];
    let mut warnings = Vec::new();
    let meters = build_meter_segments(&red, &mut warnings);
    let title = preferred(&map.title_unicode, &map.title);
    let artist = preferred(&map.artist_unicode, &map.artist);
    let level = chart_level(&map.version);
    let chart_const = level.trim_end_matches('+').parse::<f64>().unwrap_or(0.0);
    let song_id = stable_song_id(map, source, options);
    let mut output = String::new();

    writeln!(output, "' Converted from osu!mania by osu2ugc").unwrap();
    ugc_header(&mut output, "VER", &["8"]);
    ugc_header(&mut output, "EXVER", &["1"]);
    ugc_header(&mut output, "TITLE", &[&clean(title)]);
    ugc_header(&mut output, "SORT", &[&sort_key(&map.title)]);
    ugc_header(&mut output, "ARTIST", &[&clean(artist)]);
    ugc_header(&mut output, "GENRE", &[""]);
    ugc_header(&mut output, "DESIGN", &[&clean(&map.creator)]);
    ugc_header(&mut output, "DIFF", &["3"]);
    ugc_header(&mut output, "LEVEL", &[&level]);
    ugc_header(&mut output, "CONST", &[&format!("{chart_const:.5}")]);
    ugc_header(&mut output, "SONGID", &[&song_id]);
    ugc_header(&mut output, "BGM", &[&clean(&map.audio)]);
    ugc_header(
        &mut output,
        "BGMOFS",
        &[&format!("{:.5}", -origin.time / 1000.0)],
    );
    ugc_header(&mut output, "BGMPRV", &["0.00000", "0.00000"]);
    ugc_header(&mut output, "JACKET", &[""]);
    ugc_header(&mut output, "BGIMG", &[&clean(&map.background)]);
    ugc_header(&mut output, "BGMODE", &["PASSIVE", "FALSE"]);
    ugc_header(&mut output, "FLDCOL", &["-1"]);
    ugc_header(&mut output, "FLDIMG", &[""]);
    for (flag, value) in [
        ("DIFFTTL", "FALSE"),
        ("SOFFSET", "TRUE"),
        ("CLICK", "TRUE"),
        ("EXLONG", "FALSE"),
        ("BGMWCMP", "FALSE"),
        ("HIPRECISION", "TRUE"),
    ] {
        ugc_header(&mut output, "FLAG", &[flag, value]);
    }
    ugc_header(&mut output, "ATINFO", &["AUTHORS", ""]);
    ugc_header(&mut output, "ATINFO", &["SITES", ""]);
    ugc_header(&mut output, "DLURL", &[""]);
    ugc_header(&mut output, "COPYRIGHT", &[""]);
    ugc_header(&mut output, "LICENSE", &["", ""]);
    ugc_header(&mut output, "TICKS", &["480"]);
    for segment in &meters {
        ugc_header(
            &mut output,
            "BEAT",
            &[
                &segment.start_bar.to_string(),
                &segment.meter.to_string(),
                "4",
            ],
        );
    }

    let mut last_sv: Option<f64> = None;
    for point in &timing {
        let tick = nonnegative_tick(time_to_tick(point.time, &red), &mut warnings, "时间点");
        let bar_tick = format_bar_tick(tick, &meters);
        if point.uninherited {
            ugc_header(
                &mut output,
                "BPM",
                &[&bar_tick, &format!("{:.5}", 60_000.0 / point.beat_length)],
            );
            if last_sv.map_or(true, |speed| (speed - 1.0).abs() > 1e-9) {
                ugc_header(&mut output, "TIL", &["0", &bar_tick, "1.00000"]);
                last_sv = Some(1.0);
            }
        } else {
            let speed = -100.0 / point.beat_length;
            if last_sv.map_or(true, |old| (old - speed).abs() > 1e-9) {
                ugc_header(
                    &mut output,
                    "TIL",
                    &["0", &bar_tick, &format!("{speed:.5}")],
                );
                last_sv = Some(speed);
            }
        }
    }
    if last_sv.is_none() {
        ugc_header(&mut output, "TIL", &["0", "0'0", "1.00000"]);
    }
    ugc_header(&mut output, "MAINTIL", &["0"]);
    output.push_str("@ENDHEAD\n\n");

    let mut objects = map.objects.clone();
    objects.sort_by(|a, b| {
        object_time(a)
            .partial_cmp(&object_time(b))
            .unwrap_or(Ordering::Equal)
    });
    for object in &objects {
        match *object {
            HitObject::Tap { x, time } => {
                let tick = nonnegative_tick(time_to_tick(time, &red), &mut warnings, "音符");
                writeln!(
                    output,
                    "#{}:t{}{}",
                    format_bar_tick(tick, &meters),
                    base36(layout.lane_start(x) as u32),
                    layout.lane_width(x)
                )
                .unwrap();
            }
            HitObject::Hold { x, start, end } => {
                let start_tick =
                    nonnegative_tick(time_to_tick(start, &red), &mut warnings, "长按起点");
                let end_tick = nonnegative_tick(time_to_tick(end, &red), &mut warnings, "长按终点");
                writeln!(
                    output,
                    "#{}:h{}{}",
                    format_bar_tick(start_tick, &meters),
                    base36(layout.lane_start(x) as u32),
                    layout.lane_width(x)
                )
                .unwrap();
                writeln!(output, "#{}>s", end_tick.saturating_sub(start_tick)).unwrap();
                if options.damage_ln_end {
                    writeln!(
                        output,
                        "#{}:d{}{}",
                        format_bar_tick(end_tick, &meters),
                        base36(layout.lane_start(x) as u32),
                        layout.lane_width(x)
                    )
                    .unwrap();
                }
            }
        }
    }
    let _ = source;
    Ok((output, warnings))
}

fn extend_timing_for_early_notes(timing: &mut Vec<TimingPoint>, objects: &[HitObject]) {
    let first_red = timing
        .iter()
        .find(|point| point.uninherited)
        .unwrap()
        .clone();
    let earliest = objects
        .iter()
        .map(object_time)
        .chain(timing.iter().map(|point| point.time))
        .fold(first_red.time, f64::min);
    if earliest >= first_red.time {
        return;
    }
    // Extend the initial tempo backwards by whole bars. This preserves the
    // existing bar grid and moves the audio offset along with the chart origin,
    // so early notes and hold starts are not clamped to the first red line.
    let bar_ms = first_red.beat_length * f64::from(first_red.meter.max(1));
    if !bar_ms.is_finite() || bar_ms <= 0.0 {
        return;
    }
    let bars = ((first_red.time - earliest) / bar_ms).ceil();
    timing.insert(
        0,
        TimingPoint {
            time: first_red.time - bars * bar_ms,
            ..first_red
        },
    );
}

fn build_meter_segments(red: &[&TimingPoint], warnings: &mut Vec<Warning>) -> Vec<MeterSegment> {
    let mut segments = vec![MeterSegment {
        start_tick: 0,
        start_bar: 0,
        meter: red[0].meter.max(1),
    }];
    for point in red.iter().skip(1) {
        let current = *segments.last().unwrap();
        if point.meter == current.meter {
            continue;
        }
        let tick = time_to_tick(point.time, red).max(0);
        let measure_ticks = i64::from(current.meter) * 480;
        let elapsed = tick - current.start_tick;
        if elapsed % measure_ticks != 0 {
            warnings.push(Warning(format!(
                "tick {tick} 的拍号变化不在小节线上，UGC 无法精确表示，已继续使用 {}/4",
                current.meter
            )));
            continue;
        }
        segments.push(MeterSegment {
            start_tick: tick,
            start_bar: current.start_bar + elapsed / measure_ticks,
            meter: point.meter.max(1),
        });
    }
    segments
}

fn format_bar_tick(tick: i64, meters: &[MeterSegment]) -> String {
    let segment = meters
        .iter()
        .rev()
        .find(|segment| segment.start_tick <= tick)
        .unwrap_or(&meters[0]);
    let elapsed = tick - segment.start_tick;
    let measure_ticks = i64::from(segment.meter) * 480;
    format!(
        "{}'{}",
        segment.start_bar + elapsed.div_euclid(measure_ticks),
        elapsed.rem_euclid(measure_ticks)
    )
}

fn ugc_header(output: &mut String, command: &str, fields: &[&str]) {
    output.push('@');
    output.push_str(command);
    for field in fields {
        output.push('\t');
        output.push_str(field);
    }
    output.push('\n');
}

fn sort_key(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_uppercase)
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect()
}

fn base36(value: u32) -> char {
    char::from_digit(value, 36)
        .unwrap_or('0')
        .to_ascii_uppercase()
}

fn parse_beatmap(source: &str) -> Result<Beatmap, ConvertError> {
    let mut map = Beatmap::default();
    let mut section = Section::None;

    for (index, raw_line) in source.lines().enumerate() {
        let line_no = index + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = match &line[1..line.len() - 1] {
                "General" => Section::General,
                "Metadata" => Section::Metadata,
                "Difficulty" => Section::Difficulty,
                "Events" => Section::Events,
                "TimingPoints" => Section::TimingPoints,
                "HitObjects" => Section::HitObjects,
                _ => Section::None,
            };
            continue;
        }

        match section {
            Section::General | Section::Metadata | Section::Difficulty => {
                let Some((key, value)) = line.split_once(':') else {
                    continue;
                };
                let value = value.trim().to_owned();
                match key.trim() {
                    "Mode" => map.mode = value.parse().ok(),
                    "CircleSize" => map.key_count = value.parse().ok(),
                    "AudioFilename" => map.audio = value,
                    "Title" => map.title = value,
                    "TitleUnicode" => map.title_unicode = value,
                    "Artist" => map.artist = value,
                    "ArtistUnicode" => map.artist_unicode = value,
                    "Creator" => map.creator = value,
                    "Version" => map.version = value,
                    _ => {}
                }
            }
            Section::Events => {
                if map.background.is_empty() && line.starts_with("0,0,") {
                    map.background = csv_fields(line)
                        .get(2)
                        .map(|s| s.trim_matches('"').to_owned())
                        .unwrap_or_default();
                }
            }
            Section::TimingPoints => {
                let fields: Vec<_> = line.split(',').collect();
                if fields.len() < 2 {
                    return Err(invalid_line(line_no, "TimingPoint 字段不足"));
                }
                let time = number(fields[0], line_no, "时间点时间")?;
                let beat_length = number(fields[1], line_no, "beatLength")?;
                let meter = fields.get(2).and_then(|s| s.parse().ok()).unwrap_or(4);
                let uninherited = fields.get(6).map(|s| s.trim() != "0").unwrap_or(true);
                if !beat_length.is_finite() || beat_length == 0.0 {
                    return Err(invalid_line(line_no, "beatLength 必须是有限非零数"));
                }
                map.timing.push(TimingPoint {
                    time,
                    beat_length,
                    meter,
                    uninherited,
                    order: index,
                });
            }
            Section::HitObjects => {
                let fields: Vec<_> = line.split(',').collect();
                if fields.len() < 5 {
                    return Err(invalid_line(line_no, "HitObject 字段不足"));
                }
                let x: i32 = fields[0]
                    .trim()
                    .parse()
                    .map_err(|_| invalid_line(line_no, "音符横坐标无效"))?;
                let time = number(fields[2], line_no, "音符时间")?;
                let kind: i32 = fields[3]
                    .trim()
                    .parse()
                    .map_err(|_| invalid_line(line_no, "音符类型无效"))?;
                if kind & 128 != 0 {
                    let end_field = fields
                        .get(5)
                        .ok_or_else(|| invalid_line(line_no, "长按音符缺少结束时间"))?;
                    let end = number(
                        end_field.split(':').next().unwrap_or(""),
                        line_no,
                        "长按结束时间",
                    )?;
                    if end < time {
                        return Err(invalid_line(line_no, "长按结束时间早于开始时间"));
                    }
                    map.objects.push(HitObject::Hold {
                        x,
                        start: time,
                        end,
                    });
                } else if kind & 1 != 0 {
                    map.objects.push(HitObject::Tap { x, time });
                }
            }
            Section::None => {}
        }
    }
    Ok(map)
}

fn validate(map: &Beatmap) -> Result<(), ConvertError> {
    if map.mode != Some(3) {
        return Err(ConvertError::Invalid(
            "只支持 osu!mania（Mode: 3）谱面".to_owned(),
        ));
    }
    LaneLayout::from_key_count(map.key_count, SixKeyLayout::default())?;
    if map.audio.is_empty() {
        return Err(ConvertError::Invalid("缺少 AudioFilename".to_owned()));
    }
    if map.timing.iter().all(|point| !point.uninherited) {
        return Err(ConvertError::Invalid(
            "谱面没有有效的非继承 TimingPoint（红线）".to_owned(),
        ));
    }
    Ok(())
}

#[allow(dead_code)]
fn render_mgcf(map: &Beatmap, source: &str) -> Result<(String, Vec<Warning>), ConvertError> {
    let layout = LaneLayout::from_key_count(map.key_count, SixKeyLayout::default())?;
    let mut timing = map.timing.clone();
    timing.sort_by(|a, b| {
        a.time
            .partial_cmp(&b.time)
            .unwrap_or(Ordering::Equal)
            .then(a.order.cmp(&b.order))
    });
    let red: Vec<_> = timing.iter().filter(|p| p.uninherited).collect();
    let origin = red[0];
    let mut warnings = Vec::new();
    let mut output = String::new();
    let title = preferred(&map.title_unicode, &map.title);
    let artist = preferred(&map.artist_unicode, &map.artist);
    let level = chart_level(&map.version);

    line(&mut output, &["MGCF0"]);
    line(&mut output, &["VERSION", "2"]);
    line(&mut output, &["BEGIN", "META"]);
    line(&mut output, &["TITLE", &clean(title)]);
    line(&mut output, &["ARTIST", &clean(artist)]);
    line(&mut output, &["DESIGNER", &clean(&map.creator)]);
    line(&mut output, &["DIFFICULTY", "3"]);
    line(&mut output, &["PLAYLEVEL", &level]);
    line(&mut output, &["WEATTRIBUTE", ""]);
    line(&mut output, &["CHARTCONST", &level]);
    line(&mut output, &["SONGID", &stable_uuid(source)]);
    line(&mut output, &["BGM", &clean(&map.audio)]);
    line(
        &mut output,
        &["BGMOFFSET", &format_float(-origin.time / 1000.0)],
    );
    line(&mut output, &["BGMPREVIEW", "0.00000", "15.00000"]);
    line(&mut output, &["JACKET", ""]);
    line(&mut output, &["BG", &clean(&map.background)]);
    line(&mut output, &["BGSCENE", ""]);
    line(&mut output, &["BGSYNC", "1"]);
    line(&mut output, &["FIELDCOL", "0"]);
    line(&mut output, &["FIELDBG", ""]);
    line(&mut output, &["FIELDSCENE", ""]);
    line(&mut output, &["MAINTIL", "0"]);
    line(
        &mut output,
        &["MAINBPM", &format_float(60_000.0 / origin.beat_length)],
    );
    line(&mut output, &["TUTORIAL", "0"]);
    line(&mut output, &["SOFFSET", "1"]);
    line(&mut output, &["USECLICK", "1"]);
    line(&mut output, &["EXLONG", "0"]);
    line(&mut output, &["BGMWAITEND", "0"]);
    line(&mut output, &["AUTHOR_LIST", ""]);
    line(&mut output, &["AUTHOR_SITES", ""]);
    line(&mut output, &["DLURL", ""]);
    line(&mut output, &["COPYRIGHT", ""]);
    line(&mut output, &["LICENSE", "", ""]);
    line(&mut output, &["BEGIN", "HEADER"]);

    let mut last_meter = None;
    let mut last_sv: Option<f64> = None;
    for point in &timing {
        let raw_tick = time_to_tick(point.time, &red);
        let tick = nonnegative_tick(raw_tick, &mut warnings, "时间点");
        if point.uninherited {
            let bpm = 60_000.0 / point.beat_length;
            line(&mut output, &["BPM", &tick.to_string(), &format_float(bpm)]);
            if last_meter != Some(point.meter) {
                if tick % 480 != 0 {
                    warnings.push(Warning(format!(
                        "拍号变化位于 tick {tick}，Margrete 的小节位置可能需要手动检查"
                    )));
                }
                let measure = measure_at_tick(tick, last_meter.unwrap_or(point.meter));
                line(
                    &mut output,
                    &["BEAT", &measure.to_string(), &point.meter.to_string(), "4"],
                );
                last_meter = Some(point.meter);
            }
            if last_sv.map_or(true, |v| (v - 1.0).abs() > 1e-9) {
                line(&mut output, &["TIL", "0", &tick.to_string(), "1"]);
                last_sv = Some(1.0);
            }
        } else {
            let sv = -100.0 / point.beat_length;
            if last_sv.map_or(true, |v| (v - sv).abs() > 1e-9) {
                line(
                    &mut output,
                    &["TIL", "0", &tick.to_string(), &format_float(sv)],
                );
                last_sv = Some(sv);
            }
        }
    }

    line(&mut output, &["BEGIN", "NOTES"]);
    let mut objects = map.objects.clone();
    objects.sort_by(|a, b| {
        object_time(a)
            .partial_cmp(&object_time(b))
            .unwrap_or(Ordering::Equal)
    });
    for object in &objects {
        match *object {
            HitObject::Tap { x, time } => {
                let tick = nonnegative_tick(time_to_tick(time, &red), &mut warnings, "音符");
                write_note(
                    &mut output,
                    "t",
                    "N",
                    tick,
                    layout.lane_start(x),
                    layout.lane_width(x),
                );
            }
            HitObject::Hold { x, start, end } => {
                let start_tick =
                    nonnegative_tick(time_to_tick(start, &red), &mut warnings, "长按起点");
                let end_tick = nonnegative_tick(time_to_tick(end, &red), &mut warnings, "长按终点");
                write_note(
                    &mut output,
                    "h",
                    "BG",
                    start_tick,
                    layout.lane_start(x),
                    layout.lane_width(x),
                );
                write_note(
                    &mut output,
                    ".h",
                    "EN",
                    end_tick,
                    layout.lane_start(x),
                    layout.lane_width(x),
                );
            }
        }
    }
    Ok((output, warnings))
}

fn time_to_tick(time: f64, red: &[&TimingPoint]) -> i64 {
    let origin = red[0];
    if time < origin.time {
        return ((time - origin.time) / origin.beat_length * TICKS_PER_BEAT).round() as i64;
    }
    let mut ticks = 0.0;
    let mut previous = origin;
    for current in red.iter().skip(1) {
        if current.time > time {
            break;
        }
        ticks += (current.time - previous.time) / previous.beat_length * TICKS_PER_BEAT;
        previous = current;
    }
    ticks += (time - previous.time) / previous.beat_length * TICKS_PER_BEAT;
    ticks.round() as i64
}

fn nonnegative_tick(tick: i64, warnings: &mut Vec<Warning>, what: &str) -> i64 {
    if tick < 0 {
        warnings.push(Warning(format!("{what} 位于首个红线之前，已移到 tick 0")));
        0
    } else {
        tick
    }
}

fn measure_at_tick(tick: i64, meter: u32) -> i64 {
    let ticks_per_measure = i64::from(meter.max(1)) * 480;
    tick.div_euclid(ticks_per_measure)
}

struct LaneLayout {
    lanes: &'static [(i32, i32)],
}

impl LaneLayout {
    fn from_key_count(
        keys: Option<f64>,
        six_key_layout: SixKeyLayout,
    ) -> Result<Self, ConvertError> {
        match keys {
            Some(4.0) => Ok(Self {
                lanes: &[(0, 4), (4, 4), (8, 4), (12, 4)],
            }),
            Some(5.0) => Ok(Self {
                lanes: &[(0, 3), (3, 3), (6, 3), (9, 3), (12, 3)],
            }),
            Some(6.0) => Ok(Self {
                lanes: match six_key_layout {
                    SixKeyLayout::Full => &[(0, 3), (3, 2), (5, 3), (8, 3), (11, 2), (13, 3)],
                    SixKeyLayout::Centered => &[(2, 2), (4, 2), (6, 2), (8, 2), (10, 2), (12, 2)],
                },
            }),
            Some(7.0) => Ok(Self {
                lanes: &[(0, 2), (2, 2), (4, 2), (6, 2), (8, 2), (10, 2), (12, 2)],
            }),
            Some(keys) => Err(ConvertError::Invalid(format!(
                "只支持 4K、5K、6K 或 7K 谱面，当前 CircleSize 为 {keys}"
            ))),
            None => Err(ConvertError::Invalid("缺少 CircleSize".to_owned())),
        }
    }

    fn lane(&self, x: i32) -> (i32, i32) {
        // Decode the source column using its actual key count, then place it
        // using explicit destination positions and widths.
        let index = (i64::from(x).clamp(0, 511) * self.lanes.len() as i64 / 512) as usize;
        self.lanes[index]
    }

    fn lane_start(&self, x: i32) -> i32 {
        self.lane(x).0
    }

    fn lane_width(&self, x: i32) -> i32 {
        self.lane(x).1
    }
}

fn write_note(output: &mut String, kind: &str, subtype: &str, tick: i64, lane: i32, width: i32) {
    line(
        output,
        &[
            kind,
            subtype,
            "N",
            "N",
            &tick.to_string(),
            &lane.to_string(),
            &width.to_string(),
            "8",
            "0",
            "0",
        ],
    );
}

fn line(output: &mut String, fields: &[&str]) {
    let _ = writeln!(output, "{}", fields.join("\t"));
}

fn preferred<'a>(unicode: &'a str, fallback: &'a str) -> &'a str {
    if unicode.trim().is_empty() {
        fallback
    } else {
        unicode
    }
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

fn chart_level(version: &str) -> String {
    let Some(open) = version.rfind('[') else {
        return "1".to_owned();
    };
    let Some(close_offset) = version[open + 1..].find(']') else {
        return "1".to_owned();
    };
    let candidate = &version[open + 1..open + 1 + close_offset];
    if !candidate.is_empty()
        && candidate.len() <= 2
        && candidate.bytes().all(|b| b.is_ascii_digit())
    {
        candidate.to_owned()
    } else {
        "1".to_owned()
    }
}

// All converted charts use DIFF=3, so separate source difficulties must also
// have separate song IDs. Version this domain only if the identity rules change.
fn stable_song_id(map: &Beatmap, source: &str, options: ConvertOptions) -> String {
    let mut hash = Sha256::new();
    hash.update(b"SnowyRinlya/osu2ugc/song-id/v2\0");
    let layout = match map.key_count {
        Some(4.0) => "4k-full-4",
        Some(5.0) => "5k-left-3",
        Some(6.0) => match options.six_key_layout {
            SixKeyLayout::Full => "6k-full-323323",
            SixKeyLayout::Centered => "6k-centered-2",
        },
        Some(7.0) => "7k-left-2",
        _ => unreachable!("key count validated before rendering"),
    };
    hash.update(layout.as_bytes());
    hash.update([0, u8::from(options.damage_ln_end), 0]);
    // Include source content, not just its title or possibly reused BeatmapID.
    // Ignore transport/editor-only BOM, CRLF, blank lines and full-line comments.
    for line in source.trim_start_matches('\u{feff}').lines().map(str::trim) {
        if !line.is_empty() && !line.starts_with("//") {
            hash.update(line.as_bytes());
            hash.update(b"\n");
        }
    }
    let digest = hash.finalize();
    let mut id = String::from("O2U");
    // Keep the existing 35-character ASCII shape, with a dedicated prefix and
    // 128 bits from SHA-256. No finite ID scheme guarantees global uniqueness.
    for byte in &digest[..16] {
        write!(id, "{byte:02X}").unwrap();
    }
    id
}

fn stable_uuid(source: &str) -> String {
    fn fnv(bytes: &[u8], seed: u64) -> u64 {
        bytes.iter().fold(seed, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
    }
    let high = fnv(source.as_bytes(), 0xcbf29ce484222325);
    let low = fnv(source.as_bytes(), 0x84222325cbf29ce4);
    let mut bytes = ((u128::from(high) << 64) | u128::from(low)).to_be_bytes();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

fn format_float(value: f64) -> String {
    let text = format!("{value:.10}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn object_time(object: &HitObject) -> f64 {
    match object {
        HitObject::Tap { time, .. } => *time,
        HitObject::Hold { start, .. } => *start,
    }
}

fn number(value: &str, line: usize, name: &str) -> Result<f64, ConvertError> {
    let parsed: f64 = value
        .trim()
        .parse()
        .map_err(|_| invalid_line(line, &format!("{name}无效")))?;
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(invalid_line(line, &format!("{name}不是有限数")))
    }
}

fn invalid_line(line: usize, message: &str) -> ConvertError {
    ConvertError::Invalid(format!("第 {line} 行：{message}"))
}

fn csv_fields(line: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in line.chars() {
        match ch {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                result.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }
    result.push(current);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAP: &str = r#"osu file format v14

[General]
AudioFilename: song.mp3
Mode: 3

[Metadata]
Title:Test
TitleUnicode:测试
Artist:Artist
Creator:Mapper
Version:Hard [12]

[Difficulty]
CircleSize:4

[Events]
0,0,"bg,image.jpg",0,0

[TimingPoints]
1000,500,4,2,0,100,1,0
2000,-50,4,2,0,100,0,0
3000,400,4,2,0,100,1,0

[HitObjects]
64,192,1000,1,0,0:0:0:0:
192,192,1500,128,0,2500:0:0:0:0:
320,192,3000,1,0,0:0:0:0:
448,192,3400,1,0,0:0:0:0:
"#;

    #[test]
    fn converts_all_four_lanes_and_holds() {
        let (output, warnings) = convert_str(MAP).unwrap();
        assert!(warnings.is_empty());
        assert!(output.contains("@VER\t8\n"));
        assert!(output.contains("@TITLE\t测试\n"));
        assert!(output.contains("@BGIMG\tbg,image.jpg\n"));
        assert!(output.contains("@BPM\t0'0\t120.00000\n"));
        assert!(output.contains("@BPM\t1'0\t150.00000\n"));
        assert!(output.contains("@TIL\t0\t0'960\t2.00000\n"));
        assert!(output.contains("#0'0:t04\n"));
        assert!(output.contains("#0'480:h44\n#960>s\n"));
        assert!(output.contains("#1'0:t84\n"));
        assert!(output.contains("#1'480:tC4\n"));
    }

    #[test]
    fn rejects_unsupported_key_counts() {
        for keys in ["3", "8", "4.5", "NaN", "inf"] {
            let error = convert_str(&MAP.replace("CircleSize:4", &format!("CircleSize:{keys}")))
                .unwrap_err();
            assert!(error.to_string().contains("只支持 4K、5K、6K 或 7K"));
        }
    }

    #[test]
    fn converts_five_key_taps_holds_and_damage_left_aligned() {
        let mut source = MAP
            .split("[HitObjects]")
            .next()
            .unwrap()
            .replace("CircleSize:4", "CircleSize:5");
        source.push_str("[HitObjects]\n");
        for x in [51, 153, 256, 358, 460] {
            writeln!(source, "{x},192,1000,1,0,0:0:0:0:").unwrap();
            writeln!(source, "{x},192,1500,128,0,2500:0:0:0:0:").unwrap();
        }
        let (output, warnings) = convert_str_with_options(
            &source,
            ConvertOptions {
                damage_ln_end: true,
                ..ConvertOptions::default()
            },
        )
        .unwrap();
        assert!(warnings.is_empty());
        for lane in ['0', '3', '6', '9', 'C'] {
            assert!(output.contains(&format!("#0'0:t{lane}3\n")));
            assert!(output.contains(&format!("#0'480:h{lane}3\n#960>s\n#0'1440:d{lane}3\n")));
        }
        assert_eq!(output.lines().filter(|line| line.contains(':')).count(), 15);
        assert!(!convert_str(&source).unwrap().0.contains(":d"));
    }

    #[test]
    fn five_key_column_boundaries_and_clamping() {
        let layout = LaneLayout::from_key_count(Some(5.0), SixKeyLayout::default()).unwrap();
        for (start, end, expected) in [
            (0, 102, 0),
            (103, 204, 3),
            (205, 307, 6),
            (308, 409, 9),
            (410, 511, 12),
        ] {
            for x in start..=end {
                assert_eq!(layout.lane(x), (expected, 3));
            }
        }
        assert_eq!(layout.lane(i32::MIN), (0, 3));
        assert_eq!(layout.lane(i32::MAX), (12, 3));
    }

    #[test]
    fn converts_seven_lanes_taps_holds_and_damage() {
        let mut source = MAP
            .split("[HitObjects]")
            .next()
            .unwrap()
            .replace("CircleSize:4", "CircleSize:7");
        source.push_str("[HitObjects]\n");
        for x in [36, 109, 182, 256, 329, 402, 475] {
            writeln!(source, "{x},192,1000,1,0,0:0:0:0:").unwrap();
            writeln!(source, "{x},192,1500,128,0,2500:0:0:0:0:").unwrap();
        }
        let (output, warnings) = convert_str_with_options(
            &source,
            ConvertOptions {
                damage_ln_end: true,
                ..ConvertOptions::default()
            },
        )
        .unwrap();
        assert!(warnings.is_empty());
        for lane in ['0', '2', '4', '6', '8', 'A', 'C'] {
            assert!(output.contains(&format!("#0'0:t{lane}2\n")));
            assert!(output.contains(&format!("#0'480:h{lane}2\n#960>s\n#0'1440:d{lane}2\n")));
        }
        assert_eq!(output.lines().filter(|line| line.contains(':')).count(), 21);
        let (default_output, _) = convert_str(&source).unwrap();
        assert!(!default_output.contains(":d"));
    }

    #[test]
    fn seven_key_column_boundaries_and_clamping() {
        let layout = LaneLayout::from_key_count(Some(7.0), SixKeyLayout::default()).unwrap();
        for (start, end, expected) in [
            (0, 73, 0),
            (74, 146, 2),
            (147, 219, 4),
            (220, 292, 6),
            (293, 365, 8),
            (366, 438, 10),
            (439, 511, 12),
        ] {
            for x in start..=end {
                assert_eq!(layout.lane_start(x), expected);
            }
        }
        assert_eq!(layout.lane_start(i32::MIN), 0);
        assert_eq!(layout.lane_start(i32::MAX), 12);
    }

    #[test]
    fn song_ids_separate_difficulties_edits_layouts_and_ln_options() {
        fn id(source: &str, options: ConvertOptions) -> String {
            convert_str_with_options(source, options)
                .unwrap()
                .0
                .lines()
                .find_map(|line| line.strip_prefix("@SONGID\t"))
                .unwrap()
                .to_owned()
        }
        let defaults = ConvertOptions::default();
        let original = id(MAP, defaults);
        assert!(original.starts_with("O2U"));
        assert_eq!(original.len(), 35);
        assert!(original[3..].bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(original, id(MAP, defaults));
        assert_eq!(
            original,
            id(
                &format!("\u{feff}// comment\r\n{}", MAP.replace('\n', "\r\n")),
                defaults
            )
        );
        assert_ne!(
            original,
            id(&MAP.replace("Hard [12]", "Another [12]"), defaults)
        );
        assert_ne!(
            original,
            id(&MAP.replace("64,192,1000", "128,192,1000"), defaults)
        );
        assert_ne!(
            original,
            id(
                MAP,
                ConvertOptions {
                    damage_ln_end: true,
                    ..defaults
                }
            )
        );
        assert_ne!(
            original,
            id(&MAP.replace("CircleSize:4", "CircleSize:7"), defaults)
        );
        // A 6K-only option must not change identities for other key counts.
        assert_eq!(
            original,
            id(
                MAP,
                ConvertOptions {
                    six_key_layout: SixKeyLayout::Centered,
                    ..defaults
                }
            )
        );
        let registered = MAP.replace("[Metadata]", "[Metadata]\nBeatmapID:12345");
        assert_ne!(
            id(&registered, defaults),
            id(&registered.replace("64,192,1000", "128,192,1000"), defaults)
        );
    }

    #[test]
    fn accepts_utf8_bom() {
        assert!(convert_str(&format!("\u{feff}{MAP}")).is_ok());
    }

    #[test]
    fn converts_both_six_key_layouts_with_holds_and_damage() {
        let mut source = MAP
            .split("[HitObjects]")
            .next()
            .unwrap()
            .replace("CircleSize:4", "CircleSize:6");
        source.push_str("[HitObjects]\n");
        for x in [42, 128, 213, 298, 384, 469] {
            writeln!(source, "{x},192,1000,1,0,0:0:0:0:").unwrap();
            writeln!(source, "{x},192,1500,128,0,2500:0:0:0:0:").unwrap();
        }
        let mut ids = Vec::new();
        for (six_key_layout, lanes) in [
            (SixKeyLayout::Full, ["03", "32", "53", "83", "B2", "D3"]),
            (SixKeyLayout::Centered, ["22", "42", "62", "82", "A2", "C2"]),
        ] {
            let (output, warnings) = convert_str_with_options(
                &source,
                ConvertOptions {
                    damage_ln_end: true,
                    six_key_layout,
                },
            )
            .unwrap();
            assert!(warnings.is_empty());
            for lane in lanes {
                assert!(output.contains(&format!("#0'0:t{lane}\n")));
                assert!(output.contains(&format!("#0'480:h{lane}\n#960>s\n#0'1440:d{lane}\n")));
            }
            assert_eq!(output.lines().filter(|line| line.contains(':')).count(), 18);
            ids.push(
                output
                    .lines()
                    .find(|line| line.starts_with("@SONGID\t"))
                    .unwrap()
                    .to_owned(),
            );
        }
        assert_ne!(ids[0], ids[1]);
        let (default_output, _) = convert_str(&source).unwrap();
        let (full_output, _) = convert_str_with_options(
            &source,
            ConvertOptions {
                six_key_layout: SixKeyLayout::Full,
                ..ConvertOptions::default()
            },
        )
        .unwrap();
        assert_eq!(default_output, full_output);
        assert!(!default_output.contains(":d"));
    }

    #[test]
    fn six_key_column_boundaries_and_clamping() {
        let full = LaneLayout::from_key_count(Some(6.0), SixKeyLayout::Full).unwrap();
        let centered = LaneLayout::from_key_count(Some(6.0), SixKeyLayout::Centered).unwrap();
        for (start, end, full_lane, centered_lane) in [
            (0, 85, (0, 3), (2, 2)),
            (86, 170, (3, 2), (4, 2)),
            (171, 255, (5, 3), (6, 2)),
            (256, 341, (8, 3), (8, 2)),
            (342, 426, (11, 2), (10, 2)),
            (427, 511, (13, 3), (12, 2)),
        ] {
            for x in start..=end {
                assert_eq!(full.lane(x), full_lane);
                assert_eq!(centered.lane(x), centered_lane);
            }
        }
        assert_eq!(full.lane(i32::MIN), (0, 3));
        assert_eq!(full.lane(i32::MAX), (13, 3));
        assert_eq!(centered.lane(i32::MIN), (2, 2));
        assert_eq!(centered.lane(i32::MAX), (12, 2));
    }

    #[test]
    fn six_key_option_does_not_change_other_key_counts() {
        for source in [
            MAP.to_owned(),
            MAP.replace("CircleSize:4", "CircleSize:5"),
            MAP.replace("CircleSize:4", "CircleSize:7"),
        ] {
            let default = convert_str(&source).unwrap();
            let centered = convert_str_with_options(
                &source,
                ConvertOptions {
                    six_key_layout: SixKeyLayout::Centered,
                    ..ConvertOptions::default()
                },
            )
            .unwrap();
            assert_eq!(default, centered);
        }
    }

    #[test]
    fn preserves_notes_and_holds_before_the_first_red_line() {
        let source = MAP.replace(
            "[HitObjects]\n",
            "[HitObjects]\n64,192,500,1,0,0:0:0:0:\n192,192,750,128,0,1500:0:0:0:0:\n",
        );
        let (output, warnings) = convert_str(&source).unwrap();
        assert!(warnings.is_empty());
        assert!(output.contains("@BGMOFS\t1.00000\n"));
        assert!(output.contains("@BPM\t0'0\t120.00000\n"));
        assert!(output.contains("@BPM\t1'0\t120.00000\n"));
        assert!(output.contains("#0'1440:t04\n"));
        assert!(output.contains("#0'1680:h44\n#720>s\n"));
        assert!(output.contains("#1'0:t04\n"));
        // Chart time 1.5 s minus BGM delay 1 s = original note time 500 ms.
    }

    #[test]
    fn optionally_adds_damage_at_hold_end() {
        let (output, warnings) = convert_str_with_options(
            MAP,
            ConvertOptions {
                damage_ln_end: true,
                ..ConvertOptions::default()
            },
        )
        .unwrap();
        assert!(warnings.is_empty());
        assert!(output.contains("#0'480:h44\n#960>s\n#0'1440:d44\n"));
        assert_eq!(output.lines().filter(|line| line.contains(":d")).count(), 1);
    }
}
