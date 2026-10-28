//! Live system health: CPU load + temperature, memory pressure, disk, battery,
//! network throughput. `Monitor::sample` is cheap enough to call every 2s.

use std::ffi::CString;
#[cfg(target_os = "macos")]
use std::process::Command;
use std::time::Instant;
use sysinfo::{Components, CpuRefreshKind, MemoryRefreshKind, Networks, RefreshKind, System};

/// CPU die temperature at or above which we flag the Mac as running hot.
pub const HOT_CELSIUS: f32 = 85.0;
pub const WARM_CELSIUS: f32 = 75.0;

#[derive(Debug, Clone, Default)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
    pub remaining: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub cpu_load: f32,
    pub cpu_temp: Option<f32>,
    pub mem_used: u64,
    pub mem_total: u64,
    /// 0–100; mirrors Activity Monitor's memory pressure.
    pub mem_pressure: Option<u8>,
    pub swap_used: u64,
    pub disk_total: u64,
    pub disk_free: u64,
    pub battery: Option<Battery>,
    pub net_up: u64,
    pub net_down: u64,
    pub model: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Good,
    Warn,
    Critical,
}

impl Snapshot {
    pub fn temp_level(&self) -> Level {
        match self.cpu_temp {
            Some(t) if t >= HOT_CELSIUS => Level::Critical,
            Some(t) if t >= WARM_CELSIUS => Level::Warn,
            _ => Level::Good,
        }
    }
    pub fn mem_level(&self) -> Level {
        match self.mem_pressure {
            Some(p) if p >= 80 => Level::Critical,
            Some(p) if p >= 50 => Level::Warn,
            _ => Level::Good,
        }
    }
    pub fn disk_level(&self) -> Level {
        let free = self.disk_free as f64 / self.disk_total.max(1) as f64;
        if free < 0.05 {
            Level::Critical
        } else if free < 0.15 {
            Level::Warn
        } else {
            Level::Good
        }
    }
    pub fn overall(&self) -> Level {
        self.temp_level().max(self.mem_level()).max(self.disk_level())
    }
}

/// Temperature moves slowly and each sensor read is an IOHID round trip
/// (~2.5 ms), so sensors refresh on a slower cadence than CPU/memory.
const TEMP_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

pub struct Monitor {
    sys: System,
    components: Components,
    /// Indices of CPU die sensors; the only ones we read after startup.
    cpu_sensors: Vec<usize>,
    last_temp: Option<(Instant, Option<f32>)>,
    networks: Networks,
    last_net: Instant,
    last_battery: Option<(Instant, Option<Battery>)>,
    model: String,
}

impl Default for Monitor {
    fn default() -> Self {
        Self::new()
    }
}

impl Monitor {
    pub fn new() -> Self {
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
                .with_memory(MemoryRefreshKind::everything()),
        );
        let components = Components::new_with_refreshed_list();
        let cpu_sensors = cpu_sensor_indices(&components);
        Self {
            sys,
            components,
            cpu_sensors,
            last_temp: None,
            networks: Networks::new_with_refreshed_list(),
            last_net: Instant::now(),
            last_battery: None,
            model: sysctl_string("hw.model").unwrap_or_else(|| if cfg!(target_os = "linux") { "Linux" } else { "Mac" }.into()),
        }
    }

    pub fn sample(&mut self) -> Snapshot {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        self.networks.refresh(true);

        let elapsed = self.last_net.elapsed().as_secs_f64().max(0.001);
        self.last_net = Instant::now();
        let (up, down) = self
            .networks
            .iter()
            .filter(|(name, _)| !name.starts_with("lo"))
            .fold((0, 0), |(u, d), (_, n)| (u + n.transmitted(), d + n.received()));

        let (disk_total, disk_free) = statfs("/").unwrap_or((0, 0));

        Snapshot {
            cpu_load: self.sys.global_cpu_usage(),
            cpu_temp: self.temperature(),
            mem_used: self.sys.used_memory(),
            mem_total: self.sys.total_memory(),
            mem_pressure: memory_pressure(),
            swap_used: self.sys.used_swap(),
            disk_total,
            disk_free,
            battery: self.battery(),
            net_up: (up as f64 / elapsed) as u64,
            net_down: (down as f64 / elapsed) as u64,
            model: self.model.clone(),
        }
    }

    fn temperature(&mut self) -> Option<f32> {
        if let Some((at, t)) = self.last_temp {
            if at.elapsed() < TEMP_EVERY {
                return t;
            }
        }
        let list = self.components.list_mut();
        for &i in &self.cpu_sensors {
            if let Some(c) = list.get_mut(i) {
                c.refresh();
            }
        }
        let t = self.cpu_sensors.iter().filter_map(|&i| list.get(i)?.temperature()).filter(|t| *t > 0.0 && *t < 130.0).reduce(f32::max);
        self.last_temp = Some((Instant::now(), t));
        t
    }

    /// `pmset` is a process spawn, so only refresh battery every 30s.
    fn battery(&mut self) -> Option<Battery> {
        if let Some((at, b)) = &self.last_battery {
            if at.elapsed().as_secs() < 30 {
                return b.clone();
            }
        }
        let b = read_battery();
        self.last_battery = Some((Instant::now(), b.clone()));
        b
    }
}

/// Apple Silicon exposes per-die sensors ("PMU tdie*", "pACC/eACC MTR Temp");
/// Intel exposes "CPU Proximity"/"TC0P". Falls back to every sensor if none
/// of those names exist on this model.
fn cpu_sensor_indices(components: &Components) -> Vec<usize> {
    let cpuish = |label: &str| {
        let l = label.to_ascii_lowercase();
        l.contains("tdie") || l.contains("cpu") || l.contains("acc mtr") || l.contains("soc")
    };
    let valid = |c: &sysinfo::Component| c.temperature().is_some_and(|t| t > 0.0 && t < 130.0);
    let list = components.list();
    let cpu: Vec<usize> = (0..list.len()).filter(|&i| cpuish(list[i].label()) && valid(&list[i])).collect();
    if cpu.is_empty() {
        (0..list.len()).filter(|&i| valid(&list[i])).collect()
    } else {
        cpu
    }
}

#[cfg(target_os = "macos")]
fn sysctl_u32(name: &str) -> Option<u32> {
    let cname = CString::new(name).ok()?;
    let mut value: u32 = 0;
    let mut len = std::mem::size_of::<u32>();
    let rc = unsafe {
        libc::sysctlbyname(cname.as_ptr(), &mut value as *mut _ as *mut _, &mut len, std::ptr::null_mut(), 0)
    };
    (rc == 0).then_some(value)
}

#[cfg(target_os = "macos")]
fn sysctl_string(name: &str) -> Option<String> {
    let cname = CString::new(name).ok()?;
    let mut buf = [0u8; 256];
    let mut len = buf.len();
    let rc = unsafe {
        libc::sysctlbyname(cname.as_ptr(), buf.as_mut_ptr() as *mut _, &mut len, std::ptr::null_mut(), 0)
    };
    (rc == 0).then(|| String::from_utf8_lossy(&buf[..len.saturating_sub(1)]).into_owned())
}

/// `kern.memorystatus_level` is the % of memory available to the system;
/// pressure is its complement.
#[cfg(target_os = "macos")]
fn memory_pressure() -> Option<u8> {
    sysctl_u32("kern.memorystatus_level").map(|free| 100u32.saturating_sub(free) as u8)
}

fn statfs(path: &str) -> Option<(u64, u64)> {
    let cpath = CString::new(path).ok()?;
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(cpath.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let bs = st.f_bsize as u64;
    Some((st.f_blocks * bs, st.f_bavail * bs))
}

#[cfg(target_os = "macos")]
fn read_battery() -> Option<Battery> {
    let out = Command::new("/usr/bin/pmset").args(["-g", "batt"]).output().ok()?;
    parse_pmset(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(target_os = "linux")]
fn sysctl_string(_: &str) -> Option<String> {
    std::fs::read_to_string("/sys/devices/virtual/dmi/id/product_name").ok()
        .map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

#[cfg(target_os = "linux")]
fn memory_pressure() -> Option<u8> {
    // Linux PSI reports the percentage of time tasks were stalled by memory.
    std::fs::read_to_string("/proc/pressure/memory").ok()
        .and_then(|s| s.lines().find(|line| line.starts_with("some "))
            .and_then(|line| line.split_whitespace().find_map(|v| v.strip_prefix("avg10=")))
            .and_then(|v| v.parse::<f32>().ok()))
        .map(|v| v.clamp(0.0, 100.0) as u8)
}

#[cfg(target_os = "linux")]
fn read_battery() -> Option<Battery> {
    let supplies = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in supplies.flatten() {
        let path = entry.path();
        let read = |name: &str| std::fs::read_to_string(path.join(name)).ok().map(|s| s.trim().to_owned());
        if read("type").as_deref() != Some("Battery") { continue; }
        let percent = read("capacity")?.parse::<u8>().ok()?.min(100);
        let charging = matches!(read("status").as_deref(), Some("Charging" | "Full"));
        return Some(Battery { percent, charging, remaining: None });
    }
    None
}

/// Parses e.g. ` -InternalBattery-0 (id=123)	86%; discharging; 4:11 remaining present: true`
#[cfg(any(target_os = "macos", test))]
fn parse_pmset(text: &str) -> Option<Battery> {
    let line = text.lines().find(|l| l.contains("InternalBattery"))?;
    let after = line.split('\t').nth(1)?;
    let mut parts = after.split(';').map(str::trim);
    let percent = parts.next()?.trim_end_matches('%').parse().ok()?;
    let state = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("");
    let remaining = rest
        .split_whitespace()
        .next()
        .filter(|t| t.contains(':') && *t != "0:00")
        .map(|t| {
            let (h, m) = t.split_once(':').unwrap_or(("0", "0"));
            format!("{h}h {m}m")
        });
    Some(Battery {
        percent,
        charging: state == "charging" || state == "charged" || state == "finishing charge",
        remaining,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pmset() {
        let b = parse_pmset("Now drawing from 'Battery Power'\n -InternalBattery-0 (id=1)\t86%; discharging; 4:11 remaining present: true\n").unwrap();
        assert_eq!(b.percent, 86);
        assert!(!b.charging);
        assert_eq!(b.remaining.as_deref(), Some("4h 11m"));
    }
}
