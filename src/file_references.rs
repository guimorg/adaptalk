//! Safe expansion of bare, CWD-relative `@path` prompt references.

use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use thiserror::Error;

const MAX_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Error)]
pub enum FileReferenceError {
    #[error("could not resolve the current working directory")]
    CurrentDirectory(#[source] std::io::Error),
    #[error("could not resolve file-reference root {path}")]
    Root {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("file reference {reference:?} must be relative to the current working directory")]
    NotRelative { reference: String },
    #[error("file reference {reference:?} must not contain `..`")]
    ParentDirectory { reference: String },
    #[error("could not resolve file reference {reference:?}")]
    Canonicalize {
        reference: String,
        #[source]
        source: std::io::Error,
    },
    #[error("file reference {reference:?} resolves outside the current working directory")]
    OutsideWorkingDirectory { reference: String },
    #[error("file reference {reference:?} is protected")]
    Protected { reference: String },
    #[error("file reference {reference:?} is not a regular file")]
    NotRegular { reference: String },
    #[error("file reference {reference:?} exceeds the 1 MiB limit")]
    TooLarge { reference: String },
    #[error("could not read file reference {reference:?}")]
    Read {
        reference: String,
        #[source]
        source: std::io::Error,
    },
    #[error("file reference {reference:?} is not valid UTF-8")]
    NotUtf8 { reference: String },
}

/// Resolves references within one canonical working directory.
pub struct FileReferenceResolver {
    cwd: PathBuf,
    protected_config: Option<PathBuf>,
    protected_sessions: Option<PathBuf>,
}

impl FileReferenceResolver {
    pub fn for_current_dir() -> Result<Self, FileReferenceError> {
        let cwd = std::env::current_dir().map_err(FileReferenceError::CurrentDirectory)?;
        let adapt_dir = crate::config::default_config_path()
            .map_err(|error| FileReferenceError::Root {
                path: PathBuf::from("~/.adapt"),
                source: std::io::Error::other(error),
            })?
            .parent()
            .expect("config path always has a parent")
            .to_path_buf();
        Self::new(cwd, adapt_dir)
    }

    /// `adapt_dir` is injected so the path policy is deterministic in tests.
    pub fn new(
        cwd: impl AsRef<Path>,
        adapt_dir: impl AsRef<Path>,
    ) -> Result<Self, FileReferenceError> {
        let cwd = cwd
            .as_ref()
            .canonicalize()
            .map_err(|source| FileReferenceError::Root {
                path: cwd.as_ref().to_path_buf(),
                source,
            })?;
        let adapt_dir = adapt_dir.as_ref();
        Ok(Self {
            cwd,
            protected_config: canonical_if_exists(&adapt_dir.join("config.toml")),
            protected_sessions: canonical_if_exists(&adapt_dir.join("sessions")),
        })
    }

    pub fn resolve(&self, prompt: &str) -> Result<String, FileReferenceError> {
        let mut resolved = String::with_capacity(prompt.len());
        for segment in prompt.split_inclusive(char::is_whitespace) {
            let reference = segment.trim_end_matches(char::is_whitespace);
            if let Some(path) = reference.strip_prefix('@').filter(|path| !path.is_empty()) {
                resolved.push_str(&self.expand(reference, path)?);
                resolved.push_str(&segment[reference.len()..]);
            } else {
                resolved.push_str(segment);
            }
        }
        Ok(resolved)
    }

    fn expand(&self, reference: &str, path: &str) -> Result<String, FileReferenceError> {
        let candidate = Path::new(path);
        if candidate.is_absolute() || path.starts_with('~') {
            return Err(FileReferenceError::NotRelative {
                reference: reference.into(),
            });
        }
        if candidate
            .components()
            .any(|component| component == Component::ParentDir)
        {
            return Err(FileReferenceError::ParentDirectory {
                reference: reference.into(),
            });
        }
        let canonical = self.cwd.join(candidate).canonicalize().map_err(|source| {
            FileReferenceError::Canonicalize {
                reference: reference.into(),
                source,
            }
        })?;
        if !canonical.starts_with(&self.cwd) {
            return Err(FileReferenceError::OutsideWorkingDirectory {
                reference: reference.into(),
            });
        }
        if self.protected_config.as_ref() == Some(&canonical)
            || self
                .protected_sessions
                .as_ref()
                .is_some_and(|sessions| canonical.starts_with(sessions))
        {
            return Err(FileReferenceError::Protected {
                reference: reference.into(),
            });
        }
        let metadata = fs::metadata(&canonical).map_err(|source| FileReferenceError::Read {
            reference: reference.into(),
            source,
        })?;
        if !metadata.is_file() {
            return Err(FileReferenceError::NotRegular {
                reference: reference.into(),
            });
        }
        if metadata.len() > MAX_FILE_BYTES {
            return Err(FileReferenceError::TooLarge {
                reference: reference.into(),
            });
        }
        let contents =
            String::from_utf8(
                fs::read(&canonical).map_err(|source| FileReferenceError::Read {
                    reference: reference.into(),
                    source,
                })?,
            )
            .map_err(|_| FileReferenceError::NotUtf8 {
                reference: reference.into(),
            })?;
        Ok(format!(
            "<file path=\"{}\">\n{}\n</file>",
            escape_attribute(reference),
            contents
        ))
    }
}

fn canonical_if_exists(path: &Path) -> Option<PathBuf> {
    path.canonicalize().ok()
}

fn escape_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::{FileReferenceError, FileReferenceResolver, MAX_FILE_BYTES};

    struct Fixture {
        root: PathBuf,
        cwd: PathBuf,
        adapt: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "adaptalk-file-references-{name}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            let cwd = root.join("cwd");
            let adapt = root.join(".adapt");
            fs::create_dir_all(&cwd).unwrap();
            fs::create_dir_all(&adapt).unwrap();
            Self { root, cwd, adapt }
        }

        fn resolver(&self) -> FileReferenceResolver {
            FileReferenceResolver::new(&self.cwd, &self.adapt).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn expands_bare_cwd_relative_references_with_typed_paths() {
        let fixture = Fixture::new("valid");
        fs::write(fixture.cwd.join("notes.md"), "hello").unwrap();
        fs::create_dir_all(fixture.cwd.join("sub")).unwrap();
        fs::write(fixture.cwd.join("sub/file.md"), "world").unwrap();

        assert_eq!(
            fixture
                .resolver()
                .resolve("read @notes.md and @./sub/file.md")
                .unwrap(),
            "read <file path=\"@notes.md\">\nhello\n</file> and <file path=\"@./sub/file.md\">\nworld\n</file>"
        );
    }

    #[test]
    fn expands_multiple_references_without_reparsing_file_contents() {
        let fixture = Fixture::new("multiple");
        fs::write(fixture.cwd.join("one"), "@two").unwrap();
        fs::write(fixture.cwd.join("two"), "second").unwrap();
        assert_eq!(
            fixture.resolver().resolve("@one @two").unwrap(),
            "<file path=\"@one\">\n@two\n</file> <file path=\"@two\">\nsecond\n</file>"
        );
    }

    #[test]
    fn rejects_missing_large_and_non_utf8_files() {
        let fixture = Fixture::new("contents");
        assert!(matches!(
            fixture.resolver().resolve("@missing"),
            Err(FileReferenceError::Canonicalize { .. })
        ));
        fs::write(
            fixture.cwd.join("large"),
            vec![0_u8; MAX_FILE_BYTES as usize + 1],
        )
        .unwrap();
        assert!(matches!(
            fixture.resolver().resolve("@large"),
            Err(FileReferenceError::TooLarge { .. })
        ));
        fs::write(fixture.cwd.join("binary"), [0xff]).unwrap();
        assert!(matches!(
            fixture.resolver().resolve("@binary"),
            Err(FileReferenceError::NotUtf8 { .. })
        ));
    }

    #[test]
    fn accepts_a_file_exactly_one_mebibyte() {
        let fixture = Fixture::new("one-mebibyte");
        fs::write(
            fixture.cwd.join("maximum"),
            vec![b'x'; MAX_FILE_BYTES as usize],
        )
        .unwrap();
        assert!(fixture.resolver().resolve("@maximum").is_ok());
    }

    #[test]
    fn rejects_non_regular_files() {
        let fixture = Fixture::new("directory");
        fs::create_dir(fixture.cwd.join("directory")).unwrap();
        assert!(matches!(
            fixture.resolver().resolve("@directory"),
            Err(FileReferenceError::NotRegular { .. })
        ));
    }

    #[test]
    fn rejects_absolute_tilde_and_parent_paths() {
        let fixture = Fixture::new("syntax");
        assert!(matches!(
            fixture.resolver().resolve("@/tmp/file"),
            Err(FileReferenceError::NotRelative { .. })
        ));
        assert!(matches!(
            fixture.resolver().resolve("@~/file"),
            Err(FileReferenceError::NotRelative { .. })
        ));
        assert!(matches!(
            fixture.resolver().resolve("@sub/../file"),
            Err(FileReferenceError::ParentDirectory { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escapes() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new("symlink");
        fs::write(fixture.root.join("outside"), "secret").unwrap();
        symlink(fixture.root.join("outside"), fixture.cwd.join("escape")).unwrap();
        assert!(matches!(
            fixture.resolver().resolve("@escape"),
            Err(FileReferenceError::OutsideWorkingDirectory { .. })
        ));
    }

    #[test]
    fn rejects_adapt_config_and_sessions() {
        let fixture = Fixture::new("protected");
        fs::write(fixture.adapt.join("config.toml"), "bearer_token = 'secret'").unwrap();
        fs::create_dir_all(fixture.adapt.join("sessions")).unwrap();
        fs::write(fixture.adapt.join("sessions/session.json"), "secret").unwrap();
        let resolver = FileReferenceResolver::new(&fixture.root, &fixture.adapt).unwrap();
        assert!(matches!(
            resolver.resolve("@.adapt/config.toml"),
            Err(FileReferenceError::Protected { .. })
        ));
        assert!(matches!(
            resolver.resolve("@.adapt/sessions/session.json"),
            Err(FileReferenceError::Protected { .. })
        ));
    }
}
