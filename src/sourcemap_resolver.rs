use regex::Regex;
use serde::Deserialize;
use serde_json::Value;
use sourcemap::{DecodedMap, Token, decode_slice};
use std::collections::HashMap;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Component, Path, PathBuf};

use crate::color::Color;

const MAX_SOURCEMAP_BYTES: u64 = 64 * 1024 * 1024;

lazy_static::lazy_static! {
    static ref PACK_CONTEXT_REGEX: Option<Regex> = Regex::new(
        r"\[Scripting\] \[(?P<name>[^\]\r\n]+)\]"
    ).ok();
    static ref STACK_FRAME_REGEX: Option<Regex> = Regex::new(
        r"\((?P<file>[^()\r\n]+?\.m?js):(?P<line>\d+)(?::(?P<column>\d+))?\)"
    ).ok();
}

#[derive(Debug, Clone)]
struct ScriptMap {
    pack_name: String,
    pack_dir: PathBuf,
    generated_entry: String,
    map_path: PathBuf,
}

#[derive(Debug, Default)]
pub struct SourcemapResolver {
    maps: Vec<ScriptMap>,
}

impl SourcemapResolver {
    /// Discovers script modules without making a malformed or unreadable pack fatal.
    pub fn discover(bds_dir: &Path) -> Self {
        let mut maps = discover_packs_in_root(&bds_dir.join("system_behavior_packs"), None);

        if let Some((world_dir, references)) = active_world_pack_references(bds_dir) {
            maps.extend(discover_packs_in_root(
                &world_dir.join("behavior_packs"),
                Some(&references),
            ));
            maps.extend(discover_packs_in_root(
                &bds_dir.join("development_behavior_packs"),
                Some(&references),
            ));
        }

        let mut packs: HashMap<&str, usize> = HashMap::new();
        for map in &maps {
            *packs.entry(&map.pack_name).or_default() += 1;
        }

        for (_, (pack_name, count)) in packs.iter().enumerate() {
            println!(
                "{}[bds-enhancer]{} Discovered script module: {}, maps: {}",
                Color::Green,
                Color::Reset,
                pack_name,
                count
            );
        }

        Self { maps }
    }

    /// Returns the original log on every kind of failure, including an unexpected panic.
    pub fn resolve_log(&self, log: &str) -> String {
        self.resolve_log_with_generated_style(log, "", "")
    }

    /// Applies styling only to generated locations in successfully resolved frames.
    pub(crate) fn resolve_log_with_generated_style(
        &self,
        log: &str,
        generated_prefix: &str,
        generated_suffix: &str,
    ) -> String {
        catch_unwind(AssertUnwindSafe(|| {
            self.resolve_log_inner(log, generated_prefix, generated_suffix)
        }))
        .ok()
        .flatten()
        .unwrap_or_else(|| log.to_owned())
    }

    fn resolve_log_inner(
        &self,
        log: &str,
        generated_prefix: &str,
        generated_suffix: &str,
    ) -> Option<String> {
        let context = PACK_CONTEXT_REGEX.as_ref()?.captures(log)?;
        let pack_name = context.name("name")?.as_str();
        let pack_maps: Vec<&ScriptMap> = self
            .maps
            .iter()
            .filter(|candidate| candidate.pack_name == pack_name)
            .collect();

        if pack_maps.is_empty() {
            return None;
        }

        let mut resolved_any = false;
        let resolved =
            STACK_FRAME_REGEX
                .as_ref()?
                .replace_all(log, |captures: &regex::Captures<'_>| {
                    let original = captures
                        .get(0)
                        .map(|capture| capture.as_str())
                        .unwrap_or_default();
                    let Some(file) = captures.name("file").map(|capture| capture.as_str()) else {
                        return original.to_owned();
                    };
                    let Some(line) = captures
                        .name("line")
                        .and_then(|capture| capture.as_str().parse::<u32>().ok())
                    else {
                        return original.to_owned();
                    };
                    let column = captures
                        .name("column")
                        .and_then(|capture| capture.as_str().parse::<u32>().ok());

                    let candidates: Vec<&ScriptMap> = pack_maps
                        .iter()
                        .copied()
                        .filter(|candidate| {
                            generated_entry_matches(&candidate.generated_entry, file)
                        })
                        .collect();
                    let locations: Vec<OriginalLocation> = candidates
                        .into_iter()
                        .filter_map(|candidate| resolve_frame(candidate, line, column))
                        .collect();
                    let [location] = locations.as_slice() else {
                        return original.to_owned();
                    };

                    resolved_any = true;
                    format!(
                        "({}:{}) {generated_prefix}{original}{generated_suffix}",
                        location.source, location.line
                    )
                });

        resolved_any.then(|| resolved.into_owned())
    }
}

#[derive(Debug, Deserialize)]
struct Manifest {
    header: ManifestHeader,
    #[serde(default)]
    modules: Vec<ManifestModule>,
}

#[derive(Debug, Deserialize)]
struct ManifestHeader {
    name: String,
    uuid: Option<String>,
    version: Value,
}

#[derive(Debug, Deserialize)]
struct ManifestModule {
    #[serde(rename = "type")]
    module_type: String,
    entry: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorldPackReference {
    pack_id: String,
    version: Value,
}

#[derive(Debug, PartialEq, Eq)]
struct PackIdentity {
    uuid: String,
    version: String,
}

fn active_world_pack_references(bds_dir: &Path) -> Option<(PathBuf, Vec<PackIdentity>)> {
    let properties = fs::read_to_string(bds_dir.join("server.properties")).ok()?;
    let level_name = properties.lines().find_map(|line| {
        let line = line.trim();
        if line.starts_with('#') {
            return None;
        }
        line.strip_prefix("level-name=")
            .map(str::trim)
            .filter(|name| !name.is_empty())
    })?;
    let worlds_dir = bds_dir.join("worlds");
    let world_dir = safe_pack_path(&worlds_dir, Path::new(level_name))?;
    let references = fs::read(world_dir.join("world_behavior_packs.json")).ok()?;
    let references = serde_json::from_slice::<Vec<WorldPackReference>>(&references).ok()?;
    let identities = references
        .into_iter()
        .filter_map(|reference| {
            Some(PackIdentity {
                uuid: reference.pack_id,
                version: manifest_version(&reference.version)?,
            })
        })
        .collect();

    Some((world_dir, identities))
}

fn discover_packs_in_root(root: &Path, references: Option<&[PackIdentity]>) -> Vec<ScriptMap> {
    let Ok(pack_dirs) = fs::read_dir(root) else {
        return Vec::new();
    };

    pack_dirs
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .flat_map(|path| discover_pack(&path, references))
        .collect()
}

fn discover_pack(pack_dir: &Path, references: Option<&[PackIdentity]>) -> Vec<ScriptMap> {
    let Ok(manifest_bytes) = fs::read(pack_dir.join("manifest.json")) else {
        return Vec::new();
    };
    let Ok(manifest) = serde_json::from_slice::<Manifest>(&manifest_bytes) else {
        return Vec::new();
    };
    let Some(version) = manifest_version(&manifest.header.version) else {
        return Vec::new();
    };
    if let Some(references) = references {
        let Some(uuid) = manifest.header.uuid.as_deref() else {
            return Vec::new();
        };
        if !references.iter().any(|reference| {
            reference.uuid.eq_ignore_ascii_case(uuid) && reference.version == version
        }) {
            return Vec::new();
        }
    }

    manifest
        .modules
        .into_iter()
        .filter(|module| module.module_type == "script")
        .filter_map(|module| module.entry)
        .flat_map(|entry| {
            discover_script_maps(pack_dir, &manifest.header.name, &normalize_slashes(&entry))
        })
        .collect()
}

fn discover_script_maps(pack_dir: &Path, pack_name: &str, entry: &str) -> Vec<ScriptMap> {
    let Some(generated_path) = safe_pack_path(pack_dir, Path::new(entry)) else {
        return Vec::new();
    };
    let mut generated_entries = vec![entry.to_owned()];

    if let Some(script_dir) = generated_path.parent() {
        discover_generated_entries(pack_dir, script_dir, &mut generated_entries);
    }

    generated_entries.sort_unstable();
    generated_entries.dedup();
    generated_entries
        .into_iter()
        .map(|generated_entry| {
            let generated_path = pack_dir.join(&generated_entry);
            let mut map_path = generated_path.as_os_str().to_os_string();
            map_path.push(".map");

            ScriptMap {
                pack_name: pack_name.to_owned(),
                pack_dir: pack_dir.to_owned(),
                generated_entry,
                map_path: PathBuf::from(map_path),
            }
        })
        .collect()
}

fn discover_generated_entries(pack_dir: &Path, directory: &Path, entries: &mut Vec<String>) {
    let Ok(children) = fs::read_dir(directory) else {
        return;
    };

    for child in children.flatten() {
        let Ok(file_type) = child.file_type() else {
            continue;
        };
        let path = child.path();
        if file_type.is_dir() {
            discover_generated_entries(pack_dir, &path, entries);
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(generated_name) = file_name.strip_suffix(".map") else {
            continue;
        };
        if !(generated_name.ends_with(".js") || generated_name.ends_with(".mjs")) {
            continue;
        }

        let generated_path = path.with_file_name(generated_name);
        let Some(relative) = generated_path.strip_prefix(pack_dir).ok() else {
            continue;
        };
        let Some(relative) = normalize_relative_path(relative) else {
            continue;
        };
        entries.push(normalize_slashes(&relative.to_string_lossy()));
    }
}

fn manifest_version(version: &Value) -> Option<String> {
    match version {
        Value::String(version) => Some(version.clone()),
        Value::Array(parts) if !parts.is_empty() => parts
            .iter()
            .map(|part| match part {
                Value::Number(number) => Some(number.to_string()),
                Value::String(string) => Some(string.clone()),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .map(|parts| parts.join(".")),
        _ => None,
    }
}

fn generated_entry_matches(entry: &str, frame_file: &str) -> bool {
    let frame_file = normalize_slashes(frame_file);
    if entry == frame_file || frame_file.ends_with(&format!("/{entry}")) {
        return true;
    }

    Path::new(entry).file_name() == Path::new(&frame_file).file_name()
}

#[derive(Debug, PartialEq, Eq)]
struct OriginalLocation {
    source: String,
    line: u32,
}

fn resolve_frame(
    candidate: &ScriptMap,
    generated_line: u32,
    generated_column: Option<u32>,
) -> Option<OriginalLocation> {
    let generated_line = generated_line.checked_sub(1)?;
    let map_bytes = read_stable_map(&candidate.map_path)?;
    let map = decode_slice(&map_bytes).ok()?;

    if let Some(generated_column) = generated_column {
        let token = map.lookup_token(generated_line, generated_column.saturating_sub(1))?;
        return original_location_from_token(candidate, token, generated_line);
    }

    // BDS 1.26 does not print generated columns. Use the first mapped token on
    // that exact line instead of guessing column zero, which may select a token
    // from the preceding line.
    match &map {
        DecodedMap::Regular(map) => first_location_on_line(candidate, map.tokens(), generated_line),
        DecodedMap::Hermes(map) => first_location_on_line(candidate, map.tokens(), generated_line),
        DecodedMap::Index(index) => {
            let flattened = index.flatten().ok()?;
            first_location_on_line(candidate, flattened.tokens(), generated_line)
        }
    }
}

fn first_location_on_line<'a>(
    candidate: &ScriptMap,
    tokens: impl Iterator<Item = Token<'a>>,
    generated_line: u32,
) -> Option<OriginalLocation> {
    let token = tokens
        .filter(|token| token.get_dst_line() == generated_line && token.has_source())
        .min_by_key(|token| token.get_dst_col())?;
    original_location_from_token(candidate, token, generated_line)
}

fn original_location_from_token(
    candidate: &ScriptMap,
    token: Token<'_>,
    generated_line: u32,
) -> Option<OriginalLocation> {
    // lookup_token may return a token from an earlier generated line. Never use it.
    if token.get_dst_line() != generated_line || !token.has_source() {
        return None;
    }

    let source = token.get_source()?;
    let source = source_display_path(candidate, source)?;
    let line = token.get_src_line().checked_add(1)?;

    Some(OriginalLocation { source, line })
}

fn read_stable_map(path: &Path) -> Option<Vec<u8>> {
    let before = fs::metadata(path).ok()?;
    if !before.is_file() || before.len() > MAX_SOURCEMAP_BYTES {
        return None;
    }

    let bytes = fs::read(path).ok()?;
    let after = fs::metadata(path).ok()?;
    if before.len() != after.len() || bytes.len() as u64 != after.len() {
        return None;
    }
    if let (Ok(before_modified), Ok(after_modified)) = (before.modified(), after.modified())
        && before_modified != after_modified
    {
        return None;
    }

    Some(bytes)
}

fn source_display_path(candidate: &ScriptMap, source: &str) -> Option<String> {
    let entry_parent = Path::new(&candidate.generated_entry)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let relative = normalize_relative_path(&entry_parent.join(source))?;
    safe_pack_path(&candidate.pack_dir, &relative)?;
    Some(normalize_slashes(&relative.to_string_lossy()))
}

fn safe_pack_path(pack_dir: &Path, relative: &Path) -> Option<PathBuf> {
    let relative = normalize_relative_path(relative)?;
    Some(pack_dir.join(relative))
}

fn normalize_relative_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::Prefix(_) | Component::RootDir => return None,
        }
    }

    Some(normalized)
}

fn normalize_slashes(value: &str) -> String {
    value.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    const MAP: &str = r#"{"version":3,"file":"main.js","sources":["../src/main.ts"],"sourcesContent":["first\nsecond\n"],"names":[],"mappings":";;;AAAA;;;;AACA"}"#;
    const TSDOWN_MAP: &str = r#"{"version":3,"file":"main.js","names":[],"sources":["../src/main.ts"],"sourcesContent":["import { world } from '@minecraft/server';\n\nfunction throwNestedSourcemapProbe(): never {\n  throw new Error('BDS_ENHANCER_SOURCEMAP_PROBE');\n}\n\nworld.afterEvents.worldLoad.subscribe(() => {\n  console.log('Hello world!');\n  throwNestedSourcemapProbe();\n});\n"],"mappings":";;AAEA,SAAS,4BAAmC;CAC1C,MAAM,IAAI,MAAM,8BAA8B;AAChD;AAEA,MAAM,YAAY,UAAU,gBAAgB;CAC1C,QAAQ,IAAI,cAAc;CAC1B,0BAA0B;AAC5B,CAAC"}"#;

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock must be after the Unix epoch")
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "bds-enhancer-sourcemap-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&root).expect("fixture root must be created");
            Self { root }
        }

        fn add_pack(
            &self,
            root: &str,
            directory: &str,
            name: &str,
            version: &str,
            map: Option<&str>,
        ) {
            let pack = self.root.join(root).join(directory);
            fs::create_dir_all(pack.join("scripts")).expect("scripts directory must be created");
            fs::create_dir_all(pack.join("src")).expect("source directory must be created");
            let uuid = Self::pack_uuid(directory);
            let manifest = format!(
                r#"{{"format_version":3,"header":{{"name":"{name}","uuid":"{uuid}","version":"{version}"}},"modules":[{{"type":"script","entry":"scripts/main.js"}}]}}"#
            );
            fs::write(pack.join("manifest.json"), manifest).expect("manifest must be written");
            fs::write(pack.join("scripts/main.js"), "generated").expect("script must be written");
            fs::write(pack.join("src/main.ts"), "first\nsecond\n").expect("source must be written");
            if let Some(map) = map {
                fs::write(pack.join("scripts/main.js.map"), map).expect("map must be written");
            }
        }

        fn activate_world(&self, world_name: &str, packs: &[(&str, &str)]) {
            let world = self.root.join("worlds").join(world_name);
            fs::create_dir_all(&world).expect("active world directory must be created");
            fs::write(
                self.root.join("server.properties"),
                format!("server-name=Fixture\nlevel-name={world_name}\n"),
            )
            .expect("server properties must be written");
            let references: Vec<Value> = packs
                .iter()
                .map(|(directory, version)| {
                    let version: Vec<u64> = version
                        .split('.')
                        .map(|part| part.parse().expect("fixture version must be numeric"))
                        .collect();
                    serde_json::json!({
                        "pack_id": Self::pack_uuid(directory),
                        "version": version,
                    })
                })
                .collect();
            fs::write(
                world.join("world_behavior_packs.json"),
                serde_json::to_vec(&references).expect("world pack references must serialize"),
            )
            .expect("world pack references must be written");
        }

        fn pack_uuid(directory: &str) -> String {
            format!("{directory}-uuid")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn scripting_error(frames: &str) -> String {
        format!("[2026 ERROR] [Scripting] [Test Pack] Error: probe{frames}")
    }

    #[test]
    fn resolves_a_frame_for_the_matching_pack() {
        let fixture = Fixture::new();
        fixture.add_pack(
            "system_behavior_packs",
            "test",
            "Test Pack",
            "1.2.3",
            Some(MAP),
        );
        let resolver = SourcemapResolver::discover(&fixture.root);
        let input = scripting_error("\n    at probe (main.js:4)");

        assert_eq!(
            resolver.resolve_log(&input),
            scripting_error("\n    at probe (src/main.ts:1) (main.js:4)")
        );
    }

    #[test]
    fn resolves_multiple_maps_in_the_same_script_module() {
        let fixture = Fixture::new();
        fixture.add_pack(
            "system_behavior_packs",
            "test",
            "Test Pack",
            "1.2.3",
            Some(MAP),
        );
        let pack = fixture.root.join("system_behavior_packs/test");
        let secondary_map = MAP
            .replace("main.js", "aaa.js")
            .replace("main.ts", "aaa.ts");
        fs::write(pack.join("scripts/aaa.js.map"), secondary_map)
            .expect("secondary map must be written");
        let resolver = SourcemapResolver::discover(&fixture.root);
        let input = scripting_error("\n    at main (main.js:4)\n    at secondary (aaa.js:4)");

        assert_eq!(
            resolver.resolve_log(&input),
            scripting_error(
                "\n    at main (src/main.ts:1) (main.js:4)\n    at secondary (src/aaa.ts:1) (aaa.js:4)"
            )
        );
    }

    #[test]
    fn resolves_known_frames_and_preserves_unknown_frames() {
        let fixture = Fixture::new();
        fixture.add_pack(
            "system_behavior_packs",
            "test",
            "Test Pack",
            "1.2.3",
            Some(MAP),
        );
        let resolver = SourcemapResolver::discover(&fixture.root);
        let input = scripting_error("\n    at probe (main.js:4)\n    at missing (main.js:99)");

        assert_eq!(
            resolver.resolve_log(&input),
            scripting_error(
                "\n    at probe (src/main.ts:1) (main.js:4)\n    at missing (main.js:99)"
            )
        );
    }

    #[test]
    fn styles_only_generated_locations_in_resolved_frames() {
        let fixture = Fixture::new();
        fixture.add_pack(
            "system_behavior_packs",
            "test",
            "Test Pack",
            "1.2.3",
            Some(MAP),
        );
        let resolver = SourcemapResolver::discover(&fixture.root);
        let input = scripting_error("\n    at probe (main.js:4)\n    at missing (main.js:99)");

        assert_eq!(
            resolver.resolve_log_with_generated_style(&input, "<dim>", "</dim>"),
            scripting_error(
                "\n    at probe (src/main.ts:1) <dim>(main.js:4)</dim>\n    at missing (main.js:99)"
            )
        );
    }

    #[test]
    fn resolves_the_real_tsdown_watch_map_without_generated_columns() {
        let fixture = Fixture::new();
        fixture.add_pack(
            "system_behavior_packs",
            "test",
            "Test Pack",
            "1.2.3",
            Some(TSDOWN_MAP),
        );
        let resolver = SourcemapResolver::discover(&fixture.root);
        let input = scripting_error(
            "\n    at throwNestedSourcemapProbe (main.js:4)\n    at <anonymous> (main.js:8)",
        );

        assert_eq!(
            resolver.resolve_log(&input),
            scripting_error(
                "\n    at throwNestedSourcemapProbe (src/main.ts:4) (main.js:4)\n    at <anonymous> (src/main.ts:9) (main.js:8)"
            )
        );
    }

    #[test]
    fn treats_a_version_like_suffix_as_part_of_the_manifest_name() {
        let fixture = Fixture::new();
        fixture.add_pack(
            "system_behavior_packs",
            "test",
            "scriptapi-template v0.1.0",
            "0.1.0",
            Some(TSDOWN_MAP),
        );
        let resolver = SourcemapResolver::discover(&fixture.root);
        let input = "[2026 ERROR] [Scripting] [scriptapi-template v0.1.0] Error: probe\n    at probe (main.js:4)";

        assert_eq!(
            resolver.resolve_log(input),
            "[2026 ERROR] [Scripting] [scriptapi-template v0.1.0] Error: probe\n    at probe (src/main.ts:4) (main.js:4)"
        );
    }

    #[test]
    fn leaves_the_whole_log_unchanged_for_a_malformed_map() {
        let fixture = Fixture::new();
        fixture.add_pack(
            "system_behavior_packs",
            "test",
            "Test Pack",
            "1.2.3",
            Some("{"),
        );
        let resolver = SourcemapResolver::discover(&fixture.root);
        let input = scripting_error("\n    at probe (main.js:4)");

        assert_eq!(resolver.resolve_log(&input), input);
    }

    #[test]
    fn observes_map_updates_without_rebuilding_the_pack_index() {
        let fixture = Fixture::new();
        fixture.add_pack(
            "system_behavior_packs",
            "test",
            "Test Pack",
            "1.2.3",
            Some(MAP),
        );
        let resolver = SourcemapResolver::discover(&fixture.root);
        let input = scripting_error("\n    at probe (main.js:4)");
        assert_eq!(
            resolver.resolve_log(&input),
            scripting_error("\n    at probe (src/main.ts:1) (main.js:4)")
        );

        let updated_map = MAP.replace("../src/main.ts", "../src/updated.ts");
        fs::write(
            fixture
                .root
                .join("system_behavior_packs/test/scripts/main.js.map"),
            updated_map,
        )
        .expect("updated map must be written");

        assert_eq!(
            resolver.resolve_log(&input),
            scripting_error("\n    at probe (src/updated.ts:1) (main.js:4)")
        );
    }

    #[test]
    fn falls_back_during_a_broken_update_and_recovers_afterward() {
        let fixture = Fixture::new();
        fixture.add_pack(
            "system_behavior_packs",
            "test",
            "Test Pack",
            "1.2.3",
            Some(MAP),
        );
        let resolver = SourcemapResolver::discover(&fixture.root);
        let input = scripting_error("\n    at probe (main.js:4)");
        let map_path = fixture
            .root
            .join("system_behavior_packs/test/scripts/main.js.map");

        fs::write(&map_path, "{").expect("broken map must be written");
        assert_eq!(resolver.resolve_log(&input), input);

        fs::write(&map_path, MAP).expect("recovered map must be written");
        assert_eq!(
            resolver.resolve_log(&input),
            scripting_error("\n    at probe (src/main.ts:1) (main.js:4)")
        );
    }

    #[test]
    fn leaves_the_log_unchanged_when_the_map_is_missing() {
        let fixture = Fixture::new();
        fixture.add_pack("system_behavior_packs", "test", "Test Pack", "1.2.3", None);
        let resolver = SourcemapResolver::discover(&fixture.root);
        let input = scripting_error("\n    at probe (main.js:4)");

        assert_eq!(resolver.resolve_log(&input), input);
    }

    #[test]
    fn leaves_the_log_unchanged_when_pack_identity_does_not_match() {
        let fixture = Fixture::new();
        fixture.add_pack(
            "system_behavior_packs",
            "test",
            "Other Pack",
            "1.2.3",
            Some(MAP),
        );
        let resolver = SourcemapResolver::discover(&fixture.root);
        let input = scripting_error("\n    at probe (main.js:4)");

        assert_eq!(resolver.resolve_log(&input), input);
    }

    #[test]
    fn resolves_multiple_packs_by_name_and_version() {
        let fixture = Fixture::new();
        fixture.add_pack(
            "system_behavior_packs",
            "one",
            "Test Pack",
            "1.2.3",
            Some(MAP),
        );
        let second_map = MAP.replace("../src/main.ts", "../src/second.ts");
        fixture.add_pack(
            "development_behavior_packs",
            "two",
            "Second Pack",
            "2.0.0",
            Some(&second_map),
        );
        fixture.activate_world("Active World", &[("two", "2.0.0")]);
        let resolver = SourcemapResolver::discover(&fixture.root);
        let first = scripting_error("\n    at probe (main.js:4)");
        let second =
            "[2026 ERROR] [Scripting] [Second Pack] Error: probe\n    at probe (main.js:4)";

        assert_eq!(
            resolver.resolve_log(&first),
            scripting_error("\n    at probe (src/main.ts:1) (main.js:4)")
        );
        assert_eq!(
            resolver.resolve_log(second),
            "[2026 ERROR] [Scripting] [Second Pack] Error: probe\n    at probe (src/second.ts:1) (main.js:4)"
        );
    }

    #[test]
    fn ignores_the_internal_behavior_packs_root() {
        let fixture = Fixture::new();
        fixture.add_pack(
            "behavior_packs",
            "internal",
            "Test Pack",
            "1.2.3",
            Some(MAP),
        );
        let resolver = SourcemapResolver::discover(&fixture.root);
        let input = scripting_error("\n    at probe (main.js:4)");

        assert_eq!(resolver.resolve_log(&input), input);
    }

    #[test]
    fn ignores_an_unreferenced_development_pack() {
        let fixture = Fixture::new();
        fixture.add_pack(
            "development_behavior_packs",
            "dev",
            "Test Pack",
            "1.2.3",
            Some(MAP),
        );
        fixture.activate_world("Active World", &[]);
        let resolver = SourcemapResolver::discover(&fixture.root);
        let input = scripting_error("\n    at probe (main.js:4)");

        assert_eq!(resolver.resolve_log(&input), input);
    }

    #[test]
    fn discovers_a_referenced_pack_inside_the_active_world() {
        let fixture = Fixture::new();
        fixture.add_pack(
            "worlds/Active World/behavior_packs",
            "world-pack",
            "Test Pack",
            "1.2.3",
            Some(MAP),
        );
        fixture.activate_world("Active World", &[("world-pack", "1.2.3")]);
        let resolver = SourcemapResolver::discover(&fixture.root);
        let input = scripting_error("\n    at probe (main.js:4)");

        assert_eq!(
            resolver.resolve_log(&input),
            scripting_error("\n    at probe (src/main.ts:1) (main.js:4)")
        );
    }

    #[test]
    fn ignores_a_referenced_uuid_when_the_version_differs() {
        let fixture = Fixture::new();
        fixture.add_pack(
            "development_behavior_packs",
            "dev",
            "Test Pack",
            "1.2.3",
            Some(MAP),
        );
        fixture.activate_world("Active World", &[("dev", "1.2.4")]);
        let resolver = SourcemapResolver::discover(&fixture.root);
        let input = scripting_error("\n    at probe (main.js:4)");

        assert_eq!(resolver.resolve_log(&input), input);
    }

    #[test]
    fn leaves_the_log_unchanged_when_matching_candidates_are_ambiguous() {
        let fixture = Fixture::new();
        fixture.add_pack(
            "system_behavior_packs",
            "one",
            "Test Pack",
            "1.2.3",
            Some(MAP),
        );
        fixture.add_pack(
            "system_behavior_packs",
            "two",
            "Test Pack",
            "1.2.3",
            Some(MAP),
        );
        let resolver = SourcemapResolver::discover(&fixture.root);
        let input = scripting_error("\n    at probe (main.js:4)");

        assert_eq!(resolver.resolve_log(&input), input);
    }

    #[test]
    fn uses_the_only_resolvable_candidate_when_another_map_is_broken() {
        let fixture = Fixture::new();
        fixture.add_pack(
            "system_behavior_packs",
            "one",
            "Test Pack",
            "1.2.3",
            Some(MAP),
        );
        fixture.add_pack(
            "system_behavior_packs",
            "two",
            "Test Pack",
            "1.2.3",
            Some("{"),
        );
        let resolver = SourcemapResolver::discover(&fixture.root);
        let input = scripting_error("\n    at probe (main.js:4)");

        assert_eq!(
            resolver.resolve_log(&input),
            scripting_error("\n    at probe (src/main.ts:1) (main.js:4)")
        );
    }

    #[test]
    fn supports_array_manifest_versions() {
        let fixture = Fixture::new();
        let pack = fixture.root.join("system_behavior_packs/test");
        fs::create_dir_all(pack.join("scripts")).expect("scripts directory must be created");
        fs::write(
            pack.join("manifest.json"),
            r#"{"header":{"name":"Test Pack","version":[1,2,3]},"modules":[{"type":"script","entry":"scripts/main.js"}]}"#,
        )
        .expect("manifest must be written");
        fs::write(pack.join("scripts/main.js.map"), MAP).expect("map must be written");
        let resolver = SourcemapResolver::discover(&fixture.root);
        let input = scripting_error("\n    at probe (main.js:4)");

        assert_eq!(
            resolver.resolve_log(&input),
            scripting_error("\n    at probe (src/main.ts:1) (main.js:4)")
        );
    }

    #[test]
    fn rejects_source_paths_that_escape_the_pack() {
        let fixture = Fixture::new();
        let escaping_map = MAP.replace("../src/main.ts", "../../outside.ts");
        fixture.add_pack(
            "system_behavior_packs",
            "test",
            "Test Pack",
            "1.2.3",
            Some(&escaping_map),
        );
        let resolver = SourcemapResolver::discover(&fixture.root);
        let input = scripting_error("\n    at probe (main.js:4)");

        assert_eq!(resolver.resolve_log(&input), input);
    }
}
