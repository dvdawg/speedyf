//! Export a project as a bundle (projects.md sec. 5): one `.zip` holding the
//! project's `.bib` and the main PDF of each of its items, named by cite
//! key, for handing a reading list to a co-author or a submission system;
//! the project's extension files (its notepad, projects.md sec. 13) under
//! `project-files/`; and `speedyf-bundle.json`, which lets an import put it
//! all back together as a project.
//!
//! The zip is streamed to a temp file beside the destination and renamed
//! over it once complete, so a failed export leaves nothing behind and a
//! half-written archive is never visible under the chosen name. PDFs are
//! copied byte for byte and stored, not recompressed (they are compressed
//! already); the `.bib` is deflated. Library files are only ever read.

use super::store::PaperStore;
use crate::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};
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
    /// The project's name, for the import to make a project of.
    pub project_name: String,
    /// The project's extension files: (name in the zip, file on disk).
    pub files: Vec<(String, PathBuf)>,
}

/// The name of the manifest inside a bundle.
pub const MANIFEST_NAME: &str = "speedyf-bundle.json";

/// What `speedyf-bundle.json` says: enough to rebuild the project.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleManifest {
    pub version: u32,
    pub project: String,
    pub items: Vec<BundleItem>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleItem {
    pub cite_key: String,
    /// Its PDF inside the zip, when it has one there.
    #[serde(default)]
    pub pdf: Option<String>,
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
    /// Extension files carried along (a notepad and its images).
    #[serde(default)]
    pub files: u32,
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
            files: super::projectfiles::bundle_entries(self.data_dir(), project_id),
            project_name: name,
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
    let mut items: Vec<BundleItem> = Vec::new();
    let mut files: u32 = 0;
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
                items.push(BundleItem {
                    cite_key: member.cite_key.clone(),
                    pdf: None,
                });
                continue;
            };
            let large = source.metadata().map(|m| m.len()).unwrap_or(0) >= u32::MAX as u64;
            let name = unique_pdf_name(&member.cite_key, &mut taken);
            zip.start_file(name.as_str(), stored.large_file(large))
                .map_err(zip_error)?;
            io::copy(&mut io::BufReader::new(source), &mut zip).map_err(write_error)?;
            items.push(BundleItem {
                cite_key: member.cite_key.clone(),
                pdf: Some(name.clone()),
            });
            pdfs.push(name);
        }
        for (name, source) in &plan.files {
            let Ok(bytes) = std::fs::read(source) else {
                continue;
            };
            zip.start_file(name.as_str(), deflated).map_err(zip_error)?;
            zip.write_all(&bytes).map_err(write_error)?;
            files += 1;
        }
        let manifest = BundleManifest {
            version: 1,
            project: plan.project_name.clone(),
            items,
        };
        let json = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| AppError::Internal(format!("bundle manifest: {error}")))?;
        zip.start_file(MANIFEST_NAME, deflated).map_err(zip_error)?;
        zip.write_all(&json).map_err(write_error)?;
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
        files,
    })
}

/// What an import needs out of a bundle.
pub struct BundleContents {
    pub manifest: BundleManifest,
    /// PDFs written out of the zip into the temp folder: (zip name, file).
    pub pdfs: Vec<(String, PathBuf)>,
    /// Extension files: (zip name under `project-files/`, bytes).
    pub files: Vec<(String, Vec<u8>)>,
}

/// The most a bundle's entries may inflate to, all together.
const MAX_BUNDLE_INFLATED: u64 = 8 * 1024 * 1024 * 1024;

/// Read a bundle: its manifest (or one made from its `.bib` name and PDF
/// names, for a bundle written before manifests), its PDFs written into
/// `temp`, and its project files in memory. Entry names are never used as
/// paths: a PDF is written under a name made safe here, and project files
/// are placed later through `projectfiles::bundle_target`.
pub fn read_bundle(path: &Path, temp: &Path) -> AppResult<BundleContents> {
    use std::io::Read;
    let file = File::open(path)
        .map_err(|error| AppError::Io(format!("cannot open {}: {error}", path.display())))?;
    let mut archive = zip::ZipArchive::new(io::BufReader::new(file))
        .map_err(|error| AppError::Io(format!("{} is not a bundle: {error}", path.display())))?;
    let mut manifest: Option<BundleManifest> = None;
    let mut bib_stem: Option<String> = None;
    let mut pdfs = Vec::new();
    let mut files = Vec::new();
    let mut total: u64 = 0;
    let mut taken = HashSet::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| AppError::Io(format!("bundle: {error}")))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().map(|n| n.into_owned()).unwrap_or_default();
        total = total.saturating_add(entry.size());
        if total > MAX_BUNDLE_INFLATED {
            return Err(AppError::Unsupported("the bundle is too large".into()));
        }
        if name == MANIFEST_NAME {
            let mut bytes = Vec::new();
            (&mut entry)
                .take(4 * 1024 * 1024)
                .read_to_end(&mut bytes)
                .map_err(|error| AppError::Io(format!("bundle: {error}")))?;
            manifest = serde_json::from_slice(&bytes).ok();
        } else if let Some(stem) = name.strip_suffix(".bib").filter(|s| !s.contains('/')) {
            bib_stem = Some(stem.to_owned());
        } else if let Some(file_name) = name
            .strip_prefix("pdfs/")
            .filter(|rest| !rest.contains('/') && rest.to_lowercase().ends_with(".pdf"))
        {
            let stem = safe_file_stem(file_name.trim_end_matches(".pdf"), "item");
            let out = temp.join(unique_pdf_name(&stem, &mut taken).trim_start_matches("pdfs/"));
            let mut target = File::create(&out).map_err(|error| {
                AppError::Io(format!("cannot write {}: {error}", out.display()))
            })?;
            io::copy(&mut entry, &mut target)
                .map_err(|error| AppError::Io(format!("bundle: {error}")))?;
            pdfs.push((name, out));
        } else if name.starts_with(&format!("{}/", super::projectfiles::ROOT_DIR)) {
            let mut bytes = Vec::new();
            (&mut entry)
                .take(super::projectfiles::MAX_FILE_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| AppError::Io(format!("bundle: {error}")))?;
            if bytes.len() <= super::projectfiles::MAX_FILE_BYTES {
                files.push((name, bytes));
            }
        }
    }
    let manifest = match manifest {
        Some(manifest) => manifest,
        None => BundleManifest {
            version: 1,
            project: bib_stem.unwrap_or_else(|| "Imported".to_owned()),
            items: pdfs
                .iter()
                .map(|(name, _)| BundleItem {
                    cite_key: name
                        .trim_start_matches("pdfs/")
                        .trim_end_matches(".pdf")
                        .to_owned(),
                    pdf: Some(name.clone()),
                })
                .collect(),
        },
    };
    Ok(BundleContents {
        manifest,
        pdfs,
        files,
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
            project_name: "Thesis".into(),
            files: Vec::new(),
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
                "pdfs/devlin2019bert.pdf",
                MANIFEST_NAME,
            ],
            "the .bib, the PDFs and the manifest, nothing else"
        );
        let manifest: BundleManifest = serde_json::from_slice(&entries[3].1).unwrap();
        assert_eq!(manifest.project, "Thesis");
        assert_eq!(
            manifest
                .items
                .iter()
                .map(|i| (i.cite_key.as_str(), i.pdf.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                (
                    "vaswani2017attention",
                    Some("pdfs/vaswani2017attention.pdf")
                ),
                ("gone2020", None),
                ("devlin2019bert", Some("pdfs/devlin2019bert.pdf")),
                ("nofile2021", None),
            ]
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
        assert_eq!(entries(&path).len(), 4);
    }

    #[test]
    fn a_bundle_round_trips_its_notepad_and_assets() {
        let data = tempfile::tempdir().unwrap();
        let note = super::super::projectfiles::file_path(
            data.path(),
            "dev.speedyf.notepad",
            "p1",
            "notepad.md",
        )
        .unwrap();
        let asset = super::super::projectfiles::file_path(
            data.path(),
            "dev.speedyf.notepad",
            "p1",
            "assets/fig.png",
        )
        .unwrap();
        super::super::projectfiles::write_atomic(&note, "# Notes \u{e9}\n@a2020".as_bytes())
            .unwrap();
        super::super::projectfiles::write_atomic(&asset, &[0x89, b'P', b'N', b'G']).unwrap();
        let library = tempfile::tempdir().unwrap();
        let a = library.path().join("a.pdf");
        fs::write(&a, b"%PDF a").unwrap();
        let mut with_files = plan(vec![member("a2020", Some(&a))]);
        with_files.files = super::super::projectfiles::bundle_entries(data.path(), "p1");
        let out = tempfile::tempdir().unwrap();
        let path = out.path().join("b.zip");
        let report = write_bundle(&with_files, &path).unwrap();
        assert_eq!(report.files, 2);

        let temp = tempfile::tempdir().unwrap();
        let contents = read_bundle(&path, temp.path()).unwrap();
        assert_eq!(contents.manifest.project, "Thesis");
        assert_eq!(contents.pdfs.len(), 1);
        assert_eq!(fs::read(&contents.pdfs[0].1).unwrap(), b"%PDF a");
        let other = tempfile::tempdir().unwrap();
        for (name, bytes) in &contents.files {
            let target =
                super::super::projectfiles::bundle_target(other.path(), "p2", name).unwrap();
            super::super::projectfiles::write_atomic(&target, bytes).unwrap();
        }
        let back = super::super::projectfiles::file_path(
            other.path(),
            "dev.speedyf.notepad",
            "p2",
            "notepad.md",
        )
        .unwrap();
        assert_eq!(fs::read_to_string(back).unwrap(), "# Notes \u{e9}\n@a2020");
        let back_asset = super::super::projectfiles::file_path(
            other.path(),
            "dev.speedyf.notepad",
            "p2",
            "assets/fig.png",
        )
        .unwrap();
        assert_eq!(fs::read(back_asset).unwrap(), vec![0x89, b'P', b'N', b'G']);
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
