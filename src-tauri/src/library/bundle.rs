//! Export a project as a bundle (projects.md sec. 5): one `.zip` holding the
//! project's `.bib` and the main PDF of each of its items, named by cite
//! key, for handing a reading list to a co-author or a submission system.
//!
//! The zip is streamed to a temp file beside the destination and renamed
//! over it once complete, so a failed export leaves nothing behind and a
//! half-written archive is never visible under the chosen name. PDFs are
//! copied byte for byte and stored, not recompressed (they are compressed
//! already); the `.bib` is deflated. Library files are only ever read.

use super::store::PaperStore;
use crate::errors::{AppError, AppResult};
use serde::Serialize;
use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// What a bundle will hold, read from the store under its lock; the zip is
/// written afterwards without it.
pub struct BundlePlan {
    /// `<project name>.bib`, made safe as a file name.
    pub bib_name: String,
    pub bib_text: String,
    /// The papers whose keys go into the `.bib`, pinned once it is written.
    pub paper_ids: Vec<String>,
    pub members: Vec<BundleMember>,
}

/// One item of the project: its cite key and its main file, if it has one.
pub struct BundleMember {
    pub cite_key: String,
    pub main_file: Option<PathBuf>,
}

/// What an export wrote, for the inline result.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleReportDto {
    pub path: String,
    pub bib_name: String,
    /// Names inside the zip (`pdfs/<key>.pdf`), in project order.
    pub pdfs: Vec<String>,
    /// Cite keys of the items whose main file is not on disk.
    pub missing: Vec<String>,
}

impl PaperStore {
    /// The bundle for `project_id`: its `.bib` as Export .bib would write
    /// it, and each member's main file (its first, `mainfile`'s pick).
    pub fn project_bundle_plan(&self, project_id: &str) -> AppResult<BundlePlan> {
        self.ensure_writable()?;
        let name = self.project_name(project_id)?;
        let (bib_text, paper_ids) = self.render_project_bib(project_id)?;
        let members = self
            .project_members(project_id)?
            .into_iter()
            .map(|paper| BundleMember {
                cite_key: if paper.cite_key.is_empty() {
                    paper.id.clone()
                } else {
                    paper.cite_key.clone()
                },
                main_file: paper
                    .files
                    .first()
                    .filter(|file| !file.missing)
                    .map(|file| PathBuf::from(&file.path)),
            })
            .collect();
        Ok(BundlePlan {
            bib_name: format!("{}.bib", safe_file_stem(&name, "project")),
            bib_text,
            paper_ids,
            members,
        })
    }
}

/// Writes `plan` as a zip at `path`: a temp file in the destination folder,
/// renamed over `path` only once the archive is complete. A main file that
/// is gone (or cannot be opened) by the time it is copied is skipped and
/// reported missing, like one the store already knew was.
pub fn write_bundle(plan: &BundlePlan, path: &Path) -> AppResult<BundleReportDto> {
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .ok_or_else(|| AppError::Io(format!("{} has no parent folder", path.display())))?;
    let mut temp = tempfile::Builder::new()
        .prefix(".speedyf-")
        .suffix(".zip.tmp")
        .tempfile_in(dir)
        .map_err(|error| AppError::Io(format!("cannot write in {}: {error}", dir.display())))?;
    let write_error =
        |error: io::Error| AppError::Io(format!("cannot write {}: {error}", path.display()));
    let zip_error = |error: zip::result::ZipError| {
        AppError::Io(format!("cannot write {}: {error}", path.display()))
    };

    let mut pdfs = Vec::new();
    let mut missing = Vec::new();
    {
        let mut zip = ZipWriter::new(temp.as_file_mut());
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let stored = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .large_file(false);
        zip.start_file(plan.bib_name.as_str(), deflated)
            .map_err(zip_error)?;
        zip.write_all(plan.bib_text.as_bytes())
            .map_err(write_error)?;

        let mut taken: HashSet<String> = HashSet::new();
        for member in &plan.members {
            let Some(source) = member
                .main_file
                .as_deref()
                .and_then(|source| File::open(source).ok())
            else {
                missing.push(member.cite_key.clone());
                continue;
            };
            let large = source.metadata().map(|m| m.len()).unwrap_or(0) >= u32::MAX as u64;
            let name = unique_pdf_name(&member.cite_key, &mut taken);
            zip.start_file(name.as_str(), stored.large_file(large))
                .map_err(zip_error)?;
            io::copy(&mut io::BufReader::new(source), &mut zip).map_err(write_error)?;
            pdfs.push(name);
        }
        zip.finish().map_err(zip_error)?;
    }
    temp.as_file().sync_all().map_err(write_error)?;
    temp.persist(path).map_err(|error| {
        AppError::Io(format!(
            "cannot replace {}: {}",
            path.display(),
            error.error
        ))
    })?;
    Ok(BundleReportDto {
        path: path.to_string_lossy().into_owned(),
        bib_name: plan.bib_name.clone(),
        pdfs,
        missing,
    })
}

/// `pdfs/<key>.pdf`, with the key made safe as a file name on every
/// system, and a `-2`, `-3`... suffix should two keys come out the same
/// (keys are unique, but making them safe can fold two together).
fn unique_pdf_name(key: &str, taken: &mut HashSet<String>) -> String {
    let stem = safe_file_stem(key, "item");
    let mut name = format!("pdfs/{stem}.pdf");
    let mut n = 1;
    while !taken.insert(name.to_lowercase()) {
        n += 1;
        name = format!("pdfs/{stem}-{n}.pdf");
    }
    name
}

/// `text` with the characters Windows, macOS or a zip path cannot carry in
/// a file name replaced by `_`; `fallback` when nothing is left.
fn safe_file_stem(text: &str, fallback: &str) -> String {
    let cleaned: String = text
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if cleaned.is_empty() {
        fallback.to_owned()
    } else {
        cleaned.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Read;

    fn plan(members: Vec<BundleMember>) -> BundlePlan {
        BundlePlan {
            bib_name: "Thesis.bib".into(),
            bib_text: "@article{abel1999zeta,\n}\n".into(),
            paper_ids: Vec::new(),
            members,
        }
    }

    fn member(key: &str, file: Option<&Path>) -> BundleMember {
        BundleMember {
            cite_key: key.into(),
            main_file: file.map(Path::to_path_buf),
        }
    }

    fn entries(path: &Path) -> Vec<(String, Vec<u8>)> {
        let mut archive = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
        (0..archive.len())
            .map(|i| {
                let mut entry = archive.by_index(i).unwrap();
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                (entry.name().unwrap().into_owned(), bytes)
            })
            .collect()
    }

    #[test]
    fn the_zip_holds_the_bib_and_each_main_pdf_by_cite_key() {
        let library = tempfile::tempdir().unwrap();
        let a = library.path().join("Attention.pdf");
        let b = library.path().join("b.pdf");
        fs::write(&a, b"%PDF-1.7 attention").unwrap();
        fs::write(&b, b"%PDF-1.4 bert").unwrap();
        let out = tempfile::tempdir().unwrap();
        let path = out.path().join("bundle.zip");

        let report = write_bundle(
            &plan(vec![
                member("vaswani2017attention", Some(&a)),
                member("gone2020", Some(&library.path().join("gone.pdf"))),
                member("devlin2019bert", Some(&b)),
                member("nofile2021", None),
            ]),
            &path,
        )
        .unwrap();

        assert_eq!(
            report.pdfs,
            vec!["pdfs/vaswani2017attention.pdf", "pdfs/devlin2019bert.pdf"]
        );
        assert_eq!(report.missing, vec!["gone2020", "nofile2021"]);
        let entries = entries(&path);
        let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "Thesis.bib",
                "pdfs/vaswani2017attention.pdf",
                "pdfs/devlin2019bert.pdf"
            ],
            "the .bib and the PDFs, nothing else"
        );
        assert_eq!(entries[0].1, b"@article{abel1999zeta,\n}\n");
        assert_eq!(entries[1].1, b"%PDF-1.7 attention", "a byte copy");
        // The library is only read.
        assert_eq!(fs::read(&a).unwrap(), b"%PDF-1.7 attention");
        assert_eq!(fs::read_dir(library.path()).unwrap().count(), 2);
        assert_eq!(
            fs::read_dir(out.path()).unwrap().count(),
            1,
            "no temp file left beside it"
        );
    }

    #[test]
    fn names_never_collide_and_are_safe_on_every_system() {
        let mut taken = HashSet::new();
        assert_eq!(unique_pdf_name("a:b", &mut taken), "pdfs/a_b.pdf");
        assert_eq!(unique_pdf_name("a/b", &mut taken), "pdfs/a_b-2.pdf");
        assert_eq!(unique_pdf_name("A_B", &mut taken), "pdfs/A_B-3.pdf");
        assert_eq!(unique_pdf_name("..", &mut taken), "pdfs/item.pdf");
        assert_eq!(safe_file_stem("  My: Thesis?  ", "project"), "My_ Thesis_");
        assert_eq!(safe_file_stem("", "project"), "project");

        let library = tempfile::tempdir().unwrap();
        let a = library.path().join("a.pdf");
        fs::write(&a, b"one").unwrap();
        let path = library.path().join("out.zip");
        let report = write_bundle(
            &plan(vec![member("k:1", Some(&a)), member("k/1", Some(&a))]),
            &path,
        )
        .unwrap();
        assert_eq!(report.pdfs, vec!["pdfs/k_1.pdf", "pdfs/k_1-2.pdf"]);
        assert_eq!(entries(&path).len(), 3);
    }

    #[test]
    fn a_failed_write_leaves_no_temp_file_and_no_zip() {
        let out = tempfile::tempdir().unwrap();
        // The destination is a folder: the rename over it fails after the
        // archive was written to the temp file.
        let path = out.path().join("taken");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("keep"), b"x").unwrap();
        let library = tempfile::tempdir().unwrap();
        let a = library.path().join("a.pdf");
        fs::write(&a, b"one").unwrap();
        assert!(write_bundle(&plan(vec![member("a", Some(&a))]), &path).is_err());
        let left: Vec<_> = fs::read_dir(out.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, vec![std::ffi::OsString::from("taken")]);
        assert!(path.join("keep").exists());

        // No folder to write in at all.
        assert!(write_bundle(&plan(vec![]), &out.path().join("nope").join("b.zip")).is_err());
        assert!(write_bundle(&plan(vec![]), Path::new("b.zip")).is_err());
    }
}
