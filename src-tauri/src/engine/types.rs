//! Engine DTOs shared between the worker, commands, protocol handler, and the
//! TypeScript frontend (all serde-serialized as camelCase JSON).

use serde::{Deserialize, Serialize};

pub type DocId = u32;

/// Lower value = more urgent. Mirrors the spec's priority ladder.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[repr(u8)]
pub enum Priority {
    VisiblePage = 0,
    VisibleTile = 1,
    HoverPreview = 2,
    AdjacentPage = 3,
    VisibleThumb = 4,
    NearThumb = 5,
    TextExtract = 6,
    Prefetch = 7,
    Idle = 8,
}

impl Priority {
    pub fn from_u8(v: u8) -> Priority {
        match v {
            0 => Priority::VisiblePage,
            1 => Priority::VisibleTile,
            2 => Priority::HoverPreview,
            3 => Priority::AdjacentPage,
            4 => Priority::VisibleThumb,
            5 => Priority::NearThumb,
            6 => Priority::TextExtract,
            7 => Priority::Prefetch,
            _ => Priority::Idle,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct TileRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum RenderKind {
    Page,
    Thumb,
    Tile,
    Preview,
}

/// Cache key for one rendered artifact.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RenderKey {
    pub doc: DocId,
    pub src: u32,
    /// extra rotation applied on top of the page's own /Rotate (0/90/180/270)
    pub rot: u16,
    pub scale_milli: u32,
    pub kind: RenderKind,
    pub tile: Option<TileRect>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DocMetaDto {
    pub doc_id: DocId,
    pub path: String,
    pub name: String,
    pub page_count: u32,
    /// `[w, h, cropX, cropY, baseRotation]` per page for the first N pages.
    /// Sizes are DISPLAY sizes (PDFium folds the page's /Rotate into them),
    /// so cropX/cropY/baseRotation are always reported as 0 here.
    pub sizes: Vec<[f32; 5]>,
    pub estimated_size: [f32; 2],
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PageSizesDto {
    pub from: u32,
    pub sizes: Vec<[f32; 5]>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TextRun {
    pub text: String,
    /// index of the run's first character in the page's raw character stream
    pub start: u32,
    /// display-normalized page space (points, origin bottom-left, y-up)
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// One row of the formal-environment list: either a section heading or an
/// environment sitting under it, flattened into document order so the panel
/// renders exactly like the table of contents.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FormalEntryDto {
    /// true for a section or subsection heading, false for an environment.
    /// Depth alone cannot tell them apart: an environment indents to however
    /// many headings are standing above it, so it can sit at any depth.
    pub heading: bool,
    /// nesting level: section 0, subsection 1, environments below them
    pub depth: u8,
    /// as printed: "2 Results", "Theorem 3.1"
    pub label: String,
    pub page: u32,
    /// display-space y of the anchor (points, y-up) for precise scrolling
    pub y: f32,
    /// index of the anchor's first character in the page's character stream,
    /// so a search hit on the same page can be placed relative to it
    pub char_index: u32,
    /// display-space x of the anchor, when the destination carried one. Only
    /// the figure crop reads it, to tell which column a caption sits in.
    pub x: Option<f32>,
}

/// One row of the figure panel: a caption, and a crop of the artwork it
/// belongs to, ready to request through the render protocol.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FigureDto {
    /// as printed: "Figure 3", "Table 1"
    pub label: String,
    /// the caption text after the label, with LaTeX scripts restored
    pub title: String,
    pub page: u32,
    /// caption anchor (display space, y-up) — where clicking the row goes
    pub y: f32,
    /// the artwork above the caption, in device pixels at `scale_milli`
    pub tile: TileRect,
    pub scale_milli: u32,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PageTextDto {
    pub src: u32,
    pub runs: Vec<TextRun>,
    pub char_count: u32,
}

/// Where a real PDF link points. Internal destinations are resolved while the
/// source page is enumerated, keeping hover-time work to a single crop request.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LinkTarget {
    Internal {
        page: u32,
        x: Option<f32>,
        y: Option<f32>,
    },
    Uri {
        uri: String,
        citation: Option<CitationIdDto>,
    },
    Unknown,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(tag = "scheme", content = "value", rename_all = "camelCase")]
pub enum CitationIdDto {
    Doi(String),
    ArXiv(String),
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LinkDto {
    /// Display-normalized page space, y-up: [x, y, w, h].
    pub rect: [f32; 4],
    pub target: LinkTarget,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PageLinksDto {
    pub src: u32,
    pub links: Vec<LinkDto>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSpecDto {
    pub doc_id: DocId,
    pub src: u32,
    pub tile: TileRect,
    pub scale_milli: u32,
    pub text: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedCitationDto {
    pub doc_id: DocId,
    pub title: Option<String>,
    pub page_count: u32,
    pub file_name: String,
    /// Canonical, root-contained path used by the normal guarded open flow.
    pub path: String,
    pub preview: PreviewSpecDto,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStatusDto {
    /// Every folder in the library. Was a single optional root.
    pub roots: Vec<String>,
    pub indexed: u32,
    pub total: u32,
    pub scanning: bool,
    /// Documents whose full text has been extracted for search. Separate from
    /// `indexed`, which counts metadata only — a document is findable by
    /// citation long before it is findable by its contents.
    pub text_indexed: u32,
    pub text_total: u32,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MatchDto {
    pub start: u32,
    pub len: u32,
    pub snippet: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PageMatchesDto {
    pub src: u32,
    pub matches: Vec<MatchDto>,
}

/// One document that matched a library-wide search.
///
/// Carries the file rather than a `DocId`: nothing is open when a library
/// search runs, and the point of the result is to decide what to open.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LibraryHitDto {
    pub path: String,
    pub name: String,
    /// first-page title heuristic from the library scan, when there is one
    pub title: Option<String>,
    pub total_matches: u32,
    pub pages: Vec<PageMatchesDto>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySearchDto {
    pub documents: Vec<LibraryHitDto>,
    /// true when the result set hit its cap, so the UI can say so rather than
    /// implying the library holds nothing more
    pub truncated: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SearchQueryDto {
    pub pages: Vec<PageMatchesDto>,
    /// True when the global/per-page safety bound omitted further matches.
    pub truncated: bool,
    pub limit: u32,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SearchStatusDto {
    pub indexed: u32,
    pub total: u32,
    pub truncated: bool,
    pub chars_indexed: u64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FormFieldDto {
    pub name: String,
    pub kind: String,
    pub value: String,
    pub page: u32,
    pub read_only: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OutlineNodeDto {
    pub title: String,
    /// destination page index, if the bookmark resolves to one (a GoTo dest
    /// or a GoTo action) — None for bookmarks pointing elsewhere (external
    /// URIs, unrecognized actions).
    pub page: Option<u32>,
    /// destination y in page space (points, y-up), when the destination
    /// specifies one — /Fit destinations do not
    pub y: Option<f32>,
    pub children: Vec<OutlineNodeDto>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SaveResultDto {
    pub path: String,
    pub bytes: u64,
    pub duration_ms: u64,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct EngineMetricsDto {
    pub rendered: u64,
    pub skipped_stale: u64,
    pub cache_hits: u64,
    pub cache_lookups: u64,
    pub page_cache_bytes: u64,
    pub page_cache_budget: u64,
    pub thumb_cache_bytes: u64,
    pub thumb_cache_budget: u64,
    pub preview_cache_bytes: u64,
    pub preview_cache_budget: u64,
    pub text_bytes: u64,
    pub text_budget: u64,
    pub pages_indexed: u64,
    pub queue_depth: u64,
}

// ---------- Edit plan (mirrors the TypeScript EditPlan exactly) ----------

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct RectDto {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct PointDto {
    pub x: f32,
    pub y: f32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct QuadDto {
    pub p1: PointDto,
    pub p2: PointDto,
    pub p3: PointDto,
    pub p4: PointDto,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlanAnnot {
    pub kind: String,
    pub rect: RectDto,
    pub color: String,
    pub opacity: f32,
    #[serde(default)]
    pub stroke_width: Option<f32>,
    #[serde(default)]
    pub quads: Option<Vec<QuadDto>>,
    #[serde(default)]
    pub strokes: Option<Vec<Vec<PointDto>>>,
    #[serde(default)]
    pub text: Option<String>,
}

/// One annotation read back out of a PDF.
///
/// Mirrors the TypeScript `Annotation` model, plus `index` — the annotation's
/// position in PDFium's enumeration of its source page. That index is the
/// identity the save path uses to decide which annotations to replace and
/// which to leave exactly as they were imported.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationDto {
    pub index: u32,
    pub kind: String,
    pub rect: RectDto,
    pub color: String,
    pub opacity: f32,
    pub stroke_width: Option<f32>,
    pub quads: Option<Vec<QuadDto>>,
    pub strokes: Option<Vec<Vec<PointDto>>>,
    pub text: Option<String>,
    pub font_size_pt: Option<f32>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PageAnnotationsDto {
    pub src: u32,
    pub annots: Vec<AnnotationDto>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlanText {
    pub rect: RectDto,
    pub text: String,
    pub font_size_pt: f32,
    pub color: String,
    pub opacity: f32,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlanImage {
    pub rect: RectDto,
    pub source_path: String,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlanPage {
    pub src_index: Option<u32>,
    pub width_pt: f32,
    pub height_pt: f32,
    /// extra rotation to add on top of the page's existing /Rotate
    pub rotation: u16,
    pub annots: Vec<PlanAnnot>,
    pub texts: Vec<PlanText>,
    pub images: Vec<PlanImage>,
    /// Annotation indices on the *source* page to drop before writing this
    /// page's own annotations.
    ///
    /// Only annotations SpeedyF owns and the user actually touched appear
    /// here. Anything untouched is left exactly as imported, because a
    /// highlight made in another tool carries an author, dates, a popup and an
    /// appearance stream that this model does not hold — rewriting it through
    /// our own shape would quietly throw all of that away.
    #[serde(default)]
    pub drop_src_annots: Vec<u32>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EditPlan {
    pub pages: Vec<PlanPage>,
    pub form: Vec<(String, String)>,
}
