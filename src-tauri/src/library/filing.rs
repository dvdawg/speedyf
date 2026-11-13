//! Filing: a PDF from outside the library folders is copied into an inbox
//! folder inside one of them. Copy, never move, never delete - the one
//! filesystem write this feature makes outside its own data files, and the
//! one a user could not undo from inside SpeedyF (projects.md sec. 6, sec. 12).

use super::{contained_in_any, content_fingerprint};
use crate::errors::{AppError, AppResult};
use std::fs;
use std::path::{Path, PathBuf};

/// More copies of one name than anyone files on purpose.
const MAX_SUFFIX: u32 = 10_000;

/// The path of `src` inside the library: `src` itself when it already is in
/// a root, otherwise a copy in `inbox_dir`. A copy that is already there
/// with the same bytes is reused; a different file of the same name gets
/// " (2)", " (3)", ... rather than being replaced.
pub fn copy_into_inbox(src: &Path, inbox_dir: &Path, roots: &[PathBuf]) -> AppResult<PathBuf> {
    let source = src
        .canonicalize()
        .map_err(|error| AppError::Io(format!("cannot read {}: {error}", src.display())))?;
    if !source.is_file() {
        return Err(AppError::Io(format!("not a file: {}", src.display())));
    }
    if let Some(inside) = contained_in_any(roots, &source) {
        return Ok(inside);
    }

    let inbox = inbox_dir.canonicalize().map_err(|error| {
        AppError::Io(format!(
            "cannot open the inbox folder {}: {error}",
            inbox_dir.display()
        ))
    })?;
    if !inbox.is_dir() {
        return Err(AppError::Io(format!(
            "the inbox folder is not a folder: {}",
            inbox_dir.display()
        )));
    }
    // `contained_in_any` is for files; a folder gets the same test by hand.
    let in_library = roots
        .iter()
        .filter_map(|root| root.canonicalize().ok())
        .any(|root| inbox.starts_with(root));
    if !in_library {
        return Err(AppError::Io(format!(
            "the inbox folder {} is not inside a library folder",
            inbox.display()
        )));
    }

    let name = source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| AppError::Io(format!("{} has no file name", src.display())))?;
    let stem = source
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.clone());
    let extension = source
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default();
    let fingerprint = content_fingerprint(&source);

    for n in 1..=MAX_SUFFIX {
        let candidate = if n == 1 {
            inbox.join(&name)
        } else {
            inbox.join(format!("{stem} ({n}){extension}"))
        };
        if !candidate.exists() {
            fs::copy(&source, &candidate).map_err(|error| {
                AppError::Io(format!(
                    "cannot copy {} into {}: {error}",
                    src.display(),
                    inbox.display()
                ))
            })?;
            return Ok(candidate.canonicalize().unwrap_or(candidate));
        }
        if candidate.is_file()
            && fingerprint.is_some()
            && content_fingerprint(&candidate) == fingerprint
        {
            return Ok(candidate.canonicalize().unwrap_or(candidate));
        }
    }
    Err(AppError::Io(format!(
        "too many files named {name} in {}",
        inbox.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_into_an_inbox_inside_a_root_and_never_moves() {
        let root = tempfile::tempdir().expect("root");
        let outside = tempfile::tempdir().expect("outside");
        let inbox = root.path().join("inbox");
        fs::create_dir_all(&inbox).unwrap();
        let source = outside.path().join("paper.pdf");
        fs::write(&source, b"%PDF-1.4 fixture").unwrap();
        let roots = vec![root.path().canonicalize().unwrap()];

        let filed = copy_into_inbox(&source, &inbox, &roots).expect("copy");
        assert!(filed.starts_with(root.path().canonicalize().unwrap()));
        assert!(filed.ends_with("inbox/paper.pdf"));
        assert!(source.is_file(), "the original stays where it was");
        assert_eq!(fs::read(&filed).unwrap(), b"%PDF-1.4 fixture");

        // The same file again reuses the copy already there.
        let again = copy_into_inbox(&source, &inbox, &roots).expect("again");
        assert_eq!(again, filed);

        // A different file with the same name gets a suffix, and the first
        // copy is untouched.
        let other = outside.path().join("sub");
        fs::create_dir_all(&other).unwrap();
        let other_source = other.join("paper.pdf");
        fs::write(&other_source, b"%PDF-1.4 something else").unwrap();
        let second = copy_into_inbox(&other_source, &inbox, &roots).expect("second");
        assert!(
            second.ends_with("inbox/paper (2).pdf"),
            "{}",
            second.display()
        );
        assert_eq!(fs::read(&filed).unwrap(), b"%PDF-1.4 fixture");
        let third_source = outside.path().join("deeper");
        fs::create_dir_all(&third_source).unwrap();
        let third_source = third_source.join("paper.pdf");
        fs::write(&third_source, b"%PDF-1.4 a third one").unwrap();
        let third = copy_into_inbox(&third_source, &inbox, &roots).expect("third");
        assert!(
            third.ends_with("inbox/paper (3).pdf"),
            "{}",
            third.display()
        );

        // A file already inside a root is returned as it is.
        let inside = root.path().join("already.pdf");
        fs::write(&inside, b"in").unwrap();
        let unchanged = copy_into_inbox(&inside, &inbox, &roots).expect("inside");
        assert_eq!(unchanged, inside.canonicalize().unwrap());
        assert_eq!(fs::read_dir(&inbox).unwrap().count(), 3);
    }

    #[test]
    fn refuses_an_inbox_outside_every_root_and_a_missing_source() {
        let root = tempfile::tempdir().expect("root");
        let elsewhere = tempfile::tempdir().expect("elsewhere");
        let source = elsewhere.path().join("paper.pdf");
        fs::write(&source, b"pdf").unwrap();
        let roots = vec![root.path().canonicalize().unwrap()];

        let error = copy_into_inbox(&source, elsewhere.path(), &roots).expect_err("outside");
        assert!(
            error.to_string().contains("not inside a library folder"),
            "{error}"
        );
        assert!(copy_into_inbox(&source, root.path(), &[]).is_err());
        assert!(copy_into_inbox(&elsewhere.path().join("nope.pdf"), root.path(), &roots).is_err());
        assert!(copy_into_inbox(&source, &root.path().join("missing-inbox"), &roots).is_err());
        assert_eq!(
            fs::read_dir(root.path()).unwrap().count(),
            0,
            "nothing was written"
        );
    }
}
