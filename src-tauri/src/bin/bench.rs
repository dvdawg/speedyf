//! SpeedyF benchmark harness (engine-level). See bench/README.md.

use serde::Serialize;
use speedyf_lib::engine::types::{
    DocMetaDto, EditPlan, PlanPage, Priority, RenderKey, RenderKind, SaveResultDto, TileRect,
};
use speedyf_lib::engine::{EngineHandle, Work};
use speedyf_lib::errors::AppError;
use std::collections::BTreeMap;
use std::env;
use std::fmt::Display;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CATEGORIES: [&str; 6] = [
    "text-1000p",
    "scanned-large",
    "cad-page",
    "image-100p",
    "malformed",
    "edited-save",
];
const MAX_PAGE_RENDER_SAMPLES: usize = 20;
const MAX_TILE_RENDER_SAMPLES: usize = 12;
const MAX_TEXT_PAGES: u32 = 1_000;
const STALE_EXERCISE_JOBS: usize = 12;

#[derive(Debug)]
struct Config {
    corpus_dir: PathBuf,
    results_dir: PathBuf,
}

#[derive(Debug)]
struct StageError {
    stage: &'static str,
    message: String,
}

impl StageError {
    fn new(stage: &'static str, error: impl Display) -> Self {
        StageError {
            stage,
            message: error.to_string(),
        }
    }
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchMetrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    file_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    page_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    open_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_page_render_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    page_render_samples: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    page_render_p50_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    page_render_p95_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tile_render_samples: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tile_render_p50_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tile_render_p95_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_pages_measured: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_extract_ms_per_page: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_search_result_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    search_pages_scanned: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    search_result_page: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_hits: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_lookups: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_hit_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    skipped_stale_tasks: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    save_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    save_engine_reported_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    save_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    peak_rss_before_save_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    peak_rss_after_save_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    peak_rss_delta_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_open_failure_code: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CategoryReport {
    category: &'static str,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metrics: Option<BenchMetrics>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    notes: Vec<String>,
}

impl CategoryReport {
    fn skipped(category: &'static str) -> Self {
        CategoryReport {
            category,
            status: "skipped",
            file: None,
            failure_stage: None,
            reason: Some("no matching local PDF in the corpus".into()),
            metrics: None,
            notes: Vec::new(),
        }
    }

    fn failed(category: &'static str, path: &Path, error: StageError) -> Self {
        CategoryReport {
            category,
            status: "failed",
            file: Some(path.display().to_string()),
            failure_stage: Some(error.stage.into()),
            reason: Some(error.message),
            metrics: None,
            notes: Vec::new(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HostInfo {
    os: &'static str,
    arch: &'static str,
    build_profile: &'static str,
    low_memory_mode: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchReport {
    schema_version: u8,
    generated_at_unix_ms: u64,
    corpus_dir: String,
    host: HostInfo,
    categories: Vec<CategoryReport>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("speedyf-bench: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let Some(config) = parse_args()? else {
        return Ok(());
    };
    fs::create_dir_all(&config.results_dir)
        .map_err(|error| format!("cannot create results directory: {error}"))?;

    let corpus = discover_corpus(&config.corpus_dir)?;
    let pdfium_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("pdfium");
    let engine = EngineHandle::start(vec![pdfium_dir], false);
    let mut reports = Vec::with_capacity(CATEGORIES.len());

    for category in CATEGORIES {
        let Some(paths) = corpus.get(category) else {
            reports.push(CategoryReport::skipped(category));
            continue;
        };
        let path = &paths[0];
        let mut report = if category == "malformed" {
            benchmark_malformed(&engine, category, path)
        } else {
            match benchmark_valid(&engine, category, path, &config.results_dir) {
                Ok((metrics, notes)) => CategoryReport {
                    category,
                    status: "measured",
                    file: Some(path.display().to_string()),
                    failure_stage: None,
                    reason: None,
                    metrics: Some(metrics),
                    notes,
                },
                Err(error) => CategoryReport::failed(category, path, error),
            }
        };
        if paths.len() > 1 {
            report.notes.push(format!(
                "{} matching PDFs found; benchmarked the first in lexical order",
                paths.len()
            ));
        }
        reports.push(report);
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock precedes Unix epoch: {error}"))?;
    let generated_at_unix_ms = now.as_millis() as u64;
    let output = config
        .results_dir
        .join(format!("{generated_at_unix_ms}.json"));
    let report = BenchReport {
        schema_version: 1,
        generated_at_unix_ms,
        corpus_dir: config.corpus_dir.display().to_string(),
        host: HostInfo {
            os: env::consts::OS,
            arch: env::consts::ARCH,
            build_profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            low_memory_mode: false,
        },
        categories: reports,
    };
    let json = serde_json::to_vec_pretty(&report)
        .map_err(|error| format!("cannot serialize benchmark report: {error}"))?;
    fs::write(&output, json)
        .map_err(|error| format!("cannot write {}: {error}", output.display()))?;
    println!("wrote {}", output.display());
    Ok(())
}

fn parse_args() -> Result<Option<Config>, String> {
    let project_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| "src-tauri has no project parent".to_string())?
        .to_path_buf();
    let mut config = Config {
        corpus_dir: project_dir.join("bench/corpus"),
        results_dir: project_dir.join("bench/results"),
    };
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--corpus" => {
                config.corpus_dir = PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--corpus requires a directory".to_string())?,
                );
            }
            "--results" => {
                config.results_dir = PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--results requires a directory".to_string())?,
                );
            }
            "-h" | "--help" => {
                println!(
                    "Usage: speedyf-bench [--corpus DIR] [--results DIR]\n\
                     Corpus filenames select categories; see bench/README.md."
                );
                return Ok(None);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Some(config))
}

fn discover_corpus(dir: &Path) -> Result<BTreeMap<&'static str, Vec<PathBuf>>, String> {
    let mut matches: BTreeMap<&'static str, Vec<PathBuf>> = BTreeMap::new();
    let entries =
        fs::read_dir(dir).map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read corpus entry: {error}"))?;
        let path = entry.path();
        if !path.is_file()
            || !path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
        {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if let Some(category) = select_category(file_name) {
            matches.entry(category).or_default().push(path);
        }
    }
    for paths in matches.values_mut() {
        paths.sort();
    }
    Ok(matches)
}

fn select_category(file_name: &str) -> Option<&'static str> {
    let name = file_name.to_ascii_lowercase();
    CATEGORIES
        .iter()
        .copied()
        .find(|category| name.contains(category))
}

fn benchmark_malformed(
    engine: &EngineHandle,
    category: &'static str,
    path: &Path,
) -> CategoryReport {
    let started = Instant::now();
    let result = open_document(engine, path);
    let open_ms = elapsed_ms(started);
    match result {
        Err(AppError::Malformed(error)) => CategoryReport {
            category,
            status: "measured",
            file: Some(path.display().to_string()),
            failure_stage: None,
            reason: None,
            metrics: Some(BenchMetrics {
                file_bytes: fs::metadata(path).ok().map(|metadata| metadata.len()),
                open_ms: Some(open_ms),
                expected_open_failure_code: Some("malformed".into()),
                ..BenchMetrics::default()
            }),
            notes: vec![format!("expected open rejection: {error}")],
        },
        Err(error) => CategoryReport::failed(
            category,
            path,
            StageError::new("open", format!("unexpected error class: {error}")),
        ),
        Ok(meta) => {
            close_document(engine, meta.doc_id);
            CategoryReport::failed(
                category,
                path,
                StageError::new("open", "malformed corpus PDF unexpectedly opened"),
            )
        }
    }
}

fn benchmark_valid(
    engine: &EngineHandle,
    category: &'static str,
    path: &Path,
    results_dir: &Path,
) -> Result<(BenchMetrics, Vec<String>), StageError> {
    let mut metrics = BenchMetrics {
        file_bytes: fs::metadata(path).ok().map(|metadata| metadata.len()),
        ..BenchMetrics::default()
    };
    let mut notes = Vec::new();

    let open_started = Instant::now();
    let meta = open_document(engine, path).map_err(|error| StageError::new("open", error))?;
    metrics.open_ms = Some(elapsed_ms(open_started));
    metrics.page_count = Some(meta.page_count);
    let doc = meta.doc_id;

    let interactive_result = (|| {
        let first_key = RenderKey {
            doc,
            src: 0,
            rot: 0,
            scale_milli: 1_500,
            kind: RenderKind::Page,
            tile: None,
        };
        let started = Instant::now();
        render(engine, first_key, Priority::VisiblePage)
            .map_err(|error| StageError::new("first-page-render", error))?;
        metrics.first_page_render_ms = Some(elapsed_ms(started));

        let renderable_pages = meta.page_count.min(u16::MAX as u32 + 1) as usize;
        if meta.page_count as usize > renderable_pages {
            notes.push("render samples limited to the first 65,536 source pages".into());
        }
        let page_keys: Vec<RenderKey> = sample_indices(renderable_pages, MAX_PAGE_RENDER_SAMPLES)
            .into_iter()
            .map(|src| RenderKey {
                doc,
                src: src as u16,
                rot: 0,
                scale_milli: 1_000,
                kind: RenderKind::Page,
                tile: None,
            })
            .collect();
        let page_times = measure_renders(engine, &page_keys, Priority::AdjacentPage)
            .map_err(|error| StageError::new("page-render-samples", error))?;
        metrics.page_render_samples = Some(page_times.len());
        metrics.page_render_p50_ms = percentile(&page_times, 0.50);
        metrics.page_render_p95_ms = percentile(&page_times, 0.95);

        let tile_keys: Vec<RenderKey> = sample_indices(renderable_pages, MAX_TILE_RENDER_SAMPLES)
            .into_iter()
            .map(|src| RenderKey {
                doc,
                src: src as u16,
                rot: 0,
                scale_milli: 2_500,
                kind: RenderKind::Tile,
                tile: Some(TileRect {
                    x: 0,
                    y: 0,
                    w: 768,
                    h: 768,
                }),
            })
            .collect();
        let tile_times = measure_renders(engine, &tile_keys, Priority::VisibleTile)
            .map_err(|error| StageError::new("tile-render-samples", error))?;
        metrics.tile_render_samples = Some(tile_times.len());
        metrics.tile_render_p50_ms = percentile(&tile_times, 0.50);
        metrics.tile_render_p95_ms = percentile(&tile_times, 0.95);

        let cache_before = engine.metrics_snapshot();
        for key in page_keys.iter().take(4).cloned() {
            render(engine, key, Priority::VisiblePage)
                .map_err(|error| StageError::new("cache-replay", error))?;
        }
        for key in tile_keys.iter().take(4).cloned() {
            render(engine, key, Priority::VisibleTile)
                .map_err(|error| StageError::new("cache-replay", error))?;
        }
        let cache_after = engine.metrics_snapshot();
        let hits = cache_after
            .cache_hits
            .saturating_sub(cache_before.cache_hits);
        let lookups = cache_after
            .cache_lookups
            .saturating_sub(cache_before.cache_lookups);
        metrics.cache_hits = Some(hits);
        metrics.cache_lookups = Some(lookups);
        metrics.cache_hit_rate = (lookups > 0).then_some(hits as f64 / lookups as f64);

        metrics.skipped_stale_tasks = Some(
            exercise_stale_jobs(engine, &meta)
                .map_err(|error| StageError::new("stale-task-exercise", error))?,
        );

        match read_search_query(path) {
            Ok(Some(query)) => {
                let started = Instant::now();
                let max_pages = meta.page_count.min(u16::MAX as u32 + 1);
                let mut found_page = None;
                let mut scanned = 0;
                for src in 0..max_pages {
                    extract_text(engine, doc, src as u16)
                        .map_err(|error| StageError::new("first-search-result", error))?;
                    scanned += 1;
                    if let Some(result) = engine
                        .search_indexed(doc, &query, false, 100)
                        .into_iter()
                        .find(|page| !page.matches.is_empty())
                    {
                        found_page = Some(result.src);
                        break;
                    }
                }
                metrics.search_pages_scanned = Some(scanned);
                metrics.search_result_page = found_page;
                if found_page.is_some() {
                    metrics.first_search_result_ms = Some(elapsed_ms(started));
                } else {
                    notes.push(format!(
                        "search query {query:?} produced no result after {scanned} pages"
                    ));
                }
            }
            Ok(None) => notes.push(
                "first-search-result skipped: add a same-stem .query sidecar with a known term"
                    .into(),
            ),
            Err(error) => return Err(StageError::new("search-query-sidecar", error)),
        }
        Ok(())
    })();
    close_document(engine, doc);
    interactive_result?;

    // Reopen so text-extraction samples do not hit pages populated by the
    // first-result search measurement.
    let fresh_meta =
        open_document(engine, path).map_err(|error| StageError::new("text-reopen", error))?;
    let fresh_doc = fresh_meta.doc_id;
    let extraction_and_save_result = (|| {
        let text_page_count = fresh_meta
            .page_count
            .min(MAX_TEXT_PAGES)
            .min(u16::MAX as u32 + 1);
        let mut total_text_ms = 0.0;
        for src in 0..text_page_count {
            let started = Instant::now();
            extract_text(engine, fresh_doc, src as u16)
                .map_err(|error| StageError::new("text-extract", error))?;
            total_text_ms += elapsed_ms(started);
        }
        metrics.text_pages_measured = Some(text_page_count);
        metrics.text_extract_ms_per_page =
            (text_page_count > 0).then_some(total_text_ms / text_page_count as f64);

        if category == "edited-save" {
            let sizes = collect_page_sizes(engine, &fresh_meta)
                .map_err(|error| StageError::new("save-page-sizes", error))?;
            if fresh_meta.page_count > u16::MAX as u32 {
                return Err(StageError::new(
                    "save",
                    "EditPlan save is limited to 65,535 output pages",
                ));
            }
            let pages = sizes
                .into_iter()
                .enumerate()
                .map(|(src, size)| PlanPage {
                    src_index: Some(src as u16),
                    width_pt: size[0],
                    height_pt: size[1],
                    rotation: 0,
                    annots: Vec::new(),
                    texts: Vec::new(),
                    images: Vec::new(),
                })
                .collect();
            let plan = EditPlan {
                pages,
                form: Vec::new(),
            };
            let save_dir = tempfile::Builder::new()
                .prefix(".speedyf-bench-save-")
                .tempdir_in(results_dir)
                .map_err(|error| StageError::new("save-tempdir", error))?;
            let dest = save_dir.path().join("output.pdf");
            let rss_before = peak_rss_bytes();
            let started = Instant::now();
            let save_result: SaveResultDto = engine
                .call(Priority::VisiblePage, fresh_doc, |respond| Work::Save {
                    doc: fresh_doc,
                    plan,
                    dest,
                    respond,
                })
                .map_err(|error| StageError::new("save", error))?;
            let save_ms = elapsed_ms(started);
            let rss_after = peak_rss_bytes();
            metrics.save_ms = Some(save_ms);
            metrics.save_engine_reported_ms = Some(save_result.duration_ms);
            metrics.save_bytes = Some(save_result.bytes);
            metrics.peak_rss_before_save_bytes = rss_before;
            metrics.peak_rss_after_save_bytes = rss_after;
            metrics.peak_rss_delta_bytes = rss_before
                .zip(rss_after)
                .map(|(before, after)| after.saturating_sub(before));
        }
        Ok(())
    })();
    close_document(engine, fresh_doc);
    extraction_and_save_result?;

    Ok((metrics, notes))
}

fn open_document(engine: &EngineHandle, path: &Path) -> Result<DocMetaDto, AppError> {
    let path = path.to_string_lossy().into_owned();
    engine.call(Priority::VisiblePage, 0, |respond| Work::Open {
        path,
        password: None,
        respond,
    })
}

fn close_document(engine: &EngineHandle, doc: u32) {
    engine.submit(Priority::VisiblePage, doc, Work::Close { doc });
}

fn render(
    engine: &EngineHandle,
    key: RenderKey,
    priority: Priority,
) -> Result<Arc<Vec<u8>>, AppError> {
    let doc = key.doc;
    engine.call(priority, doc, |respond| Work::Render { key, respond })
}

fn measure_renders(
    engine: &EngineHandle,
    keys: &[RenderKey],
    priority: Priority,
) -> Result<Vec<f64>, AppError> {
    let mut samples = Vec::with_capacity(keys.len());
    for key in keys {
        let started = Instant::now();
        render(engine, key.clone(), priority)?;
        samples.push(elapsed_ms(started));
    }
    Ok(samples)
}

fn extract_text(engine: &EngineHandle, doc: u32, src: u16) -> Result<(), AppError> {
    engine
        .call(Priority::TextExtract, doc, |respond| Work::TextLayout {
            doc,
            src,
            respond,
        })
        .map(|_| ())
}

fn exercise_stale_jobs(engine: &EngineHandle, meta: &DocMetaDto) -> Result<u64, String> {
    let before = engine.metrics_snapshot().skipped_stale;
    let generation = engine.current_generation(meta.doc_id);
    let (tx, rx) = crossbeam_channel::bounded(STALE_EXERCISE_JOBS);
    for index in 0..STALE_EXERCISE_JOBS {
        let tx = tx.clone();
        let src = (index % meta.page_count as usize) as u16;
        let key = RenderKey {
            doc: meta.doc_id,
            src,
            rot: 0,
            scale_milli: 1_101 + index as u32,
            kind: RenderKind::Page,
            tile: None,
        };
        engine.submit_at_gen(
            Priority::Prefetch,
            meta.doc_id,
            generation,
            Work::Render {
                key,
                respond: Box::new(move |result| {
                    let _ = tx.send(result.map(|_| ()));
                }),
            },
        );
    }
    drop(tx);
    engine.bump_generation(meta.doc_id);
    for _ in 0..STALE_EXERCISE_JOBS {
        let _outcome = rx
            .recv_timeout(Duration::from_secs(30))
            .map_err(|error| format!("timed out draining stale jobs: {error}"))?;
    }
    let after = engine.metrics_snapshot().skipped_stale;
    Ok(after.saturating_sub(before))
}

fn collect_page_sizes(engine: &EngineHandle, meta: &DocMetaDto) -> Result<Vec<[f32; 2]>, AppError> {
    let mut sizes: Vec<[f32; 2]> = meta.sizes.iter().map(|size| [size[0], size[1]]).collect();
    while sizes.len() < meta.page_count as usize {
        let from = sizes.len() as u32;
        let page_sizes = engine.call(Priority::Prefetch, meta.doc_id, |respond| Work::Sizes {
            doc: meta.doc_id,
            from,
            count: 64,
            respond,
        })?;
        if page_sizes.sizes.is_empty() {
            break;
        }
        sizes.extend(page_sizes.sizes.into_iter().map(|size| [size[0], size[1]]));
    }
    if sizes.len() != meta.page_count as usize {
        return Err(AppError::Internal(format!(
            "received {} sizes for {} pages",
            sizes.len(),
            meta.page_count
        )));
    }
    Ok(sizes)
}

fn read_search_query(path: &Path) -> Result<Option<String>, std::io::Error> {
    let sidecar = path.with_extension("query");
    match fs::read_to_string(sidecar) {
        Ok(contents) => Ok(contents
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_owned)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn sample_indices(total: usize, limit: usize) -> Vec<usize> {
    if total == 0 || limit == 0 {
        return Vec::new();
    }
    if total <= limit {
        return (0..total).collect();
    }
    if limit == 1 {
        return vec![0];
    }
    (0..limit)
        .map(|index| (index as f64 * (total - 1) as f64 / (limit - 1) as f64).round() as usize)
        .collect()
}

fn percentile(samples: &[f64], quantile: f64) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let position = quantile.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    let fraction = position - lower as f64;
    let value = sorted[lower] + (sorted[upper] - sorted[lower]) * fraction;
    Some((value * 1_000_000.0).round() / 1_000_000.0)
}

fn elapsed_ms(started: Instant) -> f64 {
    let value = started.elapsed().as_secs_f64() * 1_000.0;
    (value * 1_000_000.0).round() / 1_000_000.0
}

#[cfg(unix)]
fn peak_rss_bytes() -> Option<u64> {
    // SAFETY: getrusage initializes the provided plain-old-data struct.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        Some(usage.ru_maxrss as u64)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Some((usage.ru_maxrss as u64).saturating_mul(1_024))
    }
}

#[cfg(not(unix))]
fn peak_rss_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_matching_is_case_insensitive_and_specific() {
        assert_eq!(
            select_category("TEXT-1000P-contract.pdf"),
            Some("text-1000p")
        );
        assert_eq!(
            select_category("customer-scanned-large-01.pdf"),
            Some("scanned-large")
        );
        assert_eq!(select_category("cad-page-a0.pdf"), Some("cad-page"));
        assert_eq!(select_category("image-100p-photo.pdf"), Some("image-100p"));
        assert_eq!(
            select_category("malformed-truncated.pdf"),
            Some("malformed")
        );
        assert_eq!(
            select_category("edited-save-smoke.pdf"),
            Some("edited-save")
        );
        assert_eq!(select_category("ordinary.pdf"), None);
    }

    #[test]
    fn percentile_interpolates_without_mutating_the_input() {
        let samples = vec![4.0, 1.0, 3.0, 2.0];
        assert_eq!(percentile(&samples, 0.5), Some(2.5));
        assert_eq!(percentile(&samples, 0.95), Some(3.85));
        assert_eq!(samples, vec![4.0, 1.0, 3.0, 2.0]);
        assert_eq!(percentile(&[], 0.5), None);
    }

    #[test]
    fn sample_indices_cover_both_ends_without_duplicates() {
        assert_eq!(sample_indices(1, 8), vec![0]);
        assert_eq!(sample_indices(5, 5), vec![0, 1, 2, 3, 4]);
        assert_eq!(sample_indices(100, 3), vec![0, 50, 99]);
    }
}
