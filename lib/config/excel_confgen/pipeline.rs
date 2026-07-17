use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs, io,
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

use crate::excel_confgen::{
    emit_file::emit_table_file, emit_root::emit_root_module, tables::FILTER_TABLES,
};

trait FileSystem: Sync {
    fn create_dir(&self, path: &Path) -> io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn remove_dir_all(&self, path: &Path) -> io::Result<()>;
}

struct RealFileSystem;

impl FileSystem for RealFileSystem {
    fn create_dir(&self, path: &Path) -> io::Result<()> {
        fs::create_dir(path)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }

    fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
        fs::remove_dir_all(path)
    }
}

pub fn generate_rust_modules(json_dir: &str, output_dir: &str) -> Result<()> {
    generate_rust_modules_with_fs(Path::new(json_dir), Path::new(output_dir), &RealFileSystem)
}

fn generate_rust_modules_with_fs(
    json_dir: &Path,
    output: &Path,
    file_system: &dyn FileSystem,
) -> Result<()> {
    let filter: HashSet<&str> = FILTER_TABLES.iter().copied().collect();
    let mut parsed = BTreeMap::new();

    for entry in WalkDir::new(json_dir) {
        let entry = entry
            .with_context(|| format!("failed to walk JSON directory {}", json_dir.display()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let raw = entry
            .path()
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("Invalid filename"))?;

        if !filter.contains(raw) {
            continue;
        }

        let snake = raw.to_string();
        let json = fs::read_to_string(entry.path())
            .with_context(|| format!("failed to read {}", entry.path().display()))?;
        let data: Value = serde_json::from_str(&json)
            .with_context(|| format!("failed to parse {}", entry.path().display()))?;

        let payload = data
            .as_array()
            .ok_or_else(|| anyhow!("{} must contain [table name, rows]", entry.path().display()))?;
        if payload.len() != 2 || payload[0].as_str() != Some(raw) || !payload[1].is_array() {
            return Err(anyhow!(
                "{} must contain [table name, rows] for {raw}",
                entry.path().display()
            ));
        }
        let table = raw.to_string();
        let records = payload[1].as_array().cloned().expect("checked as array");
        if parsed.insert(snake.clone(), (table, records)).is_some() {
            return Err(anyhow!("duplicate configured JSON table: {snake}"));
        }
    }

    let required: BTreeSet<&str> = FILTER_TABLES.iter().copied().collect();
    let found: BTreeSet<&str> = parsed.keys().map(String::as_str).collect();
    let missing: Vec<&str> = required.difference(&found).copied().collect();
    if !missing.is_empty() {
        return Err(anyhow!(
            "missing configured JSON tables: {}",
            missing.join(", ")
        ));
    }

    let staging = peer_directory(output, ".staging")?;
    file_system.create_dir(&staging).with_context(|| {
        format!(
            "failed to exclusively create generated staging directory {}",
            staging.display()
        )
    })?;

    let result = (|| {
        let tables: Vec<String> = parsed.keys().cloned().collect();
        for (snake, (table, records)) in &parsed {
            fs::write(
                staging.join(format!("{snake}.rs")),
                emit_table_file(table, records),
            )
            .with_context(|| format!("failed to generate Rust module {snake}"))?;
        }
        fs::write(staging.join("mod.rs"), emit_root_module(&tables))?;
        publish_generated_directory(&staging, output, file_system)
    })();

    if let Err(error) = result {
        remove_owned_staging(&staging, output, file_system).with_context(|| {
            format!("failed to clean generated staging after publish error: {error:#}")
        })?;
        return Err(error);
    }
    Ok(())
}

fn peer_directory(output: &Path, suffix: &str) -> Result<PathBuf> {
    let file_name = output
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("invalid generated output directory: {}", output.display()))?;
    let candidate = output.with_file_name(format!("{file_name}{suffix}"));
    if candidate.parent() != output.parent()
        || candidate.file_name().and_then(|name| name.to_str())
            != Some(format!("{file_name}{suffix}").as_str())
    {
        return Err(anyhow!("refusing unsafe generated output path"));
    }
    Ok(candidate)
}

fn publish_generated_directory(
    staging: &Path,
    output: &Path,
    file_system: &dyn FileSystem,
) -> Result<()> {
    let expected_staging = peer_directory(output, ".staging")?;
    if staging != expected_staging {
        return Err(anyhow!("refusing unexpected generated staging path"));
    }

    let backup = peer_directory(output, ".previous")?;
    match (output.exists(), backup.exists()) {
        (false, true) => retry_io(|| file_system.rename(&backup, output)).with_context(|| {
            format!(
                "failed to restore generated output from {}",
                backup.display()
            )
        })?,
        (true, true) => remove_dir_all_with_retry(&backup, file_system).with_context(|| {
            format!(
                "failed to remove stale generated output backup {}",
                backup.display()
            )
        })?,
        _ => {}
    }

    if !output.exists() {
        file_system.rename(staging, output)?;
        return Ok(());
    }

    file_system.rename(output, &backup)?;
    if let Err(error) = file_system.rename(staging, output) {
        retry_io(|| file_system.rename(&backup, output)).with_context(|| {
            format!("failed to restore generated output after publish error: {error}")
        })?;
        return Err(error.into());
    }

    let expected_backup = peer_directory(output, ".previous")?;
    if backup != expected_backup {
        return Err(anyhow!("refusing unexpected generated backup path"));
    }
    let _ = remove_dir_all_with_retry(&backup, file_system);
    Ok(())
}

fn retry_io(mut operation: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    let first_error = match operation() {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    operation().map_err(|_| first_error)
}

fn remove_dir_all_with_retry(path: &Path, file_system: &dyn FileSystem) -> io::Result<()> {
    retry_io(|| match file_system.remove_dir_all(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    })
}

fn remove_owned_staging(staging: &Path, output: &Path, file_system: &dyn FileSystem) -> Result<()> {
    let expected_staging = peer_directory(output, ".staging")?;
    if staging != expected_staging {
        return Err(anyhow!("refusing to remove unexpected staging path"));
    }
    remove_dir_all_with_retry(staging, file_system)
        .with_context(|| format!("failed to remove owned staging {}", staging.display()))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs, io,
        path::{Path, PathBuf},
        sync::{
            Arc, Barrier,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };

    use serde_json::json;

    use super::{FileSystem, generate_rust_modules, generate_rust_modules_with_fs};
    use crate::excel_confgen::tables::FILTER_TABLES;

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new(test_name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "sonetto-pipeline-{test_name}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn write_inputs(root: &Path, omitted: &[&str]) -> PathBuf {
        let input = root.join("input");
        fs::create_dir(&input).unwrap();
        for table in FILTER_TABLES {
            if omitted.contains(table) {
                continue;
            }
            let payload = json!([table, [{"id": 1}]]);
            fs::write(
                input.join(format!("{table}.json")),
                serde_json::to_vec(&payload).unwrap(),
            )
            .unwrap();
        }
        input
    }

    fn snapshot_files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        walkdir::WalkDir::new(root)
            .into_iter()
            .map(Result::unwrap)
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| {
                (
                    entry.path().strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(entry.path()).unwrap(),
                )
            })
            .collect()
    }

    struct InjectedFileSystem {
        rename_calls: AtomicUsize,
        remove_calls: AtomicUsize,
        fail_renames: Vec<usize>,
        fail_removes: Vec<usize>,
        create_barrier: Option<Arc<Barrier>>,
    }

    impl InjectedFileSystem {
        fn new(fail_renames: &[usize], fail_removes: &[usize]) -> Self {
            Self {
                rename_calls: AtomicUsize::new(0),
                remove_calls: AtomicUsize::new(0),
                fail_renames: fail_renames.to_vec(),
                fail_removes: fail_removes.to_vec(),
                create_barrier: None,
            }
        }

        fn with_create_barrier(parties: usize) -> Self {
            Self {
                create_barrier: Some(Arc::new(Barrier::new(parties))),
                ..Self::new(&[], &[])
            }
        }
    }

    impl FileSystem for InjectedFileSystem {
        fn create_dir(&self, path: &Path) -> io::Result<()> {
            if let Some(barrier) = &self.create_barrier {
                barrier.wait();
            }
            fs::create_dir(path)
        }

        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            let call = self.rename_calls.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail_renames.contains(&call) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("injected rename failure {call}"),
                ));
            }
            fs::rename(from, to)
        }

        fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
            let call = self.remove_calls.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail_removes.contains(&call) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("injected remove failure {call}"),
                ));
            }
            fs::remove_dir_all(path)
        }
    }

    fn existing_output(root: &Path) -> PathBuf {
        let output = root.join("output");
        fs::create_dir(&output).unwrap();
        fs::write(output.join("sentinel"), b"old output").unwrap();
        output
    }

    fn assert_no_publish_residue(output: &Path) {
        assert!(!output.with_file_name("output.staging").exists());
        assert!(!output.with_file_name("output.previous").exists());
    }

    #[test]
    fn reports_multiple_missing_tables_in_sorted_order_without_writes() {
        let temp = TempDirectory::new("missing");
        let input = write_inputs(&temp.0, &["destiny_facets_ex_level", "character_attribute"]);
        let output = temp.0.join("output");
        fs::create_dir(&output).unwrap();
        fs::write(output.join("sentinel"), b"preserve me").unwrap();
        let before = snapshot_files(&output);

        let error =
            generate_rust_modules(input.to_str().unwrap(), output.to_str().unwrap()).unwrap_err();

        assert!(
            error.to_string().contains(
                "missing configured JSON tables: character_attribute, destiny_facets_ex_level"
            ),
            "unexpected error: {error:#}"
        );
        assert_eq!(snapshot_files(&output), before);
        assert!(!output.with_file_name("output.staging").exists());
        assert!(!output.with_file_name("output.previous").exists());
    }

    #[test]
    fn rejects_malformed_table_shape_without_changing_existing_output() {
        let temp = TempDirectory::new("malformed");
        let input = write_inputs(&temp.0, &[]);
        fs::write(
            input.join("character_attribute.json"),
            br#"["character_attribute", {"not":"rows"}]"#,
        )
        .unwrap();
        let output = temp.0.join("output");
        fs::create_dir(&output).unwrap();
        fs::write(output.join("sentinel"), b"preserve me").unwrap();
        let before = snapshot_files(&output);

        let error =
            generate_rust_modules(input.to_str().unwrap(), output.to_str().unwrap()).unwrap_err();

        let message = format!("{error:#}");
        assert!(message.contains("character_attribute.json"), "{message}");
        assert!(message.contains("[table name, rows]"), "{message}");
        assert_eq!(snapshot_files(&output), before);
        assert!(!output.with_file_name("output.staging").exists());
        assert!(!output.with_file_name("output.previous").exists());
    }

    #[test]
    fn publishes_complete_output_without_temporary_directories() {
        let temp = TempDirectory::new("success");
        let input = write_inputs(&temp.0, &[]);
        let output = temp.0.join("output");
        fs::create_dir(&output).unwrap();
        fs::write(output.join("sentinel"), b"replace me").unwrap();

        generate_rust_modules(input.to_str().unwrap(), output.to_str().unwrap()).unwrap();

        assert!(!output.join("sentinel").exists());
        assert!(output.join("mod.rs").is_file());
        assert_eq!(
            fs::read_to_string(output.join("mod.rs"))
                .unwrap()
                .matches("pub mod ")
                .count(),
            FILTER_TABLES.len()
        );
        for table in FILTER_TABLES {
            assert!(output.join(format!("{table}.rs")).is_file(), "{table}");
        }
        assert!(!output.with_file_name("output.staging").exists());
        assert!(!output.with_file_name("output.previous").exists());
    }

    #[test]
    fn recovers_every_output_staging_previous_startup_combination() {
        for state in 0_u8..8 {
            let temp = TempDirectory::new(&format!("startup-{state:03b}"));
            let input = write_inputs(&temp.0, &[]);
            let output = temp.0.join("output");
            let staging = temp.0.join("output.staging");
            let previous = temp.0.join("output.previous");
            let output_exists = state & 0b001 != 0;
            let staging_exists = state & 0b010 != 0;
            let previous_exists = state & 0b100 != 0;

            for (exists, path, contents) in [
                (output_exists, &output, b"old output".as_slice()),
                (staging_exists, &staging, b"unowned staging".as_slice()),
                (previous_exists, &previous, b"previous output".as_slice()),
            ] {
                if exists {
                    fs::create_dir(path).unwrap();
                    fs::write(path.join("sentinel"), contents).unwrap();
                }
            }

            if staging_exists {
                let before = snapshot_files(&temp.0);
                let result = generate_rust_modules_with_fs(&input, &output, &super::RealFileSystem);
                assert!(
                    result.is_err(),
                    "startup state {state:03b} unexpectedly succeeded"
                );
                assert_eq!(
                    snapshot_files(&temp.0),
                    before,
                    "startup state {state:03b} changed unowned state"
                );
                continue;
            }

            generate_rust_modules_with_fs(&input, &output, &super::RealFileSystem)
                .unwrap_or_else(|error| panic!("startup state {state:03b} failed: {error:#}"));
            assert!(
                output.join("mod.rs").is_file(),
                "startup state {state:03b} did not publish"
            );
            assert_no_publish_residue(&output);
        }
    }

    #[test]
    fn concurrent_runs_exclusively_own_staging() {
        let temp = TempDirectory::new("concurrent");
        let input = write_inputs(&temp.0, &[]);
        let output = temp.0.join("output");
        let file_system = Arc::new(InjectedFileSystem::with_create_barrier(2));

        let handles: Vec<_> = (0..2)
            .map(|_| {
                let input = input.clone();
                let output = output.clone();
                let file_system = Arc::clone(&file_system);
                thread::spawn(move || {
                    generate_rust_modules_with_fs(&input, &output, file_system.as_ref())
                })
            })
            .collect();
        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
        assert!(output.join("mod.rs").is_file());
        assert_no_publish_residue(&output);
    }

    #[test]
    fn first_publish_rename_failure_cleans_owned_staging() {
        let temp = TempDirectory::new("first-rename");
        let input = write_inputs(&temp.0, &[]);
        let output = temp.0.join("output");
        let file_system = InjectedFileSystem::new(&[1], &[]);

        generate_rust_modules_with_fs(&input, &output, &file_system).unwrap_err();

        assert!(!output.exists());
        assert_no_publish_residue(&output);
    }

    #[test]
    fn output_to_previous_failure_preserves_old_output_and_cleans_staging() {
        let temp = TempDirectory::new("backup-rename");
        let input = write_inputs(&temp.0, &[]);
        let output = existing_output(&temp.0);
        let before = snapshot_files(&output);
        let file_system = InjectedFileSystem::new(&[1], &[]);

        generate_rust_modules_with_fs(&input, &output, &file_system).unwrap_err();

        assert_eq!(snapshot_files(&output), before);
        assert_no_publish_residue(&output);
    }

    #[test]
    fn staging_to_output_failure_rolls_back_old_output() {
        let temp = TempDirectory::new("publish-rename");
        let input = write_inputs(&temp.0, &[]);
        let output = existing_output(&temp.0);
        let before = snapshot_files(&output);
        let file_system = InjectedFileSystem::new(&[2], &[]);

        generate_rust_modules_with_fs(&input, &output, &file_system).unwrap_err();

        assert_eq!(snapshot_files(&output), before);
        assert_no_publish_residue(&output);
    }

    #[test]
    fn rollback_rename_is_retried_before_returning_failure() {
        let temp = TempDirectory::new("rollback-rename");
        let input = write_inputs(&temp.0, &[]);
        let output = existing_output(&temp.0);
        let before = snapshot_files(&output);
        let file_system = InjectedFileSystem::new(&[2, 3], &[]);

        generate_rust_modules_with_fs(&input, &output, &file_system).unwrap_err();

        assert_eq!(snapshot_files(&output), before);
        assert_no_publish_residue(&output);
    }

    #[test]
    fn staging_cleanup_remove_is_retried() {
        let temp = TempDirectory::new("staging-remove");
        let input = write_inputs(&temp.0, &[]);
        let output = existing_output(&temp.0);
        let before = snapshot_files(&output);
        let file_system = InjectedFileSystem::new(&[1], &[1]);

        generate_rust_modules_with_fs(&input, &output, &file_system).unwrap_err();

        assert_eq!(snapshot_files(&output), before);
        assert_no_publish_residue(&output);
    }

    #[test]
    fn previous_to_output_recovery_rename_is_retried() {
        let temp = TempDirectory::new("previous-rename");
        let input = write_inputs(&temp.0, &[]);
        let output = temp.0.join("output");
        let previous = temp.0.join("output.previous");
        fs::create_dir(&previous).unwrap();
        fs::write(previous.join("sentinel"), b"recover me").unwrap();
        let file_system = InjectedFileSystem::new(&[1], &[]);

        generate_rust_modules_with_fs(&input, &output, &file_system).unwrap();

        assert!(output.join("mod.rs").is_file());
        assert_no_publish_residue(&output);
    }

    #[test]
    fn stale_previous_cleanup_failure_preserves_existing_state() {
        let temp = TempDirectory::new("stale-previous-remove");
        let input = write_inputs(&temp.0, &[]);
        let output = existing_output(&temp.0);
        let previous = temp.0.join("output.previous");
        fs::create_dir(&previous).unwrap();
        fs::write(previous.join("sentinel"), b"previous output").unwrap();
        let output_before = snapshot_files(&output);
        let previous_before = snapshot_files(&previous);
        let file_system = InjectedFileSystem::new(&[], &[1, 2]);

        generate_rust_modules_with_fs(&input, &output, &file_system).unwrap_err();

        assert_eq!(snapshot_files(&output), output_before);
        assert_eq!(snapshot_files(&previous), previous_before);
        assert!(!output.with_file_name("output.staging").exists());
    }

    #[test]
    fn previous_cleanup_failure_keeps_published_output_and_recovers_next_run() {
        let temp = TempDirectory::new("previous-remove");
        let input = write_inputs(&temp.0, &[]);
        let output = existing_output(&temp.0);
        let file_system = InjectedFileSystem::new(&[], &[1, 2]);

        generate_rust_modules_with_fs(&input, &output, &file_system).unwrap();

        assert!(output.join("mod.rs").is_file());
        assert!(output.with_file_name("output.previous").exists());
        generate_rust_modules(&input.to_string_lossy(), &output.to_string_lossy()).unwrap();
        assert!(output.join("mod.rs").is_file());
        assert_no_publish_residue(&output);
    }
}
