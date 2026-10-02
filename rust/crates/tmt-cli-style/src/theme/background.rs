//! Pure terminal-background interpretation. The caller owns environment reads,
//! terminal I/O, raw mode and query eligibility; this module receives values.

use std::{ops::Range, time::Duration};

/// Maximum total time allowed for an OSC 11 query, including its write.
pub const QUERY_TIMEOUT: Duration = Duration::from_millis(100);
/// Maximum bytes received during a query, including unrelated input.
pub const REPLY_LIMIT: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Background {
    Dark,
    Light,
}

/// Interpret the conventional two/three-field COLORFGBG value. The final
/// field is the ANSI background index; 8 is bright black, hence dark.
/// Foreground and pixmap fields may be `default`, as in rxvt-unicode.
/// These indices describe a conventional palette, not measured RGB values.
pub fn colorfgbg(value: &str) -> Option<Background> {
    let mut fields = value.split(';');
    let foreground = fields.next()?;
    let second = fields.next()?;
    let third = fields.next();
    if fields.next().is_some() {
        return None;
    }
    let component = |field: &str| {
        field == "default" || (!field.is_empty() && field.bytes().all(|byte| byte.is_ascii_digit()))
    };
    if !component(foreground) || !component(second) {
        return None;
    }
    let background = third.unwrap_or(second);
    if background.is_empty() || !background.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    match background.parse::<u8>().ok()? {
        0..=6 | 8 => Some(Background::Dark),
        7 | 9..=15 => Some(Background::Light),
        _ => None,
    }
}

/// Classify 16-bit sRGB channels using WCAG linear relative luminance. The
/// 0.179 threshold is where dark and light foreground contrast cross over.
pub fn classify(red: u16, green: u16, blue: u16) -> Background {
    let linear = |channel: u16| {
        let channel = f64::from(channel) / f64::from(u16::MAX);
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue);
    if luminance > 0.179 {
        Background::Light
    } else {
        Background::Dark
    }
}

/// Parse one complete OSC 11 rgb response, terminated by BEL or ST.
/// Each channel has 1–4 hexadecimal digits and scales to the full 16-bit range.
pub fn osc11(reply: &[u8]) -> Option<Background> {
    let payload = reply.strip_prefix(b"\x1b]11;rgb:")?;
    let payload = payload
        .strip_suffix(b"\x07")
        .or_else(|| payload.strip_suffix(b"\x1b\\"))?;
    let payload = std::str::from_utf8(payload).ok()?;
    let mut channels = payload.split('/');
    let mut channel = || {
        let value = channels.next()?;
        if !(1..=4).contains(&value.len()) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let parsed = u32::from_str_radix(value, 16).ok()?;
        let maximum = (1_u32 << (value.len() * 4)) - 1;
        u16::try_from(parsed * u32::from(u16::MAX) / maximum).ok()
    };
    let (red, green, blue) = (channel()?, channel()?, channel()?);
    if channels.next().is_some() {
        return None;
    }
    Some(classify(red, green, blue))
}

/// Received bytes remain available to the input owner. Only `reply` denotes
/// an accepted protocol frame; everything else is unrelated or invalid input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryReply {
    pub background: Option<Background>,
    pub received: Vec<u8>,
    pub reply: Option<Range<usize>>,
}

fn frame(bytes: &[u8]) -> Option<(Range<usize>, Background)> {
    for start in 0..bytes.len() {
        if !bytes[start..].starts_with(b"\x1b]11;rgb:") {
            continue;
        }
        for end in start + 9..=bytes.len() {
            if let Some(background) = osc11(&bytes[start..end]) {
                return Some((start..end, background));
            }
        }
    }
    None
}

/// Read with one absolute deadline and byte budget. `started` is captured
/// before the caller writes the query. `now` is a monotonic clock; `read`
/// must return within its supplied remaining duration and never exceed its
/// buffer. EOF, timeout and errors yield no signal. No I/O happens here beyond
/// invoking the supplied reader, and no thread or process state is retained.
///
/// Late bytes are retained for the caller's protocol filter but never accepted
/// as a detection. Callers must also filter replies arriving after this returns.
pub fn read_osc11<E>(
    started: Duration,
    mut now: impl FnMut() -> Duration,
    mut read: impl FnMut(Duration, &mut [u8]) -> Result<usize, E>,
) -> QueryReply {
    let mut result = QueryReply {
        background: None,
        received: Vec::new(),
        reply: None,
    };
    let mut buffer = [0; REPLY_LIMIT];
    while result.received.len() < REPLY_LIMIT {
        let elapsed = now().saturating_sub(started);
        if elapsed >= QUERY_TIMEOUT {
            break;
        }
        let remaining = REPLY_LIMIT - result.received.len();
        let Ok(count) = read(QUERY_TIMEOUT - elapsed, &mut buffer[..remaining]) else {
            break;
        };
        if count == 0 || count > remaining {
            break;
        }
        result.received.extend_from_slice(&buffer[..count]);
        if now().saturating_sub(started) >= QUERY_TIMEOUT {
            break;
        }
        if let Some((reply, background)) = frame(&result.received) {
            result.reply = Some(reply);
            result.background = Some(background);
            break;
        }
    }
    result
}

#[cfg(test)]
mod tests;
