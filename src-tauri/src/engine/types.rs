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
    AdjacentPage = 2,
    VisibleThumb = 3,
    NearThumb = 4,
    TextExtract = 5,
    Prefetch = 6,
    Idle = 7,
}

impl Priority {
    pub fn from_u8(v: u8) -> Priority {
        match v {
            0 => Priority::VisiblePage,
            1 => Priority::VisibleTile,
            2 => Priority::AdjacentPage,
            3 => Priority::VisibleThumb,
            4 => Priority::NearThumb,
            5 => Priority::TextExtract,
            6 => Priority::Prefetch,
            _ => Priority::Idle,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct TileRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RenderKind {
    Page,
    Thumb,
    Tile,
}

/// Cache key for one rendered artifact.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RenderKey {
    pub doc: DocId,
    pub src: u16,
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

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PageTextDto {
    pub src: u32,
    pub runs: Vec<TextRun>,
    pub char_count: u32,
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
    pub text_bytes: u64,
    pub text_budget: u64,
    pub pages_indexed: u64,
    pub queue_depth: u64,
}

// ---------- Edit plan (mirrors the TypeScript EditPlan exactly) ----------

#[derive(Deserialize, Clone, Copy, Debug)]
pub struct RectDto {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Deserialize, Clone, Copy, Debug)]
pub struct PointDto {
    pub x: f32,
    pub y: f32,
}

#[derive(Deserialize, Clone, Copy, Debug)]
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
    pub src_index: Option<u16>,
    pub width_pt: f32,
    pub height_pt: f32,
    /// extra rotation to add on top of the page's existing /Rotate
    pub rotation: u16,
    pub annots: Vec<PlanAnnot>,
    pub texts: Vec<PlanText>,
    pub images: Vec<PlanImage>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EditPlan {
    pub pages: Vec<PlanPage>,
    pub form: Vec<(String, String)>,
}
