//! Axis ticks: nice linear steps, log decades, calendar-aware time steps,
//! and the default number and time labels.
//!
//! The algorithms follow ECharts (`numberUtil.nice`, `IntervalScale`,
//! `LogScale`, `TimeScale`) so an axis lands on the same round numbers an
//! ECharts user expects.

use crate::option::TimeUnit;

/// A value axis' extent after rounding, and its ticks.
#[derive(Debug, Clone, PartialEq)]
pub struct LinearTicks {
    pub min: f64,
    pub max: f64,
    /// The tick interval (for logs: in exponent units).
    pub step: f64,
    /// Tick values from `min` to `max` inclusive, ascending.
    pub ticks: Vec<f64>,
}

const MS_PER_SECOND: i64 = 1_000;
const MS_PER_MINUTE: i64 = 60_000;
const MS_PER_HOUR: i64 = 3_600_000;
const MS_PER_DAY: i64 = 86_400_000;
/// Years beyond any Unix ms instant; civil arithmetic clamps to it.
const MAX_YEAR: i128 = 1_000_000_000_000;
/// Mean Gregorian month and year, for picking a step only.
const MS_PER_MONTH: f64 = MS_PER_YEAR / 12.0;
const MS_PER_YEAR: f64 = 365.2425 * MS_PER_DAY as f64;

/// `x * 10^exponent`, dividing for negative exponents so a decimal such as
/// `3 * 10^-1` comes out as the nearest `f64` to `0.3`.
fn scale10(x: f64, exponent: i32) -> f64 {
    if exponent >= 0 {
        x * 10f64.powi(exponent)
    } else {
        x / 10f64.powi(-exponent)
    }
}

/// Rounds `q` to the nearest integer when it is within float noise of it.
fn snap(q: f64) -> f64 {
    let r = q.round();
    if (q - r).abs() <= 1e-9 * r.abs().max(1.0) {
        r
    } else {
        q
    }
}

/// ECharts `quantityExponent`: the power of ten at or below a positive
/// `value`.
fn quantity_exponent(value: f64) -> i32 {
    let exponent = value.log10().floor() as i32;
    if scale10(value, -exponent) >= 10.0 {
        exponent + 1
    } else {
        exponent
    }
}

/// A nice number as an integer mantissa times a power of ten, so its
/// multiples are formed exactly instead of by accumulating a float step.
#[derive(Debug, Clone, Copy)]
struct Nice {
    mantissa: f64,
    exponent: i32,
}

impl Nice {
    fn of(value: f64, round: bool) -> Option<Self> {
        if !(value.is_finite() && value > 0.0) {
            return None;
        }
        let exponent = quantity_exponent(value);
        let fraction = scale10(value, -exponent);
        if !fraction.is_finite() {
            return None;
        }
        let mantissa = if round {
            match fraction {
                f if f < 1.5 => 1.0,
                f if f < 2.5 => 2.0,
                f if f < 4.0 => 3.0,
                f if f < 7.0 => 5.0,
                _ => 10.0,
            }
        } else {
            match fraction {
                f if f < 1.0 => 1.0,
                f if f < 2.0 => 2.0,
                f if f < 3.0 => 3.0,
                f if f < 5.0 => 5.0,
                _ => 10.0,
            }
        };
        let nice = Self { mantissa, exponent };
        let value = nice.value();
        (value.is_finite() && value > 0.0).then_some(nice)
    }

    fn value(self) -> f64 {
        self.multiple(1.0)
    }

    fn multiple(self, k: f64) -> f64 {
        scale10(k * self.mantissa, self.exponent)
    }
}

/// ECharts `numberUtil.nice`: the nearest "nice" number (1, 2, 3, 5 or 10
/// times a power of ten) to `value`. `round` picks the nearest; otherwise
/// the next one up.
pub fn nice_number(value: f64, round: bool) -> f64 {
    Nice::of(value, round).map_or(value, Nice::value)
}

/// Rounds a value axis to nice ticks.
///
/// `data_min..=data_max` is the data extent (may be equal, may be
/// non-finite: then the axis is `0..=1`). `fixed_min` / `fixed_max` pin an
/// end exactly (ticks still fall on multiples of the step inside it, and
/// the pinned end is included as the first / last tick). Aims for about
/// `split_number` intervals. A zero-width extent is widened the way ECharts
/// does: by half its magnitude each way, or to `0..=1` at zero.
pub fn nice_linear(
    data_min: f64,
    data_max: f64,
    split_number: usize,
    fixed_min: Option<f64>,
    fixed_max: Option<f64>,
) -> LinearTicks {
    let split = if split_number == 0 { 5 } else { split_number } as f64;
    let mut fix_min = fixed_min.filter(|v| v.is_finite());
    let mut fix_max = fixed_max.filter(|v| v.is_finite());
    let mut lo = fix_min.unwrap_or(data_min);
    let mut hi = fix_max.unwrap_or(data_max);
    if !(hi - lo).is_finite() {
        (lo, hi, fix_min, fix_max) = (0.0, 1.0, None, None);
    }
    if lo > hi {
        std::mem::swap(&mut lo, &mut hi);
        std::mem::swap(&mut fix_min, &mut fix_max);
    }
    if lo == hi {
        if fix_min.is_some() && fix_max.is_some() {
            (fix_min, fix_max) = (None, None);
        }
        if lo == 0.0 {
            if fix_max.is_some() {
                lo = -1.0;
            } else {
                hi = 1.0;
            }
        } else {
            let half = lo.abs() / 2.0;
            if fix_min.is_none() {
                lo -= half;
            }
            if fix_max.is_none() {
                hi += half;
            }
        }
    }

    let Some(step) = Nice::of((hi - lo) / split, true) else {
        return LinearTicks {
            min: lo,
            max: hi,
            step: hi - lo,
            ticks: vec![lo, hi],
        };
    };
    let step_value = step.value();
    let (q_lo, q_hi) = (snap(lo / step_value), snap(hi / step_value));
    let k_start = if fix_min.is_some() {
        q_lo.ceil()
    } else {
        q_lo.floor()
    };
    let k_end = if fix_max.is_some() {
        q_hi.floor()
    } else {
        q_hi.ceil()
    };
    let min = fix_min.unwrap_or_else(|| step.multiple(k_start));
    let max = fix_max.unwrap_or_else(|| step.multiple(k_end));
    if k_end - k_start >= 1e6 {
        return LinearTicks {
            min,
            max,
            step: step_value,
            ticks: vec![min, max],
        };
    }

    // A multiple within float noise of a pinned end is that end.
    let tolerance = step_value * 1e-9;
    let mut ticks = Vec::with_capacity((k_end - k_start).max(0.0) as usize + 3);
    if fix_min.is_some() {
        ticks.push(min);
    }
    let mut k = k_start;
    while k <= k_end {
        let value = step.multiple(k);
        let inside = (fix_min.is_none() || value > min + tolerance)
            && (fix_max.is_none() || value < max - tolerance);
        if inside && ticks.last().is_none_or(|&last| value > last) {
            ticks.push(value);
        }
        k += 1.0;
    }
    if fix_max.is_some() && ticks.last().is_none_or(|&last| max > last) {
        ticks.push(max);
    }
    LinearTicks {
        min,
        max,
        step: step_value,
        ticks,
    }
}

/// Rounds a log axis. Returns ticks as **values** (powers of `base`), and
/// `min`/`max` as values too; `step` is in exponent units (an integer
/// number of decades, at least 1). Non-positive data is ignored; no
/// positive data gives `1..=base`.
pub fn nice_log(data_min: f64, data_max: f64, base: f64, split_number: usize) -> LinearTicks {
    let base = if base.is_finite() && base > 1.0 {
        base
    } else {
        10.0
    };
    let split = if split_number == 0 { 5 } else { split_number } as f64;
    let log = |v: f64| {
        snap(if base == 10.0 {
            v.log10()
        } else {
            v.ln() / base.ln()
        })
    };
    let pow = |e: f64| {
        let e = e as i32;
        if e >= 0 {
            base.powi(e)
        } else {
            1.0 / base.powi(-e)
        }
    };
    let positive = |v: f64| v.is_finite() && v > 0.0;

    let extent = match (positive(data_min), positive(data_max)) {
        (true, true) => Some((data_min.min(data_max), data_min.max(data_max))),
        // The extent runs down through zero: the smallest positive value is
        // unknown, so start at the first decade (or the lone value below it).
        (false, true) if data_min.is_finite() => Some((data_max.min(1.0), data_max)),
        (false, true) => Some((data_max, data_max)),
        (true, false) => Some((data_min, data_min)),
        (false, false) => None,
    };
    let (mut lo, mut hi) = match extent {
        Some((lo, hi)) => (log(lo), log(hi)),
        None => (0.0, 1.0),
    };
    if lo == hi {
        if lo == 0.0 {
            hi = 1.0;
        } else {
            let half = lo.abs() / 2.0;
            (lo, hi) = (lo - half, hi + half);
        }
    }

    // ECharts LogScale: a power-of-ten number of decades per tick.
    let span = hi - lo;
    let mut interval = scale10(1.0, quantity_exponent(span));
    if split / span * interval <= 0.5 {
        interval *= 10.0;
    }
    let interval = interval.max(1.0).round();
    let k_lo = snap(lo / interval).floor();
    let k_hi = snap(hi / interval).ceil();
    let ticks: Vec<f64> = (k_lo as i64..=k_hi as i64)
        .map(|k| pow(k as f64 * interval))
        .collect();
    LinearTicks {
        min: pow(k_lo * interval),
        max: pow(k_hi * interval),
        step: interval,
        ticks,
    }
}

/// One time-axis tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeTick {
    /// Unix milliseconds.
    pub value: i64,
    /// The step the ticks are on.
    pub unit: TimeUnit,
    /// The tick also starts the next larger unit (a new day on an hourly
    /// axis, a new year on a monthly one): labels show the larger unit.
    pub boundary: bool,
}

/// A candidate time step.
#[derive(Debug, Clone, Copy)]
enum TimeStep {
    /// A fixed duration, aligned to its multiples in local time.
    Fixed(i64, TimeUnit),
    /// Days of the month `1, 1 + n, …`, restarting each month.
    Days(i64),
    Months(i64),
    Years(i64),
}

impl TimeStep {
    fn nominal_ms(self) -> f64 {
        match self {
            Self::Fixed(ms, _) => ms as f64,
            Self::Days(n) => (n * MS_PER_DAY) as f64,
            Self::Months(n) => n as f64 * MS_PER_MONTH,
            Self::Years(n) => n as f64 * MS_PER_YEAR,
        }
    }
}

const TIME_STEPS: [TimeStep; 34] = {
    use TimeStep::{Days, Fixed, Months};
    use TimeUnit::{Hour, Millisecond, Minute, Second};
    [
        Fixed(1, Millisecond),
        Fixed(2, Millisecond),
        Fixed(5, Millisecond),
        Fixed(10, Millisecond),
        Fixed(20, Millisecond),
        Fixed(50, Millisecond),
        Fixed(100, Millisecond),
        Fixed(200, Millisecond),
        Fixed(500, Millisecond),
        Fixed(MS_PER_SECOND, Second),
        Fixed(2 * MS_PER_SECOND, Second),
        Fixed(5 * MS_PER_SECOND, Second),
        Fixed(10 * MS_PER_SECOND, Second),
        Fixed(15 * MS_PER_SECOND, Second),
        Fixed(30 * MS_PER_SECOND, Second),
        Fixed(MS_PER_MINUTE, Minute),
        Fixed(2 * MS_PER_MINUTE, Minute),
        Fixed(5 * MS_PER_MINUTE, Minute),
        Fixed(10 * MS_PER_MINUTE, Minute),
        Fixed(15 * MS_PER_MINUTE, Minute),
        Fixed(30 * MS_PER_MINUTE, Minute),
        Fixed(MS_PER_HOUR, Hour),
        Fixed(2 * MS_PER_HOUR, Hour),
        Fixed(3 * MS_PER_HOUR, Hour),
        Fixed(4 * MS_PER_HOUR, Hour),
        Fixed(6 * MS_PER_HOUR, Hour),
        Fixed(12 * MS_PER_HOUR, Hour),
        Fixed(MS_PER_DAY, TimeUnit::Day),
        Days(2),
        Days(7),
        Months(1),
        Months(2),
        Months(3),
        Months(6),
    ]
};

/// The smallest step at least `approx_ms` long.
fn pick_time_step(approx_ms: f64) -> TimeStep {
    if let Some(step) = TIME_STEPS.into_iter().find(|s| s.nominal_ms() >= approx_ms) {
        return step;
    }
    let years = approx_ms / MS_PER_YEAR;
    let mut decade = 1i64;
    loop {
        for n in [decade, 2 * decade, 5 * decade] {
            if n as f64 >= years {
                return TimeStep::Years(n);
            }
        }
        decade *= 10;
    }
}

/// Calendar ticks for `min..=max` Unix ms, about `approx_count` of them.
///
/// Steps are chosen from: 1/2/5/10/20/50/100/200/500 ms; 1/2/5/10/15/30 s;
/// 1/2/5/10/15/30 min; 1/2/3/4/6/12 h; 1/2/7 days; 1/2/3/6 months; 1/2/5/10/…
/// years. Days, months and years follow the calendar in the time zone
/// `utc_offset_minutes` east of UTC (so a month tick lands on the 1st at
/// local midnight). Returns ticks inside `min..=max`, ascending. `min ==
/// max` returns that single instant as a tick.
pub fn time_ticks(
    min: i64,
    max: i64,
    approx_count: usize,
    utc_offset_minutes: i32,
) -> Vec<TimeTick> {
    let (min, max) = (min.min(max), min.max(max));
    let offset = i64::from(utc_offset_minutes) * MS_PER_MINUTE;
    if min == max {
        return vec![single_time_tick(min, utc_offset_minutes)];
    }
    let approx_ms = (max as f64 - min as f64) / approx_count.max(1) as f64;
    let step = pick_time_step(approx_ms);
    let civil = |ms| civil_from_unix_ms(ms, utc_offset_minutes);
    let at =
        |year, month, day| unix_ms_from_civil(year, month, day, 0, 0, 0, 0, utc_offset_minutes);
    let month_index = |ms| {
        let (year, month, ..) = civil(ms);
        year * 12 + i64::from(month) - 1
    };

    let mut ticks = Vec::new();
    let mut push = |value: i64, unit: TimeUnit| {
        if (min..=max).contains(&value) {
            let boundary = is_boundary(value, unit, utc_offset_minutes);
            ticks.push(TimeTick {
                value,
                unit,
                boundary,
            });
        }
    };
    match step {
        TimeStep::Fixed(ms, unit) => {
            let local_max = max.saturating_add(offset);
            let mut local = min.saturating_add(offset).div_euclid(ms).saturating_mul(ms);
            while local <= local_max {
                push(local - offset, unit);
                local = match local.checked_add(ms) {
                    Some(next) => next,
                    None => break,
                };
            }
        }
        TimeStep::Days(n) => {
            for month in month_index(min)..=month_index(max) {
                let (year, month) = (month.div_euclid(12), month.rem_euclid(12) as u32 + 1);
                let days = days_in_month(year, month);
                // A tick closer than one step to the next 1st would crowd it.
                for day in (1..=days)
                    .step_by(n as usize)
                    .filter(|&d| days + 1 - d >= n as u32 || d == 1)
                {
                    push(at(year, month, day), TimeUnit::Day);
                }
            }
        }
        TimeStep::Months(n) => {
            let mut month = month_index(min).div_euclid(n) * n;
            while month <= month_index(max) {
                push(
                    at(month.div_euclid(12), month.rem_euclid(12) as u32 + 1, 1),
                    TimeUnit::Month,
                );
                month += n;
            }
        }
        TimeStep::Years(n) => {
            let (last, ..) = civil(max);
            let mut year = civil(min).0.div_euclid(n) * n;
            while year <= last {
                push(at(year, 1, 1), TimeUnit::Year);
                year = match year.checked_add(n) {
                    Some(next) => next,
                    None => break,
                };
            }
        }
    }
    ticks
}

/// The tick for a zero-width axis: on the coarsest unit the instant is
/// aligned to, so its label reads naturally.
fn single_time_tick(value: i64, utc_offset_minutes: i32) -> TimeTick {
    let (_, month, day, hour, minute, second, ms) = civil_from_unix_ms(value, utc_offset_minutes);
    let unit = match (month, day, hour, minute, second, ms) {
        (_, _, _, _, _, 1..) => TimeUnit::Millisecond,
        (_, _, _, _, 1.., _) => TimeUnit::Second,
        (_, _, _, 1.., _, _) => TimeUnit::Minute,
        (_, _, 1.., _, _, _) => TimeUnit::Hour,
        (_, 2.., _, _, _, _) => TimeUnit::Day,
        (2.., _, _, _, _, _) => TimeUnit::Month,
        _ => TimeUnit::Year,
    };
    TimeTick {
        value,
        unit,
        boundary: false,
    }
}

/// Whether a tick on `unit` also starts the next larger unit.
fn is_boundary(value: i64, unit: TimeUnit, utc_offset_minutes: i32) -> bool {
    let (_, month, day, hour, minute, second, ms) = civil_from_unix_ms(value, utc_offset_minutes);
    match unit {
        TimeUnit::Millisecond => ms == 0,
        TimeUnit::Second => second == 0 && ms == 0,
        TimeUnit::Minute | TimeUnit::Hour => hour == 0 && minute == 0 && second == 0 && ms == 0,
        TimeUnit::Day => day == 1,
        TimeUnit::Month => month == 1,
        TimeUnit::Year => false,
    }
}

/// The default label of a number on an axis whose ticks are `step` apart:
/// as many decimals as the step needs (at most 10; none for integer steps),
/// thousands grouped with commas, `-0` shown as `0`. Non-finite: `-`.
pub fn format_number(value: f64, step: f64) -> String {
    if !value.is_finite() {
        return "-".to_owned();
    }
    // A missing step leaves the value's own decimals.
    let step = if step.is_finite() && step != 0.0 {
        step.abs()
    } else {
        value.abs()
    };
    let decimals = (0..10)
        .find(|&d| {
            let scaled = scale10(step, d);
            scaled >= 0.5 && (scaled - scaled.round()).abs() <= 1e-6 * scaled
        })
        .unwrap_or(10) as usize;
    let digits = format!("{:.decimals$}", value.abs());
    let (int, frac) = digits
        .split_once('.')
        .map_or((digits.as_str(), None), |(i, f)| (i, Some(f)));
    let negative = value < 0.0 && digits.bytes().any(|b| b.is_ascii_digit() && b != b'0');

    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if negative {
        out.push('-');
    }
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if let Some(frac) = frac {
        out.push('.');
        out.push_str(frac);
    }
    out
}

/// The default label of a time tick, in the zone `utc_offset_minutes`
/// east of UTC:
///
/// | unit | label | boundary label |
/// | --- | --- | --- |
/// | Year | `2024` | — |
/// | Month | `03` | `2024` (January) |
/// | Day | `03-07` | `03-01` on the 1st of a month |
/// | Hour, Minute | `14:05` | `03-07` at midnight |
/// | Second | `14:05:09` | `14:05` on the minute |
/// | Millisecond | `09.250` | `14:05:09` on the second |
///
/// The boundary column applies when the tick's `boundary` is set.
pub fn format_time(tick: TimeTick, utc_offset_minutes: i32) -> String {
    let (year, month, day, hour, minute, second, ms) =
        civil_from_unix_ms(tick.value, utc_offset_minutes);
    match (tick.unit, tick.boundary) {
        (TimeUnit::Year, _) | (TimeUnit::Month, true) => format!("{year}"),
        (TimeUnit::Month, false) => format!("{month:02}"),
        (TimeUnit::Day, _) | (TimeUnit::Hour | TimeUnit::Minute, true) => {
            format!("{month:02}-{day:02}")
        }
        (TimeUnit::Hour | TimeUnit::Minute, false) | (TimeUnit::Second, true) => {
            format!("{hour:02}:{minute:02}")
        }
        (TimeUnit::Second, false) | (TimeUnit::Millisecond, true) => {
            format!("{hour:02}:{minute:02}:{second:02}")
        }
        (TimeUnit::Millisecond, false) => format!("{second:02}.{ms:03}"),
    }
}

fn is_leap_year(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Hinnant's
/// `days_from_civil`); `month` is 1..=12.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let month = i64::from(month);
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Inverse of [`days_from_civil`].
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Calendar fields of a Unix ms instant in the zone `utc_offset_minutes`
/// east of UTC: `(year, month 1..=12, day 1..=31, hour, minute, second,
/// millisecond)`. Proleptic Gregorian.
pub fn civil_from_unix_ms(ms: i64, utc_offset_minutes: i32) -> (i64, u32, u32, u32, u32, u32, u32) {
    let local = ms.saturating_add(i64::from(utc_offset_minutes) * MS_PER_MINUTE);
    let (year, month, day) = civil_from_days(local.div_euclid(MS_PER_DAY));
    let in_day = local.rem_euclid(MS_PER_DAY);
    (
        year,
        month,
        day,
        (in_day / MS_PER_HOUR) as u32,
        (in_day % MS_PER_HOUR / MS_PER_MINUTE) as u32,
        (in_day % MS_PER_MINUTE / MS_PER_SECOND) as u32,
        (in_day % MS_PER_SECOND) as u32,
    )
}

/// Inverse of [`civil_from_unix_ms`] for a local date and time.
#[allow(clippy::too_many_arguments)]
pub fn unix_ms_from_civil(
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    millisecond: u32,
    utc_offset_minutes: i32,
) -> i64 {
    // Out-of-range fields carry over (month 13 is next January, day 0 the
    // last day of the previous month).
    // Far outside the Unix ms range the result saturates.
    let months = i128::from(year) * 12 + i128::from(month) - 1;
    let year = months.div_euclid(12).clamp(-MAX_YEAR, MAX_YEAR) as i64;
    let days = i128::from(days_from_civil(year, months.rem_euclid(12) as u32 + 1, 1))
        + i128::from(day)
        - 1;
    let ms = days * i128::from(MS_PER_DAY)
        + i128::from(hour) * i128::from(MS_PER_HOUR)
        + i128::from(minute) * i128::from(MS_PER_MINUTE)
        + i128::from(second) * i128::from(MS_PER_SECOND)
        + i128::from(millisecond)
        - i128::from(utc_offset_minutes) * i128::from(MS_PER_MINUTE);
    ms.clamp(i64::MIN.into(), i64::MAX.into()) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(year: i64, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
        unix_ms_from_civil(year, month, day, hour, minute, 0, 0, 0)
    }

    fn values(ticks: &[TimeTick]) -> Vec<i64> {
        ticks.iter().map(|t| t.value).collect()
    }

    fn boundaries(ticks: &[TimeTick]) -> Vec<bool> {
        ticks.iter().map(|t| t.boundary).collect()
    }

    #[test]
    fn nice_number_matches_echarts() {
        assert_eq!(nice_number(0.2, true), 0.2);
        assert_eq!(nice_number(9.8, true), 10.0);
        assert_eq!(nice_number(3.6, true), 3.0);
        assert_eq!(nice_number(0.00072, true), 0.001);
        assert_eq!(nice_number(0.00062, true), 0.0005);
        assert_eq!(nice_number(1.7e8, true), 2e8);
        assert_eq!(nice_number(1.0, false), 2.0);
        assert_eq!(nice_number(4.2, false), 5.0);
        assert_eq!(nice_number(0.06, false), 0.1);
        assert_eq!(nice_number(1000.0, true), 1000.0);
        assert_eq!(nice_number(0.0, true), 0.0);
        assert!(nice_number(f64::NAN, true).is_nan());
    }

    #[test]
    fn linear_unit_interval() {
        let t = nice_linear(0.0, 1.0, 5, None, None);
        assert_eq!((t.min, t.max, t.step), (0.0, 1.0, 0.2));
        assert_eq!(t.ticks, vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0]);
    }

    #[test]
    fn linear_large_span() {
        let t = nice_linear(0.0, 1e9, 5, None, None);
        assert_eq!(t.step, 2e8);
        assert_eq!(t.ticks, vec![0.0, 2e8, 4e8, 6e8, 8e8, 1e9]);
    }

    #[test]
    fn linear_negative_span() {
        let t = nice_linear(-37.0, 12.0, 5, None, None);
        assert_eq!((t.min, t.max, t.step), (-40.0, 20.0, 10.0));
        assert_eq!(t.ticks, vec![-40.0, -30.0, -20.0, -10.0, 0.0, 10.0, 20.0]);
    }

    #[test]
    fn linear_single_value_widens() {
        let t = nice_linear(5.0, 5.0, 5, None, None);
        assert_eq!((t.min, t.max, t.step), (2.0, 8.0, 1.0));
        assert_eq!(t.ticks.len(), 7);

        let t = nice_linear(0.0, 0.0, 5, None, None);
        assert_eq!((t.min, t.max, t.step), (0.0, 1.0, 0.2));

        let t = nice_linear(-4.0, -4.0, 5, None, None);
        assert_eq!((t.min, t.max), (-6.0, -2.0));
    }

    #[test]
    fn linear_non_finite_is_unit() {
        for (lo, hi) in [
            (f64::NAN, 1.0),
            (f64::INFINITY, f64::NEG_INFINITY),
            (0.0, f64::INFINITY),
        ] {
            let t = nice_linear(lo, hi, 5, None, None);
            assert_eq!((t.min, t.max), (0.0, 1.0));
            assert_eq!(t.ticks, vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0]);
        }
        // A finite pinned end still holds with no data.
        let t = nice_linear(f64::NAN, f64::NAN, 5, Some(0.0), Some(50.0));
        assert_eq!(t.ticks, vec![0.0, 10.0, 20.0, 30.0, 40.0, 50.0]);
    }

    #[test]
    fn linear_fixed_ends_are_kept_exactly() {
        let t = nice_linear(4.0, 90.0, 5, Some(3.0), Some(97.0));
        assert_eq!((t.min, t.max, t.step), (3.0, 97.0, 20.0));
        assert_eq!(t.ticks, vec![3.0, 20.0, 40.0, 60.0, 80.0, 97.0]);

        let t = nice_linear(0.0, 95.0, 5, None, Some(95.0));
        assert_eq!(t.ticks, vec![0.0, 20.0, 40.0, 60.0, 80.0, 95.0]);

        // A pinned end on a multiple is not repeated.
        let t = nice_linear(3.0, 97.0, 5, Some(0.0), None);
        assert_eq!(t.ticks, vec![0.0, 20.0, 40.0, 60.0, 80.0, 100.0]);

        // Pinned on one side, a zero-width extent widens to the other.
        let t = nice_linear(10.0, 10.0, 5, Some(10.0), None);
        assert_eq!(t.min, 10.0);
        assert!(t.max >= 15.0);
        assert_eq!(t.ticks[0], 10.0);
    }

    #[test]
    fn linear_ticks_ascend_without_float_noise() {
        let t = nice_linear(0.1, 0.73, 7, None, None);
        assert!(t.ticks.windows(2).all(|w| w[0] < w[1]));
        for v in &t.ticks {
            assert!(format!("{v}").len() <= 4, "{v}");
        }
    }

    #[test]
    fn log_decades() {
        let t = nice_log(3.0, 45_000.0, 10.0, 5);
        assert_eq!((t.min, t.max, t.step), (1.0, 100_000.0, 1.0));
        assert_eq!(
            t.ticks,
            vec![1.0, 10.0, 100.0, 1_000.0, 10_000.0, 100_000.0]
        );

        let t = nice_log(0.002, 0.3, 10.0, 5);
        assert_eq!(t.ticks, vec![0.001, 0.01, 0.1, 1.0]);

        let t = nice_log(1e-10, 1e30, 10.0, 5);
        assert_eq!(t.step, 10.0);
        assert_eq!(t.ticks, vec![1e-10, 1.0, 1e10, 1e20, 1e30]);

        let t = nice_log(3.0, 100.0, 2.0, 5);
        assert_eq!(t.ticks, vec![2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0]);
    }

    #[test]
    fn log_without_positive_data() {
        for (lo, hi) in [(-5.0, 0.0), (f64::NAN, f64::NAN), (0.0, 0.0)] {
            let t = nice_log(lo, hi, 10.0, 5);
            assert_eq!((t.min, t.max, t.step), (1.0, 10.0, 1.0));
            assert_eq!(t.ticks, vec![1.0, 10.0]);
        }
        // Non-positive data is ignored; the axis starts at the first decade.
        let t = nice_log(0.0, 45_000.0, 10.0, 5);
        assert_eq!((t.min, t.max), (1.0, 100_000.0));
    }

    #[test]
    fn time_sub_second() {
        let start = utc(2024, 3, 7, 14, 5) + 9_000;
        let ticks = time_ticks(start, start + 1_000, 5, 0);
        assert_eq!(
            values(&ticks),
            (0..=5).map(|i| start + i * 200).collect::<Vec<_>>()
        );
        assert!(ticks.iter().all(|t| t.unit == TimeUnit::Millisecond));
        assert_eq!(boundaries(&ticks), [true, false, false, false, false, true]);
    }

    #[test]
    fn time_minutes() {
        let start = utc(2024, 3, 7, 14, 3);
        let ticks = time_ticks(start, start + 57 * MS_PER_MINUTE, 6, 0);
        let minutes: Vec<u32> = ticks
            .iter()
            .map(|t| civil_from_unix_ms(t.value, 0).4)
            .collect();
        assert_eq!(minutes, [10, 20, 30, 40, 50, 0]);
        assert!(
            ticks
                .iter()
                .all(|t| t.unit == TimeUnit::Minute && !t.boundary)
        );
    }

    #[test]
    fn time_hours_cross_midnight() {
        let ticks = time_ticks(utc(2024, 3, 7, 20, 0), utc(2024, 3, 8, 4, 0), 4, 0);
        let hours: Vec<u32> = ticks
            .iter()
            .map(|t| civil_from_unix_ms(t.value, 0).3)
            .collect();
        assert_eq!(hours, [20, 22, 0, 2, 4]);
        assert!(ticks.iter().all(|t| t.unit == TimeUnit::Hour));
        assert_eq!(boundaries(&ticks), [false, false, true, false, false]);
        assert_eq!(format_time(ticks[2], 0), "03-08");
        assert_eq!(format_time(ticks[1], 0), "22:00");
    }

    #[test]
    fn time_days_cross_month_end() {
        let ticks = time_ticks(utc(2024, 1, 28, 0, 0), utc(2024, 2, 4, 0, 0), 7, 0);
        let days: Vec<u32> = ticks
            .iter()
            .map(|t| civil_from_unix_ms(t.value, 0).2)
            .collect();
        assert_eq!(days, [28, 29, 30, 31, 1, 2, 3, 4]);
        assert!(ticks.iter().all(|t| t.unit == TimeUnit::Day));
        assert_eq!(
            boundaries(&ticks),
            [false, false, false, false, true, false, false, false]
        );

        // Weekly ticks restart on the 1st and skip a crowded 29th.
        let ticks = time_ticks(utc(2024, 1, 1, 0, 0), utc(2024, 2, 20, 0, 0), 8, 0);
        let days: Vec<(u32, u32)> = ticks
            .iter()
            .map(|t| {
                let c = civil_from_unix_ms(t.value, 0);
                (c.1, c.2)
            })
            .collect();
        assert_eq!(
            days,
            [(1, 1), (1, 8), (1, 15), (1, 22), (2, 1), (2, 8), (2, 15)]
        );
    }

    #[test]
    fn time_months_cross_year() {
        let ticks = time_ticks(utc(2023, 10, 1, 0, 0), utc(2024, 3, 1, 0, 0), 5, 0);
        let months: Vec<(i64, u32)> = ticks
            .iter()
            .map(|t| {
                let c = civil_from_unix_ms(t.value, 0);
                (c.0, c.1)
            })
            .collect();
        assert_eq!(
            months,
            [
                (2023, 10),
                (2023, 11),
                (2023, 12),
                (2024, 1),
                (2024, 2),
                (2024, 3)
            ]
        );
        assert!(ticks.iter().all(|t| t.unit == TimeUnit::Month));
        assert_eq!(
            boundaries(&ticks),
            [false, false, false, true, false, false]
        );
        assert_eq!(format_time(ticks[3], 0), "2024");
        assert_eq!(format_time(ticks[4], 0), "02");
    }

    #[test]
    fn time_years() {
        let ticks = time_ticks(utc(1999, 6, 1, 0, 0), utc(2024, 6, 1, 0, 0), 6, 0);
        let years: Vec<i64> = ticks
            .iter()
            .map(|t| civil_from_unix_ms(t.value, 0).0)
            .collect();
        assert_eq!(years, [2000, 2005, 2010, 2015, 2020]);
        assert!(
            ticks
                .iter()
                .all(|t| t.unit == TimeUnit::Year && !t.boundary)
        );

        let ticks = time_ticks(utc(-3000, 1, 1, 0, 0), utc(3000, 1, 1, 0, 0), 6, 0);
        let years: Vec<i64> = ticks
            .iter()
            .map(|t| civil_from_unix_ms(t.value, 0).0)
            .collect();
        assert_eq!(years, [-3000, -2000, -1000, 0, 1000, 2000, 3000]);
    }

    #[test]
    fn time_offsets_put_calendar_ticks_at_local_midnight() {
        for offset in [480, -300, 330] {
            let local = |y, m, d| unix_ms_from_civil(y, m, d, 0, 0, 0, 0, offset);
            let ticks = time_ticks(local(2024, 1, 29), local(2024, 2, 3), 5, offset);
            assert!(ticks.iter().all(|t| t.unit == TimeUnit::Day));
            for t in &ticks {
                let (_, _, _, hour, minute, ..) = civil_from_unix_ms(t.value, offset);
                assert_eq!((hour, minute), (0, 0));
            }
            assert_eq!(ticks.len(), 6);
            assert_eq!(
                boundaries(&ticks),
                [false, false, false, true, false, false]
            );
            assert_eq!(format_time(ticks[3], offset), "02-01");

            let ticks = time_ticks(local(2023, 11, 15), local(2024, 4, 15), 5, offset);
            assert_eq!(ticks[0].value, local(2023, 12, 1));
            assert!(
                ticks
                    .iter()
                    .any(|t| t.value == local(2024, 1, 1) && t.boundary)
            );
        }
        // Local midnight of 2024-03-01 at +08:00 is 16:00 UTC the day before.
        let ticks = time_ticks(utc(2024, 2, 28, 0, 0), utc(2024, 3, 2, 0, 0), 3, 480);
        assert!(
            ticks
                .iter()
                .any(|t| t.value == utc(2024, 2, 29, 16, 0) && t.boundary)
        );
        // At -05:00 it is 05:00 UTC.
        let ticks = time_ticks(utc(2024, 2, 28, 0, 0), utc(2024, 3, 2, 0, 0), 3, -300);
        assert!(
            ticks
                .iter()
                .any(|t| t.value == utc(2024, 3, 1, 5, 0) && t.boundary)
        );
    }

    #[test]
    fn time_single_instant() {
        let at = utc(2024, 3, 7, 14, 5) + 9_250;
        let ticks = time_ticks(at, at, 5, 0);
        assert_eq!(values(&ticks), [at]);
        assert_eq!(ticks[0].unit, TimeUnit::Millisecond);

        let ticks = time_ticks(utc(2024, 1, 1, 0, 0), utc(2024, 1, 1, 0, 0), 5, 0);
        assert_eq!(ticks[0].unit, TimeUnit::Year);
        assert_eq!(format_time(ticks[0], 0), "2024");
    }

    #[test]
    fn time_ticks_stay_inside_and_ascend() {
        let (min, max) = (utc(2024, 3, 7, 14, 3) + 123, utc(2024, 3, 9, 2, 0) + 77);
        for count in [1, 3, 10, 50, 500] {
            let ticks = time_ticks(min, max, count, 60);
            assert!(!ticks.is_empty());
            assert!(ticks.windows(2).all(|w| w[0].value < w[1].value));
            assert!(ticks.iter().all(|t| (min..=max).contains(&t.value)));
            assert!(ticks.len() <= count + 2, "{count}: {}", ticks.len());
        }
    }

    #[test]
    fn civil_round_trip() {
        assert_eq!(utc(1970, 1, 1, 0, 0), 0);
        assert_eq!(utc(2024, 1, 1, 0, 0), 1_704_067_200_000);
        assert_eq!(utc(2024, 2, 29, 0, 0), 1_709_164_800_000);
        assert_eq!(utc(1900, 1, 1, 0, 0), -2_208_988_800_000);
        assert_eq!(civil_from_unix_ms(-1, 0), (1969, 12, 31, 23, 59, 59, 999));
        assert_eq!(
            civil_from_unix_ms(951_782_400_000, 0),
            (2000, 2, 29, 0, 0, 0, 0)
        );
        assert_eq!(civil_from_unix_ms(0, 480), (1970, 1, 1, 8, 0, 0, 0));
        assert_eq!(civil_from_unix_ms(0, -300), (1969, 12, 31, 19, 0, 0, 0));
        // 1900 is not a leap year; 2000 is.
        assert_eq!(utc(1900, 3, 1, 0, 0) - utc(1900, 2, 28, 0, 0), MS_PER_DAY);
        assert_eq!(
            utc(2000, 3, 1, 0, 0) - utc(2000, 2, 28, 0, 0),
            2 * MS_PER_DAY
        );
        // Out-of-range fields carry over.
        assert_eq!(
            unix_ms_from_civil(2023, 13, 1, 0, 0, 0, 0, 0),
            utc(2024, 1, 1, 0, 0)
        );
        assert_eq!(
            unix_ms_from_civil(2024, 3, 0, 0, 0, 0, 0, 0),
            utc(2024, 2, 29, 0, 0)
        );

        let mut ms = -62_135_596_800_000 - 123_456_789; // just before year 1
        while ms < 4_102_444_800_000 {
            for offset in [0, 480, -300, 345] {
                let (y, mo, d, h, mi, s, milli) = civil_from_unix_ms(ms, offset);
                assert_eq!(unix_ms_from_civil(y, mo, d, h, mi, s, milli, offset), ms);
            }
            ms += 7_654_321_987;
        }
    }

    #[test]
    fn format_numbers() {
        assert_eq!(format_number(1_234_567.891, 0.01), "1,234,567.89");
        assert_eq!(format_number(1_234_567.0, 1_000.0), "1,234,567");
        assert_eq!(format_number(0.30000000000000004, 0.1), "0.3");
        assert_eq!(format_number(-1_234.5, 0.5), "-1,234.5");
        assert_eq!(format_number(0.25, 0.05), "0.25");
        assert_eq!(format_number(100.0, 25.0), "100");
        assert_eq!(format_number(999.0, 1.0), "999");
        assert_eq!(format_number(-0.0, 1.0), "0");
        assert_eq!(format_number(-0.00001, 0.1), "0.0");
        assert_eq!(format_number(1.0, 1e-12), "1.0000000000");
        assert_eq!(format_number(f64::NAN, 1.0), "-");
        assert_eq!(format_number(f64::INFINITY, 1.0), "-");
        assert_eq!(format_number(1e9, 2e8), "1,000,000,000");
    }

    #[test]
    fn format_time_labels() {
        let at = unix_ms_from_civil(2024, 3, 7, 14, 5, 9, 250, 480);
        let label = |value, unit, boundary| {
            format_time(
                TimeTick {
                    value,
                    unit,
                    boundary,
                },
                480,
            )
        };
        assert_eq!(label(at, TimeUnit::Year, false), "2024");
        assert_eq!(label(at, TimeUnit::Month, false), "03");
        assert_eq!(label(at, TimeUnit::Month, true), "2024");
        assert_eq!(label(at, TimeUnit::Day, false), "03-07");
        assert_eq!(label(at, TimeUnit::Day, true), "03-07");
        assert_eq!(label(at, TimeUnit::Hour, false), "14:05");
        assert_eq!(label(at, TimeUnit::Minute, false), "14:05");
        assert_eq!(label(at, TimeUnit::Hour, true), "03-07");
        assert_eq!(label(at, TimeUnit::Second, false), "14:05:09");
        assert_eq!(label(at, TimeUnit::Second, true), "14:05");
        assert_eq!(label(at, TimeUnit::Millisecond, false), "09.250");
        assert_eq!(label(at, TimeUnit::Millisecond, true), "14:05:09");
        // The same instant in UTC.
        let utc_label = format_time(
            TimeTick {
                value: at,
                unit: TimeUnit::Hour,
                boundary: false,
            },
            0,
        );
        assert_eq!(utc_label, "06:05");
    }
}
