//! Civil dates and the month grid a date picker lays out.
//!
//! Deliberately not a date-time library: no time of day, no zones, no parsing.
//! A picker needs to know which day sits in which cell of a month grid and
//! which days are selectable, and nothing else. Applications keep using their
//! own date type for storage and formatting.

/// A date on the proleptic Gregorian calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CivilDate {
    year: i32,
    month: u8,
    day: u8,
}

/// Day a week starts on, which shifts the whole grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WeekStart {
    #[default]
    Monday,
    Sunday,
}

/// Day of the week, Monday first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl Weekday {
    /// 0-based index within a week that starts on `start`.
    pub fn index_from(self, start: WeekStart) -> u32 {
        let monday_based = self as u32;
        match start {
            WeekStart::Monday => monday_based,
            WeekStart::Sunday => (monday_based + 1) % 7,
        }
    }
}

/// Days in `month` of `year`, honouring leap years.
pub fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

pub fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

impl CivilDate {
    /// Builds a date, rejecting a month or day the calendar does not have.
    pub fn new(year: i32, month: u8, day: u8) -> Option<Self> {
        if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
            return None;
        }
        Some(Self { year, month, day })
    }

    /// Builds a date, clamping the day into the month instead of failing.
    /// Moving a selected 31st into a 30-day month lands on the 30th.
    pub fn clamped(year: i32, month: u8, day: u8) -> Option<Self> {
        let month = month.clamp(1, 12);
        let last = days_in_month(year, month);
        (last > 0).then(|| Self {
            year,
            month,
            day: day.clamp(1, last),
        })
    }

    pub fn year(self) -> i32 {
        self.year
    }

    pub fn month(self) -> u8 {
        self.month
    }

    pub fn day(self) -> u8 {
        self.day
    }

    /// Days since 1970-01-01, negative before it.
    ///
    /// Uses the shift-the-year-to-March formulation, so leap days fall at the
    /// end of the internal year and no month-length table is needed.
    pub fn days_since_epoch(self) -> i64 {
        let year = i64::from(self.year) - i64::from(self.month <= 2);
        let era = if year >= 0 { year } else { year - 399 } / 400;
        let year_of_era = year - era * 400;
        let month = i64::from(self.month);
        let day = i64::from(self.day);
        let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }

    /// Inverse of [`Self::days_since_epoch`].
    pub fn from_days_since_epoch(days: i64) -> Self {
        let days = days + 719_468;
        let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
        let day_of_era = days - era * 146_097;
        let year_of_era =
            (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let year = year_of_era + era * 400;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let internal_month = (5 * day_of_year + 2) / 153;
        let day = (day_of_year - (153 * internal_month + 2) / 5 + 1) as u8;
        let month = (internal_month + if internal_month < 10 { 3 } else { -9 }) as u8;
        Self {
            year: (year + i64::from(month <= 2)) as i32,
            month,
            day,
        }
    }

    pub fn weekday(self) -> Weekday {
        // 1970-01-01 was a Thursday.
        let index = (self.days_since_epoch() + 3).rem_euclid(7);
        match index {
            0 => Weekday::Monday,
            1 => Weekday::Tuesday,
            2 => Weekday::Wednesday,
            3 => Weekday::Thursday,
            4 => Weekday::Friday,
            5 => Weekday::Saturday,
            _ => Weekday::Sunday,
        }
    }

    /// Same day in a month `months` away, clamping the day (Jan 31 + 1 month
    /// is Feb 28 or 29, not March 3).
    #[must_use]
    pub fn shift_months(self, months: i32) -> Self {
        let total = i64::from(self.year) * 12 + i64::from(self.month) - 1 + i64::from(months);
        let year = total.div_euclid(12) as i32;
        let month = (total.rem_euclid(12) + 1) as u8;
        Self::clamped(year, month, self.day).unwrap_or(self)
    }

    #[must_use]
    pub fn shift_days(self, days: i32) -> Self {
        Self::from_days_since_epoch(self.days_since_epoch() + i64::from(days))
    }
}

/// One cell of a month grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DayCell {
    pub date: CivilDate,
    /// Whether the date belongs to the month being shown, as opposed to the
    /// leading or trailing days that fill the first and last week.
    pub in_month: bool,
}

/// A month laid out as whole weeks.
///
/// Always six rows: a grid that changed height as the user paged months would
/// reflow everything below it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonthGrid {
    pub year: i32,
    pub month: u8,
    pub week_start: WeekStart,
    pub weeks: Vec<[DayCell; 7]>,
}

impl MonthGrid {
    pub const WEEKS: usize = 6;

    pub fn new(year: i32, month: u8, week_start: WeekStart) -> Option<Self> {
        let first = CivilDate::new(year, month, 1)?;
        let lead = first.weekday().index_from(week_start) as i32;
        let start = first.shift_days(-lead);
        let mut weeks = Vec::with_capacity(Self::WEEKS);
        for week in 0..Self::WEEKS {
            let mut row = [DayCell {
                date: start,
                in_month: false,
            }; 7];
            for (index, cell) in row.iter_mut().enumerate() {
                let date = start.shift_days((week * 7 + index) as i32);
                *cell = DayCell {
                    date,
                    in_month: date.year() == year && date.month() == month,
                };
            }
            weeks.push(row);
        }
        Some(Self {
            year,
            month,
            week_start,
            weeks,
        })
    }

    /// Every cell in reading order.
    pub fn cells(&self) -> impl Iterator<Item = &DayCell> {
        self.weeks.iter().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_round_trips_across_eras_and_leap_boundaries() {
        for (year, month, day) in [
            (1970, 1, 1),
            (1969, 12, 31),
            (2000, 2, 29),
            (1900, 2, 28),
            (2024, 2, 29),
            (2026, 9, 9),
            (1, 1, 1),
            (-1, 12, 31),
        ] {
            let date = CivilDate::new(year, month, day).expect("valid date");
            assert_eq!(
                CivilDate::from_days_since_epoch(date.days_since_epoch()),
                date,
                "{year}-{month}-{day} did not round trip"
            );
        }
    }

    #[test]
    fn leap_years_follow_the_gregorian_rule() {
        assert!(is_leap_year(2024));
        assert!(is_leap_year(2000));
        assert!(!is_leap_year(1900), "a century is not a leap year");
        assert!(!is_leap_year(2023));
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
        assert!(CivilDate::new(1900, 2, 29).is_none());
    }

    #[test]
    fn weekdays_match_known_dates() {
        let cases = [
            ((1970, 1, 1), Weekday::Thursday),
            ((2000, 1, 1), Weekday::Saturday),
            ((2026, 9, 9), Weekday::Wednesday),
        ];
        for ((year, month, day), expected) in cases {
            assert_eq!(
                CivilDate::new(year, month, day).unwrap().weekday(),
                expected,
                "{year}-{month}-{day}"
            );
        }
    }

    #[test]
    fn shifting_months_clamps_instead_of_overflowing_into_the_next_month() {
        let end_of_january = CivilDate::new(2023, 1, 31).unwrap();
        assert_eq!(
            end_of_january.shift_months(1),
            CivilDate::new(2023, 2, 28).unwrap()
        );
        assert_eq!(
            CivilDate::new(2024, 1, 31).unwrap().shift_months(1),
            CivilDate::new(2024, 2, 29).unwrap(),
            "a leap February keeps the 29th"
        );
        // Crossing a year boundary in both directions.
        assert_eq!(
            CivilDate::new(2023, 12, 15).unwrap().shift_months(1),
            CivilDate::new(2024, 1, 15).unwrap()
        );
        assert_eq!(
            CivilDate::new(2023, 1, 15).unwrap().shift_months(-1),
            CivilDate::new(2022, 12, 15).unwrap()
        );
    }

    #[test]
    fn a_month_grid_is_always_six_whole_weeks_starting_on_the_chosen_day() {
        // 2026-09-01 is a Tuesday.
        let grid = MonthGrid::new(2026, 9, WeekStart::Monday).unwrap();
        assert_eq!(grid.weeks.len(), MonthGrid::WEEKS);
        assert_eq!(grid.cells().count(), 42);
        assert_eq!(
            grid.weeks[0][0].date,
            CivilDate::new(2026, 8, 31).unwrap(),
            "a Monday-start grid leads with the previous Monday"
        );
        assert!(!grid.weeks[0][0].in_month);
        assert_eq!(grid.weeks[0][1].date, CivilDate::new(2026, 9, 1).unwrap());
        assert!(grid.weeks[0][1].in_month);

        let sunday = MonthGrid::new(2026, 9, WeekStart::Sunday).unwrap();
        assert_eq!(
            sunday.weeks[0][0].date,
            CivilDate::new(2026, 8, 30).unwrap(),
            "a Sunday-start grid leads one day earlier"
        );

        // Every in-month day appears exactly once, in order.
        let in_month = grid
            .cells()
            .filter(|cell| cell.in_month)
            .map(|cell| cell.date.day())
            .collect::<Vec<_>>();
        assert_eq!(in_month, (1..=30).collect::<Vec<_>>());
    }

    #[test]
    fn a_grid_row_is_contiguous_days() {
        let grid = MonthGrid::new(2024, 2, WeekStart::Monday).unwrap();
        for week in &grid.weeks {
            for pair in week.windows(2) {
                assert_eq!(
                    pair[1].date,
                    pair[0].date.shift_days(1),
                    "cells within a week must be consecutive"
                );
            }
        }
    }
}
