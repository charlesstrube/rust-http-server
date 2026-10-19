use std::{
    fs::canonicalize,
    io,
    path::{Component, Path},
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
    #[error("Path does not start with /files/")]
    MissingFilesPrefix,
    #[error("I/O error")]
    Io(#[source] io::Error),
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

impl StaticFile {
    pub fn open(dir: &std::path::Path, file_path: &str) -> Result<Self, StaticFileError> {
        let file_path = match file_path.strip_prefix("/files/") {
            Some(file) => file,
            None => return Err(StaticFileError::MissingFilesPrefix),
        };

        let is_plain = Path::new(file_path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)));

        // there are no Component::RootDir or Component::ParentDir
        if !is_plain {
            return Err(StaticFileError::InvalidPath);
        }

        let served_dir = canonicalize(dir)?;
        let real_path = canonicalize(served_dir.join(file_path))?;

        // check if there's no symlink
        if !real_path.starts_with(&served_dir) {
            return Err(StaticFileError::InvalidPath);
        }

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

impl From<StaticFileError> for Response {
    fn from(_error: StaticFileError) -> Self {
        let status = match _error {
            StaticFileError::NotFound
            | StaticFileError::MissingFilesPrefix
            | StaticFileError::InvalidPath => StatusCode::NotFound,
            StaticFileError::Forbidden => StatusCode::Forbidden,
            StaticFileError::Io(err) => {
                eprintln!("static file error: {err}");
                StatusCode::InternalServerError
            }
        };

        Response::new(status, Headers::default(), String::new())
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

        assert!(StaticFile::open(dir.path(), "/files/foo").is_ok());
    }

    #[test]
    fn converts_into_200_response() {
        // "foo" → [Normal("foo")]
        let dir = dir_with_foo();

        assert_eq!(response(dir.path(), "/files/foo").status(), &StatusCode::Ok);
    }

    #[test]
    fn body_is_file_contents() {
        // "foo" → [Normal("foo")]
        let dir = dir_with_foo();

        assert_eq!(response(dir.path(), "/files/foo").body(), SPEC_CONTENTS);
    }

    // RFC 2046 §4.5.1: application/octet-stream = arbitrary binary data.
    #[test]
    fn sets_content_type_octet_stream() {
        // "foo" → [Normal("foo")]
        let dir = dir_with_foo();

        assert_eq!(
            response(dir.path(), "/files/foo").header("Content-Type"),
            Some("application/octet-stream")
        );
    }

    #[test]
    fn sets_content_length_to_file_size() {
        // "foo" → [Normal("foo")]
        let dir = dir_with_foo();

        assert_eq!(
            response(dir.path(), "/files/foo").header("Content-Length"),
            Some("13")
        );
    }

    #[test]
    fn matches_spec_bytes() {
        // "foo" → [Normal("foo")]
        let dir = dir_with_foo();

        assert_eq!(
            String::from_utf8(response(dir.path(), "/files/foo").into_bytes()).unwrap(),
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 13\r\n\r\nHello, World!"
        );
    }

    #[test]
    fn keeps_contents_verbatim() {
        // "raw.txt" → [Normal("raw.txt")]
        let dir = TempDir::new();
        let contents = "line 1\r\nline 2\n\n  trailing spaces  \n";
        dir.write("raw.txt", contents);

        assert_eq!(response(dir.path(), "/files/raw.txt").body(), contents);
    }

    #[test]
    fn empty_file_is_200_with_zero_length() {
        // "empty" → [Normal("empty")]
        let dir = TempDir::new();
        dir.write("empty", "");

        let response = response(dir.path(), "/files/empty");

        assert_eq!(response.status(), &StatusCode::Ok);
        assert_eq!(response.header("Content-Length"), Some("0"));
    }

    #[test]
    fn reads_file_in_subdirectory() {
        // "sub/nested" → [Normal("sub"), Normal("nested")]
        let dir = TempDir::new();
        let sub = dir.mkdir("sub");
        std::fs::write(sub.join("nested"), "deep").unwrap();

        assert_eq!(response(dir.path(), "/files/sub/nested").body(), "deep");
    }

    // --- Missing / invalid ---

    #[test]
    fn missing_file_is_not_found() {
        // "non_existant_file" → [Normal("non_existant_file")], valid path but no such file
        let dir = dir_with_foo();

        assert!(matches!(
            StaticFile::open(dir.path(), "/files/non_existant_file"),
            Err(StaticFileError::NotFound)
        ));
    }

    #[test]
    fn missing_file_responds_404() {
        // "non_existant_file" → [Normal("non_existant_file")], valid path but no such file
        let dir = dir_with_foo();

        assert_404(response(dir.path(), "/files/non_existant_file"));
    }

    #[test]
    fn empty_filename_responds_404() {
        // "" → [] (empty list: `all(..)` is vacuously true)
        // join("") gives the served dir itself, and reading a directory fails
        let dir = dir_with_foo();

        assert_404(response(dir.path(), "/files/"));
    }

    #[test]
    fn directory_is_not_a_file() {
        // "sub" → [Normal("sub")], valid path but it is a directory
        let dir = dir_with_foo();
        dir.mkdir("sub");

        assert_404(response(dir.path(), "/files/sub"));
    }

    #[test]
    fn rejects_path_without_files_prefix() {
        // "/foo" has no "/files/" prefix: strip_prefix fails before any component check
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

        assert_404(response(&public, "/files/../secret"));
    }

    #[test]
    fn cannot_read_absolute_path() {
        // dir = <tmp>/public, request = "/files/" + "<tmp>/secret"
        // "<tmp>/secret" → [RootDir, Normal("var"), ..., Normal("secret")]
        // join() would replace dir with this absolute path
        let dir = TempDir::new();
        dir.write("secret", "top secret");
        let public = dir.mkdir("public");
        let absolute = dir.path().join("secret");

        eprintln!("{}", &public.display());

        assert_404(response(&public, &format!("/files/{}", absolute.display())));
    }

    // --- Path components ---
    // Allowlist policy: the requested path may only contain `Component::Normal` segments.

    #[test]
    fn double_dots_inside_a_name_are_allowed() {
        // "v1..2.txt" → [Normal("v1..2.txt")], the dots are part of the name
        let dir = TempDir::new();
        dir.write("v1..2.txt", "version");

        assert_eq!(response(dir.path(), "/files/v1..2.txt").body(), "version");
    }

    #[test]
    fn name_starting_with_double_dots_is_allowed() {
        // "..hidden" → [Normal("..hidden")], only a bare ".." is ParentDir
        let dir = TempDir::new();
        dir.write("..hidden", "not a parent dir");

        assert_eq!(
            response(dir.path(), "/files/..hidden").body(),
            "not a parent dir"
        );
    }

    #[test]
    fn dot_dot_is_an_invalid_path() {
        // "../foo" → [ParentDir, Normal("foo")]
        let dir = dir_with_foo();

        assert!(matches!(
            StaticFile::open(dir.path(), "/files/../foo"),
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

        assert_404(response(&public, "/files/sub/../../secret"));
    }

    // Strict policy: `..` is rejected even when it would stay inside the directory.
    #[test]
    fn dot_dot_is_rejected_even_when_staying_inside() {
        // "sub/../foo" → [Normal("sub"), ParentDir, Normal("foo")], would resolve to <dir>/foo
        let dir = dir_with_foo();
        dir.mkdir("sub");

        assert_404(response(dir.path(), "/files/sub/../foo"));
    }

    #[test]
    fn absolute_path_is_an_invalid_path() {
        // "/files//foo" → after strip_prefix: "/foo" → [RootDir, Normal("foo")]
        let dir = dir_with_foo();

        assert!(matches!(
            StaticFile::open(dir.path(), "/files//foo"),
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

        assert_404(response(&public, "/files/link"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_inside_directory_is_allowed() {
        // <dir>/alias -> <dir>/foo
        // "alias" → [Normal("alias")], canonical path stays inside dir
        let dir = dir_with_foo();
        std::os::unix::fs::symlink(dir.path().join("foo"), dir.path().join("alias")).unwrap();

        assert_eq!(response(dir.path(), "/files/alias").body(), "Hello, World!");
    }
}
