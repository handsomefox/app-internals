//! Whether a path lies inside a folder the app trusts, and whether two folders
//! overlap.
//!
//! An app that deletes or moves files checks every path with
//! [`is_safe_path`] first: the path must lie strictly inside one of the roots
//! the app trusts, and nothing between the root and the path may be a link.
//! The check is lexical, so it works for paths that do not exist yet, and it
//! rejects anything it cannot read the same way Windows will:
//!
//! - a `..` component anywhere, because it is resolved before links are;
//! - on Windows, a component ending in `.` or a space, which Win32 trims, so
//!   `cache.` would reach an excluded `cache`; a component holding `:`, which
//!   names an alternate data stream; and a reserved device name such as
//!   `NUL` or `COM1`;
//! - on Windows, a path that `std::path::absolute` reads differently, which
//!   catches any other rewrite Win32 makes to a path;
//! - a component that is not valid Unicode, because two such names would
//!   compare equal once converted;
//! - device paths (`\\.\`) and verbatim paths other than a drive or a share.
//!
//! A drive path, its verbatim form (`\\?\C:\`), a share (`\\server\share\`),
//! and its verbatim form (`\\?\UNC\server\share\`) each compare equal to the
//! other spelling. Folder redirection can put `AppData` on a share.
//!
//! On Windows the comparison ignores case as the file system does: each
//! character is compared by its simple uppercase form, so `k` matches `K`
//! but not the Kelvin sign `K`. Elsewhere, case matters.
//!
//! [`overlap`] answers a different question, for folders the user picks:
//! whether two folders are the same or one holds the other once every link
//! is followed. A backup destination that is a junction into the folder it
//! backs up overlaps it.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Component, MAIN_SEPARATOR, MAIN_SEPARATOR_STR, Path, PathBuf, Prefix};

/// Whether metadata from [`fs::symlink_metadata`] describes a Windows reparse
/// point: a junction, a mount point, a symbolic link, or a cloud placeholder.
/// Always `false` elsewhere, where [`is_link`] covers symbolic links.
#[must_use]
pub fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

/// Whether metadata from [`fs::symlink_metadata`] describes a symbolic link or
/// a Windows reparse point. A guard treats either as content: it never
/// follows one or counts what it points to.
#[must_use]
pub fn is_link(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink() || is_reparse_point(metadata)
}

/// Whether `path` lies strictly inside one of `roots`, never a root itself,
/// and no existing folder between that root and `path`, or `path` itself, is
/// a link.
///
/// A missing folder on the way passes, because nothing below it can exist
/// yet. Call this again immediately before touching the path. A folder whose
/// name holds a `*` or `?` passes as missing too, so a glob pattern can be
/// checked before it is expanded. Any other error reading a folder fails the
/// check.
///
/// Relative paths and roots are resolved against the current folder. See the
/// crate documentation for the paths it always rejects.
#[must_use]
pub fn is_safe_path(path: &Path, roots: &[&Path]) -> bool {
    let Some(target) = Strict::new(path) else {
        return false;
    };
    roots.iter().any(|root| {
        Strict::new(root).is_some_and(|root| {
            target
                .key
                .depth_below(&root.key)
                .is_some_and(|depth| depth > 0 && no_links_below(&target.path, depth))
        })
    })
}

/// A comparison key for `path`: cleaned lexically, with `..` taken out
/// against the part before it, and folded as [`is_safe_path`] folds case.
/// Two spellings of one path give one key, so it suits removing duplicates
/// and matching paths against a list.
///
/// The key never touches the file system and accepts any path. A name that
/// is not valid Unicode is converted lossily, so two such names can share a
/// key: use [`is_safe_path`], not this, to decide what may be deleted.
#[must_use]
pub fn normalized_key(path: &Path) -> String {
    let lexical = Lexical::new(path);
    let mut key = lexical.anchor;
    if lexical.rooted && !key.ends_with(MAIN_SEPARATOR) {
        key.push(MAIN_SEPARATOR);
    }
    key.push_str(&lexical.parts.join(MAIN_SEPARATOR_STR));
    if key.is_empty() {
        key.push('.');
    }
    key
}

/// Whether `path` is `ancestor` or lies inside it, compared as
/// [`normalized_key`] compares: lexically, without touching the file system.
/// It suits matching against a list the user keeps, such as exclusions.
#[must_use]
pub fn is_within(path: &Path, ancestor: &Path) -> bool {
    let path = Lexical::new(path);
    let ancestor = Lexical::new(ancestor);
    path.anchor == ancestor.anchor
        && path.rooted == ancestor.rooted
        && path.parts.starts_with(&ancestor.parts)
}

/// `path` with every link in its existing part followed: the deepest folder
/// that exists is canonicalized, and the missing rest is appended as written.
///
/// # Errors
///
/// Returns [`io::ErrorKind::InvalidInput`] for a relative path or one that
/// [`is_safe_path`] would reject on its face, an error when a link on the way
/// points at nothing, since the folder made through it would land elsewhere,
/// and any error other than "not found" from the file system.
pub fn resolve(path: &Path) -> io::Result<PathBuf> {
    if !path.is_absolute() {
        return Err(invalid("the path must be absolute"));
    }
    if Strict::new(path).is_none() {
        return Err(invalid("the path cannot be checked safely"));
    }
    let mut current = path;
    let mut missing = Vec::new();
    loop {
        match fs::canonicalize(current) {
            Ok(mut resolved) => {
                resolved.extend(missing.iter().rev());
                return Ok(resolved);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // A link that points at nothing is found here, but its target
                // is not.
                match fs::symlink_metadata(current) {
                    Ok(_) => {
                        return Err(io::Error::new(
                            io::ErrorKind::NotFound,
                            format!("{} is a link to something missing", current.display()),
                        ));
                    }
                    Err(missing_error) if missing_error.kind() == io::ErrorKind::NotFound => {}
                    Err(other) => return Err(other),
                }
                let (Some(name), Some(parent)) = (current.file_name(), current.parent()) else {
                    return Err(error);
                };
                missing.push(name);
                current = parent;
            }
            Err(error) => return Err(error),
        }
    }
}

/// How two folders lie relative to each other once links are followed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Overlap {
    /// Neither holds the other.
    Apart,
    /// Both name one folder.
    Same,
    /// The first lies inside the second.
    FirstInsideSecond,
    /// The second lies inside the first.
    SecondInsideFirst,
}

/// Whether `first` and `second` are one folder, or one lies inside the other,
/// after [`resolve`] follows the links in each.
///
/// # Errors
///
/// Returns the error from [`resolve`] for either path, or
/// [`io::ErrorKind::InvalidInput`] when a resolved path cannot be compared,
/// such as a volume without a drive letter. An app should treat any error
/// as an overlap.
pub fn overlap(first: &Path, second: &Path) -> io::Result<Overlap> {
    let key = |path: &Path| {
        let resolved = resolve(path)?;
        Strict::new(&resolved)
            .map(|strict| strict.key)
            .ok_or_else(|| invalid("the resolved path cannot be compared"))
    };
    let (first, second) = (key(first)?, key(second)?);
    Ok(
        match (first.depth_below(&second), second.depth_below(&first)) {
            (Some(0), _) => Overlap::Same,
            (Some(_), _) => Overlap::FirstInsideSecond,
            (None, Some(_)) => Overlap::SecondInsideFirst,
            (None, None) => Overlap::Apart,
        },
    )
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

/// Checks `depth` levels from `path` up, `path` included: none may be a link.
fn no_links_below(path: &Path, depth: usize) -> bool {
    path.ancestors()
        .take(depth)
        .all(|candidate| match fs::symlink_metadata(candidate) {
            Ok(metadata) => !is_link(&metadata),
            // Windows reports a name holding `*` as invalid where Unix reports
            // it missing; either way nothing below it exists.
            Err(error) => matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::InvalidFilename
            ),
        })
}

/// Folds case as the platform's file system compares names.
fn fold(text: &str) -> String {
    if cfg!(windows) {
        // NTFS compares names through a one-to-one uppercase table. A
        // character whose uppercase form is longer, such as `ß`, maps to
        // itself there.
        text.chars()
            .map(|character| {
                let mut upper = character.to_uppercase();
                match (upper.next(), upper.next()) {
                    (Some(single), None) => single,
                    _ => character,
                }
            })
            .collect()
    } else {
        text.to_owned()
    }
}

/// A path read for [`normalized_key`] and [`is_within`]: lossy and total.
struct Lexical {
    anchor: String,
    rooted: bool,
    parts: Vec<String>,
}

impl Lexical {
    fn new(path: &Path) -> Self {
        let mut lexical = Self {
            anchor: String::new(),
            rooted: false,
            parts: Vec::new(),
        };
        for component in path.components() {
            match component {
                Component::Prefix(prefix) => {
                    let (anchor, share) = match prefix.kind() {
                        Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                            (drive_anchor(drive), false)
                        }
                        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => (
                            share_anchor(&server.to_string_lossy(), &share.to_string_lossy()),
                            true,
                        ),
                        Prefix::Verbatim(rest) => {
                            (format!(r"\\?\{}", rest.to_string_lossy()), false)
                        }
                        Prefix::DeviceNS(rest) => {
                            (format!(r"\\.\{}", rest.to_string_lossy()), false)
                        }
                    };
                    lexical.anchor = fold(&anchor);
                    // A share is always rooted, written with `\` or not.
                    lexical.rooted |= share;
                }
                Component::RootDir => lexical.rooted = true,
                Component::CurDir => {}
                Component::ParentDir => {
                    if lexical.parts.pop().is_none() && !lexical.rooted {
                        lexical.parts.push("..".to_owned());
                    }
                }
                Component::Normal(name) => lexical.parts.push(fold(&name.to_string_lossy())),
            }
        }
        lexical
    }
}

fn drive_anchor(drive: u8) -> String {
    format!("{}:", char::from(drive.to_ascii_uppercase()))
}

fn share_anchor(server: &str, share: &str) -> String {
    format!(r"\\{server}\{share}")
}

/// An absolute path that passed every check in the crate documentation.
struct Strict {
    key: Key,
    /// The absolute path, without `.` components, for walking its folders.
    path: PathBuf,
}

/// The parts of a [`Strict`] path, folded, starting with its drive, share,
/// or `/`.
#[derive(Debug, Eq, PartialEq)]
struct Key(Vec<String>);

impl Key {
    /// How many components `self` lies below `ancestor`, or `None` when it
    /// is not `ancestor` or inside it.
    fn depth_below(&self, ancestor: &Self) -> Option<usize> {
        self.0
            .starts_with(&ancestor.0)
            .then(|| self.0.len() - ancestor.0.len())
    }
}

impl Strict {
    fn new(path: &Path) -> Option<Self> {
        if path.as_os_str().is_empty() {
            return None;
        }
        let joined = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().ok()?.join(path)
        };
        let key = strict_key(&joined)?;
        // Win32 rewrites some paths before it opens them. Whatever it would
        // rewrite that the checks above missed shows up as a different key.
        if cfg!(windows) && strict_key(&std::path::absolute(path).ok()?)? != key {
            return None;
        }
        let path = joined.components().collect();
        Some(Self { key, path })
    }
}

fn strict_key(path: &Path) -> Option<Key> {
    let mut components = path.components().peekable();
    let mut parts = Vec::new();
    match components.next()? {
        Component::RootDir if cfg!(not(windows)) => parts.push("/".to_owned()),
        Component::Prefix(prefix) => {
            let anchor = match prefix.kind() {
                Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                    // `C:` without a root is relative to the drive's
                    // current folder.
                    if components.next()? != Component::RootDir {
                        return None;
                    }
                    drive_anchor(drive)
                }
                Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                    if components.peek() == Some(&Component::RootDir) {
                        components.next();
                    }
                    fold(&share_anchor(strict_name(server)?, strict_name(share)?))
                }
                Prefix::Verbatim(_) | Prefix::DeviceNS(_) => return None,
            };
            parts.push(anchor);
        }
        _ => return None,
    }
    for component in components {
        let Component::Normal(name) = component else {
            // `..`, a `.` in a verbatim path, or a second root.
            return None;
        };
        parts.push(fold(strict_name(name)?));
    }
    Some(Key(parts))
}

/// `name` as text when it is a name every check can read.
fn strict_name(name: &OsStr) -> Option<&str> {
    let name = name.to_str()?;
    if name.is_empty() {
        return None;
    }
    if cfg!(windows)
        && (name.ends_with(['.', ' '])
            || name.contains([':', '/', '\\', '\0'])
            || is_device_name(name))
    {
        return None;
    }
    Some(name)
}

/// Whether Win32 may read `name` as a device rather than a file, as it reads
/// `NUL`, `nul.txt`, or `COM1 .log` on some versions of Windows.
fn is_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    let stem = stem.to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let Some(number) = stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"))
    else {
        return false;
    };
    let mut digits = number.chars();
    matches!(
        (digits.next(), digits.next()),
        (Some('0'..='9' | '¹' | '²' | '³'), None)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::create_dir_all;

    /// `path` with `/` as the platform's separator, under the platform's
    /// root: `/a` is `C:\a` on Windows.
    fn abs(path: &str) -> PathBuf {
        let path = path.replace('/', MAIN_SEPARATOR_STR);
        if cfg!(windows) {
            PathBuf::from(format!("C:{path}"))
        } else {
            PathBuf::from(path)
        }
    }

    #[test]
    fn a_root_itself_is_never_safe() {
        let root = abs("/home/user/appdata");
        assert!(!is_safe_path(&root, &[&root]));
        assert!(!is_safe_path(&abs("/home/user/appdata/"), &[&root]));
        assert!(!is_safe_path(&abs("/home/user/appdata/."), &[&root]));
    }

    #[test]
    fn a_path_strictly_inside_is_safe() {
        let root = abs("/home/user/appdata");
        assert!(is_safe_path(&abs("/home/user/appdata/x"), &[&root]));
        assert!(is_safe_path(&abs("/home/user/appdata/x/y/z"), &[&root]));
        assert!(is_safe_path(&abs("/home/user/appdata/./x"), &[&root]));
        assert!(is_safe_path(&abs("/home/user/appdata//x/"), &[&root]));
        let other = abs("/elsewhere");
        assert!(is_safe_path(&abs("/home/user/appdata/x"), &[&other, &root]));
    }

    #[test]
    fn outside_and_sibling_prefixes_are_unsafe() {
        let root = abs("/home/user/appdata");
        assert!(!is_safe_path(&abs("/home/user/appdata2/x"), &[&root]));
        assert!(!is_safe_path(&abs("/home/user"), &[&root]));
        assert!(!is_safe_path(&abs("/elsewhere"), &[&root]));
        assert!(!is_safe_path(&abs("/"), &[&root]));
    }

    #[test]
    fn a_parent_component_is_never_safe() {
        let root = abs("/home/user/appdata");
        assert!(!is_safe_path(&abs("/home/user/appdata/../other"), &[&root]));
        assert!(!is_safe_path(&abs("/home/user/appdata/a/../b"), &[&root]));
        assert!(!is_safe_path(&abs("/home/user/appdata/a/.."), &[&root]));
        let dotted = abs("/home/user/../user/appdata");
        assert!(!is_safe_path(&abs("/home/user/appdata/x"), &[&dotted]));
    }

    #[test]
    fn empty_inputs_are_unsafe() {
        let root = abs("/home/user/appdata");
        assert!(!is_safe_path(Path::new(""), &[&root]));
        assert!(!is_safe_path(&abs("/home/user/appdata/x"), &[]));
        assert!(!is_safe_path(
            &abs("/home/user/appdata/x"),
            &[Path::new("")]
        ));
    }

    #[test]
    fn relative_paths_resolve_against_the_current_folder() {
        let root = Path::new(".");
        assert!(is_safe_path(&root.join("missing-directory/blob"), &[root]));
        assert!(is_safe_path(Path::new("missing-directory/blob"), &[root]));
        assert!(!is_safe_path(root, &[root]));
        let current = std::env::current_dir().unwrap();
        assert!(is_safe_path(Path::new("missing-directory"), &[&current]));
    }

    #[test]
    fn glob_pattern_ancestors_with_wildcards_are_safe() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        create_dir_all(root.join("sub")).unwrap();
        // The parent exists: Windows reports the name as invalid.
        assert!(is_safe_path(
            &root.join("sub").join("*").join("Cache"),
            &[&root]
        ));
        assert!(is_safe_path(&root.join("sub").join("?.tmp"), &[&root]));
        // The parent is missing: Windows reports it as not found.
        assert!(is_safe_path(
            &root.join("missing").join("*").join("Cache"),
            &[&root]
        ));
    }

    #[test]
    fn existing_folders_without_links_are_safe() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        create_dir_all(root.join("a/b")).unwrap();
        fs::write(root.join("a/b/file"), b"x").unwrap();
        assert!(is_safe_path(&root.join("a/b/file"), &[&root]));
    }

    #[test]
    fn a_file_used_as_a_folder_is_unsafe() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        create_dir_all(&root).unwrap();
        fs::write(root.join("file"), b"x").unwrap();
        // Unix reports "not a directory", which proves nothing about what
        // lies below.
        if cfg!(unix) {
            assert!(!is_safe_path(&root.join("file/child"), &[&root]));
        }
    }

    #[test]
    fn keys_clean_lexically() {
        assert_eq!(
            normalized_key(&abs("/a/b/./c/../d")),
            normalized_key(&abs("/a/b/d"))
        );
        assert_eq!(normalized_key(&abs("/a/b/")), normalized_key(&abs("/a/b")));
        assert_eq!(normalized_key(Path::new("")), ".");
        // Windows folds case to upper, so the key keeps `a` only elsewhere.
        let a = if cfg!(windows) { "A" } else { "a" };
        assert_eq!(
            normalized_key(Path::new("../a")),
            format!("..{MAIN_SEPARATOR}{a}")
        );
        assert_ne!(
            normalized_key(&abs("/a/b")),
            normalized_key(Path::new("a/b"))
        );
    }

    #[test]
    fn within_includes_the_folder_itself() {
        assert!(is_within(&abs("/a/b/c"), &abs("/a/b")));
        assert!(is_within(&abs("/a/b"), &abs("/a/b")));
        assert!(is_within(&abs("/a/b/../b/c"), &abs("/a/b")));
        assert!(!is_within(&abs("/a/b"), &abs("/a/b/c")));
        assert!(!is_within(&abs("/a/bc"), &abs("/a/b")));
        assert!(!is_within(&abs("/a/b/../x"), &abs("/a/b")));
        assert!(!is_within(Path::new("a/b"), &abs("/a")));
    }

    #[test]
    fn resolve_appends_the_missing_part() {
        let dir = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        assert_eq!(
            resolve(&dir.path().join("new/inner")).unwrap(),
            base.join("new/inner")
        );
        assert_eq!(resolve(dir.path()).unwrap(), base);
    }

    #[test]
    fn resolve_refuses_what_it_cannot_check() {
        assert_eq!(
            resolve(Path::new("backups")).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve(&dir.path().join("a/../b")).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn overlap_names_which_folder_holds_which() {
        let dir = tempfile::tempdir().unwrap();
        let save = dir.path().join("save/76561198000000000");
        let backups = dir.path().join("documents/backups");
        create_dir_all(&save).unwrap();
        assert_eq!(overlap(&save, &backups).unwrap(), Overlap::Apart);
        assert_eq!(overlap(&save, &save).unwrap(), Overlap::Same);
        assert_eq!(
            overlap(&save, &save.join("backups")).unwrap(),
            Overlap::SecondInsideFirst
        );
        assert_eq!(
            overlap(&backups.join("save"), &backups).unwrap(),
            Overlap::FirstInsideSecond
        );
        assert_eq!(
            overlap(&save, &dir.path().join("save/7656119800000000")).unwrap(),
            Overlap::Apart
        );
    }

    #[test]
    fn device_names_are_recognized() {
        for name in [
            "NUL", "nul.txt", "Con", "COM1", "lpt9.log", "COM¹", "aux .x", "CONIN$",
        ] {
            assert!(is_device_name(name), "{name}");
        }
        for name in [
            "NULL", "console", "COM10", "LPT", "COMA", "nul-file", "x.nul",
        ] {
            assert!(!is_device_name(name), "{name}");
        }
    }

    #[cfg(unix)]
    mod unix {
        use super::*;
        use std::os::unix::ffi::OsStrExt as _;
        use std::os::unix::fs::symlink;

        #[test]
        fn a_symlink_on_the_way_is_unsafe() {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("safe");
            let outside = dir.path().join("outside");
            create_dir_all(outside.join("cache")).unwrap();
            create_dir_all(&root).unwrap();
            symlink(&outside, root.join("linked")).unwrap();

            assert!(!is_safe_path(&root.join("linked/cache"), &[&root]));
            assert!(!is_safe_path(&root.join("linked"), &[&root]));
            assert!(!is_safe_path(&root.join("linked/missing/deeper"), &[&root]));
            // A link above the root is the root's own business.
            let linked_root = dir.path().join("root-link");
            symlink(&root, &linked_root).unwrap();
            create_dir_all(root.join("real")).unwrap();
            assert!(is_safe_path(&linked_root.join("real"), &[&linked_root]));
        }

        #[test]
        fn case_matters() {
            let root = Path::new("/home/user/safe");
            assert!(!is_safe_path(Path::new("/home/user/SAFE/file"), &[root]));
            assert!(!is_within(Path::new("/home/user/SAFE/file"), root));
            assert_ne!(
                normalized_key(Path::new("/A")),
                normalized_key(Path::new("/a"))
            );
        }

        #[test]
        fn names_that_are_not_unicode_are_unsafe() {
            let root = Path::new("/home/user/safe");
            let bad = Path::new(OsStr::from_bytes(b"/home/user/safe/\xff"));
            assert!(!is_safe_path(bad, &[root]));
            let bad_root = Path::new(OsStr::from_bytes(b"/home/\xfe"));
            assert!(!is_safe_path(Path::new("/home/\u{fffd}/x"), &[bad_root]));
        }

        #[test]
        fn names_windows_would_trim_are_fine_here() {
            let root = Path::new("/home/user/safe");
            assert!(is_safe_path(Path::new("/home/user/safe/cache."), &[root]));
            assert!(is_safe_path(Path::new("/home/user/safe/a:b"), &[root]));
            assert!(is_safe_path(Path::new("/home/user/safe/nul"), &[root]));
        }

        #[test]
        fn overlap_follows_links() {
            let dir = tempfile::tempdir().unwrap();
            let save = dir.path().join("save");
            create_dir_all(&save).unwrap();
            let disguised = dir.path().join("disguised");
            symlink(&save, &disguised).unwrap();
            assert_eq!(
                overlap(&save, &disguised.join("backups")).unwrap(),
                Overlap::SecondInsideFirst
            );
            assert_eq!(overlap(&save, &disguised).unwrap(), Overlap::Same);
        }

        #[test]
        fn resolve_refuses_a_link_to_nothing() {
            let dir = tempfile::tempdir().unwrap();
            let dangling = dir.path().join("dangling");
            symlink(dir.path().join("missing"), &dangling).unwrap();
            assert_eq!(
                resolve(&dangling.join("backups")).unwrap_err().kind(),
                io::ErrorKind::NotFound
            );
        }
    }

    #[cfg(windows)]
    mod windows {
        use super::*;
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt as _;

        #[test]
        fn case_is_ignored_as_ntfs_ignores_it() {
            let root = Path::new(r"C:\Home\User\AppData");
            assert!(is_safe_path(Path::new(r"c:\home\user\appdata\X"), &[root]));
            assert!(is_within(Path::new(r"c:\HOME\user\appdata\x"), root));
            assert_eq!(
                normalized_key(Path::new(r"c:\a\B")),
                normalized_key(Path::new(r"C:\A\b"))
            );
            // Non-ASCII letters fold too.
            assert!(is_safe_path(
                Path::new(r"C:\Users\ÖLAF\x"),
                &[Path::new(r"C:\Users\ölaf")]
            ));
        }

        #[test]
        fn the_kelvin_sign_is_not_a_k() {
            let root = Path::new(r"C:\Users\me\Kache");
            assert!(!is_safe_path(
                Path::new("C:\\Users\\me\\\u{212A}ache\\x"),
                &[root]
            ));
            assert!(!is_within(
                Path::new("C:\\Users\\me\\\u{212A}ache\\x"),
                root
            ));
            assert!(is_safe_path(Path::new(r"C:\Users\me\kache\x"), &[root]));
            // `ß` has no one-character uppercase, so it stays apart from `SS`.
            assert!(!is_safe_path(
                Path::new(r"C:\Users\me\STRASSE\x"),
                &[Path::new(r"C:\Users\me\straße")]
            ));
        }

        #[test]
        fn names_win32_would_trim_are_unsafe() {
            let root = Path::new(r"C:\root");
            for path in [
                r"C:\root\cache.",
                r"C:\root\cache ",
                r"C:\root\cache.\file",
                r"C:\root\cache \file",
                r"C:\root\cache...",
                r"\\?\C:\root\cache.",
            ] {
                assert!(!is_safe_path(Path::new(path), &[root]), "{path}");
            }
            assert!(!is_safe_path(
                Path::new(r"C:\root\x"),
                &[Path::new(r"C:\root.")]
            ));
        }

        #[test]
        fn streams_and_devices_are_unsafe() {
            let root = Path::new(r"C:\root");
            for path in [
                r"C:\root\file:stream",
                r"C:\root\file::$DATA",
                r"C:\root\NUL",
                r"C:\root\con.txt",
                r"C:\root\COM1\x",
                r"\\.\C:\root\x",
                r"\\?\Volume{00000000-0000-0000-0000-000000000000}\root\x",
            ] {
                assert!(!is_safe_path(Path::new(path), &[root]), "{path}");
            }
        }

        #[test]
        fn drive_relative_paths_are_unsafe() {
            assert!(!is_safe_path(
                Path::new(r"C:root\x"),
                &[Path::new(r"C:\root")]
            ));
            assert!(!is_safe_path(
                Path::new(r"C:\root\x"),
                &[Path::new(r"C:root")]
            ));
        }

        #[test]
        fn verbatim_and_plain_spellings_agree() {
            let plain = Path::new(r"C:\root");
            let verbatim = Path::new(r"\\?\C:\root");
            assert!(is_safe_path(Path::new(r"\\?\C:\root\x"), &[plain]));
            assert!(is_safe_path(Path::new(r"C:\root\x"), &[verbatim]));
            assert!(!is_safe_path(verbatim, &[plain]));
            assert!(!is_safe_path(Path::new(r"\\?\C:\root\.\x"), &[plain]));
            assert!(!is_safe_path(Path::new(r"\\?\C:\root\a/b"), &[plain]));
            assert_eq!(normalized_key(verbatim), normalized_key(plain));
            assert!(is_within(Path::new(r"\\?\c:\ROOT\x"), plain));
        }

        #[test]
        fn shares_and_their_verbatim_spelling_agree() {
            let share = Path::new(r"\\Server\Share\Users\me\AppData");
            let verbatim = Path::new(r"\\?\UNC\server\share\users\me\appdata");
            assert!(is_safe_path(
                Path::new(r"\\server\share\Users\me\AppData\x"),
                &[share]
            ));
            assert!(is_safe_path(&verbatim.join("x"), &[share]));
            assert!(is_safe_path(&share.join("x"), &[verbatim]));
            assert!(!is_safe_path(verbatim, &[share]));
            assert!(!is_safe_path(
                Path::new(r"\\server\other\Users\me\AppData\x"),
                &[share]
            ));
            assert!(!is_safe_path(
                Path::new(r"\\other\share\Users\me\AppData\x"),
                &[share]
            ));
            assert!(!is_safe_path(Path::new(r"C:\Users\me\AppData\x"), &[share]));
            assert!(is_safe_path(
                Path::new(r"\\server\share\x"),
                &[Path::new(r"\\server\share")]
            ));
            assert_eq!(normalized_key(share), normalized_key(verbatim));
            assert_eq!(
                normalized_key(Path::new(r"\\server\share")),
                normalized_key(Path::new(r"\\server\share\"))
            );
        }

        #[test]
        fn names_that_are_not_unicode_are_unsafe() {
            let mut path = OsString::from(r"C:\root\");
            path.push(OsString::from_wide(&[0xD800]));
            assert!(!is_safe_path(Path::new(&path), &[Path::new(r"C:\root")]));
        }

        #[test]
        fn a_junction_on_the_way_is_unsafe() {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("safe");
            let outside = dir.path().join("outside");
            create_dir_all(outside.join("cache")).unwrap();
            create_dir_all(&root).unwrap();
            let junction = root.join("linked");
            // `mklink /J` needs no privilege, unlike a symbolic link.
            let made = std::process::Command::new("cmd")
                .arg("/c")
                .arg("mklink")
                .arg("/J")
                .arg(&junction)
                .arg(&outside)
                .output()
                .unwrap();
            assert!(made.status.success(), "{made:?}");

            let metadata = fs::symlink_metadata(&junction).unwrap();
            assert!(is_reparse_point(&metadata));
            assert!(is_link(&metadata));
            assert!(!is_safe_path(&junction.join("cache"), &[&root]));
            assert!(!is_safe_path(&junction, &[&root]));
            // Followed, the junction leads outside.
            assert_eq!(
                overlap(&outside, &junction.join("cache")).unwrap(),
                Overlap::SecondInsideFirst
            );
        }
    }
}
