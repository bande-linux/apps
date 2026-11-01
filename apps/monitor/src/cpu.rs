use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CpuTicks {
    pub user: u64,
    pub system: u64,
    pub idle: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CpuShare {
    pub user: f32,
    pub system: f32,
    pub idle: f32,
}

impl CpuTicks {
    fn parse(fields: &[u64]) -> Self {
        let field = |index: usize| fields.get(index).copied().unwrap_or(0);
        let user = field(0) + field(1);
        let system = field(2) + field(5) + field(6);
        let idle = field(3) + field(4);
        Self {
            user,
            system,
            idle,
            total: user + system + idle + field(7),
        }
    }

    pub fn share_since(self, earlier: Self) -> CpuShare {
        let total = self.total.saturating_sub(earlier.total);
        if total == 0 {
            return CpuShare::default();
        }
        let fraction = |now: u64, then: u64| now.saturating_sub(then) as f32 / total as f32;
        CpuShare {
            user: fraction(self.user, earlier.user),
            system: fraction(self.system, earlier.system),
            idle: fraction(self.idle, earlier.idle),
        }
    }
}

pub fn read_cpu_ticks(root: &Path) -> io::Result<(CpuTicks, Vec<CpuTicks>)> {
    let stat = fs::read_to_string(root.join("proc/stat"))?;
    let mut total = None;
    let mut cores = Vec::new();
    for line in stat.lines() {
        let mut words = line.split_whitespace();
        let Some(label) = words.next().filter(|label| label.starts_with("cpu")) else {
            continue;
        };
        let fields = words
            .map(str::parse)
            .collect::<Result<Vec<u64>, _>>()
            .map_err(io::Error::other)?;
        let ticks = CpuTicks::parse(&fields);
        if label == "cpu" {
            total = Some(ticks);
        } else {
            cores.push(ticks);
        }
    }
    let total = total.ok_or_else(|| io::Error::other("no cpu line in /proc/stat"))?;
    Ok((total, cores))
}
