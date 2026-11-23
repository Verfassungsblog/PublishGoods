//! Git versioning of templates.
//!
//! Every template directory (`<data_path>/templates/<id>`) is the working tree of its own git
//! repository, which is kept outside of it (`<data_path>/template-git/<id>.git`): the template
//! directory is served to the renderer and edited through the API, the repository must be in
//! neither. Each mutating template route calls [`commit`] after it changed something; that
//! commits the whole directory, plus a snapshot of the template's database metadata as
//! `template.json`, which exists only in the repository: the directory is what the renderer gets,
//! and it has no use for the metadata. Commits always use a fixed system identity and carry no user data, as the
//! repositories are meant to be pushed to external remotes later.

use crate::db::repositories::{DbError, templates};
use crate::settings::Settings;
use dashmap::DashMap;
use git2::{
    ErrorCode, FileMode, Odb, Oid, Repository, RepositoryInitOptions, Signature, TreeBuilder,
};
use log::{error, info, warn};
use sqlx::PgPool;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;
use vb_exchange::export_formats::ExportFormat;

const AUTHOR_NAME: &str = "PublishGoods";
const AUTHOR_EMAIL: &str = "templates@publishgoods.invalid";
/// Name of the metadata snapshot at the root of each template repository.
const METADATA_FILE: &str = "template.json";
const BRANCH: &str = "main";
/// Name of the [`StatCache`] in the git dir.
const STAT_CACHE_FILE: &str = "stat-cache.json";
/// How much older than the commit a file's timestamps must be for its hash to be cached. A file
/// rewritten within the file system's timestamp granularity keeps its timestamps, so a younger
/// one could change later without its stat showing it.
const RACY_WINDOW: Duration = Duration::from_secs(2);

/// Whether a single path component is (or, on NTFS/HFS, would be) the `.git` directory itself.
/// Such paths are never committed (libgit2 refuses them, and a nested repository must not end up
/// in the history), so they can't be versioned: the API doesn't accept them as new names.
pub fn is_git_dir_name(component: &str) -> bool {
    // Windows file systems drop trailing dots and spaces, so `.git.` is `.git` there.
    let name = component.trim_end_matches(['.', ' ']);
    name.eq_ignore_ascii_case(".git") || name.eq_ignore_ascii_case("git~1")
}

/// One lock per template, see [`lock`]. Entries are never removed: a removed mutex could still be
/// held while a later [`lock`] creates a new one, and an entry is only a few bytes.
static LOCKS: LazyLock<DashMap<Uuid, Arc<tokio::sync::Mutex<()>>>> = LazyLock::new(DashMap::new);

/// Exclusive access to the files and the repository of one template, see [`lock`].
pub struct TemplateLock {
    _guard: tokio::sync::OwnedMutexGuard<()>,
}

/// Locks the template: until the returned guard is dropped, no other request changes its files
/// or commits to its repository. Mutating routes take the lock *before* they change anything
/// and hand it to [`commit`], so a commit only ever sees the files of its own request, under
/// that request's message. The lock isn't reentrant: [`commit`] takes the guard instead of
/// locking itself. [`commit`] moves the guard into the blocking task that writes the commit,
/// so it is held until libgit2 is done even if the request is cancelled meanwhile.
pub async fn lock(template_id: Uuid) -> TemplateLock {
    let mutex = LOCKS.entry(template_id).or_default().clone();
    TemplateLock {
        _guard: mutex.lock_owned().await,
    }
}

#[derive(Debug)]
enum GitError {
    Git(git2::Error),
    Io(std::io::Error),
    Db(DbError),
    Json(serde_json::Error),
    Join(tokio::task::JoinError),
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitError::Git(e) => write!(f, "git error: {e}"),
            GitError::Io(e) => write!(f, "io error: {e}"),
            GitError::Db(e) => write!(f, "database error: {e:?}"),
            GitError::Json(e) => write!(f, "json error: {e}"),
            GitError::Join(e) => write!(f, "task error: {e}"),
        }
    }
}

impl std::error::Error for GitError {}

impl From<git2::Error> for GitError {
    fn from(e: git2::Error) -> Self {
        GitError::Git(e)
    }
}
impl From<std::io::Error> for GitError {
    fn from(e: std::io::Error) -> Self {
        GitError::Io(e)
    }
}
impl From<DbError> for GitError {
    fn from(e: DbError) -> Self {
        GitError::Db(e)
    }
}
impl From<serde_json::Error> for GitError {
    fn from(e: serde_json::Error) -> Self {
        GitError::Json(e)
    }
}
impl From<tokio::task::JoinError> for GitError {
    fn from(e: tokio::task::JoinError) -> Self {
        GitError::Join(e)
    }
}

/// The part of a template that lives in the database, as committed to `template.json`.
/// Leaves out `version` (changes on every edit) and `owner_team_id` (internal), and orders the
/// export formats, so the file only changes when the template actually does.
#[derive(serde::Serialize)]
struct TemplateSnapshot<'a> {
    id: Uuid,
    name: &'a str,
    description: &'a str,
    export_formats: BTreeMap<&'a str, &'a ExportFormat>,
}

/// Records the current state of the template in its repository, as a commit with `message`.
/// `lock` is the template's [`lock`], taken before the change that is recorded. Failures are
/// logged, not returned: the files are already changed at this point, and the next commit picks
/// up whatever this one missed.
pub async fn commit(
    pool: &PgPool,
    settings: &Settings,
    template_id: Uuid,
    lock: TemplateLock,
    message: &str,
) {
    if let Err(e) = try_commit(pool, settings, template_id, lock, message).await {
        error!("Couldn't commit template {template_id} ({message}): {e}");
    }
}

/// Like [`commit`], but returns the new commit's id (`None` if nothing changed) and the error.
async fn try_commit(
    pool: &PgPool,
    settings: &Settings,
    template_id: Uuid,
    lock: TemplateLock,
    message: &str,
) -> Result<Option<Oid>, GitError> {
    let template = templates::get(pool, template_id).await?;
    let snapshot = TemplateSnapshot {
        id: template.id,
        name: &template.name,
        description: &template.description,
        export_formats: template
            .export_formats
            .iter()
            .map(|(slug, format)| (slug.as_str(), format))
            .collect(),
    };
    let json = serde_json::to_string_pretty(&snapshot)?;

    let workdir = settings.template_dir(template_id);
    let git_dir = settings.template_git_dir(template_id);
    let message = message.to_string();
    // The guard lives in the blocking task: dropping this future (client disconnect, shutdown)
    // must not release the lock while the task is still writing to the repository.
    tokio::task::spawn_blocking(move || {
        let _lock = lock;
        std::fs::create_dir_all(&workdir)?;
        commit_dir(
            &workdir,
            &git_dir,
            &message,
            &[(METADATA_FILE, json.as_bytes())],
        )
    })
    .await?
}

/// Commits the files in `workdir` to the repository in `git_dir` (initializing it first if there
/// is none), together with `root_files` (name and content), which are added to the root of the
/// tree without ever being written to `workdir`; they replace a file of the same name in
/// `workdir`. Returns `None`, without committing, if nothing changed.
///
/// The tree is built from the bytes of the files, not through git's index and working tree
/// machinery: file contents are committed exactly as they are, whatever `.gitignore`,
/// `.gitattributes` or the server user's git config (`core.autocrlf`, ...) say.
fn commit_dir(
    workdir: &Path,
    git_dir: &Path,
    message: &str,
    root_files: &[(&str, &[u8])],
) -> Result<Option<Oid>, GitError> {
    // `open`, not `discover`: never pick up a repository further up the tree. The repository is
    // bare, so nothing is ever written into `workdir`.
    let repo = match Repository::open(git_dir) {
        Ok(repo) => repo,
        Err(e) if e.code() == ErrorCode::NotFound => {
            let mut options = RepositoryInitOptions::new();
            options.initial_head(BRANCH).bare(true);
            Repository::init_opts(git_dir, &options)?
        }
        Err(e) => return Err(e.into()),
    };

    let mut cache = StatCache::load(git_dir);
    let odb = repo.odb()?;
    // Unlike any other tree, the root of a commit may be empty. `insert` replaces a workdir file
    // of the same name.
    let mut root = build_tree(&repo, &odb, workdir, Some(""), &mut cache)?;
    for (name, content) in root_files {
        root.insert(name, repo.blob(content)?, FileMode::Blob.into())?;
    }
    let tree_id = root.write()?;
    // The blobs are written by now, so the cache never points at a missing object, whether or
    // not a commit follows.
    if let Err(e) = cache.save(git_dir) {
        warn!("Couldn't save the stat cache of {git_dir:?}: {e}");
    }

    let parent = match repo.head() {
        Ok(head) => Some(head.peel_to_commit()?),
        Err(e) if matches!(e.code(), ErrorCode::UnbornBranch | ErrorCode::NotFound) => None,
        Err(e) => return Err(e.into()),
    };
    if parent.as_ref().is_some_and(|p| p.tree_id() == tree_id) {
        return Ok(None);
    }

    let tree = repo.find_tree(tree_id)?;
    let signature = Signature::now(AUTHOR_NAME, AUTHOR_EMAIL)?;
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    let oid = repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        message,
        &tree,
        &parents,
    )?;
    Ok(Some(oid))
}

/// What a file's metadata says about its content: if none of it changed, neither did the content.
/// `ctime` and `ino` catch what `mtime` and `size` miss: a rewrite that restores the old mtime
/// (`cp -p`, `touch -r`), or another file renamed over this one.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct FileStat {
    size: u64,
    mtime_ns: i128,
    ctime_ns: i128,
    ino: u64,
    dev: u64,
}

impl FileStat {
    #[cfg(unix)]
    fn of(metadata: &std::fs::Metadata) -> Option<FileStat> {
        use std::os::unix::fs::MetadataExt;
        let ns = |secs: i64, nanos: i64| i128::from(secs) * 1_000_000_000 + i128::from(nanos);
        Some(FileStat {
            size: metadata.size(),
            mtime_ns: ns(metadata.mtime(), metadata.mtime_nsec()),
            ctime_ns: ns(metadata.ctime(), metadata.ctime_nsec()),
            ino: metadata.ino(),
            dev: metadata.dev(),
        })
    }

    /// Without a ctime and an inode, the stat can't tell whether a file changed.
    #[cfg(not(unix))]
    fn of(_metadata: &std::fs::Metadata) -> Option<FileStat> {
        None
    }
}

#[derive(PartialEq, serde::Serialize, serde::Deserialize)]
struct CachedBlob {
    // not flattened: serde can't deserialize a flattened i128
    stat: FileStat,
    oid: String,
}

/// The blob ids of the files committed last time, with their [`FileStat`], so a commit only
/// reads and hashes the files that changed since. Kept as [`STAT_CACHE_FILE`] in the git dir, so
/// it goes with the repository. It is only ever an optimization: a missing or broken cache just
/// means every file is hashed.
struct StatCache {
    old: HashMap<String, CachedBlob>,
    new: HashMap<String, CachedBlob>,
    /// Files whose timestamps are younger than this aren't cached, see [`RACY_WINDOW`].
    cutoff_ns: i128,
}

impl StatCache {
    fn load(git_dir: &Path) -> StatCache {
        let path = git_dir.join(STAT_CACHE_FILE);
        let old = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                warn!("Ignoring broken stat cache {path:?}: {e}");
                HashMap::new()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => {
                warn!("Couldn't read stat cache {path:?}: {e}");
                HashMap::new()
            }
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        StatCache {
            old,
            new: HashMap::new(),
            cutoff_ns: now.saturating_sub(RACY_WINDOW).as_nanos() as i128,
        }
    }

    /// The blob id of the file at `path` (relative to the working tree), if it is unchanged since
    /// it was cached and its blob is still in `odb`.
    fn get(&self, odb: &Odb, path: &str, stat: &FileStat) -> Option<Oid> {
        let cached = self.old.get(path).filter(|cached| cached.stat == *stat)?;
        let oid = Oid::from_str(&cached.oid).ok()?;
        odb.exists(oid).then_some(oid)
    }

    /// Remembers `oid` for the next commit, unless the file is too young, see [`RACY_WINDOW`].
    fn insert(&mut self, path: String, stat: FileStat, oid: Oid) {
        if stat.mtime_ns < self.cutoff_ns && stat.ctime_ns < self.cutoff_ns {
            let oid = oid.to_string();
            self.new.insert(path, CachedBlob { stat, oid });
        }
    }

    /// Replaces the cache in `git_dir` with the files seen since [`StatCache::load`], so deleted
    /// files drop out. Skips the write if that is what it already holds.
    fn save(&self, git_dir: &Path) -> Result<(), GitError> {
        if self.new == self.old {
            return Ok(());
        }
        let path = git_dir.join(STAT_CACHE_FILE);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&self.new)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

/// Writes the files below `dir` as git objects and returns the tree for `dir`, not yet written,
/// so the caller can still add entries. Subdirectories without entries are left out (git doesn't
/// track empty directories). Symlinks and other special files are left out too: following a link
/// could pull files from outside of the template into its history. So are `.git`-like names, see
/// [`is_git_dir_name`].
///
/// `prefix` is the path of `dir` in the tree (`""` for the root), the key of its files in
/// `cache`; `None` if it isn't valid UTF-8, which leaves its files uncached.
fn build_tree<'r>(
    repo: &'r Repository,
    odb: &Odb,
    dir: &Path,
    prefix: Option<&str>,
    cache: &mut StatCache,
) -> Result<TreeBuilder<'r>, GitError> {
    let mut builder = repo.treebuilder(None)?;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_str().is_some_and(is_git_dir_name) {
            continue;
        }
        let file_type = entry.file_type()?;
        let path = entry.path();
        let key = prefix
            .zip(name.to_str())
            .map(|(prefix, name)| match prefix {
                "" => name.to_string(),
                _ => format!("{prefix}/{name}"),
            });
        let (id, mode) = if file_type.is_dir() {
            let subtree = build_tree(repo, odb, &path, key.as_deref(), cache)?;
            if subtree.is_empty() {
                continue;
            }
            (subtree.write()?, FileMode::Tree)
        } else if file_type.is_file() {
            // Stat before reading: if the file changes in between, the cached stat is the old
            // one, and the next commit hashes it again.
            let metadata = entry.metadata()?;
            let stat = key.zip(FileStat::of(&metadata));
            let cached = stat
                .as_ref()
                .and_then(|(key, stat)| cache.get(odb, key, stat));
            let id = match cached {
                Some(id) => id,
                None => repo.blob(&std::fs::read(&path)?)?,
            };
            if let Some((key, stat)) = stat {
                cache.insert(key, stat, id);
            }
            (
                id,
                if is_executable(&metadata) {
                    FileMode::BlobExecutable
                } else {
                    FileMode::Blob
                },
            )
        } else {
            continue;
        };
        // One name libgit2 refuses mustn't stop the template from being versioned at all.
        if let Err(e) = builder.insert(&name, id, mode.into()) {
            warn!("Not committing {path:?}: {e}");
        }
    }
    Ok(builder)
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    false
}

/// Removes the lock files (`*.lock`, anywhere in it) a crashed process left in the repository in
/// `git_dir`, which would block all later commits. Returns whether there were any. Only call this
/// when no commit can be running.
fn remove_stale_locks(git_dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(git_dir) else {
        return false;
    };
    let mut removed = false;
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(file_type) if file_type.is_dir() => removed |= remove_stale_locks(&path),
            Ok(_) if path.extension().is_some_and(|ext| ext == "lock") => {
                removed |= std::fs::remove_file(&path).is_ok();
            }
            _ => {}
        }
    }
    removed
}

/// Gives every template that already has files but no repository yet an initial commit, so the
/// first real change doesn't end up bundled with the whole pre-versioning state. Also removes
/// lock files left behind by a crashed process, which would block all later commits. Meant to run
/// once at startup, before requests are served. Files of templates that aren't in the database are
/// deliberately left alone: an empty or wrong database must not cost the template files.
pub async fn import_existing(pool: &PgPool, settings: &Settings) {
    let ids = match templates::list_ids(pool).await {
        Ok(ids) => ids,
        Err(e) => {
            error!("Couldn't list templates for the initial git import: {e:?}");
            return;
        }
    };
    // All file system checks in one blocking task. Nothing is served yet, so no commit can be
    // running and every lock file is stale.
    let blocking_settings = settings.clone();
    let unversioned = tokio::task::spawn_blocking(move || {
        ids.into_iter()
            .filter(|&id| {
                let git_dir = blocking_settings.template_git_dir(id);
                if remove_stale_locks(&git_dir) {
                    info!("Removed stale git locks of template {id}");
                }
                !git_dir.exists() && blocking_settings.template_dir(id).exists()
            })
            .collect::<Vec<_>>()
    })
    .await;
    let unversioned = match unversioned {
        Ok(ids) => ids,
        Err(e) => {
            error!("Couldn't check templates for the initial git import: {e}");
            return;
        }
    };
    for id in unversioned {
        match try_commit(pool, settings, id, lock(id).await, "Initial import").await {
            Ok(Some(_)) => info!("Created git repository for template {id}"),
            Ok(None) => {}
            Err(e) => error!("Couldn't import template {id} into git: {e}"),
        }
    }
}

/// Deletes the files and the repository of templates that were deleted from the database (a
/// team's templates go with the team). Runs in the background: it waits for each template's
/// lock, so a request that deletes a team or an account shouldn't be held up by an upload to one
/// of the templates. Failures are logged: the templates are gone either way, only their files
/// linger.
pub fn remove_files_in_background(settings: &Settings, template_ids: Vec<Uuid>) {
    if template_ids.is_empty() {
        return;
    }
    let settings = settings.clone();
    tokio::spawn(async move {
        for id in template_ids {
            let _lock = lock(id).await;
            for path in [settings.template_dir(id), settings.template_git_dir(id)] {
                match tokio::fs::remove_dir_all(&path).await {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => error!("Couldn't delete {path:?} of deleted template {id}: {e}"),
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A fresh directory under the system temp dir, removed on drop.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("vb-template-git-{}", Uuid::new_v4()));
            std::fs::create_dir_all(path.join("work")).unwrap();
            TempDir(path)
        }
        /// The working tree.
        fn work(&self) -> PathBuf {
            self.0.join("work")
        }
        /// The repository, next to the working tree.
        fn git(&self) -> PathBuf {
            self.0.join("repo.git")
        }
        fn commit(&self, message: &str) -> Option<Oid> {
            commit_dir(&self.work(), &self.git(), message, &[]).unwrap()
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn log(dir: &Path) -> Vec<(String, String)> {
        let repo = Repository::open(dir).unwrap();
        let mut walk = repo.revwalk().unwrap();
        walk.push_head().unwrap();
        walk.map(|oid| {
            let commit = repo.find_commit(oid.unwrap()).unwrap();
            (
                commit.message().unwrap().to_string(),
                commit.author().email().unwrap().to_string(),
            )
        })
        .collect()
    }

    #[test]
    fn first_commit_initializes_repository_on_main() {
        let tmp = TempDir::new();
        std::fs::write(tmp.work().join("a.txt"), "a").unwrap();
        assert!(tmp.commit("first").is_some());
        let repo = Repository::open(tmp.git()).unwrap();
        assert_eq!(repo.head().unwrap().shorthand().unwrap(), "main");
        assert_eq!(log(&tmp.git()), vec![("first".into(), AUTHOR_EMAIL.into())]);
        // nothing but the files themselves in the working tree
        let entries: Vec<_> = std::fs::read_dir(tmp.work())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("a.txt")]);
    }

    #[test]
    fn unchanged_tree_makes_no_commit() {
        let tmp = TempDir::new();
        std::fs::write(tmp.work().join("a.txt"), "a").unwrap();
        tmp.commit("first");
        assert!(tmp.commit("second").is_none());
        assert_eq!(log(&tmp.git()).len(), 1);
    }

    #[test]
    fn changes_deletions_and_renames_are_committed() {
        let tmp = TempDir::new();
        std::fs::create_dir(tmp.work().join("assets")).unwrap();
        std::fs::write(tmp.work().join("assets/a.txt"), "a").unwrap();
        tmp.commit("add");

        std::fs::write(tmp.work().join("assets/a.txt"), "changed").unwrap();
        assert!(tmp.commit("edit").is_some());

        std::fs::rename(
            tmp.work().join("assets/a.txt"),
            tmp.work().join("assets/b.txt"),
        )
        .unwrap();
        assert!(tmp.commit("rename").is_some());

        std::fs::remove_file(tmp.work().join("assets/b.txt")).unwrap();
        assert!(tmp.commit("delete").is_some());

        let repo = Repository::open(tmp.git()).unwrap();
        let tree = repo.head().unwrap().peel_to_tree().unwrap();
        assert!(tree.get_path(Path::new("assets/a.txt")).is_err());
        assert!(tree.get_path(Path::new("assets/b.txt")).is_err());
        assert_eq!(log(&tmp.git()).len(), 4);
    }

    /// The content of `path` in the latest commit.
    fn committed(tmp: &TempDir, path: &str) -> Option<Vec<u8>> {
        let repo = Repository::open(tmp.git()).unwrap();
        let tree = repo.head().unwrap().peel_to_tree().unwrap();
        let entry = tree.get_path(Path::new(path)).ok()?;
        Some(repo.find_blob(entry.id()).unwrap().content().to_vec())
    }

    #[test]
    fn git_special_files_are_ordinary_template_files() {
        let tmp = TempDir::new();
        std::fs::create_dir(tmp.work().join("assets")).unwrap();
        std::fs::write(tmp.work().join(".gitignore"), "*\n").unwrap();
        std::fs::write(tmp.work().join("assets/.gitignore"), "*\n").unwrap();
        std::fs::write(tmp.work().join("assets/.gitmodules"), "m").unwrap();
        std::fs::write(tmp.work().join("assets/preview.pdf"), "pdf").unwrap();
        tmp.commit("add");
        assert_eq!(committed(&tmp, ".gitignore").unwrap(), b"*\n");
        assert_eq!(committed(&tmp, "assets/.gitignore").unwrap(), b"*\n");
        assert_eq!(committed(&tmp, "assets/.gitmodules").unwrap(), b"m");
        assert_eq!(committed(&tmp, "assets/preview.pdf").unwrap(), b"pdf");
    }

    #[test]
    fn contents_are_committed_unchanged() {
        let tmp = TempDir::new();
        // attributes inside the template and the repository's config (which stands in for the
        // global one) must not convert line endings or anything else
        std::fs::write(tmp.work().join(".gitattributes"), "* text eol=lf\n").unwrap();
        std::fs::write(tmp.work().join("crlf.txt"), "a\r\nb\r\n").unwrap();
        std::fs::write(tmp.work().join("binary.bin"), [0u8, 159, 146, 150, 13, 10]).unwrap();
        tmp.commit("init");
        let repo = Repository::open(tmp.git()).unwrap();
        repo.config()
            .unwrap()
            .set_str("core.autocrlf", "true")
            .unwrap();
        repo.config()
            .unwrap()
            .set_bool("core.safecrlf", true)
            .unwrap();
        std::fs::write(tmp.work().join("crlf2.txt"), "c\r\nd\r\n").unwrap();
        assert!(tmp.commit("second").is_some());

        for (path, content) in [
            ("crlf.txt", &b"a\r\nb\r\n"[..]),
            ("crlf2.txt", b"c\r\nd\r\n"),
            ("binary.bin", &[0, 159, 146, 150, 13, 10]),
        ] {
            assert_eq!(committed(&tmp, path).unwrap(), content, "{path}");
        }
    }

    #[test]
    fn only_regular_files_are_committed() {
        let tmp = TempDir::new();
        std::fs::create_dir_all(tmp.work().join("empty/nested")).unwrap();
        std::fs::write(tmp.work().join("a.txt"), "a").unwrap();
        #[cfg(unix)]
        {
            std::fs::write(tmp.0.join("secret"), "secret").unwrap();
            std::os::unix::fs::symlink(tmp.0.join("secret"), tmp.work().join("link")).unwrap();
        }
        tmp.commit("add");
        assert!(committed(&tmp, "a.txt").is_some());
        assert!(committed(&tmp, "link").is_none());
        assert!(committed(&tmp, "empty").is_none());
    }

    #[test]
    fn an_empty_template_directory_is_committed_as_an_empty_tree() {
        let tmp = TempDir::new();
        assert!(tmp.commit("empty").is_some());
        assert!(tmp.commit("again").is_none());
    }

    #[test]
    fn doesnt_use_a_repository_further_up() {
        let tmp = TempDir::new();
        let outer = Repository::init(&tmp.0).unwrap();
        std::fs::write(tmp.work().join("a.txt"), "a").unwrap();
        tmp.commit("first");
        assert_eq!(log(&tmp.git()).len(), 1);
        assert!(outer.head().is_err(), "the outer repository got a commit");
    }

    #[tokio::test]
    async fn the_lock_is_exclusive_per_template() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let held = lock(a).await;
        let wait = |id| tokio::time::timeout(Duration::from_millis(100), lock(id));
        assert!(wait(a).await.is_err(), "locked twice");
        assert!(wait(b).await.is_ok(), "templates block each other");
        drop(held);
        assert!(wait(a).await.is_ok());
    }

    #[test]
    fn root_files_are_committed_but_not_written() {
        let tmp = TempDir::new();
        std::fs::write(tmp.work().join("a.txt"), "a").unwrap();
        // replaced by the root file of the same name, in the root only
        std::fs::write(tmp.work().join("meta.json"), "old").unwrap();
        std::fs::create_dir(tmp.work().join("sub")).unwrap();
        std::fs::write(tmp.work().join("sub/meta.json"), "sub").unwrap();
        commit_dir(&tmp.work(), &tmp.git(), "first", &[("meta.json", b"v1")]).unwrap();
        assert_eq!(committed(&tmp, "meta.json").unwrap(), b"v1");
        assert_eq!(committed(&tmp, "sub/meta.json").unwrap(), b"sub");
        assert_eq!(committed(&tmp, "a.txt").unwrap(), b"a");
        assert_eq!(std::fs::read(tmp.work().join("meta.json")).unwrap(), b"old");

        // unchanged: no commit; changed: a commit with nothing else changed
        assert!(
            commit_dir(&tmp.work(), &tmp.git(), "same", &[("meta.json", b"v1")])
                .unwrap()
                .is_none()
        );
        assert!(
            commit_dir(&tmp.work(), &tmp.git(), "next", &[("meta.json", b"v2")])
                .unwrap()
                .is_some()
        );
        assert_eq!(committed(&tmp, "meta.json").unwrap(), b"v2");
    }

    #[test]
    fn git_dir_names_are_case_insensitive() {
        assert!(is_git_dir_name(".git"));
        assert!(is_git_dir_name(".GIT"));
        assert!(!is_git_dir_name("style.css"));
        assert!(!is_git_dir_name(".github"));
        assert!(!is_git_dir_name(".gitignore"));
        assert!(!is_git_dir_name(".gitattributes"));
        // NTFS/HFS variants of .git
        assert!(is_git_dir_name(".git."));
        assert!(is_git_dir_name(".git.."));
        assert!(is_git_dir_name(".git "));
        assert!(is_git_dir_name("GIT~1"));
    }

    #[test]
    fn stale_locks_dont_block_commits_once_removed() {
        let tmp = TempDir::new();
        std::fs::write(tmp.work().join("a.txt"), "a").unwrap();
        tmp.commit("first");
        std::fs::write(tmp.work().join("a.txt"), "b").unwrap();
        for lock in ["HEAD.lock", "config.lock", "refs/heads/main.lock"] {
            std::fs::write(tmp.git().join(lock), "").unwrap();
        }
        assert!(commit_dir(&tmp.work(), &tmp.git(), "blocked", &[]).is_err());

        assert!(remove_stale_locks(&tmp.git()));
        assert!(!remove_stale_locks(&tmp.git()));
        // only lock files go
        assert!(tmp.git().join("HEAD").exists());
        assert!(tmp.git().join("config").exists());
        assert!(tmp.commit("second").is_some());
        assert_eq!(log(&tmp.git()).len(), 2);
    }

    /// Caches `oid` for `path` with the file's current stat, as if it had been committed long
    /// ago (fresh files aren't cached, see [`RACY_WINDOW`]).
    #[cfg(unix)]
    fn cache(tmp: &TempDir, path: &str, oid: Oid) {
        let mut cache = StatCache::load(&tmp.git());
        cache.new = std::mem::take(&mut cache.old);
        let stat = FileStat::of(&std::fs::metadata(tmp.work().join(path)).unwrap()).unwrap();
        let oid = oid.to_string();
        cache.new.insert(path.into(), CachedBlob { stat, oid });
        cache.save(&tmp.git()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unchanged_files_arent_hashed_again() {
        let tmp = TempDir::new();
        std::fs::create_dir(tmp.work().join("sub")).unwrap();
        std::fs::write(tmp.work().join("sub/b.txt"), "b").unwrap();
        tmp.commit("first");
        // a cached id is taken as it is, without reading the file
        let fake = Repository::open(tmp.git()).unwrap().blob(b"fake").unwrap();
        cache(&tmp, "sub/b.txt", fake);
        assert!(tmp.commit("second").is_some());
        assert_eq!(committed(&tmp, "sub/b.txt").unwrap(), b"fake");
    }

    #[cfg(unix)]
    #[test]
    fn a_rewrite_that_restores_the_mtime_is_committed() {
        let tmp = TempDir::new();
        let path = tmp.work().join("a.txt");
        std::fs::write(&path, "aaaa").unwrap();
        tmp.commit("first");
        let repo = Repository::open(tmp.git()).unwrap();
        cache(&tmp, "a.txt", repo.blob(b"aaaa").unwrap());
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        // stands in for the racy window `cache` skipped: the rewrite must get a later ctime
        std::thread::sleep(Duration::from_millis(50));
        std::fs::write(&path, "bbbb").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
        assert!(tmp.commit("second").is_some());
        assert_eq!(committed(&tmp, "a.txt").unwrap(), b"bbbb");
    }

    #[test]
    fn fresh_files_arent_cached() {
        let tmp = TempDir::new();
        std::fs::write(tmp.work().join("a.txt"), "aaaa").unwrap();
        tmp.commit("first");
        assert!(StatCache::load(&tmp.git()).old.is_empty());
        std::fs::write(tmp.work().join("a.txt"), "bbbb").unwrap();
        assert!(tmp.commit("second").is_some());
        assert_eq!(committed(&tmp, "a.txt").unwrap(), b"bbbb");
    }

    #[cfg(unix)]
    #[test]
    fn a_cached_blob_missing_from_the_repository_is_hashed_again() {
        let tmp = TempDir::new();
        std::fs::write(tmp.work().join("a.txt"), "a").unwrap();
        tmp.commit("first");
        cache(&tmp, "a.txt", Oid::from_str(&"1".repeat(40)).unwrap());
        std::fs::write(tmp.work().join("b.txt"), "b").unwrap();
        assert!(tmp.commit("second").is_some());
        assert_eq!(committed(&tmp, "a.txt").unwrap(), b"a");
    }

    #[test]
    fn a_broken_stat_cache_is_ignored() {
        let tmp = TempDir::new();
        std::fs::write(tmp.work().join("a.txt"), "a").unwrap();
        tmp.commit("first");
        std::fs::write(tmp.git().join(STAT_CACHE_FILE), "not json").unwrap();
        std::fs::write(tmp.work().join("a.txt"), "b").unwrap();
        assert!(tmp.commit("second").is_some());
        assert_eq!(committed(&tmp, "a.txt").unwrap(), b"b");
    }

    #[test]
    fn unaddable_git_like_paths_dont_block_commits() {
        let tmp = TempDir::new();
        std::fs::create_dir_all(tmp.work().join("assets/.git.")).unwrap();
        std::fs::write(tmp.work().join("assets/.git./x"), "x").unwrap();
        std::fs::write(tmp.work().join("assets/.git.."), "y").unwrap();
        std::fs::write(tmp.work().join("assets/a.css"), "a").unwrap();
        assert!(tmp.commit("add").is_some());
        let repo = Repository::open(tmp.git()).unwrap();
        let tree = repo.head().unwrap().peel_to_tree().unwrap();
        assert!(tree.get_path(Path::new("assets/a.css")).is_ok());
    }
}
