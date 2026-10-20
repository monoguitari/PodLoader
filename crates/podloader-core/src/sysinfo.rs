use std::fs;
use std::path::Path;

pub fn normalize_firewire_guid(raw: &str) -> String {
    raw.trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X")
        .to_string()
}

pub fn parse_sysinfo(sysinfo_path: &Path) -> (Option<String>, Option<String>) {
    let Ok(content) = fs::read_to_string(sysinfo_path) else {
        return (None, None);
    };
    let mut guid = None;
    let mut model = None;
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("FireWireGuid:") {
            let val = normalize_firewire_guid(rest);
            if !val.is_empty() {
                guid = Some(val);
            }
        } else if let Some(rest) = trimmed.strip_prefix("ModelNumStr:") {
            let val = rest.trim().to_string();
            if !val.is_empty() {
                model = Some(val);
            }
        }
    }
    (guid, model)
}

pub fn cache_guid_in_sysinfo(sysinfo_path: &Path, guid: &str) -> std::io::Result<()> {
    if let Some(parent) = sysinfo_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let normalized = normalize_firewire_guid(guid);
    let existing = fs::read_to_string(sysinfo_path).unwrap_or_default();
    let mut lines: Vec<String> = existing
        .lines()
        .filter(|l| !l.trim().starts_with("FireWireGuid:"))
        .map(|l| l.to_string())
        .collect();
    lines.push(format!("FireWireGuid: 0x{}", normalized));
    fs::write(sysinfo_path, format!("{}\n", lines.join("\n")))
}
