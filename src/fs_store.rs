use std::{
    ffi::OsStr,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[cfg(unix)]
use std::collections::HashSet;

#[cfg(unix)]
use cap_std::fs::MetadataExt;
use cap_std::{
    ambient_authority,
    fs::{Dir, DirEntry, OpenOptions},
};
use serde::Serialize;

use crate::error::AppError;

#[derive(Clone)]
pub struct FileStore {
    root: Arc<Dir>,
    max_file_bytes: usize,
    mutations: Arc<Mutex<()>>,
    temp_counter: Arc<AtomicU64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Directory,
    File,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TreeEntry {
    pub path: String,
    pub kind: EntryKind,
}

impl FileStore {
    pub fn open(path: &Path, max_file_bytes: usize) -> Result<Self, AppError> {
        let root = Dir::open_ambient_dir(path, ambient_authority()).map_err(AppError::from_io)?;
        Ok(Self {
            root: Arc::new(root),
            max_file_bytes,
            mutations: Arc::new(Mutex::new(())),
            temp_counter: Arc::new(AtomicU64::new(0)),
        })
    }

    pub fn max_file_bytes(&self) -> usize {
        self.max_file_bytes
    }

    pub fn list(&self, raw_path: &str) -> Result<Vec<Entry>, AppError> {
        let path = validate_directory_path(raw_path)?;
        if !path.as_os_str().is_empty() {
            let metadata = self
                .root
                .symlink_metadata(&path)
                .map_err(AppError::from_io)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(AppError::Unsupported(
                    "Only regular directories can be opened",
                ));
            }
        }

        let directory = self
            .root
            .open_dir(path_or_dot(&path))
            .map_err(AppError::from_io)?;
        let mut entries = Vec::new();
        for result in directory.entries().map_err(AppError::from_io)? {
            let entry = result.map_err(AppError::from_io)?;
            let file_name = entry.file_name();
            let (name, utf8_name) = match file_name.to_str() {
                Some(name) => (name.to_owned(), true),
                None => (file_name.to_string_lossy().into_owned(), false),
            };
            let kind = if !utf8_name {
                EntryKind::Other
            } else {
                match entry.file_type() {
                    Ok(file_type) if file_type.is_dir() => EntryKind::Directory,
                    Ok(file_type) if file_type.is_file() => EntryKind::File,
                    _ => EntryKind::Other,
                }
            };
            entries.push(Entry { name, kind });
        }
        entries.sort_by(|left, right| {
            kind_rank(&left.kind)
                .cmp(&kind_rank(&right.kind))
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(entries)
    }

    pub fn list_tree(&self) -> Result<Vec<TreeEntry>, AppError> {
        let root = self.root.open_dir(".").map_err(AppError::from_io)?;
        let mut entries = Vec::new();
        let mut seen = SeenDirectories::default();
        collect_tree(&root, "", &mut entries, &mut seen)?;
        Ok(entries)
    }

    pub fn read_file(&self, raw_path: &str) -> Result<Vec<u8>, AppError> {
        let path = validate_entry_path(raw_path)?;
        reject_final_symlink_or_non_file(&self.root, &path)?;
        let mut file = self.root.open(&path).map_err(AppError::from_io)?;
        let metadata = file.metadata().map_err(AppError::from_io)?;
        if !metadata.is_file() {
            return Err(AppError::Unsupported("Only regular files can be opened"));
        }
        if metadata.len() > self.max_file_bytes as u64 {
            return Err(AppError::TooLarge);
        }
        let mut contents = Vec::with_capacity(metadata.len() as usize);
        Read::by_ref(&mut file)
            .take(self.max_file_bytes as u64 + 1)
            .read_to_end(&mut contents)
            .map_err(AppError::from_io)?;
        validate_text(&contents, self.max_file_bytes)?;
        Ok(contents)
    }

    pub fn write_file(&self, raw_path: &str, contents: &[u8]) -> Result<(), AppError> {
        validate_text(contents, self.max_file_bytes)?;
        let path = validate_entry_path(raw_path)?;
        let _guard = self.mutations.lock().expect("mutation lock poisoned");
        reject_final_symlink_or_non_file(&self.root, &path)?;

        let (parent, name) = split_parent_name(&path)?;
        let directory = self
            .root
            .open_dir(path_or_dot(parent))
            .map_err(AppError::from_io)?;
        let existing = directory.open(name).map_err(AppError::from_io)?;
        let permissions = existing
            .metadata()
            .map_err(AppError::from_io)?
            .permissions();
        drop(existing);

        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let (temp_name, mut temp) = (0..128)
            .find_map(|_| {
                let temp_name = self.unique_temp_name();
                match directory.open_with(&temp_name, &options) {
                    Ok(file) => Some(Ok((temp_name, file))),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => None,
                    Err(error) => Some(Err(AppError::from_io(error))),
                }
            })
            .unwrap_or_else(|| {
                Err(AppError::Io(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "could not allocate a temporary save file",
                )))
            })?;

        let result = (|| -> io::Result<()> {
            temp.set_permissions(permissions)?;
            temp.write_all(contents)?;
            temp.flush()?;
            temp.sync_all()?;
            drop(temp);
            directory.rename(&temp_name, &directory, name)?;
            sync_directory(&directory)
        })();

        if let Err(error) = result {
            let _ = directory.remove_file(&temp_name);
            return Err(AppError::from_io(error));
        }
        Ok(())
    }

    pub fn create_file(&self, raw_directory: &str, raw_name: &str) -> Result<String, AppError> {
        let directory_path = validate_directory_path(raw_directory)?;
        let name = validate_name(raw_name)?;
        let _guard = self.mutations.lock().expect("mutation lock poisoned");
        reject_final_symlink_or_non_directory(&self.root, &directory_path)?;
        let directory = self
            .root
            .open_dir(path_or_dot(&directory_path))
            .map_err(AppError::from_io)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let file = directory
            .open_with(name, &options)
            .map_err(AppError::from_io)?;
        file.sync_all().map_err(AppError::from_io)?;
        sync_directory(&directory).map_err(AppError::from_io)?;
        Ok(join_api_path(raw_directory, raw_name))
    }

    pub fn create_directory(
        &self,
        raw_directory: &str,
        raw_name: &str,
    ) -> Result<String, AppError> {
        let directory_path = validate_directory_path(raw_directory)?;
        let name = validate_name(raw_name)?;
        let _guard = self.mutations.lock().expect("mutation lock poisoned");
        reject_final_symlink_or_non_directory(&self.root, &directory_path)?;
        let directory = self
            .root
            .open_dir(path_or_dot(&directory_path))
            .map_err(AppError::from_io)?;
        directory.create_dir(name).map_err(AppError::from_io)?;
        sync_directory(&directory).map_err(AppError::from_io)?;
        Ok(join_api_path(raw_directory, raw_name))
    }

    pub fn rename_file(&self, raw_path: &str, raw_name: &str) -> Result<String, AppError> {
        let path = validate_entry_path(raw_path)?;
        let new_name = validate_name(raw_name)?;
        let _guard = self.mutations.lock().expect("mutation lock poisoned");
        reject_final_symlink_or_non_file(&self.root, &path)?;
        let (parent, old_name) = split_parent_name(&path)?;
        if old_name == OsStr::new(new_name) {
            return Ok(join_parent_and_name(parent, new_name));
        }
        let directory = self
            .root
            .open_dir(path_or_dot(parent))
            .map_err(AppError::from_io)?;
        rename_no_replace(&directory, old_name, OsStr::new(new_name))?;
        sync_directory(&directory).map_err(AppError::from_io)?;
        Ok(join_parent_and_name(parent, new_name))
    }

    pub fn rename_directory(&self, raw_path: &str, raw_name: &str) -> Result<String, AppError> {
        let path = validate_entry_path(raw_path)?;
        let new_name = validate_name(raw_name)?;
        let _guard = self.mutations.lock().expect("mutation lock poisoned");
        let metadata = self
            .root
            .symlink_metadata(&path)
            .map_err(AppError::from_io)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(AppError::Unsupported(
                "Only regular directories can be renamed",
            ));
        }
        let (parent, old_name) = split_parent_name(&path)?;
        if old_name == OsStr::new(new_name) {
            return Ok(join_parent_and_name(parent, new_name));
        }
        let directory = self
            .root
            .open_dir(path_or_dot(parent))
            .map_err(AppError::from_io)?;
        rename_no_replace(&directory, old_name, OsStr::new(new_name))?;
        sync_directory(&directory).map_err(AppError::from_io)?;
        Ok(join_parent_and_name(parent, new_name))
    }

    pub fn delete_file(&self, raw_path: &str) -> Result<(), AppError> {
        let path = validate_entry_path(raw_path)?;
        let _guard = self.mutations.lock().expect("mutation lock poisoned");
        reject_final_symlink_or_non_file(&self.root, &path)?;
        let (parent, name) = split_parent_name(&path)?;
        let directory = self
            .root
            .open_dir(path_or_dot(parent))
            .map_err(AppError::from_io)?;
        directory.remove_file(name).map_err(AppError::from_io)?;
        sync_directory(&directory).map_err(AppError::from_io)
    }

    pub fn delete_directory(&self, raw_path: &str) -> Result<(), AppError> {
        let path = validate_entry_path(raw_path)?;
        let _guard = self.mutations.lock().expect("mutation lock poisoned");
        let (parent, name) = split_parent_name(&path)?;
        let directory = self
            .root
            .open_dir(path_or_dot(parent))
            .map_err(AppError::from_io)?;
        let metadata = directory
            .symlink_metadata(name)
            .map_err(AppError::from_io)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(AppError::Unsupported(
                "Only regular directories can be deleted",
            ));
        }
        directory.remove_dir_all(name).map_err(AppError::from_io)?;
        sync_directory(&directory).map_err(AppError::from_io)
    }

    fn unique_temp_name(&self) -> String {
        let counter = self.temp_counter.fetch_add(1, Ordering::Relaxed);
        format!(".tpad-save-{}-{counter}.tmp", std::process::id())
    }
}

#[derive(Default)]
struct SeenDirectories {
    #[cfg(unix)]
    identities: HashSet<(u64, u64)>,
}

impl SeenDirectories {
    fn insert(&mut self, directory: &Dir) -> Result<bool, AppError> {
        #[cfg(unix)]
        {
            let metadata = directory.dir_metadata().map_err(AppError::from_io)?;
            Ok(self.identities.insert((metadata.dev(), metadata.ino())))
        }
        #[cfg(not(unix))]
        {
            let _ = directory;
            Ok(true)
        }
    }
}

fn collect_tree(
    directory: &Dir,
    parent_path: &str,
    output: &mut Vec<TreeEntry>,
    seen: &mut SeenDirectories,
) -> Result<(), AppError> {
    if !seen.insert(directory)? {
        return Ok(());
    }

    let mut children: Vec<(String, EntryKind, DirEntry)> = Vec::new();
    for result in directory.entries().map_err(AppError::from_io)? {
        let entry = result.map_err(AppError::from_io)?;
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let kind = if file_type.is_dir() {
            EntryKind::Directory
        } else if file_type.is_file() {
            EntryKind::File
        } else {
            // Symlinks and special files are deliberately not searchable.
            continue;
        };
        children.push((name, kind, entry));
    }
    children.sort_by(|left, right| {
        kind_rank(&left.1)
            .cmp(&kind_rank(&right.1))
            .then_with(|| left.0.to_lowercase().cmp(&right.0.to_lowercase()))
            .then_with(|| left.0.cmp(&right.0))
    });

    for (name, kind, entry) in children {
        let path = if parent_path.is_empty() {
            name
        } else {
            format!("{parent_path}/{name}")
        };
        output.push(TreeEntry {
            path: path.clone(),
            kind: kind.clone(),
        });
        if kind == EntryKind::Directory {
            // Opening through the directory entry keeps resolution relative to
            // this capability; cap-std rejects paths that escape the root.
            if let Ok(child_directory) = entry.open_dir() {
                collect_tree(&child_directory, &path, output, seen)?;
            }
        }
    }
    Ok(())
}

fn kind_rank(kind: &EntryKind) -> u8 {
    match kind {
        EntryKind::Directory => 0,
        EntryKind::File => 1,
        EntryKind::Other => 2,
    }
}

fn validate_text(contents: &[u8], max_file_bytes: usize) -> Result<(), AppError> {
    if contents.len() > max_file_bytes {
        return Err(AppError::TooLarge);
    }
    if contents.contains(&0) || std::str::from_utf8(contents).is_err() {
        return Err(AppError::Unsupported(
            "The file is not valid plain UTF-8 text",
        ));
    }
    Ok(())
}

fn validate_directory_path(raw: &str) -> Result<PathBuf, AppError> {
    if raw.is_empty() {
        return Ok(PathBuf::new());
    }
    validate_entry_path(raw)
}

fn validate_entry_path(raw: &str) -> Result<PathBuf, AppError> {
    if raw.is_empty()
        || raw.starts_with('/')
        || raw.starts_with('\\')
        || raw.contains('\\')
        || raw.contains('\0')
        || looks_like_windows_absolute(raw)
    {
        return Err(AppError::InvalidPath);
    }
    let components: Vec<&str> = raw.split('/').collect();
    if components
        .iter()
        .any(|component| component.is_empty() || *component == "." || *component == "..")
    {
        return Err(AppError::InvalidPath);
    }
    Ok(components.iter().collect())
}

fn validate_name(raw: &str) -> Result<&str, AppError> {
    if raw.is_empty()
        || raw == "."
        || raw == ".."
        || raw.contains('/')
        || raw.contains('\\')
        || raw.contains('\0')
    {
        return Err(AppError::InvalidName);
    }
    Ok(raw)
}

fn looks_like_windows_absolute(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn path_or_dot(path: &Path) -> &Path {
    if path.as_os_str().is_empty() {
        Path::new(".")
    } else {
        path
    }
}

fn split_parent_name(path: &Path) -> Result<(&Path, &OsStr), AppError> {
    let name = path.file_name().ok_or(AppError::InvalidPath)?;
    Ok((path.parent().unwrap_or_else(|| Path::new("")), name))
}

fn join_api_path(directory: &str, name: &str) -> String {
    if directory.is_empty() {
        name.to_owned()
    } else {
        format!("{directory}/{name}")
    }
}

fn join_parent_and_name(parent: &Path, name: &str) -> String {
    match parent.to_str() {
        Some("") | None => name.to_owned(),
        Some(parent) => format!("{parent}/{name}"),
    }
}

fn reject_final_symlink_or_non_file(root: &Dir, path: &Path) -> Result<(), AppError> {
    let metadata = root.symlink_metadata(path).map_err(AppError::from_io)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AppError::Unsupported("Only regular files can be edited"));
    }
    Ok(())
}

fn reject_final_symlink_or_non_directory(root: &Dir, path: &Path) -> Result<(), AppError> {
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    let metadata = root.symlink_metadata(path).map_err(AppError::from_io)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AppError::Unsupported(
            "Only regular directories can be used",
        ));
    }
    Ok(())
}

fn sync_directory(directory: &Dir) -> io::Result<()> {
    directory.open(".")?.sync_all()
}

#[cfg(unix)]
fn rename_no_replace(directory: &Dir, old_name: &OsStr, new_name: &OsStr) -> Result<(), AppError> {
    use rustix::fs::{RenameFlags, renameat_with};
    renameat_with(
        directory,
        old_name,
        directory,
        new_name,
        RenameFlags::NOREPLACE,
    )
    .map_err(|error| {
        let io_error = io::Error::from_raw_os_error(error.raw_os_error());
        AppError::from_io(io_error)
    })
}

#[cfg(not(unix))]
fn rename_no_replace(directory: &Dir, old_name: &OsStr, new_name: &OsStr) -> Result<(), AppError> {
    if directory.symlink_metadata(new_name).is_ok() {
        return Err(AppError::Conflict("An entry with that name already exists"));
    }
    directory
        .rename(old_name, directory, new_name)
        .map_err(AppError::from_io)
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::Arc, thread};

    use tempfile::TempDir;

    use super::*;

    fn store(temp: &TempDir) -> FileStore {
        FileStore::open(temp.path(), 64).unwrap()
    }

    #[test]
    fn rejects_unsafe_paths() {
        for path in [
            "",
            "/etc/passwd",
            "../secret",
            "a/../b",
            "a//b",
            "a\\b",
            "C:/x",
        ] {
            assert!(validate_entry_path(path).is_err(), "accepted {path:?}");
        }
        assert!(validate_directory_path("").is_ok());
        assert!(validate_entry_path("projects/app.txt").is_ok());
        assert!(validate_entry_path("%2e%2e/secret").is_ok());
    }

    #[test]
    fn lists_directories_then_files() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("z.txt"), "z").unwrap();
        fs::write(temp.path().join("A.txt"), "a").unwrap();
        fs::create_dir(temp.path().join("projects")).unwrap();
        let entries = store(&temp).list("").unwrap();
        assert_eq!(entries[0].name, "projects");
        assert_eq!(entries[1].name, "A.txt");
        assert_eq!(entries[2].name, "z.txt");
    }

    #[test]
    fn lists_tree_recursively_with_relative_paths_and_directory_first_order() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("projects/app")).unwrap();
        fs::write(temp.path().join("projects/app/notes.txt"), "notes").unwrap();
        fs::write(temp.path().join("ideas.txt"), "ideas").unwrap();
        fs::create_dir(temp.path().join("archive")).unwrap();

        let entries = store(&temp).list_tree().unwrap();
        let paths: Vec<_> = entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry.kind.clone()))
            .collect();
        assert_eq!(
            paths,
            [
                ("archive", EntryKind::Directory),
                ("projects", EntryKind::Directory),
                ("projects/app", EntryKind::Directory),
                ("projects/app/notes.txt", EntryKind::File),
                ("ideas.txt", EntryKind::File),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn tree_omits_symlinks_and_does_not_follow_cycles_or_external_targets() {
        let temp = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("inside")).unwrap();
        fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        std::os::unix::fs::symlink(".", temp.path().join("inside/loop")).unwrap();
        std::os::unix::fs::symlink(outside.path(), temp.path().join("external")).unwrap();

        let entries = store(&temp).list_tree().unwrap();
        let paths: Vec<_> = entries.iter().map(|entry| entry.path.as_str()).collect();
        assert_eq!(paths, ["inside"]);
    }

    #[test]
    fn creates_reads_writes_renames_and_deletes_utf8() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        store.create_directory("", "projects").unwrap();
        assert_eq!(
            store.create_file("projects", "cider.md").unwrap(),
            "projects/cider.md"
        );
        let text = "Živjo\n  whitespace\n".as_bytes();
        store.write_file("projects/cider.md", text).unwrap();
        assert_eq!(store.read_file("projects/cider.md").unwrap(), text);
        assert_eq!(
            store.rename_file("projects/cider.md", "tpad.conf").unwrap(),
            "projects/tpad.conf"
        );
        store.delete_file("projects/tpad.conf").unwrap();
        store.delete_directory("projects").unwrap();
    }

    #[test]
    fn renames_directories_without_replacing_and_preserves_contents() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        store.create_directory("", "projects").unwrap();
        store.create_directory("projects", "app").unwrap();
        store.create_file("projects/app", "notes.txt").unwrap();
        store
            .write_file("projects/app/notes.txt", "plain text".as_bytes())
            .unwrap();
        store.create_directory("", "archive").unwrap();

        assert_eq!(store.rename_directory("projects", "work").unwrap(), "work");
        assert_eq!(
            store.read_file("work/app/notes.txt").unwrap(),
            b"plain text"
        );
        assert!(matches!(
            store.rename_directory("work", "archive"),
            Err(AppError::Conflict(_))
        ));
        assert!(temp.path().join("work/app/notes.txt").is_file());
        assert!(temp.path().join("archive").is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_rename_a_symlink_as_a_directory() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("target")).unwrap();
        std::os::unix::fs::symlink("target", temp.path().join("alias")).unwrap();

        assert!(matches!(
            store(&temp).rename_directory("alias", "renamed"),
            Err(AppError::Unsupported(_))
        ));
        assert!(temp.path().join("alias").is_symlink());
        assert!(temp.path().join("target").is_dir());
    }

    #[test]
    fn rejects_binary_oversized_and_overwriting_rename() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("binary"), [0xff, 0xfe]).unwrap();
        fs::write(temp.path().join("nul"), b"a\0b").unwrap();
        fs::write(temp.path().join("large"), vec![b'x'; 65]).unwrap();
        fs::write(temp.path().join("a"), "a").unwrap();
        fs::write(temp.path().join("b"), "b").unwrap();
        let store = store(&temp);
        assert!(matches!(
            store.read_file("binary"),
            Err(AppError::Unsupported(_))
        ));
        assert!(matches!(
            store.read_file("nul"),
            Err(AppError::Unsupported(_))
        ));
        assert!(matches!(store.read_file("large"), Err(AppError::TooLarge)));
        assert!(matches!(
            store.rename_file("a", "b"),
            Err(AppError::Conflict(_))
        ));
        assert_eq!(fs::read_to_string(temp.path().join("a")).unwrap(), "a");
        assert_eq!(fs::read_to_string(temp.path().join("b")).unwrap(), "b");
        assert!(matches!(
            store.write_file("a", b"x\0y"),
            Err(AppError::Unsupported(_))
        ));
        assert_eq!(fs::read_to_string(temp.path().join("a")).unwrap(), "a");
    }

    #[test]
    fn concurrent_saves_are_complete() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("note.txt"), "start").unwrap();
        let store = Arc::new(store(&temp));
        let payloads = [b"aaaaaaaaaaaaaaaa".to_vec(), b"bbbbbbbbbbbbbbbb".to_vec()];
        let handles: Vec<_> = payloads
            .clone()
            .into_iter()
            .map(|payload| {
                let store = Arc::clone(&store);
                thread::spawn(move || store.write_file("note.txt", &payload).unwrap())
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        let result = fs::read(temp.path().join("note.txt")).unwrap();
        assert!(payloads.contains(&result));
    }

    #[test]
    fn concurrent_creation_has_one_winner() {
        let temp = TempDir::new().unwrap();
        let store = Arc::new(store(&temp));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let store = Arc::clone(&store);
                thread::spawn(move || store.create_file("", "same.txt"))
            })
            .collect();
        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(AppError::Conflict(_))))
                .count(),
            3
        );
    }

    #[test]
    fn recursively_deletes_directory_contents() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("folder/nested")).unwrap();
        fs::write(temp.path().join("folder/note.txt"), "text").unwrap();
        fs::write(temp.path().join("folder/nested/other.txt"), "more text").unwrap();

        store(&temp).delete_directory("folder").unwrap();

        assert!(!temp.path().join("folder").exists());
    }

    #[cfg(unix)]
    #[test]
    fn recursive_delete_does_not_follow_nested_symlinks() {
        use std::os::unix::fs::symlink;
        let temp = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("folder/nested")).unwrap();
        fs::write(outside.path().join("keep.txt"), "keep me").unwrap();
        symlink(outside.path(), temp.path().join("folder/nested/outside")).unwrap();

        store(&temp).delete_directory("folder").unwrap();

        assert!(!temp.path().join("folder").exists());
        assert_eq!(
            fs::read_to_string(outside.path().join("keep.txt")).unwrap(),
            "keep me"
        );
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinks_including_escape() {
        use std::os::unix::fs::symlink;
        let temp = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        fs::write(outside.path().join("secret"), "secret").unwrap();
        symlink(outside.path().join("secret"), temp.path().join("escape")).unwrap();
        symlink(outside.path(), temp.path().join("escape-dir")).unwrap();
        let store = store(&temp);
        assert!(matches!(
            store.read_file("escape"),
            Err(AppError::Unsupported(_))
        ));
        assert!(matches!(
            store.write_file("escape", b"changed"),
            Err(AppError::Unsupported(_))
        ));
        assert!(matches!(
            store.delete_directory("escape-dir"),
            Err(AppError::Unsupported(_))
        ));
        assert!(store.read_file("escape-dir/secret").is_err());
        assert!(
            store
                .create_file("escape-dir", "created-outside.txt")
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(outside.path().join("secret")).unwrap(),
            "secret"
        );
        assert!(!outside.path().join("created-outside.txt").exists());
    }
}
