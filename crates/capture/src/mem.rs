//! Memory guard for captures on a busy shared server (D-027).

/// `MemAvailable` from /proc/meminfo in MB.
pub fn available_mb() -> Option<u64> {
    parse_available(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

fn parse_available(meminfo: &str) -> Option<u64> {
    let line = meminfo.lines().find(|l| l.starts_with("MemAvailable:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

#[derive(Clone, Copy)]
pub struct MemGuard {
    /// Stop the capture when available memory falls below this.
    pub abort_below_mb: u64,
    pub probe: fn() -> Option<u64>,
}

impl MemGuard {
    pub fn new(abort_below_mb: u64) -> Self {
        Self {
            abort_below_mb,
            probe: available_mb,
        }
    }

    /// `Some(available)` when below the limit. An unreadable probe never blocks.
    pub fn short(&self) -> Option<u64> {
        (self.probe)().filter(|mb| *mb < self.abort_below_mb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_meminfo() {
        let text = "MemTotal:       24604672 kB\nMemFree:         1605632 kB\nMemAvailable:    3750912 kB\n";
        assert_eq!(parse_available(text), Some(3663));
        assert_eq!(parse_available("MemTotal: 1 kB\n"), None);
    }

    #[test]
    fn guard_threshold() {
        let low = MemGuard {
            abort_below_mb: 1000,
            probe: || Some(500),
        };
        assert_eq!(low.short(), Some(500));
        let ok = MemGuard {
            abort_below_mb: 1000,
            probe: || Some(5000),
        };
        assert_eq!(ok.short(), None);
        let unknown = MemGuard {
            abort_below_mb: 1000,
            probe: || None,
        };
        assert_eq!(unknown.short(), None);
    }
}
