//! A process sample and the groups derived from it. Reused until refreshed.
use macscrap_core::background::ProcessInfo;
use std::collections::BTreeMap;

pub struct ProcessGroup {
    pub key: String,
    pub name: String,
    pub cpu: f32,
    pub memory: u64,
    pub members: Vec<usize>,
}

pub struct ProcessSnapshot {
    pub processes: Vec<ProcessInfo>,
    pub groups: Vec<ProcessGroup>,
}

impl ProcessSnapshot {
    pub fn new(processes: Vec<ProcessInfo>) -> Self {
        let mut by_key = BTreeMap::<String, Vec<usize>>::new();
        for (index, process) in processes.iter().enumerate() {
            by_key.entry(group_key(process)).or_default().push(index);
        }
        let mut groups: Vec<_> = by_key.into_iter().map(|(key, members)| {
            let first = &processes[members[0]];
            let name = first.app_name.as_ref().filter(|n| !n.is_empty()).cloned()
                .or_else(|| (!first.name.is_empty()).then(|| first.name.clone()))
                .unwrap_or_else(|| "Other processes".into());
            let cpu = members.iter().map(|&i| processes[i].cpu).sum();
            let memory = members.iter().map(|&i| processes[i].memory).sum();
            ProcessGroup { key, name, cpu, memory, members }
        }).collect();
        groups.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
        Self { processes, groups }
    }
}

fn group_key(process: &ProcessInfo) -> String {
    if let Some(name) = process.app_name.as_ref().map(|n| n.trim()).filter(|n| !n.is_empty()) {
        return format!("name:{}", name.to_ascii_lowercase());
    }
    if let Some(exe) = process.executable.as_ref().and_then(|p| p.file_name()).and_then(|n| n.to_str()) {
        return format!("exe:{}", exe.to_ascii_lowercase());
    }
    let name = process.name.trim();
    if !name.is_empty() { return format!("proc:{}", name.to_ascii_lowercase()); }
    "other-processes".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn process(pid: u32, name: &str, cpu: f32, memory: u64) -> ProcessInfo {
        ProcessInfo { pid, name: name.into(), cpu, memory, app_bundle: None, app_name: Some(name.into()), executable: None, started: 0, can_quit: false }
    }
    #[test]
    fn groups_are_built_and_sorted_for_each_sample() {
        let snapshot = ProcessSnapshot::new(vec![process(1, "Browser", 5., 10), process(2, "Browser", 15., 20), process(3, "Editor", 30., 40)]);
        assert_eq!(snapshot.groups.len(), 2);
        assert_eq!(snapshot.groups[0].name, "Editor");
        assert_eq!(snapshot.groups[1].members, vec![0, 1]);
        assert_eq!(snapshot.groups[1].cpu, 20.);
        assert_eq!(snapshot.groups[1].memory, 30);
    }
}
