//! Sending a document to a printer, and asking a printer what it can do.
//!
//! Sits beside `external`, not inside `engine`: nothing here touches PDFium or
//! the worker thread. The print-ready PDF is built by the engine (see
//! `Work::BuildPrintPdf`); this module only takes the finished file and hands
//! it to CUPS.
//!
//! Everything that decides *what* gets run is a pure function with tests. The
//! job description crosses from the webview, so it is treated as untrusted:
//! nothing reaches an argument list without matching a grammar first.

use crate::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Files we made, and the only ones we will print or delete.
const TEMP_PREFIX: &str = "speedyf-print-";
/// A print job left behind by a crash is swept after this long.
const STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(60 * 60);
/// CUPS destination names, option keys and option values all live in this
/// alphabet. Anything outside it is refused rather than escaped.
const MAX_TOKEN_BYTES: usize = 128;
const MAX_COPIES: u32 = 99;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PrinterDto {
    pub name: String,
    pub is_default: bool,
}

/// One thing a printer can be asked to vary, as it reports it.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PrintOptionDto {
    /// what CUPS calls it: "Duplex"
    pub key: String,
    /// what a person calls it: "2-Sided Printing"
    pub label: String,
    pub choices: Vec<String>,
    pub default: String,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PrintJobDto {
    pub printer: String,
    pub copies: u32,
    /// None prints everything; otherwise a CUPS page-range list.
    pub range: Option<String>,
    /// Chosen values for the options above, as (key, value).
    pub options: Vec<(String, String)>,
}

/// A CUPS name: letters, digits, and the three punctuation marks CUPS allows.
/// Deliberately narrow — this is the difference between an argument and an
/// instruction.
fn is_safe_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TOKEN_BYTES
        // A leading dash makes a value indistinguishable from a flag once it
        // is in an argument list: a printer named "-d" would be read by lp as
        // an option, not a destination. Injection does not need a shell.
        && !value.starts_with('-')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// A CUPS page-range list: `3`, `2-5`, `1,4,7-9`. No spaces, no open ends.
fn is_page_range(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TOKEN_BYTES
        && value.split(',').all(|part| {
            let mut bounds = part.split('-');
            let (Some(first), second, None) = (bounds.next(), bounds.next(), bounds.next()) else {
                return false;
            };
            let page = |text: &str| -> Option<u32> {
                (!text.is_empty() && text.len() <= 6 && text.bytes().all(|b| b.is_ascii_digit()))
                    .then(|| text.parse().ok())
                    .flatten()
                    .filter(|n| *n > 0)
            };
            match (page(first), second) {
                (Some(from), None) => {
                    let _ = from;
                    true
                }
                (Some(from), Some(to)) => page(to).is_some_and(|to| to >= from),
                _ => false,
            }
        })
}

/// Where a print job's PDF goes. The name is what lets us prove later that a
/// path we are asked to print or delete is one of ours.
pub fn temp_print_path() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{TEMP_PREFIX}{}-{n}.pdf", std::process::id()))
}

/// Whether `path` is a print file we created, in the directory we create them.
///
/// Guards both printing and deleting: the webview names these paths, so
/// without this, "discard my print job" is an arbitrary file delete.
pub fn is_print_temp(path: &Path) -> bool {
    let named = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(TEMP_PREFIX) && name.ends_with(".pdf"));
    if !named {
        return false;
    }
    // Compare resolved parents: macOS hands out /var/folders/… which is a
    // symlink to /private/var/folders/….
    let parent = path.parent().and_then(|p| p.canonicalize().ok());
    let temp = std::env::temp_dir().canonicalize().ok();
    matches!((parent, temp), (Some(a), Some(b)) if a == b)
}

/// Delete a finished print job. A path that is not ours is refused, not
/// ignored — a caller asking for it has a bug worth surfacing.
pub fn discard(path: &str) -> AppResult<()> {
    let path = Path::new(path);
    if !is_print_temp(path) {
        return Err(AppError::Unsupported(
            "refusing to delete a file that is not a print job".into(),
        ));
    }
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        // Already gone is the outcome we wanted.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(AppError::Io(format!("could not remove print job: {e}"))),
    }
}

/// Copy a prepared print job somewhere the user chose — "print to PDF",
/// which for us is just keeping the file we were going to print anyway.
///
/// Refuses a source we did not prepare, for the same reason `discard` does.
/// The destination is the user's own pick from a native save dialog, so it is
/// not second-guessed beyond requiring a parent directory that exists.
pub fn export_to(source: &str, dest: &str) -> AppResult<()> {
    let source = Path::new(source);
    if !is_print_temp(source) {
        return Err(AppError::Unsupported(
            "refusing to export a file that is not a print job".into(),
        ));
    }
    let dest = Path::new(dest);
    if dest.parent().is_some_and(|parent| !parent.is_dir()) {
        return Err(AppError::Io("that folder does not exist".into()));
    }
    std::fs::copy(source, dest)
        .map(|_| ())
        .map_err(|e| AppError::Io(format!("could not save the PDF: {e}")))
}

/// Remove print jobs a previous run left behind. Best effort by design: this
/// runs at startup and must never be the reason the app fails to open.
pub fn sweep_stale_prints() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_print_temp(&path) {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > STALE_AFTER);
        if stale {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// The exact argument list for `lp`, or a refusal.
///
/// Every field of the job is checked against a grammar before it becomes an
/// argument. Nothing is escaped or quoted, because nothing is interpolated:
/// the caller passes these to `Command::arg`, which never involves a shell.
pub fn lp_arguments(job: &PrintJobDto, path: &Path) -> AppResult<Vec<String>> {
    let refuse = |what: &str| AppError::Unsupported(format!("refusing to print: {what}"));

    if !is_safe_token(&job.printer) {
        return Err(refuse("printer name is not a CUPS destination"));
    }
    if job.copies == 0 || job.copies > MAX_COPIES {
        return Err(refuse("copies must be between 1 and 99"));
    }
    if !is_print_temp(path) {
        return Err(refuse("that file is not a prepared print job"));
    }

    let mut args = vec![
        "-d".to_string(),
        job.printer.clone(),
        "-n".to_string(),
        job.copies.to_string(),
    ];

    if let Some(range) = &job.range {
        if !is_page_range(range) {
            return Err(refuse("page range is malformed"));
        }
        args.push("-o".to_string());
        args.push(format!("page-ranges={range}"));
    }

    for (key, value) in &job.options {
        if !is_safe_token(key) || !is_safe_token(value) {
            return Err(refuse("printer option is not a plain CUPS token"));
        }
        args.push("-o".to_string());
        args.push(format!("{key}={value}"));
    }

    // "--" would be cleaner, but lp does not accept it; the path is safe by
    // construction because is_print_temp just proved we created it.
    args.push(path.to_string_lossy().into_owned());
    Ok(args)
}

/// Printers CUPS knows about, with the default marked.
pub fn parse_printers(lpstat_p: &str, lpstat_d: &str) -> Vec<PrinterDto> {
    // "system default destination: HP_ENVY_7640_series"
    let default = lpstat_d
        .lines()
        .find_map(|line| {
            line.rsplit_once(": ")
                .map(|(_, name)| name.trim().to_string())
        })
        .filter(|name| is_safe_token(name));

    lpstat_p
        .lines()
        // "printer HP_ENVY_7640_series is idle.  enabled since …"
        .filter_map(|line| line.strip_prefix("printer ")?.split_whitespace().next())
        .filter(|name| is_safe_token(name))
        .map(|name| PrinterDto {
            is_default: default.as_deref() == Some(name),
            name: name.to_string(),
        })
        .collect()
}

/// What a printer can vary, from `lpoptions -l`.
///
/// Each line reads `Key/Human Label: choice *default choice`, where the
/// asterisk marks what the printer would do if we said nothing.
pub fn parse_lpoptions(text: &str) -> Vec<PrintOptionDto> {
    text.lines()
        .filter_map(|line| {
            let (name, values) = line.split_once(':')?;
            let (key, label) = name.split_once('/').unwrap_or((name, name));
            let key = key.trim();
            if !is_safe_token(key) {
                return None;
            }
            let mut choices = Vec::new();
            let mut default = String::new();
            for raw in values.split_whitespace() {
                let (starred, value) = match raw.strip_prefix('*') {
                    Some(rest) => (true, rest),
                    None => (false, raw),
                };
                if !is_safe_token(value) {
                    continue;
                }
                if starred {
                    default = value.to_string();
                }
                choices.push(value.to_string());
            }
            // A printer that offers one choice is offering nothing.
            if choices.len() < 2 {
                return None;
            }
            if default.is_empty() {
                default = choices[0].clone();
            }
            Some(PrintOptionDto {
                key: key.to_string(),
                label: label.trim().to_string(),
                choices,
                default,
            })
        })
        .collect()
}

/// Run one of the CUPS command-line tools and hand back its stdout.
///
/// This is the *only* platform-gated thing in the module, and deliberately so.
/// Gating the callers instead left every parser and validator below
/// unreachable on Windows, which is a dead-code error under `-D warnings` —
/// the same shape of bug as the ungated `RunEvent::Opened`. Keeping one code
/// path for every platform means a lint can no longer disagree across them.
#[cfg(unix)]
fn run(program: &str, args: &[String]) -> AppResult<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| AppError::Io(format!("could not run {program}: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        return Err(AppError::Io(format!(
            "{program} failed{}",
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Windows ships no CUPS tools. Refusing here rather than at each call site
/// is what keeps the rest of this module platform-independent.
#[cfg(not(unix))]
fn run(_program: &str, _args: &[String]) -> AppResult<String> {
    Err(AppError::Unsupported(
        "printing is not supported on this platform yet".into(),
    ))
}

/// Printers CUPS knows about.
///
/// A machine with no printers is not an error — `lpstat` exits non-zero for
/// it, and so does a platform with no `lpstat` at all. An empty list is
/// exactly what the dialog should show in both cases, and "Save as PDF" keeps
/// working regardless, since that path never touches a printer.
pub fn list_printers() -> AppResult<Vec<PrinterDto>> {
    let printers = run("lpstat", &["-p".to_string()]).unwrap_or_default();
    let default = run("lpstat", &["-d".to_string()]).unwrap_or_default();
    Ok(parse_printers(&printers, &default))
}

pub fn printer_options(printer: &str) -> AppResult<Vec<PrintOptionDto>> {
    if !is_safe_token(printer) {
        return Err(AppError::Unsupported(
            "that is not a CUPS destination".into(),
        ));
    }
    let text = run(
        "lpoptions",
        &["-l".to_string(), "-d".to_string(), printer.to_string()],
    )
    .unwrap_or_default();
    Ok(parse_lpoptions(&text))
}

/// Hand the job to CUPS.
///
/// Waits for `lp` and reports what it said. Unlike opening a browser, a print
/// that did not happen is worth telling the user about — and `lp` fails
/// cheaply and immediately when a queue is wrong. The job is validated before
/// the platform is consulted, so a malformed one is refused the same way
/// everywhere.
pub fn submit(job: &PrintJobDto, path: &Path) -> AppResult<()> {
    let args = lp_arguments(job, path)?;
    run("lp", &args).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(printer: &str) -> PrintJobDto {
        PrintJobDto {
            printer: printer.into(),
            copies: 1,
            range: None,
            options: Vec::new(),
        }
    }

    /// A path that passes `is_print_temp`, so argument tests exercise the
    /// grammar rather than the file check.
    fn print_path() -> PathBuf {
        temp_print_path()
    }

    #[test]
    fn reads_the_printers_lpstat_reports() {
        // Verbatim from a real machine.
        let p = "printer HP_ENVY_7640_series is idle.  enabled since Sat May 30 12:05:02 2026\n\
                 printer Office_Laser is idle.  enabled since Mon Jan  6 09:00:00 2026\n";
        let d = "system default destination: HP_ENVY_7640_series\n";
        assert_eq!(
            parse_printers(p, d),
            vec![
                PrinterDto {
                    name: "HP_ENVY_7640_series".into(),
                    is_default: true
                },
                PrinterDto {
                    name: "Office_Laser".into(),
                    is_default: false
                },
            ]
        );
    }

    #[test]
    fn survives_a_machine_with_no_printers() {
        assert!(parse_printers("", "no system default destination\n").is_empty());
        assert!(parse_printers("lpstat: No destinations added.\n", "").is_empty());
    }

    #[test]
    fn reads_what_a_printer_can_vary() {
        // Verbatim from `lpoptions -l` on a real HP ENVY.
        let text = "Collate/Collate: True *False\n\
                    cupsPrintQuality/Quality: Draft *Normal High\n\
                    Duplex/2-Sided Printing: None *DuplexNoTumble DuplexTumble\n\
                    MediaType/MediaType: stationery photographic-glossy *any\n";
        let options = parse_lpoptions(text);
        let duplex = options
            .iter()
            .find(|option| option.key == "Duplex")
            .expect("duplex");
        assert_eq!(duplex.label, "2-Sided Printing");
        assert_eq!(duplex.default, "DuplexNoTumble");
        assert_eq!(duplex.choices, ["None", "DuplexNoTumble", "DuplexTumble"]);
        assert_eq!(
            options
                .iter()
                .find(|o| o.key == "cupsPrintQuality")
                .unwrap()
                .default,
            "Normal"
        );
    }

    #[test]
    fn ignores_options_that_offer_no_choice() {
        // A single-valued option is a fact, not a control worth showing.
        assert!(parse_lpoptions("PageRegion/Setting Page Region: *Letter\n").is_empty());
        assert!(parse_lpoptions("not an option line\n").is_empty());
    }

    #[test]
    fn builds_the_argument_list_for_an_ordinary_job() {
        let path = print_path();
        let mut j = job("HP_ENVY_7640_series");
        j.copies = 2;
        j.range = Some("1-4,9".into());
        j.options = vec![
            ("Duplex".into(), "DuplexNoTumble".into()),
            ("PageSize".into(), "Letter".into()),
        ];
        assert_eq!(
            lp_arguments(&j, &path).expect("valid job"),
            vec![
                "-d".to_string(),
                "HP_ENVY_7640_series".into(),
                "-n".into(),
                "2".into(),
                "-o".into(),
                "page-ranges=1-4,9".into(),
                "-o".into(),
                "Duplex=DuplexNoTumble".into(),
                "-o".into(),
                "PageSize=Letter".into(),
                path.to_string_lossy().into_owned(),
            ]
        );
    }

    #[test]
    fn refuses_a_printer_name_that_is_an_instruction() {
        // The job description comes from the webview. None of these should
        // ever reach an argument list, escaped or otherwise.
        let path = print_path();
        for hostile in [
            "HP; rm -rf /",
            "HP ENVY",
            "$(whoami)",
            "`id`",
            "../../etc/passwd",
            "-d",
            "",
        ] {
            assert!(
                lp_arguments(&job(hostile), &path).is_err(),
                "should have refused {hostile:?}"
            );
        }
    }

    #[test]
    fn refuses_absurd_copy_counts() {
        let path = print_path();
        for copies in [0, 100, 10_000] {
            let mut j = job("Printer");
            j.copies = copies;
            assert!(lp_arguments(&j, &path).is_err(), "copies={copies}");
        }
    }

    #[test]
    fn refuses_a_malformed_page_range() {
        let path = print_path();
        for range in ["1-", "-3", "1;2", "1 2", "0", "5-2", "a-b", "1-2-3", ""] {
            let mut j = job("Printer");
            j.range = Some(range.into());
            assert!(lp_arguments(&j, &path).is_err(), "range={range:?}");
        }
        for range in ["1", "2-5", "1,4,7-9", "12"] {
            let mut j = job("Printer");
            j.range = Some(range.into());
            assert!(lp_arguments(&j, &path).is_ok(), "range={range:?}");
        }
    }

    #[test]
    fn refuses_to_print_a_file_it_did_not_prepare() {
        // Otherwise "print this" is "read any file on disk to a printer".
        let mut j = job("Printer");
        j.copies = 1;
        for path in [
            PathBuf::from("/etc/passwd"),
            std::env::temp_dir().join("someone-elses.pdf"),
            PathBuf::from("/tmp/speedyf-print-1-0.pdf.txt"),
            PathBuf::from(""),
        ] {
            assert!(
                lp_arguments(&j, &path).is_err(),
                "should have refused {}",
                path.display()
            );
        }
    }

    #[test]
    fn exports_a_prepared_job_and_refuses_anything_else() {
        let source = temp_print_path();
        std::fs::write(&source, b"%PDF-1.7\n").expect("write");
        let dest = std::env::temp_dir().join("speedyf-export-test.pdf");

        export_to(&source.to_string_lossy(), &dest.to_string_lossy()).expect("export");
        assert_eq!(std::fs::read(&dest).expect("read"), b"%PDF-1.7\n");

        // Exporting is a read of the source, so the source must still be ours.
        assert!(export_to("/etc/passwd", &dest.to_string_lossy()).is_err());
        let _ = std::fs::remove_file(&dest);
        let _ = std::fs::remove_file(&source);
    }

    #[test]
    fn refuses_to_delete_a_file_it_did_not_prepare() {
        assert!(discard("/etc/passwd").is_err());
        assert!(discard(&std::env::temp_dir().join("notes.pdf").to_string_lossy()).is_err());
    }

    #[test]
    fn discards_its_own_print_job_and_tolerates_it_being_gone() {
        let path = temp_print_path();
        std::fs::write(&path, b"%PDF-1.7\n").expect("write");
        assert!(is_print_temp(&path));
        discard(&path.to_string_lossy()).expect("first discard");
        assert!(!path.exists());
        // Discarding twice is how cancel-then-close behaves; not an error.
        discard(&path.to_string_lossy()).expect("second discard");
    }

    #[test]
    fn every_temp_path_is_distinct() {
        let a = temp_print_path();
        let b = temp_print_path();
        assert_ne!(a, b, "two dialogs open at once must not share a file");
    }
}
