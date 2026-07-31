use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;

use iced::widget::canvas;
use iced::{Color, Element, Length, Pixels, Point, Rectangle, Renderer, Size, Theme, mouse, touch};

use crate::theme::{ThemeTokens, ui_font};

const DAYS_PER_WEEK: usize = 7;

#[derive(Debug, Clone, PartialEq)]
pub struct CalendarHeatmapDatum<T = ()> {
    pub date: String,
    pub value: f32,
    pub data: Option<T>,
}

impl<T> CalendarHeatmapDatum<T> {
    pub fn new(date: impl Into<String>, value: f32) -> Self {
        Self {
            date: date.into(),
            value,
            data: None,
        }
    }

    pub fn data(mut self, data: T) -> Self {
        self.data = Some(data);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CalendarHeatmapCell<T = ()> {
    pub date: String,
    pub value: f32,
    pub data: Option<T>,
    pub level: u8,
    pub week_start: String,
    pub title: String,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CalendarHeatmapActiveCell<T = ()> {
    pub date: String,
    pub value: f32,
    pub data: Option<T>,
    pub level: u8,
    pub title: String,
    pub x: f32,
    pub y: f32,
}

impl<T: Clone> From<&CalendarHeatmapCell<T>> for CalendarHeatmapActiveCell<T> {
    fn from(cell: &CalendarHeatmapCell<T>) -> Self {
        Self {
            date: cell.date.clone(),
            value: cell.value,
            data: cell.data.clone(),
            level: cell.level,
            title: cell.title.clone(),
            x: cell.x,
            y: cell.y,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CalendarHeatmapMonthLabel {
    pub key: String,
    pub label: String,
    pub x: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CalendarHeatmapDayLabel {
    pub day: u8,
    pub label: String,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CalendarHeatmapModel<T = ()> {
    pub cells: Vec<CalendarHeatmapCell<T>>,
    pub month_labels: Vec<CalendarHeatmapMonthLabel>,
    pub day_labels: Vec<CalendarHeatmapDayLabel>,
    pub width: f32,
    pub height: f32,
    pub cell_size: f32,
    pub cell_gap: f32,
    pub cell_radius: f32,
    pub label_width: f32,
    pub month_label_height: f32,
}

impl<T: Clone> CalendarHeatmapModel<T> {
    pub fn cell_at(&self, point: Point) -> Option<CalendarHeatmapActiveCell<T>> {
        let relative_x = point.x - self.label_width;
        let relative_y = point.y - self.month_label_height;
        if relative_x < 0.0 || relative_y < 0.0 {
            return None;
        }
        let step = self.cell_size + self.cell_gap;
        let week = (relative_x / step).floor() as usize;
        let day = (relative_y / step).floor() as usize;
        if day >= DAYS_PER_WEEK {
            return None;
        }
        let local_x = relative_x - week as f32 * step;
        let local_y = relative_y - day as f32 * step;
        if local_x > self.cell_size || local_y > self.cell_size {
            return None;
        }
        self.cells.get(week * DAYS_PER_WEEK + day).map(Into::into)
    }

    fn cell_index_at(&self, point: Point) -> Option<usize> {
        let relative_x = point.x - self.label_width;
        let relative_y = point.y - self.month_label_height;
        if relative_x < 0.0 || relative_y < 0.0 {
            return None;
        }
        let step = self.cell_size + self.cell_gap;
        let week = (relative_x / step).floor() as usize;
        let day = (relative_y / step).floor() as usize;
        if day >= DAYS_PER_WEEK {
            return None;
        }
        let local_x = relative_x - week as f32 * step;
        let local_y = relative_y - day as f32 * step;
        (local_x <= self.cell_size && local_y <= self.cell_size)
            .then_some(week * DAYS_PER_WEEK + day)
            .filter(|index| *index < self.cells.len())
    }
}

pub type CalendarLevelResolver<T> = Rc<dyn Fn(&CalendarHeatmapDatum<T>, (f32, f32)) -> u8>;
pub type CalendarTitleFormatter<T> = Rc<dyn Fn(&CalendarHeatmapDatum<T>) -> String>;

pub enum CalendarLevelStrategy<T> {
    Relative {
        levels: u8,
    },
    Thresholds(Vec<f32>),
    Custom {
        levels: u8,
        resolve: CalendarLevelResolver<T>,
    },
}

impl<T> Default for CalendarLevelStrategy<T> {
    fn default() -> Self {
        Self::Relative { levels: 5 }
    }
}

pub struct CalendarHeatmapOptions<T> {
    pub cell_size: f32,
    pub cell_gap: f32,
    pub cell_radius: f32,
    pub label_width: f32,
    pub month_label_height: f32,
    pub week_starts_on: u8,
    pub weekday_labels: Vec<(u8, String)>,
    pub level_strategy: CalendarLevelStrategy<T>,
    pub month_formatter: Rc<dyn Fn(i32, u8) -> String>,
    pub title_formatter: CalendarTitleFormatter<T>,
}

impl<T> Default for CalendarHeatmapOptions<T> {
    fn default() -> Self {
        Self {
            cell_size: 11.0,
            cell_gap: 3.0,
            cell_radius: 2.0,
            label_width: 42.0,
            month_label_height: 14.0,
            week_starts_on: 0,
            weekday_labels: vec![
                (1, "周一".to_owned()),
                (3, "周三".to_owned()),
                (5, "周五".to_owned()),
            ],
            level_strategy: CalendarLevelStrategy::default(),
            month_formatter: Rc::new(|_year, month| format!("{month}月")),
            title_formatter: Rc::new(|datum| format!("{}: {}", datum.date, datum.value)),
        }
    }
}

impl<T> CalendarHeatmapOptions<T> {
    pub fn cell_metrics(mut self, size: f32, gap: f32, radius: f32) -> Self {
        self.cell_size = finite_positive(size, 11.0);
        self.cell_gap = finite_non_negative(gap, 3.0);
        self.cell_radius = finite_non_negative(radius, 2.0).min(self.cell_size / 2.0);
        self
    }

    pub fn week_starts_on(mut self, day: i32) -> Self {
        self.week_starts_on = day.rem_euclid(7) as u8;
        self
    }

    pub fn weekday_labels(
        mut self,
        labels: impl IntoIterator<Item = (u8, impl Into<String>)>,
    ) -> Self {
        self.weekday_labels = labels
            .into_iter()
            .map(|(day, label)| (day % 7, label.into()))
            .collect();
        self
    }

    pub fn level_strategy(mut self, strategy: CalendarLevelStrategy<T>) -> Self {
        self.level_strategy = strategy;
        self
    }

    pub fn month_formatter(mut self, formatter: impl Fn(i32, u8) -> String + 'static) -> Self {
        self.month_formatter = Rc::new(formatter);
        self
    }

    pub fn title_formatter(
        mut self,
        formatter: impl Fn(&CalendarHeatmapDatum<T>) -> String + 'static,
    ) -> Self {
        self.title_formatter = Rc::new(formatter);
        self
    }
}

pub fn build_calendar_heatmap_model<T: Clone>(
    data: &[CalendarHeatmapDatum<T>],
    mut options: CalendarHeatmapOptions<T>,
) -> CalendarHeatmapModel<T> {
    options.cell_size = finite_positive(options.cell_size, 11.0);
    options.cell_gap = finite_non_negative(options.cell_gap, 3.0);
    options.cell_radius =
        finite_non_negative(options.cell_radius, 2.0).min(options.cell_size / 2.0);
    options.label_width = finite_non_negative(options.label_width, 42.0);
    options.month_label_height = finite_non_negative(options.month_label_height, 14.0);

    let mut dated: Vec<_> = data
        .iter()
        .filter_map(|datum| parse_date(&datum.date).map(|day| (day, datum)))
        .collect();
    dated.sort_by_key(|(day, _)| *day);
    let Some((first, _)) = dated.first() else {
        return empty_model(&options);
    };
    let last = dated.last().map_or(*first, |(day, _)| *day);
    let by_day: BTreeMap<_, _> = dated.iter().map(|(day, datum)| (*day, *datum)).collect();
    let values: Vec<_> = dated
        .iter()
        .map(|(_, datum)| datum.value)
        .filter(|value| value.is_finite())
        .collect();
    let range = values
        .iter()
        .copied()
        .fold(None, |range, value| {
            Some(match range {
                None => (value, value),
                Some((min, max)) => (f32::min(min, value), f32::max(max, value)),
            })
        })
        .unwrap_or((0.0, 0.0));
    let start = start_of_week(*first, options.week_starts_on);
    let end = start_of_week(last, options.week_starts_on) + 6;
    let week_count = ((end - start + 1) / 7) as usize;
    let mut cells = Vec::with_capacity(week_count * DAYS_PER_WEEK);
    for week in 0..week_count {
        let week_start = start + week as i64 * 7;
        for offset in 0..DAYS_PER_WEEK {
            let day = week_start + offset as i64;
            let source = by_day.get(&day).copied();
            let (year, month, date) = civil_from_days(day);
            let date_key = format!("{year:04}-{month:02}-{date:02}");
            let fallback = CalendarHeatmapDatum::new(date_key.clone(), 0.0);
            let datum = source.unwrap_or(&fallback);
            cells.push(CalendarHeatmapCell {
                date: datum.date.clone(),
                value: datum.value,
                data: datum.data.clone(),
                level: resolve_level(datum, &options.level_strategy, range),
                week_start: date_key_from_days(week_start),
                title: (options.title_formatter)(datum),
                x: options.label_width + week as f32 * (options.cell_size + options.cell_gap),
                y: options.month_label_height
                    + offset as f32 * (options.cell_size + options.cell_gap),
            });
        }
    }
    let width = options.label_width
        + (week_count as f32 * (options.cell_size + options.cell_gap) - options.cell_gap).max(0.0)
        + 2.0;
    let height = options.month_label_height
        + DAYS_PER_WEEK as f32 * options.cell_size
        + (DAYS_PER_WEEK - 1) as f32 * options.cell_gap
        + 2.0;
    let month_labels = build_month_labels(&cells, *first, last, &options);
    let day_labels = build_day_labels(&options);
    CalendarHeatmapModel {
        cells,
        month_labels,
        day_labels,
        width,
        height,
        cell_size: options.cell_size,
        cell_gap: options.cell_gap,
        cell_radius: options.cell_radius,
        label_width: options.label_width,
        month_label_height: options.month_label_height,
    }
}

fn empty_model<T>(options: &CalendarHeatmapOptions<T>) -> CalendarHeatmapModel<T> {
    CalendarHeatmapModel {
        cells: Vec::new(),
        month_labels: Vec::new(),
        day_labels: build_day_labels(options),
        width: options.label_width + 2.0,
        height: options.month_label_height
            + DAYS_PER_WEEK as f32 * options.cell_size
            + (DAYS_PER_WEEK - 1) as f32 * options.cell_gap
            + 2.0,
        cell_size: options.cell_size,
        cell_gap: options.cell_gap,
        cell_radius: options.cell_radius,
        label_width: options.label_width,
        month_label_height: options.month_label_height,
    }
}

fn build_day_labels<T>(options: &CalendarHeatmapOptions<T>) -> Vec<CalendarHeatmapDayLabel> {
    options
        .weekday_labels
        .iter()
        .map(|(day, label)| {
            let offset = (i32::from(*day) - i32::from(options.week_starts_on)).rem_euclid(7) as f32;
            CalendarHeatmapDayLabel {
                day: *day,
                label: label.clone(),
                x: 0.0,
                y: options.month_label_height
                    + offset * (options.cell_size + options.cell_gap)
                    + options.cell_size
                    - 1.0,
            }
        })
        .collect()
}

fn build_month_labels<T>(
    cells: &[CalendarHeatmapCell<T>],
    start: i64,
    end: i64,
    options: &CalendarHeatmapOptions<T>,
) -> Vec<CalendarHeatmapMonthLabel> {
    let mut labels = Vec::new();
    let mut last_month = None;
    for (week, cells) in cells.chunks(DAYS_PER_WEEK).enumerate() {
        let month = cells
            .iter()
            .filter_map(|cell| parse_date(&cell.date))
            .filter(|day| *day >= start && *day <= end)
            .map(civil_from_days)
            .find(|(_, month, _)| Some(*month) != last_month);
        let Some((year, month, _)) = month else {
            continue;
        };
        last_month = Some(month);
        labels.push(CalendarHeatmapMonthLabel {
            key: cells
                .first()
                .map_or_else(|| week.to_string(), |cell| cell.week_start.clone()),
            label: (options.month_formatter)(year, month),
            x: options.label_width
                + week as f32 * (options.cell_size + options.cell_gap)
                + options.cell_size / 2.0,
        });
    }
    labels
}

fn resolve_level<T>(
    datum: &CalendarHeatmapDatum<T>,
    strategy: &CalendarLevelStrategy<T>,
    range: (f32, f32),
) -> u8 {
    match strategy {
        CalendarLevelStrategy::Thresholds(thresholds) => thresholds
            .iter()
            .filter(|threshold| datum.value >= **threshold)
            .count()
            .min(u8::MAX as usize) as u8,
        CalendarLevelStrategy::Custom { levels, resolve } => {
            resolve(datum, range).min((*levels).max(2).saturating_sub(1))
        }
        CalendarLevelStrategy::Relative { levels } => {
            let upper = (*levels).max(2).saturating_sub(1);
            if datum.value <= 0.0 || range.1 <= 0.0 {
                0
            } else {
                (((datum.value / range.1) * f32::from(upper)).ceil() as u8).clamp(1, upper)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CalendarHeatmapEvent<T = ()> {
    CellEnter(CalendarHeatmapActiveCell<T>),
    CellMove(CalendarHeatmapActiveCell<T>),
    CellLeave,
}

pub struct CalendarHeatmap<'a, T, Message> {
    model: &'a CalendarHeatmapModel<T>,
    on_event: Rc<dyn Fn(CalendarHeatmapEvent<T>) -> Message + 'a>,
    tokens: ThemeTokens,
}

impl<'a, T, Message> CalendarHeatmap<'a, T, Message>
where
    T: Clone + 'a,
    Message: 'a,
{
    pub fn new(
        model: &'a CalendarHeatmapModel<T>,
        on_event: impl Fn(CalendarHeatmapEvent<T>) -> Message + 'a,
        theme: impl Into<ThemeTokens>,
    ) -> Self {
        Self {
            model,
            on_event: Rc::new(on_event),
            tokens: theme.into(),
        }
    }

    pub fn view(self) -> Element<'a, Message> {
        let width = self.model.width;
        let height = self.model.height;
        canvas(self)
            .width(Length::Fixed(width))
            .height(Length::Fixed(height))
            .into()
    }
}

pub struct CalendarHeatmapState {
    active: Option<usize>,
    fingerprint: Cell<u64>,
    cache: canvas::Cache,
}

impl Default for CalendarHeatmapState {
    fn default() -> Self {
        Self {
            active: None,
            fingerprint: Cell::new(0),
            cache: canvas::Cache::new(),
        }
    }
}

impl<T, Message> canvas::Program<Message> for CalendarHeatmap<'_, T, Message>
where
    T: Clone,
{
    type State = CalendarHeatmapState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let point = match event {
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => cursor.position_in(bounds),
            canvas::Event::Mouse(mouse::Event::CursorLeft) => None,
            canvas::Event::Touch(touch::Event::FingerMoved { position, .. })
            | canvas::Event::Touch(touch::Event::FingerPressed { position, .. }) => {
                Some(Point::new(position.x - bounds.x, position.y - bounds.y))
            }
            canvas::Event::Touch(
                touch::Event::FingerLifted { .. } | touch::Event::FingerLost { .. },
            ) => None,
            _ => return None,
        };
        let next = point.and_then(|point| self.model.cell_index_at(point));
        let event = match (state.active, next) {
            (Some(previous), Some(next)) if previous == next => self
                .model
                .cells
                .get(next)
                .map(|cell| CalendarHeatmapEvent::CellMove(cell.into())),
            (_, Some(next)) => self
                .model
                .cells
                .get(next)
                .map(|cell| CalendarHeatmapEvent::CellEnter(cell.into())),
            (Some(_), None) => Some(CalendarHeatmapEvent::CellLeave),
            (None, None) => None,
        };
        state.active = next;
        event.map(|event| canvas::Action::publish((self.on_event)(event)))
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let fingerprint = model_fingerprint(self.model, self.tokens);
        if state.fingerprint.replace(fingerprint) != fingerprint {
            state.cache.clear();
        }
        let static_layer = state.cache.draw(renderer, bounds.size(), |frame| {
            draw_static_heatmap(frame, self.model, self.tokens);
        });
        let mut active_layer = canvas::Frame::new(renderer, bounds.size());
        if let Some(cell) = state.active.and_then(|index| self.model.cells.get(index)) {
            draw_active_cell(&mut active_layer, cell, self.model, self.tokens);
        }
        vec![static_layer, active_layer.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if cursor.is_over(bounds) {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::None
        }
    }
}

fn draw_static_heatmap<T>(
    frame: &mut canvas::Frame,
    model: &CalendarHeatmapModel<T>,
    tokens: ThemeTokens,
) {
    let colors = tokens.colors;
    for label in &model.month_labels {
        frame.fill_text(canvas::Text {
            content: label.label.clone(),
            position: Point::new(label.x, 10.0),
            color: colors.muted,
            size: Pixels(10.0),
            font: ui_font(iced::font::Weight::Normal),
            align_x: iced::alignment::Horizontal::Center.into(),
            ..canvas::Text::default()
        });
    }
    for label in &model.day_labels {
        frame.fill_text(canvas::Text {
            content: label.label.clone(),
            position: Point::new(label.x, label.y),
            color: colors.muted,
            size: Pixels(11.0),
            font: ui_font(iced::font::Weight::Normal),
            ..canvas::Text::default()
        });
    }
    let max_level = model
        .cells
        .iter()
        .map(|cell| cell.level)
        .max()
        .unwrap_or(4)
        .max(1);
    for cell in &model.cells {
        let amount = f32::from(cell.level) / f32::from(max_level);
        let fill = if cell.level == 0 {
            colors.subtle
        } else {
            mix_color(colors.accent, colors.subtle, 0.25 + amount * 0.75)
        };
        let path = canvas::Path::rounded_rectangle(
            Point::new(cell.x, cell.y),
            Size::new(model.cell_size, model.cell_size),
            model.cell_radius.into(),
        );
        frame.fill(&path, fill);
        frame.stroke(
            &path,
            canvas::Stroke::default()
                .with_color(with_alpha(colors.background, 0.2))
                .with_width(1.0),
        );
    }
}

fn draw_active_cell<T>(
    frame: &mut canvas::Frame,
    cell: &CalendarHeatmapCell<T>,
    model: &CalendarHeatmapModel<T>,
    tokens: ThemeTokens,
) {
    let colors = tokens.colors;
    let highlight = canvas::Path::rounded_rectangle(
        Point::new(cell.x - 1.0, cell.y - 1.0),
        Size::new(model.cell_size + 2.0, model.cell_size + 2.0),
        (model.cell_radius + 1.0).into(),
    );
    frame.stroke(
        &highlight,
        canvas::Stroke::default()
            .with_color(colors.text)
            .with_width(1.5),
    );

    let tooltip_size = Size::new(176.0, 24.0);
    let tooltip_x = if cell.x > model.width / 2.0 {
        (cell.x + model.cell_size - tooltip_size.width).max(0.0)
    } else {
        cell.x.min((model.width - tooltip_size.width).max(0.0))
    };
    let tooltip_y = if cell.y < model.height / 2.0 {
        (cell.y + model.cell_size + 6.0).min(model.height - tooltip_size.height)
    } else {
        (cell.y - tooltip_size.height - 6.0).max(0.0)
    };
    let tooltip = canvas::Path::rounded_rectangle(
        Point::new(tooltip_x, tooltip_y),
        tooltip_size,
        tokens.metrics.radius_sm.into(),
    );
    frame.fill(&tooltip, colors.surface);
    frame.stroke(
        &tooltip,
        canvas::Stroke::default()
            .with_color(colors.border_soft)
            .with_width(1.0),
    );
    frame.fill_text(canvas::Text {
        content: cell.title.clone(),
        position: Point::new(tooltip_x + 7.0, tooltip_y + 16.0),
        color: colors.text,
        size: Pixels(11.0),
        font: ui_font(iced::font::Weight::Normal),
        ..canvas::Text::default()
    });
}

fn model_fingerprint<T>(model: &CalendarHeatmapModel<T>, tokens: ThemeTokens) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for value in [
        model.cells.len() as u64,
        u64::from(model.width.to_bits()),
        u64::from(model.height.to_bits()),
        u64::from(tokens.colors.accent.r.to_bits()),
        u64::from(tokens.colors.accent.g.to_bits()),
        u64::from(tokens.colors.accent.b.to_bits()),
    ] {
        hash ^= value;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    for cell in &model.cells {
        hash ^= u64::from(cell.level);
        hash = hash.wrapping_mul(0x100000001b3);
        hash = hash_bytes(hash, cell.date.as_bytes());
    }
    for label in &model.month_labels {
        hash = hash_bytes(hash, label.label.as_bytes());
    }
    for label in &model.day_labels {
        hash = hash_bytes(hash, label.label.as_bytes());
    }
    hash
}

fn hash_bytes(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn mix_color(foreground: Color, background: Color, ratio: f32) -> Color {
    let ratio = ratio.clamp(0.0, 1.0);
    Color {
        r: foreground.r * ratio + background.r * (1.0 - ratio),
        g: foreground.g * ratio + background.g * (1.0 - ratio),
        b: foreground.b * ratio + background.b * (1.0 - ratio),
        a: foreground.a * ratio + background.a * (1.0 - ratio),
    }
}

fn with_alpha(color: Color, alpha: f32) -> Color {
    Color {
        a: color.a * alpha,
        ..color
    }
}

fn finite_positive(value: f32, fallback: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

fn finite_non_negative(value: f32, fallback: f32) -> f32 {
    if value.is_finite() && value >= 0.0 {
        value
    } else {
        fallback
    }
}

fn parse_date(value: &str) -> Option<i64> {
    let mut parts = value.split('-');
    let year = parts.next()?.parse().ok()?;
    let month: u8 = parts.next()?.parse().ok()?;
    let day: u8 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) {
        return None;
    }
    let max_day = days_in_month(year, month);
    if day == 0 || day > max_day {
        return None;
    }
    Some(days_from_civil(year, month, day))
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 31,
    }
}

fn start_of_week(day: i64, week_starts_on: u8) -> i64 {
    let weekday = (day + 4).rem_euclid(7);
    day - (weekday - i64::from(week_starts_on)).rem_euclid(7)
}

fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let year = i64::from(year) - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(day: i64) -> (i32, u8, u8) {
    let day = day + 719_468;
    let era = day.div_euclid(146_097);
    let day_of_era = day - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year as i32, month as u8, day as u8)
}

fn date_key_from_days(day: i64) -> String {
    let (year, month, day) = civil_from_days(day);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::{
        CalendarHeatmapDatum, CalendarHeatmapOptions, build_calendar_heatmap_model,
        civil_from_days, days_from_civil,
    };
    use iced::Point;

    #[test]
    fn model_builds_complete_weeks_and_hits_only_cell_paint_bounds() {
        let model = build_calendar_heatmap_model(
            &[
                CalendarHeatmapDatum::<()>::new("2026-06-01", 2.0),
                CalendarHeatmapDatum::<()>::new("2026-06-03", 8.0),
            ],
            CalendarHeatmapOptions::default(),
        );
        assert_eq!(model.cells.len(), 7);
        let cell = model
            .cells
            .iter()
            .find(|cell| cell.date == "2026-06-03")
            .expect("June 3 cell");
        assert_eq!(cell.level, 4);
        assert_eq!(
            model
                .cell_at(Point::new(cell.x + 1.0, cell.y + 1.0))
                .expect("painted cell")
                .date,
            "2026-06-03"
        );
        assert!(
            model
                .cell_at(Point::new(cell.x + model.cell_size + 1.0, cell.y + 1.0))
                .is_none()
        );
    }

    #[test]
    fn date_conversion_round_trips_leap_days() {
        let days = days_from_civil(2024, 2, 29);
        assert_eq!(civil_from_days(days), (2024, 2, 29));
    }

    #[test]
    fn empty_model_preserves_weekday_axis_contract() {
        let model = build_calendar_heatmap_model::<()>(&[], CalendarHeatmapOptions::default());
        assert_eq!(model.day_labels.len(), 3);
        assert!(model.cells.is_empty());
    }
}
