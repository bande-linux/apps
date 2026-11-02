use std::collections::VecDeque;

use bande_toolkit::ui::{Series, SeriesColor};

use crate::engine::Sample;
use crate::resources::size;

pub const HISTORY: usize = 60;
const ONE_CORE: f64 = 100.0;
const HEADROOM: f64 = 1.25;

pub fn percent(cpu: f64) -> String {
    match cpu < 10.0 {
        true => format!("{cpu:.1} %"),
        false => format!("{cpu:.0} %"),
    }
}

pub fn rate(bytes: f64) -> String {
    match bytes < 1.0 {
        true => "0 B/s".to_owned(),
        false => format!("{}/s", size(bytes as u64)),
    }
}

#[derive(Debug, Clone, Default)]
pub struct Usage {
    samples: VecDeque<Sample>,
}

pub struct Graphs {
    pub cpu: (Vec<Series>, String),
    pub memory: (Vec<Series>, String),
    pub network: (Vec<Series>, String),
    pub disk: (Vec<Series>, String),
}

fn scaled(values: &[f64], floor: f64) -> Vec<f32> {
    let peak = values.iter().copied().fold(floor, f64::max) * HEADROOM;
    values.iter().map(|value| (value / peak) as f32).collect()
}

fn pair(first: &[f64], second: &[f64], text: impl Fn(f64, f64) -> String) -> (Vec<Series>, String) {
    let peak = first.iter().chain(second).copied().fold(1.0, f64::max) * HEADROOM;
    let line = |values: &[f64]| {
        values
            .iter()
            .map(|value| (value / peak) as f32)
            .collect::<Vec<_>>()
    };
    let last = |values: &[f64]| values.last().copied().unwrap_or_default();
    (
        vec![
            Series::new(SeriesColor::Primary, line(first)),
            Series::new(SeriesColor::Secondary, line(second)),
        ],
        text(last(first), last(second)),
    )
}

impl Usage {
    pub fn push(&mut self, sample: Sample) {
        if self.samples.len() > HISTORY {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    pub fn latest(&self) -> Option<&Sample> {
        self.samples.back()
    }

    fn rates(&self, pick: fn(&Sample) -> u64) -> Vec<f64> {
        self.samples
            .iter()
            .zip(self.samples.iter().skip(1))
            .map(|(before, after)| {
                let seconds = after.at.duration_since(before.at).as_secs_f64().max(0.001);
                pick(after).saturating_sub(pick(before)) as f64 / seconds
            })
            .collect()
    }

    pub fn graphs(&self) -> Graphs {
        let cpu: Vec<f64> = self.samples.iter().map(|sample| sample.cpu).collect();
        let memory: Vec<f64> = self
            .samples
            .iter()
            .map(|sample| sample.memory as f64)
            .collect();
        let latest = self.latest().copied();
        Graphs {
            cpu: (
                vec![Series::new(SeriesColor::Primary, scaled(&cpu, ONE_CORE))],
                percent(latest.map_or(0.0, |sample| sample.cpu)),
            ),
            memory: (
                vec![Series::new(SeriesColor::Success, scaled(&memory, 1.0))],
                size(latest.map_or(0, |sample| sample.memory)),
            ),
            network: pair(
                &self.rates(|sample| sample.received),
                &self.rates(|sample| sample.sent),
                |down, up| format!("↓ {}  ↑ {}", rate(down), rate(up)),
            ),
            disk: pair(
                &self.rates(|sample| sample.read),
                &self.rates(|sample| sample.written),
                |read, written| format!("R {}  W {}", rate(read), rate(written)),
            ),
        }
    }
}
