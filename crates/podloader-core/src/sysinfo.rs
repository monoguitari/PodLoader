use std::fs;
use std::path::Path;

pub fn normalize_firewire_guid(raw: &str) -> String {
    raw.trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X")
        .to_uppercase()
}

pub fn parse_sysinfo(sysinfo_path: &Path) -> (Option<String>, Option<String>) {
    let mut guid = None;
    let mut model = None;

    if let Ok(content) = fs::read_to_string(sysinfo_path) {
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some((k, v)) = trimmed.split_once(':') {
                let key_lower = k.trim().to_ascii_lowercase();
                if key_lower == "firewireguid" || key_lower == "firewireid" {
                    let val = normalize_firewire_guid(v);
                    if !val.is_empty() {
                        guid = Some(val);
                    }
                } else if key_lower == "modelnumstr" {
                    let val = v.trim().to_string();
                    if !val.is_empty() {
                        model = Some(val);
                    }
                }
            }
        }
    }

    if guid.is_none() {
        if let Some(device_dir) = sysinfo_path.parent() {
            // Check SysInfoExtended XML plist (<key>FireWireGUID</key><string>...</string>)
            let ext_path = device_dir.join("SysInfoExtended");
            if let Ok(xml) = fs::read_to_string(&ext_path) {
                if let Some(found) = extract_plist_string_value(&xml, "FireWireGUID") {
                    let normalized = normalize_firewire_guid(&found);
                    if normalized.len() == 16 && normalized.chars().all(|c| c.is_ascii_hexdigit()) {
                        guid = Some(normalized);
                    }
                }
            }
            // Check HashInfo binary file (bytes 6..22 hold the 16-byte ASCII hex FireWire GUID)
            if guid.is_none() {
                let hash_info_path = device_dir.join("HashInfo");
                if let Ok(bytes) = fs::read(&hash_info_path) {
                    if bytes.len() >= 22 {
                        if let Ok(hex_str) = std::str::from_utf8(&bytes[6..22]) {
                            let normalized = normalize_firewire_guid(hex_str);
                            if normalized.len() == 16
                                && normalized.chars().all(|c| c.is_ascii_hexdigit())
                            {
                                guid = Some(normalized);
                            }
                        }
                    }
                }
            }
        }
    }

    (guid, model)
}

fn extract_plist_string_value(xml: &str, key: &str) -> Option<String> {
    let needle = format!("<key>{}</key>", key);
    let pos = xml.find(&needle)?;
    let rest = &xml[pos + needle.len()..];
    let s_start = rest.find("<string>")? + "<string>".len();
    let s_end = rest[s_start..].find("</string>")?;
    Some(rest[s_start..s_start + s_end].trim().to_string())
}

pub fn cache_guid_in_sysinfo(sysinfo_path: &Path, guid: &str) -> std::io::Result<()> {
    if let Some(parent) = sysinfo_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let normalized = normalize_firewire_guid(guid);
    let existing = fs::read_to_string(sysinfo_path).unwrap_or_default();
    let mut lines: Vec<String> = existing
        .lines()
        .filter(|l| {
            let lower = l.trim().to_ascii_lowercase();
            !lower.starts_with("firewireguid:") && !lower.starts_with("firewireid:")
        })
        .map(|l| l.to_string())
        .collect();
    lines.push(format!("FirewireGuid: 0x{}", normalized));
    fs::write(sysinfo_path, format!("{}\n", lines.join("\n")))
}
