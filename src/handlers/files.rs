use std::{
    fs::{File, canonicalize},
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

use crate::http::{headers::Headers, response::Response, status::StatusCode};

pub struct StaticFile {
    value: String,
}

#[derive(Debug, thiserror::Error)]
pub enum StaticFileError {
    #[error("File not found")]
    NotFound,
    #[error("Permission denied")]
    Forbidden,
    #[error("Invalid path")]
    InvalidPath,
    #[error("I/O error")]
    Io(#[source] io::Error),
}

impl From<StaticFileError> for Response {
    fn from(_error: StaticFileError) -> Self {
        let status = match _error {
            StaticFileError::NotFound | StaticFileError::InvalidPath => StatusCode::NotFound,
            StaticFileError::Forbidden => StatusCode::Forbidden,
            StaticFileError::Io(err) => {
                eprintln!("static file error: {err}");
                StatusCode::InternalServerError
            }
        };

        Response::new(status, Headers::default(), String::new())
    }
}

impl From<io::Error> for StaticFileError {
    fn from(_error: io::Error) -> Self {
        match _error.kind() {
            // file does not exist
            io::ErrorKind::NotFound
            // last element isn't a file but a directory
            | io::ErrorKind::IsADirectory
            // one of the directories in the path is in reality a file
            | io::ErrorKind::NotADirectory
            // name too long
            | io::ErrorKind::InvalidFilename => Self::NotFound,
            // can't be read
            io::ErrorKind::PermissionDenied => StaticFileError::Forbidden,
            // rest
            _ => StaticFileError::Io(_error),
        }
    }
}

pub fn check_is_path_plain(file_path: &str) -> Result<(), StaticFileError> {
    let is_plain = Path::new(file_path)
        .components()
        .all(|component| matches!(component, Component::Normal(_)));

    // there are no Component::RootDir or Component::ParentDir
    if !is_plain {
        Err(StaticFileError::InvalidPath)
    } else {
        Ok(())
    }
}

pub fn check_path(dir: &std::path::Path, file_path: &str) -> Result<PathBuf, StaticFileError> {
    check_is_path_plain(file_path)?;

    let served_dir = canonicalize(dir)?;
    let real_path = canonicalize(served_dir.join(file_path))?;

    // check if there's no symlink
    if !real_path.starts_with(&served_dir) {
        return Err(StaticFileError::InvalidPath);
    }

    Ok(real_path)
}

impl StaticFile {
    pub fn open(dir: &std::path::Path, file_path: &str) -> Result<Self, StaticFileError> {
        let real_path = check_path(dir, file_path)?;

        Ok(Self {
            value: std::fs::read_to_string(real_path)?,
        })
    }
}

impl From<StaticFile> for Response {
    fn from(static_file: StaticFile) -> Self {
        Response::new(
            StatusCode::Ok,
            Headers::from(vec![(
                String::from("Content-Type"),
                String::from("application/octet-stream"),
            )]),
            static_file.value,
        )
    }
}

pub struct CreatedFile {}

impl CreatedFile {
    pub fn create(
        dir: &std::path::Path,
        file_path: &str,
        contents: &str,
    ) -> Result<CreatedFile, StaticFileError> {
        // A single non-Normal component (RootDir, ParentDir, CurDir) turns the whole result into None
        let mut segments = Path::new(file_path)
            .components()
            .map(|component| match component {
                Component::Normal(segment) => Some(segment),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .ok_or(StaticFileError::InvalidPath)?;

        // "" has no segment, so no file name
        let file_name = segments.pop().ok_or(StaticFileError::InvalidPath)?;

        let served_dir = canonicalize(dir)?;
        // The target does not exist yet, but its parent must: resolving it reveals a symlinked directory
        let parent = canonicalize(served_dir.join(segments.iter().collect::<PathBuf>()))?;

        if !parent.starts_with(&served_dir) {
            return Err(StaticFileError::InvalidPath);
        }

        let target = parent.join(file_name);

        // Unlike `exists()`, `symlink_metadata` does not follow links, so dangling links are caught too
        match std::fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(StaticFileError::InvalidPath);
            }
            Ok(_) => {}
            // The normal case for a POST: the file does not exist yet
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let mut file = File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(target)?;

        file.write_all(contents.as_bytes())?;

        Ok(CreatedFile {})
    }
}

impl From<CreatedFile> for Response {
    fn from(_: CreatedFile) -> Self {
        Response::new(StatusCode::Created, Headers::default(), String::new())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::http::{response::Response, status::StatusCode};
    use crate::test_support::TempDir;

    const SPEC_CONTENTS: &str = "Hello, World!";

    fn dir_with_foo() -> TempDir {
        let dir = TempDir::new();
        dir.write("foo", SPEC_CONTENTS);
        dir
    }

    fn response(dir: &std::path::Path, path: &str) -> Response {
        match StaticFile::open(dir, path) {
            Ok(file) => file.into(),
            Err(error) => error.into(),
        }
    }

    fn assert_404(response: Response) {
        assert_eq!(response.status(), &StatusCode::NotFound);
        assert_eq!(response.body(), "");
        assert_eq!(response.header("Content-Type"), None);
    }

    // --- Existing file ---

    #[test]
    fn opens_existing_file() {
        // "foo" → [Normal("foo")]
        let dir = dir_with_foo();

        assert!(StaticFile::open(dir.path(), "foo").is_ok());
    }

    #[test]
    fn converts_into_200_response() {
        // "foo" → [Normal("foo")]
        let dir = dir_with_foo();

        assert_eq!(response(dir.path(), "foo").status(), &StatusCode::Ok);
    }

    #[test]
    fn body_is_file_contents() {
        // "foo" → [Normal("foo")]
        let dir = dir_with_foo();

        assert_eq!(response(dir.path(), "foo").body(), SPEC_CONTENTS);
    }

    // RFC 2046 §4.5.1: application/octet-stream = arbitrary binary data.
    #[test]
    fn sets_content_type_octet_stream() {
        // "foo" → [Normal("foo")]
        let dir = dir_with_foo();

        assert_eq!(
            response(dir.path(), "foo").header("Content-Type"),
            Some("application/octet-stream")
        );
    }

    #[test]
    fn sets_content_length_to_file_size() {
        // "foo" → [Normal("foo")]
        let dir = dir_with_foo();

        assert_eq!(
            response(dir.path(), "foo").header("Content-Length"),
            Some("13")
        );
    }

    #[test]
    fn matches_spec_bytes() {
        // "foo" → [Normal("foo")]
        let dir = dir_with_foo();

        assert_eq!(
            String::from_utf8(response(dir.path(), "foo").into_bytes()).unwrap(),
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 13\r\n\r\nHello, World!"
        );
    }

    #[test]
    fn keeps_contents_verbatim() {
        // "raw.txt" → [Normal("raw.txt")]
        let dir = TempDir::new();
        let contents = "line 1\r\nline 2\n\n  trailing spaces  \n";
        dir.write("raw.txt", contents);

        assert_eq!(response(dir.path(), "raw.txt").body(), contents);
    }

    #[test]
    fn empty_file_is_200_with_zero_length() {
        // "empty" → [Normal("empty")]
        let dir = TempDir::new();
        dir.write("empty", "");

        let response = response(dir.path(), "empty");

        assert_eq!(response.status(), &StatusCode::Ok);
        assert_eq!(response.header("Content-Length"), Some("0"));
    }

    #[test]
    fn reads_file_in_subdirectory() {
        // "sub/nested" → [Normal("sub"), Normal("nested")]
        let dir = TempDir::new();
        let sub = dir.mkdir("sub");
        std::fs::write(sub.join("nested"), "deep").unwrap();

        assert_eq!(response(dir.path(), "sub/nested").body(), "deep");
    }

    // --- Missing / invalid ---

    #[test]
    fn missing_file_is_not_found() {
        // "non_existant_file" → [Normal("non_existant_file")], valid path but no such file
        let dir = dir_with_foo();

        assert!(matches!(
            StaticFile::open(dir.path(), "non_existant_file"),
            Err(StaticFileError::NotFound)
        ));
    }

    #[test]
    fn missing_file_responds_404() {
        // "non_existant_file" → [Normal("non_existant_file")], valid path but no such file
        let dir = dir_with_foo();

        assert_404(response(dir.path(), "non_existant_file"));
    }

    #[test]
    fn empty_filename_responds_404() {
        // "" → [] (empty list: `all(..)` is vacuously true)
        // join("") gives the served dir itself, and reading a directory fails
        let dir = dir_with_foo();

        assert_404(response(dir.path(), ""));
    }

    #[test]
    fn directory_is_not_a_file() {
        // "sub" → [Normal("sub")], valid path but it is a directory
        let dir = dir_with_foo();
        dir.mkdir("sub");

        assert_404(response(dir.path(), "sub"));
    }

    #[test]
    fn rejects_path_without_files_prefix() {
        // "/foo" has no "" prefix: strip_prefix fails before any component check
        let dir = dir_with_foo();

        assert!(StaticFile::open(dir.path(), "/foo").is_err());
    }

    // Path traversal: a request must never escape the served directory.
    #[test]
    fn cannot_escape_directory_with_dot_dot() {
        // dir = <tmp>/public, target = <tmp>/secret
        // "../secret" → [ParentDir, Normal("secret")]
        let dir = TempDir::new();
        dir.write("secret", "top secret");
        let public = dir.mkdir("public");

        assert_404(response(&public, "../secret"));
    }

    #[test]
    fn cannot_read_absolute_path() {
        // dir = <tmp>/public, request =  "<tmp>/secret"
        // "<tmp>/secret" → [RootDir, Normal("var"), ..., Normal("secret")]
        // join() would replace dir with this absolute path
        let dir = TempDir::new();
        dir.write("secret", "top secret");
        let public = dir.mkdir("public");
        let absolute = dir.path().join("secret");

        eprintln!("{}", &public.display());

        assert_404(response(&public, &format!("{}", absolute.display())));
    }

    // --- Path components ---
    // Allowlist policy: the requested path may only contain `Component::Normal` segments.

    #[test]
    fn double_dots_inside_a_name_are_allowed() {
        // "v1..2.txt" → [Normal("v1..2.txt")], the dots are part of the name
        let dir = TempDir::new();
        dir.write("v1..2.txt", "version");

        assert_eq!(response(dir.path(), "v1..2.txt").body(), "version");
    }

    #[test]
    fn name_starting_with_double_dots_is_allowed() {
        // "..hidden" → [Normal("..hidden")], only a bare ".." is ParentDir
        let dir = TempDir::new();
        dir.write("..hidden", "not a parent dir");

        assert_eq!(response(dir.path(), "..hidden").body(), "not a parent dir");
    }

    #[test]
    fn dot_dot_is_an_invalid_path() {
        // "../foo" → [ParentDir, Normal("foo")]
        let dir = dir_with_foo();

        assert!(matches!(
            StaticFile::open(dir.path(), "../foo"),
            Err(StaticFileError::InvalidPath)
        ));
    }

    #[test]
    fn dot_dot_after_a_subdirectory_cannot_escape() {
        // dir = <tmp>/public, target = <tmp>/secret
        // "sub/../../secret" → [Normal("sub"), ParentDir, ParentDir, Normal("secret")]
        let dir = TempDir::new();
        dir.write("secret", "top secret");
        let public = dir.mkdir("public");
        std::fs::create_dir_all(public.join("sub")).unwrap();

        assert_404(response(&public, "sub/../../secret"));
    }

    // Strict policy: `..` is rejected even when it would stay inside the directory.
    #[test]
    fn dot_dot_is_rejected_even_when_staying_inside() {
        // "sub/../foo" → [Normal("sub"), ParentDir, Normal("foo")], would resolve to <dir>/foo
        let dir = dir_with_foo();
        dir.mkdir("sub");

        assert_404(response(dir.path(), "sub/../foo"));
    }

    #[test]
    fn absolute_path_is_an_invalid_path() {
        // "/foo" → after strip_prefix: "/foo" → [RootDir, Normal("foo")]
        let dir = dir_with_foo();

        assert!(matches!(
            StaticFile::open(dir.path(), "/foo"),
            Err(StaticFileError::InvalidPath)
        ));
    }

    // A symlink is a `Normal` component, so only resolving the real path catches this one.
    #[cfg(unix)]
    #[test]
    fn symlink_cannot_escape_directory() {
        // dir = <tmp>/public, <tmp>/public/link -> <tmp>/secret
        // "link" → [Normal("link")]: lexically valid, only canonicalize() reveals the escape
        let dir = TempDir::new();
        dir.write("secret", "top secret");
        let public = dir.mkdir("public");
        std::os::unix::fs::symlink(dir.path().join("secret"), public.join("link")).unwrap();

        assert_404(response(&public, "link"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_inside_directory_is_allowed() {
        // <dir>/alias -> <dir>/foo
        // "alias" → [Normal("alias")], canonical path stays inside dir
        let dir = dir_with_foo();
        std::os::unix::fs::symlink(dir.path().join("foo"), dir.path().join("alias")).unwrap();

        assert_eq!(response(dir.path(), "alias").body(), "Hello, World!");
    }

    // --- Create a file (POST {filename}) ---

    fn create_response(dir: &std::path::Path, path: &str, contents: &str) -> Response {
        match CreatedFile::create(dir, path, contents) {
            Ok(file) => file.into(),
            Err(error) => error.into(),
        }
    }

    #[test]
    fn creates_file_with_contents() {
        let dir = TempDir::new();

        assert!(CreatedFile::create(dir.path(), "number", "12345").is_ok());
        assert_eq!(dir.read("number"), "12345");
    }

    // RFC 9110 §15.3.2: 201 Created.
    #[test]
    fn create_converts_into_201_response() {
        let dir = TempDir::new();

        assert_eq!(
            create_response(dir.path(), "number", "12345").status(),
            &StatusCode::Created
        );
    }

    // The tester expects `HTTP/1.1 201 Created\r\n\r\n`; `Content-Length: 0` is the only header.
    #[test]
    fn create_matches_spec_bytes() {
        let dir = TempDir::new();

        assert_eq!(
            String::from_utf8(create_response(dir.path(), "number", "12345").into_bytes()).unwrap(),
            "HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n"
        );
    }

    #[test]
    fn create_keeps_contents_verbatim() {
        let dir = TempDir::new();
        let contents = "line 1\r\nline 2\n\r\n\r\n  trailing spaces  \n";

        CreatedFile::create(dir.path(), "raw.txt", contents).unwrap();

        assert_eq!(dir.read("raw.txt"), contents);
    }

    #[test]
    fn create_with_empty_body_creates_empty_file() {
        let dir = TempDir::new();

        assert_eq!(
            create_response(dir.path(), "empty", "").status(),
            &StatusCode::Created
        );
        assert_eq!(dir.read("empty"), "");
    }

    // Design choice: POST on an existing name replaces its contents.
    #[test]
    fn create_overwrites_existing_file() {
        let dir = dir_with_foo();

        CreatedFile::create(dir.path(), "foo", "new").unwrap();

        assert_eq!(dir.read("foo"), "new");
    }

    #[test]
    fn created_file_can_then_be_opened() {
        let dir = TempDir::new();
        CreatedFile::create(dir.path(), "round-trip", "there and back").unwrap();

        assert_eq!(response(dir.path(), "round-trip").body(), "there and back");
    }

    #[test]
    fn create_in_existing_subdirectory() {
        let dir = TempDir::new();
        let sub = dir.mkdir("sub");

        CreatedFile::create(dir.path(), "sub/nested", "deep").unwrap();

        assert_eq!(std::fs::read_to_string(sub.join("nested")).unwrap(), "deep");
    }

    // Design choice: missing parent directories are not created.
    #[test]
    fn create_in_missing_subdirectory_responds_404() {
        let dir = TempDir::new();

        assert_404(create_response(dir.path(), "nope/nested", "x"));
        assert!(!dir.path().join("nope").exists());
    }

    // "" → join("") is the served dir itself: a directory cannot be written as a file.
    #[test]
    fn create_with_empty_filename_is_an_error() {
        let dir = TempDir::new();

        assert!(CreatedFile::create(dir.path(), "", "x").is_err());
    }

    #[test]
    fn create_on_a_directory_is_an_error() {
        let dir = TempDir::new();
        dir.mkdir("sub");

        assert!(CreatedFile::create(dir.path(), "sub", "x").is_err());
        assert!(dir.path().join("sub").is_dir());
    }

    // --- Create: path traversal ---
    // Same allowlist policy as `open`, but the target does not exist yet:
    // `canonicalize` fails on a missing path, so the check must be done another way.

    #[test]
    fn create_cannot_escape_directory_with_dot_dot() {
        // dir = <tmp>/public, target = <tmp>/evil
        let dir = TempDir::new();
        let public = dir.mkdir("public");

        assert!(matches!(
            CreatedFile::create(&public, "../evil", "pwned"),
            Err(StaticFileError::InvalidPath)
        ));
        assert!(!dir.path().join("evil").exists());
    }

    #[test]
    fn create_cannot_write_absolute_path() {
        //  "<tmp>/evil" → [RootDir, ..., Normal("evil")]
        let dir = TempDir::new();
        let public = dir.mkdir("public");
        let absolute = dir.path().join("evil");

        assert_404(create_response(
            &public,
            &format!("{}", absolute.display()),
            "pwned",
        ));
        assert!(!absolute.exists());
    }

    #[test]
    fn create_rejects_dot_dot_even_when_staying_inside() {
        // "sub/../foo" → [Normal("sub"), ParentDir, Normal("foo")]
        let dir = TempDir::new();
        dir.mkdir("sub");

        assert!(matches!(
            CreatedFile::create(dir.path(), "sub/../foo", "x"),
            Err(StaticFileError::InvalidPath)
        ));
        assert!(!dir.path().join("foo").exists());
    }

    // <public>/link -> <tmp>/outside: "link/evil" is lexically valid but lands outside.
    #[cfg(unix)]
    #[test]
    fn create_cannot_escape_through_symlinked_directory() {
        let dir = TempDir::new();
        let outside = dir.mkdir("outside");
        let public = dir.mkdir("public");
        std::os::unix::fs::symlink(&outside, public.join("link")).unwrap();

        assert_404(create_response(&public, "link/evil", "pwned"));
        assert!(!outside.join("evil").exists());
    }

    // <public>/link -> <tmp>/secret: writing to "link" would follow the symlink.
    #[cfg(unix)]
    #[test]
    fn create_cannot_overwrite_file_outside_through_symlink() {
        let dir = TempDir::new();
        dir.write("secret", "top secret");
        let public = dir.mkdir("public");
        std::os::unix::fs::symlink(dir.path().join("secret"), public.join("link")).unwrap();

        assert_404(create_response(&public, "link", "pwned"));
        assert_eq!(dir.read("secret"), "top secret");
    }

    // <public>/link -> <tmp>/planted (does not exist): `exists()` and `canonicalize()`
    // both fail on a dangling symlink, yet `fs::write` would follow it and create the target.
    #[cfg(unix)]
    #[test]
    fn create_cannot_write_through_dangling_symlink() {
        let dir = TempDir::new();
        let public = dir.mkdir("public");
        std::os::unix::fs::symlink(dir.path().join("planted"), public.join("link")).unwrap();

        assert_404(create_response(&public, "link", "pwned"));
        assert!(!dir.path().join("planted").exists());
    }
}
