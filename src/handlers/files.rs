use std::io;

use crate::http::{headers::Headers, response::Response, status::StatusCode};

pub struct StaticFile {
    value: String,
}

#[derive(Debug, thiserror::Error)]
pub enum StaticFileError {
    #[error("File not found")]
    NotFound,
    #[error("Path does not start with /files/")]
    NoDirectoryPrefix,
}

impl From<io::Error> for StaticFileError {
    fn from(_error: io::Error) -> Self {
        StaticFileError::NotFound
    }
}

fn cleanup_path(file_path: &str) -> String {
    file_path
        .replace("..", "")
        .replace("//", "/")
        .trim_start_matches("/")
        .to_string()
}

impl StaticFile {
    pub fn open(dir: &std::path::Path, file_path: &str) -> Result<Self, StaticFileError> {
        let Some(file_path) = file_path.strip_prefix("/files/") else {
            return Err(StaticFileError::NoDirectoryPrefix);
        };
        let full_file_path = dir.join(cleanup_path(file_path));

        Ok(Self {
            value: std::fs::read_to_string(full_file_path)?,
        })
    }
}

impl From<StaticFile> for Response {
    fn from(echo: StaticFile) -> Self {
        Response::new(
            StatusCode::Ok,
            Headers::from(vec![(
                String::from("Content-Type"),
                String::from("application/octet-stream"),
            )]),
            echo.value,
        )
    }
}

impl From<StaticFileError> for Response {
    fn from(_error: StaticFileError) -> Self {
        Response::new(StatusCode::NotFound, Headers::default(), String::new())
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
        let dir = dir_with_foo();

        assert!(StaticFile::open(dir.path(), "/files/foo").is_ok());
    }

    #[test]
    fn converts_into_200_response() {
        let dir = dir_with_foo();

        assert_eq!(response(dir.path(), "/files/foo").status(), &StatusCode::Ok);
    }

    #[test]
    fn body_is_file_contents() {
        let dir = dir_with_foo();

        assert_eq!(response(dir.path(), "/files/foo").body(), SPEC_CONTENTS);
    }

    // RFC 2046 §4.5.1: application/octet-stream = arbitrary binary data.
    #[test]
    fn sets_content_type_octet_stream() {
        let dir = dir_with_foo();

        assert_eq!(
            response(dir.path(), "/files/foo").header("Content-Type"),
            Some("application/octet-stream")
        );
    }

    #[test]
    fn sets_content_length_to_file_size() {
        let dir = dir_with_foo();

        assert_eq!(
            response(dir.path(), "/files/foo").header("Content-Length"),
            Some("13")
        );
    }

    #[test]
    fn matches_spec_bytes() {
        let dir = dir_with_foo();

        assert_eq!(
            String::from_utf8(response(dir.path(), "/files/foo").into_bytes()).unwrap(),
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 13\r\n\r\nHello, World!"
        );
    }

    #[test]
    fn keeps_contents_verbatim() {
        let dir = TempDir::new();
        let contents = "line 1\r\nline 2\n\n  trailing spaces  \n";
        dir.write("raw.txt", contents);

        assert_eq!(response(dir.path(), "/files/raw.txt").body(), contents);
    }

    #[test]
    fn empty_file_is_200_with_zero_length() {
        let dir = TempDir::new();
        dir.write("empty", "");

        let response = response(dir.path(), "/files/empty");

        assert_eq!(response.status(), &StatusCode::Ok);
        assert_eq!(response.header("Content-Length"), Some("0"));
    }

    #[test]
    fn reads_file_in_subdirectory() {
        let dir = TempDir::new();
        let sub = dir.mkdir("sub");
        std::fs::write(sub.join("nested"), "deep").unwrap();

        assert_eq!(response(dir.path(), "/files/sub/nested").body(), "deep");
    }

    // --- Missing / invalid ---

    #[test]
    fn missing_file_is_not_found() {
        let dir = dir_with_foo();

        assert!(matches!(
            StaticFile::open(dir.path(), "/files/non_existant_file"),
            Err(StaticFileError::NotFound)
        ));
    }

    #[test]
    fn missing_file_responds_404() {
        let dir = dir_with_foo();

        assert_404(response(dir.path(), "/files/non_existant_file"));
    }

    #[test]
    fn empty_filename_responds_404() {
        let dir = dir_with_foo();

        assert_404(response(dir.path(), "/files/"));
    }

    #[test]
    fn directory_is_not_a_file() {
        let dir = dir_with_foo();
        dir.mkdir("sub");

        assert_404(response(dir.path(), "/files/sub"));
    }

    #[test]
    fn rejects_path_without_files_prefix() {
        let dir = dir_with_foo();

        assert!(StaticFile::open(dir.path(), "/foo").is_err());
    }

    // Path traversal: a request must never escape the served directory.
    #[test]
    fn cannot_escape_directory_with_dot_dot() {
        let dir = TempDir::new();
        dir.write("secret", "top secret");
        let public = dir.mkdir("public");

        assert_404(response(&public, "/files/../secret"));
    }

    #[test]
    fn cannot_read_absolute_path() {
        let dir = TempDir::new();
        dir.write("secret", "top secret");
        let public = dir.mkdir("public");
        let absolute = dir.path().join("secret");

        println!("{}", &public.display());

        assert_404(response(&public, &format!("/files/{}", absolute.display())));
    }
}
